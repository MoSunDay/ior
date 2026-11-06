//! ASR engine selection: a pure data enum plus a fallback chain executor.
//!
//! No trait objects and no resident state: the addon freezes the user
//! policy into `Vec<EngineConfig>` once at startup, then every utterance
//! walks that chain. The local SenseVoice engine is inserted first when
//! its CLI and model are present; the remote node02 endpoint stays the
//! compatibility fallback. A future GPU engine slots in as another enum
//! arm *before* `SenseVoice` in [`resolve_chain`] — no ABI change.

use std::fs;
use std::path::Path;

use crate::asr::{self, AsrSettings};
use crate::capture;
use crate::sensevoice::{self, SenseVoiceConfig, SenseVoiceSettings};

/// `voice.asr.engine`: which engine(s) to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EnginePolicy {
    /// Probe and prefer local SenseVoice, then fall back remotely.
    #[default]
    Auto,
    /// Prefer local SenseVoice.
    SenseVoice,
    /// Only the OpenAI-compatible remote endpoint.
    Remote,
}

impl EnginePolicy {
    /// Canonical TOML string.
    pub fn as_str(self) -> &'static str {
        match self {
            EnginePolicy::Auto => "auto",
            EnginePolicy::SenseVoice => "sensevoice",
            EnginePolicy::Remote => "remote",
        }
    }

    /// Parses `"auto"` | `"sensevoice"` | `"remote"`.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "auto" => Some(EnginePolicy::Auto),
            "sensevoice" => Some(EnginePolicy::SenseVoice),
            "remote" => Some(EnginePolicy::Remote),
            _ => None,
        }
    }
}

/// One frozen engine. Variants carry the exact settings the worker needs,
/// so the worker never re-reads config or re-probes the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineConfig {
    /// Local sherpa-onnx CLI process (CPU int8).
    SenseVoice(SenseVoiceSettings),
    /// OpenAI-compatible remote endpoint (node02 Qwen3-ASR).
    Remote(AsrSettings),
}

impl EngineConfig {
    /// Short journal label.
    pub fn label(&self) -> &'static str {
        match self {
            EngineConfig::SenseVoice(_) => "sensevoice",
            EngineConfig::Remote(_) => "remote",
        }
    }
}

/// Everything [`resolve_chain`] needs; built from `[voice.asr]` plus the
/// existing `[llm.asr]` remote settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineOptions {
    /// User policy.
    pub policy: EnginePolicy,
    /// Degrade to the next engine on failure.
    pub fallback: bool,
    /// Raw local-engine knobs.
    pub sensevoice: SenseVoiceConfig,
    /// Remote fallback endpoint.
    pub remote: AsrSettings,
}

/// Resolves policy + host probes into an ordered engine chain. `auto`
/// without a local model/binary is remote-only, so existing installs see
/// no behaviour change. `fallback = false` keeps only the preferred
/// engine when it is available (and yields an empty chain otherwise).
pub fn resolve_chain(options: &EngineOptions) -> Vec<EngineConfig> {
    let local = sensevoice::resolve(&options.sensevoice);
    match options.policy {
        EnginePolicy::Remote => vec![EngineConfig::Remote(options.remote.clone())],
        EnginePolicy::SenseVoice => match local {
            Some(local) => with_optional_remote(local, options),
            None if options.fallback => vec![EngineConfig::Remote(options.remote.clone())],
            None => Vec::new(),
        },
        EnginePolicy::Auto => match local {
            Some(local) => with_optional_remote(local, options),
            None => vec![EngineConfig::Remote(options.remote.clone())],
        },
    }
}

fn with_optional_remote(local: SenseVoiceSettings, options: &EngineOptions) -> Vec<EngineConfig> {
    let mut chain = vec![EngineConfig::SenseVoice(local)];
    if options.fallback {
        chain.push(EngineConfig::Remote(options.remote.clone()));
    }
    chain
}

/// `"sensevoice -> remote"` for the startup journal.
pub fn chain_summary(chain: &[EngineConfig]) -> String {
    chain
        .iter()
        .map(EngineConfig::label)
        .collect::<Vec<_>>()
        .join(" -> ")
}

/// One successful transcription plus degradation lines for the journal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transcription {
    /// Raw engine transcript (trimmed; worker still applies t2s).
    pub text: String,
    /// Fallback diagnostics, e.g.
    /// `asr engine sensevoice: ...; fallback -> remote`.
    pub journal: Vec<String>,
}

/// Runs the chain in order. The first non-empty transcript wins. A failed
/// engine logs one journal line and the next engine is tried; if every
/// engine fails the returned error carries each attempt once.
pub fn transcribe_best(chain: &[EngineConfig], wav: &[u8]) -> Result<Transcription, String> {
    transcribe_best_inner(chain, wav, &|| false)
}

/// Like [`transcribe_best`], but checks `cancelled` before and after each
/// engine. A canceled utterance never falls through to the next engine; the
/// caller must also check before polish/commit.
pub fn transcribe_best_with_cancel<F: Fn() -> bool>(
    chain: &[EngineConfig],
    wav: &[u8],
    cancelled: &F,
) -> Result<Transcription, String> {
    transcribe_best_inner(chain, wav, cancelled)
}

fn transcribe_best_inner<F: Fn() -> bool>(
    chain: &[EngineConfig],
    wav: &[u8],
    cancelled: &F,
) -> Result<Transcription, String> {
    if chain.is_empty() {
        return Err("no ASR engine available (check voice.asr and run \
             scripts/download_models.sh --sensevoice)"
            .to_string());
    }
    let mut journal = Vec::new();
    let mut failures = Vec::new();
    for (idx, engine) in chain.iter().enumerate() {
        if cancelled() {
            return Err("cancelled".to_string());
        }
        let result = match engine {
            EngineConfig::SenseVoice(settings) => sensevoice::transcribe(settings, wav),
            EngineConfig::Remote(settings) => asr::transcribe(settings, wav),
        };
        if cancelled() {
            return Err("cancelled".to_string());
        }
        match result {
            Ok(text) if !text.trim().is_empty() => {
                return Ok(Transcription { text, journal });
            }
            Ok(_) => {
                let failure = format!("asr engine {}: empty transcript", engine.label());
                record_failure(failure, idx, chain, &mut journal, &mut failures);
            }
            Err(err) => {
                let failure = format!("asr engine {}: {err}", engine.label());
                record_failure(failure, idx, chain, &mut journal, &mut failures);
            }
        }
    }
    Err(format!("all ASR engines failed: {}", failures.join("; ")))
}

fn record_failure(
    failure: String,
    idx: usize,
    chain: &[EngineConfig],
    journal: &mut Vec<String>,
    failures: &mut Vec<String>,
) {
    match chain.get(idx + 1) {
        Some(next) => journal.push(format!("{failure}; fallback -> {}", next.label())),
        None => journal.push(failure.clone()),
    }
    failures.push(failure);
}

/// Best-effort GPU probe used for a startup journal hint today. Detection
/// is deliberately dependency-free: NVIDIA device nodes, the proc driver
/// version file, or `nvidia-smi` on `PATH`. When a GPU engine lands it is
/// inserted before SenseVoice in [`resolve_chain`].
pub fn detect_gpu() -> bool {
    gpu_device_present(Path::new("/dev"))
        || Path::new("/proc/driver/nvidia/version").is_file()
        || capture::find_in_path("nvidia-smi").is_some()
}

fn gpu_device_present(dir: &Path) -> bool {
    fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .any(|entry| entry.file_name().to_string_lossy().starts_with("nvidia"))
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests;
