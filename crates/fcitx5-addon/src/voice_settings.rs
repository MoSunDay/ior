//! Frozen settings snapshot for the voice pipeline.
//!
//! Split out of [`crate::voice_state`] so the runtime state machine stays
//! within the repo per-file line budget. `settings_from_config` parses the
//! `[voice]` / `[voice.asr]` / `[llm.asr]` / `[llm]` config sections once
//! at engine startup; the worker thread only ever sees the resulting value
//! type, including the already-probed ASR engine chain.

use ior_config as user_config;
use voice::asr::AsrSettings;
use voice::engine::{self, EngineConfig, EngineOptions, EnginePolicy};
use voice::hotkey::{self, Hotkey};
use voice::sensevoice::SenseVoiceConfig;

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
    /// `voice.live`: surface provisional transcripts while recording.
    pub live: bool,
    /// `voice.live_interval_ms` between live partial transcriptions.
    pub live_interval_ms: u32,
    /// `[voice.asr]`: resolved engine chain, preferred engine first.
    pub asr: Vec<EngineConfig>,
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

/// Copies the `[voice]` / `[voice.asr]` / `[llm.asr]` / `[llm]` sections
/// into worker settings. `hotkey = "none"` (or any unparsable spec)
/// leaves the hotkey dead (`keysym 0`) after a journal diagnostic.
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
    let remote = AsrSettings {
        endpoint: config.llm.asr.endpoint.clone(),
        model: config.llm.asr.model.clone(),
        api_key_env: config.llm.asr.api_key_env.clone(),
        language: config.llm.asr.language.clone(),
        timeout_ms: u64::from(config.llm.asr.timeout_ms),
    };
    let options = EngineOptions {
        policy: engine_policy(config.voice.asr.engine),
        fallback: config.voice.asr.fallback,
        sensevoice: SenseVoiceConfig {
            command: config.voice.asr.sensevoice_command.clone(),
            model_dir: config.voice.asr.sensevoice_model_dir.clone(),
            num_threads: config.voice.asr.sensevoice_threads,
            timeout_ms: u64::from(config.voice.asr.timeout_ms),
        },
        remote,
    };
    let asr = engine::resolve_chain(&options);
    log_engine_chain(&asr, config.voice.asr.engine);
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
        live: config.voice.live,
        live_interval_ms: config.voice.live_interval_ms,
        asr,
        polish: config.llm.enabled.then(|| PolishSettings {
            endpoint: config.llm.endpoint.clone(),
            model: config.llm.model.clone(),
            api_key_env: config.llm.api_key_env.clone(),
            timeout_ms: 10_000,
        }),
    }
}

fn engine_policy(mode: user_config::AsrEngineMode) -> EnginePolicy {
    match mode {
        user_config::AsrEngineMode::Auto => EnginePolicy::Auto,
        user_config::AsrEngineMode::SenseVoice => EnginePolicy::SenseVoice,
        user_config::AsrEngineMode::Remote => EnginePolicy::Remote,
    }
}

/// Startup engine-chain journal; a missing local engine adds a hint line.
fn log_engine_chain(chain: &[EngineConfig], mode: user_config::AsrEngineMode) {
    if engine::detect_gpu() {
        eprintln!("ior: asr GPU detected (no GPU engine yet; using configured chain)");
    }
    if chain.is_empty() {
        eprintln!(
            "ior: asr no engine available (voice.asr.engine = \"sensevoice\" with fallback = false)"
        );
        return;
    }
    eprintln!("ior: asr engines: {}", engine::chain_summary(chain));
    let has_local = chain
        .iter()
        .any(|engine| matches!(engine, EngineConfig::SenseVoice(_)));
    if !has_local && mode != user_config::AsrEngineMode::Remote {
        eprintln!(
            "ior: asr local SenseVoice unavailable; install sherpa-onnx-offline and run \
             scripts/download_models.sh --sensevoice (or set voice.asr.sensevoice_command / \
             voice.asr.sensevoice_model_dir)"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ior_config::Config;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    static SEQ: AtomicU32 = AtomicU32::new(0);

    fn remote_of(settings: &VoiceSettings) -> &AsrSettings {
        match settings.asr.last() {
            Some(EngineConfig::Remote(remote)) => remote,
            other => panic!("expected a trailing remote engine, got {other:?}"),
        }
    }

    fn temp_model_dir() -> PathBuf {
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("ior-voice-settings-{}-{seq}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create model dir");
        fs::write(dir.join("model.int8.onnx"), b"model").expect("model");
        fs::write(dir.join("tokens.txt"), b"tokens").expect("tokens");
        dir
    }

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
        assert!(settings.live, "live preview is the default UX");
        assert_eq!(settings.live_interval_ms, 2_000);
        assert_eq!(remote_of(&settings).model, "Qwen/Qwen3-ASR-4B");
        assert!(settings.polish.is_none(), "llm.enabled defaults false");
    }

    #[test]
    fn remote_policy_freezes_only_the_remote_engine() {
        let mut config = Config::default();
        config.voice.asr.engine = user_config::AsrEngineMode::Remote;
        let settings = settings_from_config(&config);
        assert_eq!(settings.asr.len(), 1);
        assert_eq!(settings.asr[0].label(), "remote");
    }

    #[test]
    fn sensevoice_policy_freezes_local_first_when_probed() {
        let dir = temp_model_dir();
        let mut config = Config::default();
        config.voice.asr.engine = user_config::AsrEngineMode::SenseVoice;
        config.voice.asr.sensevoice_command = "sh".to_string();
        config.voice.asr.sensevoice_model_dir = dir.to_string_lossy().into_owned();
        config.voice.asr.sensevoice_threads = 3;
        let settings = settings_from_config(&config);
        let labels: Vec<&str> = settings.asr.iter().map(EngineConfig::label).collect();
        assert_eq!(labels, vec!["sensevoice", "remote"]);
        match &settings.asr[0] {
            EngineConfig::SenseVoice(local) => {
                assert_eq!(local.num_threads, 3);
                assert_eq!(local.model_dir, dir);
            }
            other => panic!("expected sensevoice, got {other:?}"),
        }
        let _ = fs::remove_dir_all(dir);
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
