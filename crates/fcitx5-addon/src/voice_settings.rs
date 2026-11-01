//! Frozen settings snapshot for the voice pipeline.
//!
//! Split out of [`crate::voice_state`] so the runtime state machine stays
//! within the repo per-file line budget. `settings_from_config` parses the
//! `[voice]` / `[llm.asr]` / `[llm]` config sections once at engine startup;
//! the worker thread only ever sees the resulting value type.

use ior_config as user_config;
use voice::asr::AsrSettings;
use voice::hotkey::{self, Hotkey};

/// `voice.state` codes shared with the C++ shell.
pub const STATE_IDLE: u32 = 0;
/// A capture process is streaming audio.
pub const STATE_RECORDING: u32 = 1;
/// Recording ended; the worker is uploading / polishing.
pub const STATE_TRANSCRIBING: u32 = 2;

/// Frozen voice settings handed to the worker thread (cloned on start).
#[derive(Debug, Clone)]
pub struct VoiceSettings {
    /// `voice.enabled`.
    pub enabled: bool,
    /// Parsed `voice.hotkey`.
    pub hotkey: Hotkey,
    /// Display name of the hotkey (for preedit hints).
    pub hotkey_label: String,
    /// True when `voice.hotkey_mode = "hold"`.
    pub hold_mode: bool,
    /// `voice.capture_command`.
    pub capture_command: String,
    /// `voice.sample_rate`.
    pub sample_rate: u32,
    /// `voice.max_duration_ms`.
    pub max_duration_ms: u32,
    /// `[llm.asr]` transcription endpoint.
    pub asr: AsrSettings,
    /// `[llm]` polish endpoint when `llm.enabled = true`.
    pub polish: Option<PolishSettings>,
}

/// `[llm]` polish endpoint (chat completions).
#[derive(Debug, Clone)]
pub struct PolishSettings {
    /// `http://host:port/v1/chat/completions`.
    pub endpoint: String,
    /// Model id.
    pub model: String,
    /// Environment variable name for the API key.
    pub api_key_env: String,
    /// Network timeout, milliseconds.
    pub timeout_ms: u64,
}

/// Copies the `[voice]` / `[llm.asr]` / `[llm]` sections into worker
/// settings. An unparsable hotkey is reported and replaced by the
/// default rather than silently disabling voice input.
pub fn settings_from_config(config: &user_config::Config) -> VoiceSettings {
    let fallback = Hotkey {
        keysym: hotkey::KEY_GRAVE,
        mods: hotkey::MOD_CTRL,
    };
    let hotkey = hotkey::parse_hotkey(&config.voice.hotkey).unwrap_or(fallback);
    if hotkey::parse_hotkey(&config.voice.hotkey).is_none() {
        eprintln!(
            "ior: voice.hotkey {:?} not recognized; using ctrl+grave",
            config.voice.hotkey
        );
    }
    VoiceSettings {
        enabled: config.voice.enabled,
        hotkey_label: hotkey::hotkey_name(hotkey),
        hotkey,
        hold_mode: config.voice.hotkey_mode == user_config::HotkeyMode::Hold,
        capture_command: config.voice.capture_command.clone(),
        sample_rate: config.voice.sample_rate,
        max_duration_ms: config.voice.max_duration_ms,
        asr: AsrSettings {
            endpoint: config.llm.asr.endpoint.clone(),
            model: config.llm.asr.model.clone(),
            api_key_env: config.llm.asr.api_key_env.clone(),
            language: config.llm.asr.language.clone(),
            timeout_ms: u64::from(config.llm.asr.timeout_ms),
        },
        polish: config.llm.enabled.then(|| PolishSettings {
            endpoint: config.llm.endpoint.clone(),
            model: config.llm.model.clone(),
            api_key_env: config.llm.api_key_env.clone(),
            timeout_ms: 10_000,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ior_config::Config;

    #[test]
    fn settings_map_the_config_sections() {
        let settings = settings_from_config(&Config::default());
        assert!(settings.enabled);
        assert_eq!(
            settings.hotkey,
            Hotkey {
                keysym: hotkey::KEY_GRAVE,
                mods: hotkey::MOD_CTRL
            }
        );
        assert_eq!(settings.hotkey_label, "ctrl+`");
        assert!(!settings.hold_mode);
        assert_eq!(settings.asr.model, "Qwen/Qwen3-ASR-4B");
        assert!(settings.polish.is_none(), "llm.enabled defaults false");
    }
}
