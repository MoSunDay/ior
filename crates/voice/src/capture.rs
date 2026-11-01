//! Microphone capture without dependencies: spawn a recorder command,
//! read s16le mono PCM from its stdout, and wrap it into a WAV for the
//! ASR upload. `pw-record` writes a WAV container, so streams are sniffed
//! for a RIFF header and stripped back to raw PCM before re-wrapping.

use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Stdio};

/// Mono recording, fixed by the pipeline.
pub const CHANNELS: u16 = 1;
/// 16-bit signed little-endian samples, fixed by the pipeline.
pub const BITS_PER_SAMPLE: u16 = 16;

/// Candidate argv for `capture_command = "auto"`, in probe order.
pub fn auto_commands(sample_rate: u32) -> Vec<Vec<String>> {
    let rate = sample_rate.to_string();
    vec![
        // PipeWire: WAV container on stdout (header stripped later).
        vec![
            "pw-record".into(),
            "--format=s16".into(),
            "--rate".into(),
            rate.clone(),
            "--channels=1".into(),
            "-".into(),
        ],
        // PulseAudio/PipeWire parec: raw PCM.
        vec![
            "parec".into(),
            "--raw".into(),
            "--format=s16le".into(),
            "--rate".into(),
            rate.clone(),
            "--channels=1".into(),
        ],
        // ALSA: raw PCM.
        vec![
            "arecord".into(),
            "-q".into(),
            "-t".into(),
            "raw".into(),
            "-f".into(),
            "S16_LE".into(),
            "-r".into(),
            rate,
            "-c".into(),
            "1".into(),
        ],
    ]
}

/// Resolves `voice.capture_command`: `"auto"` probes pw-record / parec /
/// arecord on `PATH`; anything else is an argv split on spaces (lets
/// tests stub capture with `cat fixture.wav`). `None` when `auto` finds
/// no recorder or the custom command is empty.
pub fn resolve_capture_command(command: &str, sample_rate: u32) -> Option<Vec<String>> {
    let trimmed = command.trim();
    if trimmed == "auto" {
        return auto_commands(sample_rate)
            .into_iter()
            .find(|argv| find_in_path(&argv[0]).is_some());
    }
    let argv: Vec<String> = trimmed.split_whitespace().map(String::from).collect();
    (!argv.is_empty()).then_some(argv)
}

/// First directory on `PATH` containing an executable `name`.
pub fn find_in_path(name: &str) -> Option<PathBuf> {
    if name.contains('/') {
        return is_executable(Path::new(name)).then(|| PathBuf::from(name));
    }
    std::env::var_os("PATH")?
        .to_string_lossy()
        .split(':')
        .filter(|dir| !dir.is_empty())
        .map(|dir| Path::new(dir).join(name))
        .find(|path| is_executable(path))
}

fn is_executable(path: &Path) -> bool {
    let Ok(meta) = fs::metadata(path) else {
        return false;
    };
    !meta.is_dir() && is_executable_mode(&meta)
}

#[cfg(unix)]
fn is_executable_mode(meta: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable_mode(_meta: &fs::Metadata) -> bool {
    false
}

/// A running capture process. Fields are public so worker loops can
/// stream-read stdout and kill the process on stop; [`Drop`] guarantees
/// the child is reaped even on early error returns.
pub struct Recorder {
    /// The capture process.
    pub child: Child,
    /// Its stdout pipe (raw audio stream).
    pub stdout: ChildStdout,
}

/// Spawns `argv` with stdout piped and stderr inherited (recorder
/// diagnostics go to the journal, not into the audio stream).
pub fn start(argv: &[String]) -> io::Result<Recorder> {
    let mut child = Command::new(&argv[0])
        .args(&argv[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?;
    let stdout = child.stdout.take().expect("stdout was piped");
    Ok(Recorder { child, stdout })
}

/// Stops a recorder: kill (it would never exit on its own) and reap.
pub fn stop(recorder: &mut Recorder) {
    let _ = recorder.child.kill();
    let _ = recorder.child.wait();
}

impl Drop for Recorder {
    fn drop(&mut self) {
        stop(self);
    }
}

/// Reads everything the recorder writes until EOF, `max_bytes`, or
/// `should_stop` returning true (silence included, so `auto` recorders
/// always produce chunks for the stop check to run).
pub fn read_all(
    recorder: &mut Recorder,
    max_bytes: usize,
    should_stop: Option<&dyn Fn() -> bool>,
) -> io::Result<Vec<u8>> {
    let mut pcm = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        if should_stop.is_some_and(|check| check()) {
            stop(recorder); // close the pipe; Drop would do it anyway
            break;
        }
        match recorder.stdout.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                pcm.extend_from_slice(&chunk[..n]);
                if pcm.len() >= max_bytes {
                    stop(recorder);
                    break;
                }
            }
            Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
            Err(err) => return Err(err),
        }
    }
    Ok(pcm)
}

/// Wraps raw s16le mono PCM into a canonical 44-byte WAV header.
pub fn wav_bytes(pcm: &[u8], sample_rate: u32) -> Vec<u8> {
    let data_len = pcm.len() as u32;
    let block_align = CHANNELS * BITS_PER_SAMPLE / 8;
    let byte_rate = sample_rate * u32::from(block_align);
    let mut out = Vec::with_capacity(44 + pcm.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&CHANNELS.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&BITS_PER_SAMPLE.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    out.extend_from_slice(pcm);
    out
}

/// Strips a canonical 44-byte WAV header (`pw-record` output); any other
/// stream is assumed raw PCM and passed through unchanged.
pub fn strip_wav_header(stream: &[u8]) -> Result<&[u8], String> {
    let is_wav = stream.len() >= 44
        && &stream[0..4] == b"RIFF"
        && &stream[8..12] == b"WAVE"
        && &stream[36..40] == b"data";
    if is_wav {
        Ok(&stream[44..])
    } else if stream.len() >= 12 && &stream[0..4] == b"RIFF" {
        Err("RIFF stream without a canonical data chunk at byte 36".to_string())
    } else {
        Ok(stream)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[test]
    fn auto_commands_carry_the_sample_rate() {
        let argv = auto_commands(16_000);
        assert_eq!(argv.len(), 3);
        assert!(argv[0].contains(&"-".to_string()), "pw-record to stdout");
        for candidate in argv {
            assert!(
                candidate.iter().any(|arg| arg == "16000"),
                "rate missing: {candidate:?}"
            );
        }
    }

    #[test]
    fn custom_capture_commands_split_on_spaces() {
        assert_eq!(
            resolve_capture_command("  cat /tmp/fixture.pcm  ", 16_000),
            Some(vec!["cat".to_string(), "/tmp/fixture.pcm".to_string()])
        );
        assert_eq!(resolve_capture_command("   ", 16_000), None);
    }

    #[test]
    fn wav_header_roundtrips() {
        let pcm = vec![1u8, 2, 3, 4, 5, 6, 7, 8];
        let wav = wav_bytes(&pcm, 16_000);
        assert_eq!(wav.len(), 44 + pcm.len());
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(&wav[36..40], b"data");
        // RIFF size, fmt fields, data size — all little-endian.
        assert_eq!(u32::from_le_bytes(wav[4..8].try_into().unwrap()), 36 + 8);
        assert_eq!(u16::from_le_bytes(wav[22..24].try_into().unwrap()), 1);
        assert_eq!(u32::from_le_bytes(wav[24..28].try_into().unwrap()), 16_000);
        assert_eq!(u16::from_le_bytes(wav[34..36].try_into().unwrap()), 16);
        assert_eq!(u32::from_le_bytes(wav[40..44].try_into().unwrap()), 8);
        assert_eq!(strip_wav_header(&wav).unwrap(), pcm.as_slice());
    }

    #[test]
    fn non_wav_streams_pass_through_untouched() {
        let raw = vec![9u8, 9, 9];
        assert_eq!(strip_wav_header(&raw).unwrap(), raw.as_slice());
        // RIFF without the canonical layout is an error, not a passthrough.
        let mut broken = vec![0u8; 44];
        broken[0..4].copy_from_slice(b"RIFF");
        assert!(strip_wav_header(&broken).is_err());
    }

    static SEQ: AtomicU32 = AtomicU32::new(0);

    fn unique_temp_path() -> PathBuf {
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "ior-voice-capture-{}-{seq}.pcm",
            std::process::id()
        ))
    }

    #[test]
    fn recorder_streams_a_stubbed_file_to_eof() {
        let path = unique_temp_path();
        let pcm: Vec<u8> = (0..1024u32).map(|n| n as u8).collect();
        fs::write(&path, &pcm).expect("write fixture");
        let argv = ["cat".to_string(), path.to_string_lossy().into_owned()];
        let mut recorder = start(&argv).expect("spawn cat");
        let stream = read_all(&mut recorder, usize::MAX, None).expect("read stdout");
        assert_eq!(stream, pcm);
        drop(recorder); // kills + reaps (cat already exited)

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn read_all_honours_the_stop_check() {
        let path = unique_temp_path();
        fs::write(&path, b"").expect("touch fixture");
        // yes(1) never EOFs: only the stop check (or the cap) can end it.
        let argv = ["yes".to_string(), "ior".to_string()];
        let mut recorder = start(&argv).expect("spawn yes");
        let stream = read_all(&mut recorder, usize::MAX, Some(&|| true)).expect("read");
        assert!(stream.is_empty(), "stopped before any read");
        drop(recorder);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn read_all_stops_at_the_byte_cap() {
        let path = unique_temp_path();
        let pcm = vec![0xABu8; 4096];
        fs::write(&path, &pcm).expect("write fixture");
        let argv = ["cat".to_string(), path.to_string_lossy().into_owned()];
        let mut recorder = start(&argv).expect("spawn cat");
        let stream = read_all(&mut recorder, 1024, None).expect("read stdout");
        assert!(stream.len() >= 1024 && stream.len() < 4096 + 8192);
        drop(recorder);

        let _ = fs::remove_file(&path);
    }
}
