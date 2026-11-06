//! `[voice]` schema extraction. Split from `schema.rs` to keep that
//! module inside the repo line budget; parsing semantics are unchanged.

use super::{expect_bool, expect_int_in, expect_nonempty_str, expect_table, parse_enum};
use crate::model::{AsrEngineMode, HotkeyMode, Voice, VoiceAsr};
use crate::value::Value;

pub(super) fn parse_voice(value: &Value, warnings: &mut Vec<String>) -> Result<Voice, String> {
    let table = expect_table(value, "voice")?;
    let mut voice = Voice::default();
    for (key, value) in table.iter() {
        match key {
            "enabled" => voice.enabled = expect_bool(value, "voice.enabled")?,
            "hotkey" => voice.hotkey = expect_nonempty_str(value, "voice.hotkey")?.to_string(),
            "hotkey_mode" => {
                voice.hotkey_mode = parse_enum(
                    value,
                    "voice.hotkey_mode",
                    &["toggle", "hold"],
                    HotkeyMode::parse,
                )?
            }
            "capture_command" => {
                voice.capture_command =
                    expect_nonempty_str(value, "voice.capture_command")?.to_string()
            }
            "sample_rate" => {
                voice.sample_rate =
                    u32::try_from(expect_int_in(value, "voice.sample_rate", 8_000, 192_000)?)
                        .expect("range checked")
            }
            "max_duration_ms" => {
                voice.max_duration_ms = u32::try_from(expect_int_in(
                    value,
                    "voice.max_duration_ms",
                    1_000,
                    600_000,
                )?)
                .expect("range checked")
            }
            "live" => voice.live = expect_bool(value, "voice.live")?,
            "live_interval_ms" => {
                voice.live_interval_ms =
                    u32::try_from(expect_int_in(value, "voice.live_interval_ms", 500, 10_000)?)
                        .expect("range checked")
            }
            "asr" => voice.asr = parse_voice_asr(value, warnings)?,
            other => warnings.push(format!("unknown key 'voice.{other}'")),
        }
    }
    Ok(voice)
}

/// Parses the `[voice.asr]` engine-chain section.
fn parse_voice_asr(value: &Value, warnings: &mut Vec<String>) -> Result<VoiceAsr, String> {
    let table = expect_table(value, "voice.asr")?;
    let mut asr = VoiceAsr::default();
    for (key, value) in table.iter() {
        match key {
            "engine" => {
                asr.engine = parse_enum(
                    value,
                    "voice.asr.engine",
                    &["auto", "sensevoice", "remote"],
                    AsrEngineMode::parse,
                )?
            }
            "fallback" => asr.fallback = expect_bool(value, "voice.asr.fallback")?,
            "sensevoice_command" => {
                asr.sensevoice_command =
                    expect_nonempty_str(value, "voice.asr.sensevoice_command")?.to_string()
            }
            "sensevoice_model_dir" => {
                asr.sensevoice_model_dir =
                    expect_nonempty_str(value, "voice.asr.sensevoice_model_dir")?.to_string()
            }
            "sensevoice_threads" => {
                asr.sensevoice_threads =
                    u32::try_from(expect_int_in(value, "voice.asr.sensevoice_threads", 1, 64)?)
                        .expect("range checked")
            }
            "timeout_ms" => {
                asr.timeout_ms =
                    u32::try_from(expect_int_in(value, "voice.asr.timeout_ms", 100, 600_000)?)
                        .expect("range checked")
            }
            other => warnings.push(format!("unknown key 'voice.asr.{other}'")),
        }
    }
    Ok(asr)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::default_table;
    use crate::schema::from_table;

    #[test]
    fn voice_section_overrides_defaults_and_warns_on_unknown_keys() {
        let mut t = default_table();
        let voice = t.get_table_mut("voice").unwrap();
        voice.insert("hotkey".into(), Value::Str("ctrl+`".into()));
        voice.insert("hotkey_mode".into(), Value::Str("hold".into()));
        voice.insert("enabled".into(), Value::Bool(false));
        voice.insert("sample_rate".into(), Value::Int(44_100));
        voice.insert("live".into(), Value::Bool(true));
        voice.insert("live_interval_ms".into(), Value::Int(1_500));
        voice.insert("mystery".into(), Value::Bool(true));
        let asr = voice
            .get_table_mut("asr")
            .expect("voice.asr defaults are renderable");
        asr.insert("engine".into(), Value::Str("sensevoice".into()));
        asr.insert("fallback".into(), Value::Bool(false));
        asr.insert("sensevoice_threads".into(), Value::Int(4));
        asr.insert("timeout_ms".into(), Value::Int(7_500));
        asr.insert("mystery".into(), Value::Bool(true));
        let loaded = from_table(&t).expect("voice section");
        assert!(!loaded.config.voice.enabled);
        assert_eq!(loaded.config.voice.hotkey, "ctrl+`");
        assert_eq!(loaded.config.voice.hotkey_mode, HotkeyMode::Hold);
        assert_eq!(loaded.config.voice.sample_rate, 44_100);
        assert!(loaded.config.voice.live);
        assert_eq!(loaded.config.voice.live_interval_ms, 1_500);
        assert_eq!(
            loaded.config.voice.capture_command, "auto",
            "untouched keys keep defaults"
        );
        assert_eq!(loaded.config.voice.asr.engine, AsrEngineMode::SenseVoice);
        assert!(!loaded.config.voice.asr.fallback);
        assert_eq!(loaded.config.voice.asr.sensevoice_threads, 4);
        assert_eq!(loaded.config.voice.asr.timeout_ms, 7_500);
        assert_eq!(loaded.config.voice.asr.sensevoice_command, "auto");
        assert_eq!(
            loaded.warnings,
            vec![
                "unknown key 'voice.asr.mystery'",
                "unknown key 'voice.mystery'"
            ]
        );
    }

    #[test]
    fn voice_enum_and_range_errors_carry_paths() {
        let mut t = default_table();
        t.get_table_mut("voice")
            .unwrap()
            .insert("hotkey_mode".into(), Value::Str("click".into()));
        assert_eq!(
            from_table(&t).unwrap_err(),
            "voice.hotkey_mode: expected one of \"toggle\", \"hold\", got \"click\""
        );

        let mut t = default_table();
        t.get_table_mut("voice")
            .unwrap()
            .insert("sample_rate".into(), Value::Int(4_000));
        assert_eq!(
            from_table(&t).unwrap_err(),
            "voice.sample_rate: expected an integer in 8000..=192000, got 4000"
        );

        let mut t = default_table();
        t.get_table_mut("voice")
            .unwrap()
            .insert("max_duration_ms".into(), Value::Str("long".into()));
        assert_eq!(
            from_table(&t).unwrap_err(),
            "voice.max_duration_ms: expected an integer, got string"
        );

        let mut t = default_table();
        t.get_table_mut("voice")
            .unwrap()
            .insert("hotkey".into(), Value::Str("   ".into()));
        assert_eq!(
            from_table(&t).unwrap_err(),
            "voice.hotkey: expected a non-empty string"
        );

        let mut t = default_table();
        t.get_table_mut("voice")
            .unwrap()
            .insert("live".into(), Value::Int(1));
        assert_eq!(
            from_table(&t).unwrap_err(),
            "voice.live: expected a boolean, got integer"
        );

        for (value, needle) in [(499, "in 500..=10000"), (10_001, "in 500..=10000")] {
            let mut t = default_table();
            t.get_table_mut("voice")
                .unwrap()
                .insert("live_interval_ms".into(), Value::Int(value));
            let err = from_table(&t).unwrap_err();
            assert_eq!(
                err,
                format!("voice.live_interval_ms: expected an integer in 500..=10000, got {value}")
            );
            assert!(err.contains(needle));
        }

        for (key, value, needle) in [
            ("engine", Value::Str("gpu".into()), "expected one of"),
            ("sensevoice_threads", Value::Int(0), "in 1..=64"),
            (
                "timeout_ms",
                Value::Str("long".into()),
                "expected an integer",
            ),
        ] {
            let mut t = default_table();
            t.get_table_mut("voice")
                .unwrap()
                .get_table_mut("asr")
                .unwrap()
                .insert(key.into(), value);
            let err = from_table(&t).unwrap_err();
            assert!(err.starts_with(&format!("voice.asr.{key}: ")), "{err}");
            assert!(err.contains(needle), "{err}");
        }
    }
}
