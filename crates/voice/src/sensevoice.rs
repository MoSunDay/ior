//! Local SenseVoice-Small ASR through the `sherpa-onnx-offline` CLI.
//! The model runs in a child process exactly like capture: no onnxruntime is
//! linked into fcitx5, preserving the zero-dependency invariant. A model load
//! per utterance costs a few hundred ms on CPU int8 (push-to-talk acceptable).

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use crate::capture;
use crate::json;
use crate::temp_wav;

/// The int8 model file expected directly inside the model directory.
pub const MODEL_FILE: &str = "model.int8.onnx";
/// The token table expected next to the model.
pub const TOKENS_FILE: &str = "tokens.txt";
/// Default `voice.asr.sensevoice_command`.
pub const DEFAULT_COMMAND: &str = "auto";
/// Default `voice.asr.sensevoice_model_dir`.
pub const DEFAULT_MODEL_DIR: &str = "auto";
/// Default `voice.asr.sensevoice_threads`.
pub const DEFAULT_THREADS: u32 = 2;
/// Default `voice.asr.timeout_ms` (one model load plus decode).
pub const DEFAULT_TIMEOUT_MS: u64 = 20_000;

const POLL_INTERVAL: Duration = Duration::from_millis(20);

/// Raw `[voice.asr]` knobs; the addon freezes them into settings once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SenseVoiceConfig {
    /// `"auto"` probes `sherpa-onnx-offline` on `PATH`; anything else is
    /// an argv split on spaces (same semantics as `capture_command`).
    pub command: String,
    /// `"auto"` probes the XDG data dir; anything else is a model dir.
    pub model_dir: String,
    /// `--num-threads` passed to the CLI.
    pub num_threads: u32,
    /// Wall-clock timeout for one CLI invocation, milliseconds.
    pub timeout_ms: u64,
}

impl Default for SenseVoiceConfig {
    fn default() -> Self {
        SenseVoiceConfig {
            command: DEFAULT_COMMAND.to_string(),
            model_dir: DEFAULT_MODEL_DIR.to_string(),
            num_threads: DEFAULT_THREADS,
            timeout_ms: DEFAULT_TIMEOUT_MS,
        }
    }
}

/// A probe-resolved local engine: concrete argv prefix and an existing
/// model directory containing [`MODEL_FILE`] and [`TOKENS_FILE`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SenseVoiceSettings {
    /// Argv prefix, e.g. `["/usr/bin/sherpa-onnx-offline"]`.
    pub argv: Vec<String>,
    /// Directory holding the model and tokens.
    pub model_dir: PathBuf,
    /// `--num-threads` value.
    pub num_threads: u32,
    /// Wall-clock timeout for one CLI invocation, milliseconds.
    pub timeout_ms: u64,
}

/// Resolves raw knobs into a runnable engine, or `None` when either the
/// command or the model directory cannot be found.
pub fn resolve(config: &SenseVoiceConfig) -> Option<SenseVoiceSettings> {
    let model_dir = probe_model_dir(&config.model_dir)?;
    let argv = resolve_command(&config.command)?;
    Some(SenseVoiceSettings {
        argv,
        model_dir,
        num_threads: config.num_threads,
        timeout_ms: config.timeout_ms,
    })
}

/// Resolves `voice.asr.sensevoice_model_dir`: an explicit path must exist
/// and contain both model files; `"auto"` tries
/// `$XDG_DATA_HOME/ior/models/sensevoice-small` then
/// `$HOME/.local/share/ior/models/sensevoice-small`.
pub fn probe_model_dir(explicit: &str) -> Option<PathBuf> {
    let trimmed = explicit.trim();
    if trimmed == "auto" {
        return auto_model_dirs()
            .into_iter()
            .find(|dir| has_model_files(dir));
    }
    let dir = PathBuf::from(trimmed);
    has_model_files(&dir).then_some(dir)
}

/// Candidate model directories in probe order. `$IOR_MODELS_DIR` is the
/// downloader's root override and wins, then `$XDG_DATA_HOME`, then `$HOME`.
fn auto_model_dirs() -> Vec<PathBuf> {
    auto_model_dirs_from(
        std::env::var_os("IOR_MODELS_DIR").map(PathBuf::from),
        std::env::var_os("XDG_DATA_HOME").map(PathBuf::from),
        std::env::var_os("HOME").map(PathBuf::from),
    )
}

fn auto_model_dirs_from(
    models_root: Option<PathBuf>,
    data_home: Option<PathBuf>,
    home: Option<PathBuf>,
) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(root) = models_root {
        if !root.as_os_str().is_empty() {
            dirs.push(root.join("sensevoice-small"));
        }
    }
    if let Some(data_home) = data_home {
        if !data_home.as_os_str().is_empty() {
            let dir = data_home.join("ior/models/sensevoice-small");
            if !dirs.contains(&dir) {
                dirs.push(dir);
            }
        }
    }
    if let Some(home) = home {
        if !home.as_os_str().is_empty() {
            let dir = home.join(".local/share/ior/models/sensevoice-small");
            if !dirs.contains(&dir) {
                dirs.push(dir);
            }
        }
    }
    dirs
}

fn has_model_files(dir: &Path) -> bool {
    dir.join(MODEL_FILE).is_file() && dir.join(TOKENS_FILE).is_file()
}

/// Resolves `voice.asr.sensevoice_command`: `"auto"` finds
/// `sherpa-onnx-offline` on `PATH`; anything else is an argv split on
/// spaces (tests may point it at a wrapper script).
pub fn resolve_command(command: &str) -> Option<Vec<String>> {
    let trimmed = command.trim();
    if trimmed == "auto" {
        return capture::find_in_path("sherpa-onnx-offline")
            .map(|path| vec![path.to_string_lossy().into_owned()]);
    }
    let argv: Vec<String> = trimmed.split_whitespace().map(String::from).collect();
    (!argv.is_empty()).then_some(argv)
}

/// Builds the exact CLI argv. Pure, so the flag order is pinned by a
/// byte-exact test.
pub fn build_argv(settings: &SenseVoiceSettings, wav_path: &Path) -> Vec<String> {
    let mut argv = settings.argv.clone();
    argv.push(format!(
        "--sense-voice-model={}",
        settings.model_dir.join(MODEL_FILE).display()
    ));
    argv.push(format!(
        "--tokens={}",
        settings.model_dir.join(TOKENS_FILE).display()
    ));
    argv.push(format!("--num-threads={}", settings.num_threads));
    argv.push(wav_path.to_string_lossy().into_owned());
    argv
}

/// Runs one transcription. The WAV is staged in a mode-0600,
/// process-unique temp file and removed best-effort afterwards; child
/// stdout/stderr are drained on helper threads so a long token list can
/// never deadlock the process while the timeout monitor waits.
pub fn transcribe(settings: &SenseVoiceSettings, wav: &[u8]) -> Result<String, String> {
    let wav_path = temp_wav::write(wav).map_err(|err| format!("sensevoice temp wav: {err}"))?;
    let result = run_offline(settings, &wav_path);
    let _ = fs::remove_file(&wav_path);
    result
}

fn run_offline(settings: &SenseVoiceSettings, wav_path: &Path) -> Result<String, String> {
    let argv = build_argv(settings, wav_path);
    let mut child = Command::new(&argv[0])
        .args(&argv[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| format!("sensevoice spawn {}: {err}", argv[0]))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "sensevoice missing stdout pipe".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "sensevoice missing stderr pipe".to_string())?;
    let stdout_reader = read_on_thread(stdout);
    let stderr_reader = read_on_thread(stderr);

    let deadline = Instant::now() + Duration::from_millis(settings.timeout_ms.max(1));
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            Ok(None) => std::thread::sleep(POLL_INTERVAL),
            Err(err) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("sensevoice wait: {err}"));
            }
        }
    };
    let Some(status) = status else {
        // Do not join the drain threads here: a non-exec wrapper can leave a
        // grandchild holding the pipes past the deadline. Detaching lets the
        // timeout return on time; the threads exit when the pipe closes.
        return Err(format!(
            "sensevoice timed out after {}ms",
            settings.timeout_ms
        ));
    };
    let stdout = stdout_reader.join().unwrap_or_default();
    let stderr = stderr_reader.join().unwrap_or_default();
    if !status.success() {
        return Err(format!(
            "sensevoice exit {}: {}",
            status_code(&status),
            journal_tail(&stderr)
        ));
    }
    let stdout = String::from_utf8_lossy(&stdout);
    parse_transcript(&stdout).ok_or_else(|| {
        format!(
            "sensevoice produced no transcript ({})",
            journal_tail(&stderr)
        )
    })
}

fn read_on_thread(mut stream: impl Read + Send + 'static) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut out = Vec::new();
        let _ = stream.read_to_end(&mut out);
        out
    })
}

fn status_code(status: &ExitStatus) -> String {
    status
        .code()
        .map_or_else(|| "signal".to_string(), |code| code.to_string())
}

/// Last non-empty stderr line, clipped for a one-line journal entry.
fn journal_tail(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let line = text
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("")
        .trim();
    let mut tail: String = line.chars().take(300).collect();
    if line.chars().count() > 300 {
        tail.push_str("...");
    }
    if tail.is_empty() {
        "no stderr".to_string()
    } else {
        tail
    }
}

/// Extracts the transcript from the CLI's last non-empty line. The CLI
/// prints a JSON object for SenseVoice (`"text": "..."`); older/plain
/// builds print the text directly or as `Text: ...`.
pub fn parse_transcript(stdout: &str) -> Option<String> {
    let line = stdout.lines().rev().find(|line| !line.trim().is_empty())?;
    let line = line.trim();
    if line.starts_with('{') {
        let text = json::extract_string_field(line, "text")?;
        let text = text.trim();
        return (!text.is_empty()).then(|| text.to_string());
    }
    for prefix in ["Text:", "text:", "TEXT:"] {
        if let Some(rest) = line.strip_prefix(prefix) {
            let rest = rest.trim();
            return (!rest.is_empty()).then(|| rest.to_string());
        }
    }
    (!line.is_empty()).then(|| line.to_string())
}

#[cfg(test)]
mod tests;
