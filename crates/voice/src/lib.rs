//! Voice input for ior (phase 2): local SenseVoice first, remote Qwen
//! fallback.
//!
//! Still zero third-party dependencies: capture shells out to the
//! system recorder (pw-record / parec / arecord), local ASR shells out
//! to `sherpa-onnx-offline`, HTTP is a hand-rolled client over
//! `std::net`, and the remote engine talks to the OpenAI-compatible
//! `/v1/audio/transcriptions` endpoint on node02.
//!
//! - [`hotkey`]: `"ctrl+grave"` to keysym + fcitx5-aligned modifier bits.
//! - [`capture`]: recorder process, raw-PCM read loop, WAV wrap/unwrap.
//! - [`sensevoice`]: optional local SenseVoice-Small CLI process.
//! - [`engine`]: frozen engine chain, fallback execution and GPU probe.
//! - [`http`]: http://-only POST client (multipart and JSON bodies).
//! - [`json`]: string escape/extract for the optional polish call.
//! - [`asr`]: remote transcription request assembly and response handling.
//!
//! Remaining roadmap (V2-V4): silero VAD gating, streaming bilingual
//! ASR (sherpa-onnx zipformer), batch re-decode (whisper-rs), and a
//! resident GPU engine slot ahead of the local CPU engine.

pub mod asr;
pub mod capture;
pub mod engine;
pub mod hotkey;
pub mod http;
pub mod json;
pub mod sensevoice;
mod temp_wav;

/// One downloadable model in the phase-2 voice manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelSpec {
    pub name: &'static str,
    pub role: &'static str,
    pub approx_size_mb: u32,
}

/// Planned pipeline capabilities, capture first, batch fallback last.
/// This roadmap list is not the runtime engine chain selected by
/// [`engine::resolve_chain`].
pub const STAGES: &[&str] = &[
    "capture(cpal/PipeWire)",
    "vad(silero-onnx)",
    "asr-stream(sherpa-onnx zipformer)",
    "asr-local(sherpa-onnx sensevoice)",
    "asr-remote(qwen3-asr)",
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
            name: "sensevoice-small",
            role: "asr-local",
            approx_size_mb: 230,
        },
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
            summary.contains("asr-local(sherpa-onnx sensevoice)"),
            "summary: {summary}"
        );
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
    fn manifest_has_five_models_with_unique_roles() {
        let manifest = default_model_manifest();
        assert_eq!(manifest.len(), 5);
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
