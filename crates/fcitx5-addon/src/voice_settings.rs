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
/// settings. `hotkey = "none"` (or any unparsable spec) leaves the
/// hotkey dead (`keysym 0`) after a journal diagnostic — falling back to
/// the default ctrl+grave made voice impossible to turn off by config.
pub fn settings_from_config(config: &user_config::Config) -> VoiceSettings {
    let spec = config.voice.hotkey.trim();
    let disabled = Hotkey { keysym: 0, mods: 0 };
    let hotkey = if spec.eq_ignore_ascii_case("none") {
        disabled
    } else {
        match hotkey::parse_hotkey(spec) {
            Some(hotkey) => hotkey,
            None => {
                eprintln!(
                    "ior: voice.hotkey {:?} not recognized; hotkey disabled \
                     (use e.g. \"ctrl+grave\", or \"none\" to turn it off)",
                    config.voice.hotkey
                );
                disabled
            }
        }
    };
    VoiceSettings {
        enabled: config.voice.enabled,
        hotkey_label: if hotkey.keysym == 0 {
            "none".to_string()
        } else {
            hotkey::hotkey_name(hotkey)
        },
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

    fn settings_with_hotkey(spec: &str) -> VoiceSettings {
        let mut config = Config::default();
        config.voice.hotkey = spec.to_string();
        settings_from_config(&config)
    }

    #[test]
    fn hotkey_none_disables_the_hotkey() {
        for spec in ["none", "None", " NONE "] {
            let settings = settings_with_hotkey(spec);
            assert_eq!(settings.hotkey.keysym, 0, "{spec:?}");
            assert_eq!(settings.hotkey.mods, 0, "{spec:?}");
            assert_eq!(settings.hotkey_label, "none", "{spec:?}");
            assert!(settings.enabled, "enabled stays a separate switch");
        }
    }

    #[test]
    fn unparsable_hotkey_disables_rather_than_reenabling_the_default() {
        // A typo used to silently resurrect ctrl+grave — voice could not
        // be turned off via the hotkey setting (regression pin).
        let settings = settings_with_hotkey("off");
        assert_eq!(settings.hotkey.keysym, 0);
        assert_eq!(settings.hotkey_label, "none");
    }

    #[test]
    fn parsed_hotkey_survives_the_none_path() {
        let settings = settings_with_hotkey("ctrl+shift+u");
        assert_eq!(settings.hotkey.mods, hotkey::MOD_CTRL | hotkey::MOD_SHIFT);
        assert_eq!(settings.hotkey_label, "ctrl+shift+u");
    }
}
