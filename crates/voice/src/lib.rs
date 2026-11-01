//! Voice input for ior (phase 2, V1: hotkey + remote Qwen ASR).
//!
//! Still zero third-party dependencies: capture shells out to the
//! system recorder (pw-record / parec / arecord), HTTP is a hand-rolled
//! client over `std::net`, and ASR talks to the OpenAI-compatible
//! `/v1/audio/transcriptions` endpoint deployed on node02 (see
//! scripts/deploy-asr-node02.sh and docs/NODE02-DEPLOY.md).
//!
//! - [`hotkey`]: `"ctrl+grave"` to keysym + fcitx5-aligned modifier bits.
//! - [`capture`]: recorder process, raw-PCM read loop, WAV wrap/unwrap.
//! - [`http`]: http://-only POST client (multipart and JSON bodies).
//! - [`json`]: string escape/extract for the optional polish call.
//! - [`asr`]: transcription request assembly and text-response handling.
//!
//! Remaining roadmap (V2-V4): silero VAD gating, streaming bilingual
//! ASR (sherpa-onnx zipformer), batch re-decode (whisper-rs), and local
//! model management (see scripts/download_models.sh).

pub mod asr;
pub mod capture;
pub mod hotkey;
pub mod http;
pub mod json;

/// One downloadable model in the phase-2 voice manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelSpec {
    pub name: &'static str,
    pub role: &'static str,
    pub approx_size_mb: u32,
}

/// Planned pipeline stages, capture first, batch fallback last.
pub const STAGES: &[&str] = &[
    "capture(cpal/PipeWire)",
    "vad(silero-onnx)",
    "asr-stream(sherpa-onnx zipformer)",
    "asr-batch(whisper-rs)",
];

/// Default voice hotkey; configurable via `voice.hotkey` in
/// `fcitx5/ior/config.toml` (parsed by [`hotkey::parse_hotkey`]).
pub const PUSH_TO_TALK_DEFAULT_KEY: &str = "ctrl+grave";

/// Stage list joined with " -> " for docs and log lines.
pub fn pipeline_summary() -> String {
    STAGES.join(" -> ")
}

/// Models the voice pipeline plans to bundle or download in phase 2.
pub fn default_model_manifest() -> Vec<ModelSpec> {
    vec![
        ModelSpec {
            name: "silero-vad",
            role: "vad",
            approx_size_mb: 2,
        },
        ModelSpec {
            name: "sherpa-onnx-zipformer-zh-en",
            role: "asr-stream",
            approx_size_mb: 100,
        },
        ModelSpec {
            name: "whisper-small",
            role: "asr-batch-accurate",
            approx_size_mb: 460,
        },
        ModelSpec {
            name: "whisper-turbo",
            role: "asr-batch-fast",
            approx_size_mb: 750,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipeline_summary_lists_stages_in_order() {
        let summary = pipeline_summary();
        assert!(summary.contains("vad(silero-onnx)"), "summary: {summary}");
        assert!(
            summary.starts_with("capture(cpal/PipeWire)"),
            "summary: {summary}"
        );
        assert!(
            summary.ends_with("asr-batch(whisper-rs)"),
            "summary: {summary}"
        );
        assert_eq!(summary.matches(" -> ").count(), STAGES.len() - 1);
    }

    #[test]
    fn manifest_has_four_models_with_unique_roles() {
        let manifest = default_model_manifest();
        assert_eq!(manifest.len(), 4);
        let mut roles: Vec<&str> = manifest.iter().map(|spec| spec.role).collect();
        let total = roles.len();
        roles.sort_unstable();
        roles.dedup();
        assert_eq!(roles.len(), total, "roles must be unique: {manifest:?}");
        assert!(manifest.iter().all(|spec| !spec.name.is_empty()));
        assert!(manifest.iter().all(|spec| spec.approx_size_mb > 0));
    }

    #[test]
    fn push_to_talk_key_is_documented() {
        assert_eq!(PUSH_TO_TALK_DEFAULT_KEY, "ctrl+grave");
        assert!(hotkey::parse_hotkey(PUSH_TO_TALK_DEFAULT_KEY).is_some());
    }
}
