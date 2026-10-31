//! Voice input planning surface for ior (phase 2 placeholder).
//!
//! M0 keeps this crate dependency-free: it only names the planned pipeline
//! stages, the model manifest, and the push-to-talk hotkey so that docs,
//! scripts, and the addon can refer to stable identifiers. No audio,
//! ONNX, or ASR code lives here yet.
//!
//! Phase-2 roadmap (V1-V4):
//! - V1: push-to-talk capture (cpal/PipeWire) gated by silero VAD.
//! - V2: streaming bilingual ASR (sherpa-onnx zipformer) for live text.
//! - V3: batch re-decode (whisper-rs) for accuracy after each utterance.
//! - V4: model download/management (see scripts/download_models.sh) and
//!   addon wiring plus latency tuning.

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

/// Planned push-to-talk hotkey (configurable in the final addon).
pub const PUSH_TO_TALK_DEFAULT_KEY: &str = "F4";

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
        assert_eq!(PUSH_TO_TALK_DEFAULT_KEY, "F4");
    }
}
