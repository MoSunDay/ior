//! Schema extraction: turn a [`Value`] tree into a typed
//! [`Config`](crate::Config). Wrong values fail with dotted-path errors
//! (`engine.enter_action: expected one of ...`); unknown keys are
//! collected as warnings instead of failing the load.

use crate::model::{
    default_switches, Config, DoublePinyin, Engine, EnterAction, Fuzzy, Llm, LlmAsr, PunctValue,
};
use crate::value::{kind, Table, Value};

mod voice;

/// A successfully extracted config plus non-fatal warnings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loaded {
    /// The typed configuration.
    pub config: Config,
    /// Unknown-key warnings (never fatal), e.g. `unknown key 'engine.typo'`.
    pub warnings: Vec<String>,
}

/// Extracts a typed [`Config`] from a Value tree. Missing sections fall
/// back to defaults; unknown keys become warnings; wrong values produce
/// `Err` messages prefixed with the dotted key path.
pub fn from_table(table: &Table) -> Result<Loaded, String> {
    let mut warnings = Vec::new();
    let mut engine = Engine::default();
    let mut switches = None;
    let mut punct = None;
    let mut voice = None;
    let mut llm = None;
    for (key, value) in table.iter() {
        match key {
            "engine" => engine = parse_engine(value, &mut warnings)?,
            "switches" => switches = Some(parse_switches(value)?),
            "punct" => punct = Some(parse_punct(value)?),
            "voice" => voice = Some(voice::parse_voice(value, &mut warnings)?),
            "llm" => llm = Some(parse_llm(value, &mut warnings)?),
            other => warnings.push(format!(
                "unknown key '{other}' (known sections: engine, switches, punct, voice, llm)"
            )),
        }
    }
    Ok(Loaded {
        config: Config {
            engine,
            switches: switches.unwrap_or_else(default_switches),
            punct: punct.unwrap_or_default(),
            voice: voice.unwrap_or_default(),
            llm: llm.unwrap_or_default(),
        },
        warnings,
    })
}

fn parse_engine(value: &Value, warnings: &mut Vec<String>) -> Result<Engine, String> {
    let table = expect_table(value, "engine")?;
    let mut engine = Engine::default();
    for (key, value) in table.iter() {
        match key {
            "enter_action" => {
                engine.enter_action = parse_enum(
                    value,
                    "engine.enter_action",
                    &["raw", "candidate"],
                    EnterAction::parse,
                )?
            }
            "double_pinyin" => {
                engine.double_pinyin = parse_enum(
                    value,
                    "engine.double_pinyin",
                    &["off", "natural", "xiaohe", "mspy", "custom"],
                    DoublePinyin::parse,
                )?
            }
            "chinese_punct" => engine.chinese_punct = expect_bool(value, "engine.chinese_punct")?,
            "full_width" => engine.full_width = expect_bool(value, "engine.full_width")?,
            "user_learning" => engine.user_learning = expect_bool(value, "engine.user_learning")?,
            "page_size" => engine.page_size = expect_count(value, "engine.page_size")?,
            "max_candidates" => {
                engine.max_candidates = expect_count(value, "engine.max_candidates")?
            }
            "prev_page_keys" => {
                engine.prev_page_keys = expect_str_array(value, "engine.prev_page_keys")?
            }
            "next_page_keys" => {
                engine.next_page_keys = expect_str_array(value, "engine.next_page_keys")?
            }
            "fuzzy" => engine.fuzzy = parse_fuzzy(value, warnings)?,
            other => warnings.push(format!("unknown key 'engine.{other}'")),
        }
    }
    Ok(engine)
}

fn parse_fuzzy(value: &Value, warnings: &mut Vec<String>) -> Result<Fuzzy, String> {
    let table = expect_table(value, "engine.fuzzy")?;
    let mut fuzzy = Fuzzy::default();
    for (key, value) in table.iter() {
        let slot = match key {
            "z_zh" => Some(&mut fuzzy.z_zh),
            "c_ch" => Some(&mut fuzzy.c_ch),
            "s_sh" => Some(&mut fuzzy.s_sh),
            "n_l" => Some(&mut fuzzy.n_l),
            "f_h" => Some(&mut fuzzy.f_h),
            "an_ang" => Some(&mut fuzzy.an_ang),
            "en_eng" => Some(&mut fuzzy.en_eng),
            "in_ing" => Some(&mut fuzzy.in_ing),
            _ => None,
        };
        match slot {
            Some(flag) => *flag = expect_bool(value, &format!("engine.fuzzy.{key}"))?,
            None => warnings.push(format!("unknown key 'engine.fuzzy.{key}'")),
        }
    }
    Ok(fuzzy)
}

fn parse_switches(value: &Value) -> Result<Vec<(String, bool)>, String> {
    let table = expect_table(value, "switches")?;
    let mut out = Vec::new();
    for (key, value) in table.iter() {
        // Dynamic by design: any switch name is accepted (only bool values
        // are enforced); the addon ignores switches it does not know.
        out.push((
            key.to_string(),
            expect_bool(value, &format!("switches.{key}"))?,
        ));
    }
    Ok(out)
}

fn parse_punct(value: &Value) -> Result<Vec<(String, PunctValue)>, String> {
    let table = expect_table(value, "punct")?;
    let mut out = Vec::new();
    for (key, value) in table.iter() {
        let entry = match value {
            Value::Str(s) => PunctValue::Char(s.clone()),
            Value::Array(items) => {
                let mut texts = Vec::new();
                for item in items {
                    match item {
                        Value::Str(s) => texts.push(s.clone()),
                        other => {
                            return Err(format!(
                                "punct.{key}: array items must be strings, got {}",
                                kind(other)
                            ))
                        }
                    }
                }
                PunctValue::Alternatives(texts)
            }
            other => {
                return Err(format!(
                    "punct.{key}: expected a string or an array of strings, got {}",
                    kind(other)
                ))
            }
        };
        out.push((key.to_string(), entry));
    }
    Ok(out)
}

/// Parses the `[voice]` section (defaults for missing keys).
/// Parses the `[llm]` section with its nested `[llm.asr]`.
fn parse_llm(value: &Value, warnings: &mut Vec<String>) -> Result<Llm, String> {
    let table = expect_table(value, "llm")?;
    let mut llm = Llm::default();
    for (key, value) in table.iter() {
        match key {
            "enabled" => llm.enabled = expect_bool(value, "llm.enabled")?,
            "endpoint" => llm.endpoint = expect_url(value, "llm.endpoint")?.to_string(),
            "model" => llm.model = expect_nonempty_str(value, "llm.model")?.to_string(),
            "api_key_env" => {
                llm.api_key_env = expect_nonempty_str(value, "llm.api_key_env")?.to_string()
            }
            "asr" => llm.asr = parse_llm_asr(value, warnings)?,
            other => warnings.push(format!("unknown key 'llm.{other}'")),
        }
    }
    Ok(llm)
}

/// Parses the `[llm.asr]` transcription-service section.
fn parse_llm_asr(value: &Value, warnings: &mut Vec<String>) -> Result<LlmAsr, String> {
    let table = expect_table(value, "llm.asr")?;
    let mut asr = LlmAsr::default();
    for (key, value) in table.iter() {
        match key {
            "endpoint" => asr.endpoint = expect_url(value, "llm.asr.endpoint")?.to_string(),
            "model" => asr.model = expect_nonempty_str(value, "llm.asr.model")?.to_string(),
            "api_key_env" => {
                asr.api_key_env = expect_nonempty_str(value, "llm.asr.api_key_env")?.to_string()
            }
            "language" => {
                asr.language = expect_nonempty_str(value, "llm.asr.language")?.to_string()
            }
            "timeout_ms" => {
                asr.timeout_ms =
                    u32::try_from(expect_int_in(value, "llm.asr.timeout_ms", 100, 600_000)?)
                        .expect("range checked")
            }
            other => warnings.push(format!("unknown key 'llm.asr.{other}'")),
        }
    }
    Ok(asr)
}

/// Parses a string enum with a precise "expected one of ..." error.
pub(super) fn parse_enum<T>(
    value: &Value,
    path: &str,
    options: &[&str],
    parse: fn(&str) -> Option<T>,
) -> Result<T, String> {
    let text = expect_str(value, path)?;
    parse(text).ok_or_else(|| {
        let list = options
            .iter()
            .map(|option| format!("\"{option}\""))
            .collect::<Vec<_>>()
            .join(", ");
        format!("{path}: expected one of {list}, got \"{text}\"")
    })
}

pub(super) fn expect_table<'v>(value: &'v Value, path: &str) -> Result<&'v Table, String> {
    match value {
        Value::Table(t) => Ok(t),
        other => Err(format!("{path}: expected a table, got {}", kind(other))),
    }
}

pub(super) fn expect_str<'v>(value: &'v Value, path: &str) -> Result<&'v str, String> {
    match value {
        Value::Str(s) => Ok(s),
        other => Err(format!("{path}: expected a string, got {}", kind(other))),
    }
}

pub(super) fn expect_bool(value: &Value, path: &str) -> Result<bool, String> {
    match value {
        Value::Bool(b) => Ok(*b),
        other => Err(format!("{path}: expected a boolean, got {}", kind(other))),
    }
}

/// Integer >= 1 (page sizes and candidate caps make no sense at 0).
fn expect_count(value: &Value, path: &str) -> Result<usize, String> {
    match value {
        Value::Int(n) if *n >= 1 => {
            usize::try_from(*n).map_err(|_| format!("{path}: integer {n} is too large"))
        }
        Value::Int(n) => Err(format!("{path}: expected an integer >= 1, got {n}")),
        other => Err(format!("{path}: expected an integer, got {}", kind(other))),
    }
}

/// Array of non-empty strings (keysym names); an empty array disables
/// that paging direction.
fn expect_str_array(value: &Value, path: &str) -> Result<Vec<String>, String> {
    match value {
        Value::Array(items) => {
            let mut out = Vec::new();
            for item in items {
                match item {
                    Value::Str(s) if !s.is_empty() => out.push(s.clone()),
                    Value::Str(_) => return Err(format!("{path}: expected no empty strings")),
                    other => {
                        return Err(format!(
                            "{path}: array items must be strings, got {}",
                            kind(other)
                        ))
                    }
                }
            }
            Ok(out)
        }
        other => Err(format!("{path}: expected an array, got {}", kind(other))),
    }
}

/// A string that must be non-empty (hotkeys, commands, model ids).
pub(super) fn expect_nonempty_str<'v>(value: &'v Value, path: &str) -> Result<&'v str, String> {
    let text = expect_str(value, path)?;
    if text.trim().is_empty() {
        return Err(format!("{path}: expected a non-empty string"));
    }
    Ok(text)
}

/// An `http://` URL; the voice HTTP client speaks plain HTTP only, so
/// anything else is a deployment error caught at load time, not at record
/// time.
fn expect_url<'v>(value: &'v Value, path: &str) -> Result<&'v str, String> {
    let text = expect_nonempty_str(value, path)?;
    if !text.starts_with("http://") {
        return Err(format!("{path}: expected an http:// URL, got \"{text}\""));
    }
    Ok(text)
}

/// An integer inside `[min, max]` (sample rates, durations, timeouts).
pub(super) fn expect_int_in(value: &Value, path: &str, min: i64, max: i64) -> Result<i64, String> {
    match value {
        Value::Int(n) if *n >= min && *n <= max => Ok(*n),
        Value::Int(n) => Err(format!(
            "{path}: expected an integer in {min}..={max}, got {n}"
        )),
        other => Err(format!("{path}: expected an integer, got {}", kind(other))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::default_table;

    #[test]
    fn from_table_roundtrips_default_tree() {
        let loaded = from_table(&default_table()).expect("defaults are valid");
        assert_eq!(loaded.config, Config::default());
        assert!(loaded.warnings.is_empty());
    }

    #[test]
    fn missing_sections_fall_back_to_defaults() {
        let mut t = Table::new();
        t.insert("engine".into(), Value::Table(Table::new()));
        let loaded = from_table(&t).expect("empty engine is fine");
        assert_eq!(loaded.config.engine, Engine::default());
        assert_eq!(loaded.config.switches, default_switches());
    }

    #[test]
    fn enum_errors_carry_path_and_options() {
        let mut t = default_table();
        t.get_table_mut("engine")
            .unwrap()
            .insert("enter_action".into(), Value::Str("space".into()));
        assert_eq!(
            from_table(&t).unwrap_err(),
            "engine.enter_action: expected one of \"raw\", \"candidate\", got \"space\""
        );

        let mut t = default_table();
        t.get_table_mut("engine")
            .unwrap()
            .insert("double_pinyin".into(), Value::Str("shuangpin".into()));
        let err = from_table(&t).unwrap_err();
        assert!(err.starts_with("engine.double_pinyin: expected one of"));

        // rime's file name for 小鶴 is `flypy`; it is accepted as an alias
        let mut t = default_table();
        t.get_table_mut("engine")
            .unwrap()
            .insert("double_pinyin".into(), Value::Str("flypy".into()));
        assert_eq!(
            from_table(&t).unwrap().config.engine.double_pinyin,
            DoublePinyin::Xiaohe
        );

        let mut t = default_table();
        t.get_table_mut("engine")
            .unwrap()
            .insert("enter_action".into(), Value::Int(1));
        assert_eq!(
            from_table(&t).unwrap_err(),
            "engine.enter_action: expected a string, got integer"
        );
    }

    #[test]
    fn type_errors_carry_paths() {
        let mut t = default_table();
        t.get_table_mut("engine")
            .unwrap()
            .insert("chinese_punct".into(), Value::Int(1));
        assert_eq!(
            from_table(&t).unwrap_err(),
            "engine.chinese_punct: expected a boolean, got integer"
        );

        let mut t = default_table();
        t.get_table_mut("engine")
            .unwrap()
            .insert("full_width".into(), Value::Str("yes".into()));
        assert_eq!(
            from_table(&t).unwrap_err(),
            "engine.full_width: expected a boolean, got string"
        );

        let mut t = default_table();
        t.get_table_mut("engine")
            .unwrap()
            .insert("page_size".into(), Value::Str("big".into()));
        assert_eq!(
            from_table(&t).unwrap_err(),
            "engine.page_size: expected an integer, got string"
        );

        let mut t = default_table();
        t.get_table_mut("engine")
            .unwrap()
            .insert("page_size".into(), Value::Int(0));
        assert_eq!(
            from_table(&t).unwrap_err(),
            "engine.page_size: expected an integer >= 1, got 0"
        );

        let mut t = default_table();
        t.insert("engine".into(), Value::Int(1));
        assert_eq!(
            from_table(&t).unwrap_err(),
            "engine: expected a table, got integer"
        );
    }

    #[test]
    fn page_keys_parse_and_validate() {
        // Valid arrays override the defaults (rime luna_pinyin style).
        let mut t = default_table();
        t.get_table_mut("engine").unwrap().insert(
            "prev_page_keys".into(),
            Value::Array(vec![
                Value::Str("comma".into()),
                Value::Str("Page_Up".into()),
            ]),
        );
        t.get_table_mut("engine").unwrap().insert(
            "next_page_keys".into(),
            Value::Array(vec![Value::Str("period".into())]),
        );
        let loaded = from_table(&t).expect("page keys");
        assert_eq!(loaded.config.engine.prev_page_keys, ["comma", "Page_Up"]);
        assert_eq!(loaded.config.engine.next_page_keys, ["period"]);

        // An empty array is legal: it disables that paging direction.
        let mut t = default_table();
        t.get_table_mut("engine")
            .unwrap()
            .insert("prev_page_keys".into(), Value::Array(Vec::new()));
        assert!(from_table(&t)
            .unwrap()
            .config
            .engine
            .prev_page_keys
            .is_empty());

        // Non-array and non-string items are errors.
        let mut t = default_table();
        t.get_table_mut("engine")
            .unwrap()
            .insert("next_page_keys".into(), Value::Str("Page_Down".into()));
        assert_eq!(
            from_table(&t).unwrap_err(),
            "engine.next_page_keys: expected an array, got string"
        );

        let mut t = default_table();
        t.get_table_mut("engine").unwrap().insert(
            "prev_page_keys".into(),
            Value::Array(vec![Value::Str("minus".into()), Value::Int(1)]),
        );
        assert_eq!(
            from_table(&t).unwrap_err(),
            "engine.prev_page_keys: array items must be strings, got integer"
        );

        let mut t = default_table();
        t.get_table_mut("engine").unwrap().insert(
            "prev_page_keys".into(),
            Value::Array(vec![Value::Str(String::new())]),
        );
        assert_eq!(
            from_table(&t).unwrap_err(),
            "engine.prev_page_keys: expected no empty strings"
        );
    }

    #[test]
    fn fuzzy_flags_and_unknown_key_warnings() {
        let mut t = default_table();
        let engine = t.get_table_mut("engine").unwrap();
        let fuzzy = engine.get_table_mut("fuzzy").expect("fuzzy table");
        fuzzy.insert("z_zh".into(), Value::Bool(true));
        fuzzy.insert("typo".into(), Value::Bool(true));
        engine.insert("whats_this".into(), Value::Bool(false));
        t.insert("mystery".into(), Value::Table(Table::new()));

        let loaded = from_table(&t).expect("unknown keys are warnings");
        assert!(loaded.config.engine.fuzzy.z_zh);
        assert_eq!(
            loaded.warnings,
            vec![
                "unknown key 'engine.fuzzy.typo'".to_string(),
                "unknown key 'engine.whats_this'".to_string(),
                "unknown key 'mystery' (known sections: engine, switches, punct, voice, llm)"
                    .to_string(),
            ]
        );
    }

    #[test]
    fn switches_are_dynamic_but_typed() {
        let mut t = default_table();
        let switches = t.get_table_mut("switches").unwrap();
        switches.insert("ascii_mode".into(), Value::Bool(true));
        switches.insert("custom_thing".into(), Value::Bool(true));
        let loaded = from_table(&t).expect("dynamic switches");
        assert!(loaded.warnings.is_empty());
        assert!(loaded
            .config
            .switches
            .iter()
            .any(|(name, on)| name == "custom_thing" && *on));

        let mut t = default_table();
        t.get_table_mut("switches")
            .unwrap()
            .insert("ascii_mode".into(), Value::Str("yes".into()));
        assert_eq!(
            from_table(&t).unwrap_err(),
            "switches.ascii_mode: expected a boolean, got string"
        );
    }

    #[test]
    fn punct_accepts_strings_and_string_arrays() {
        let mut t = default_table();
        t.insert(
            "punct".into(),
            Value::Table(Table::from_pairs(vec![
                ("comma", Value::Str("，".into())),
                (
                    "dash",
                    Value::Array(vec![Value::Str("—".into()), Value::Str("－".into())]),
                ),
            ])),
        );
        let loaded = from_table(&t).expect("punct entries");
        assert_eq!(
            loaded.config.punct,
            vec![
                ("comma".to_string(), PunctValue::Char("，".to_string())),
                (
                    "dash".to_string(),
                    PunctValue::Alternatives(vec!["—".to_string(), "－".to_string()])
                ),
            ]
        );

        let mut t = default_table();
        t.insert(
            "punct".into(),
            Value::Table(Table::from_pairs(vec![("x", Value::Int(1))])),
        );
        assert_eq!(
            from_table(&t).unwrap_err(),
            "punct.x: expected a string or an array of strings, got integer"
        );
    }

    #[test]
    fn llm_sections_nest_and_validate() {
        let mut t = default_table();
        let llm = t.get_table_mut("llm").unwrap();
        llm.insert("enabled".into(), Value::Bool(true));
        let asr = llm.get_table_mut("asr").unwrap();
        asr.insert(
            "endpoint".into(),
            Value::Str("http://10.0.0.2:9000/v1/audio/transcriptions".into()),
        );
        asr.insert("model".into(), Value::Str("Qwen/Qwen3-ASR-8B".into()));
        asr.insert("timeout_ms".into(), Value::Int(2_000));
        let loaded = from_table(&t).expect("llm section");
        assert!(loaded.config.llm.enabled);
        assert_eq!(
            loaded.config.llm.asr.endpoint,
            "http://10.0.0.2:9000/v1/audio/transcriptions"
        );
        assert_eq!(loaded.config.llm.asr.model, "Qwen/Qwen3-ASR-8B");
        assert_eq!(loaded.config.llm.asr.timeout_ms, 2_000);
        assert_eq!(loaded.config.llm.asr.language, "zh");
        assert_eq!(
            loaded.config.llm.endpoint,
            "http://node02:8001/v1/chat/completions"
        );

        // [llm] present without [llm.asr]: nested defaults survive.
        let mut t = default_table();
        *t.get_table_mut("llm").unwrap() =
            Table::from_pairs(vec![("enabled".to_string(), Value::Bool(false))]);
        let loaded = from_table(&t).expect("llm without asr");
        assert_eq!(loaded.config.llm.asr, LlmAsr::default());

        for endpoint in ["node02:8000", "https://node02:8000"] {
            let mut t = default_table();
            t.get_table_mut("llm")
                .unwrap()
                .get_table_mut("asr")
                .unwrap()
                .insert("endpoint".into(), Value::Str(endpoint.into()));
            let err = from_table(&t).unwrap_err();
            assert_eq!(
                err,
                format!("llm.asr.endpoint: expected an http:// URL, got \"{endpoint}\"")
            );
        }
    }
    #[test]
    fn unknown_top_level_sections_still_warn() {
        let mut t = default_table();
        t.insert("voise".into(), Value::Table(Table::new()));
        let loaded = from_table(&t).expect("typo is a warning");
        assert_eq!(
            loaded.warnings,
            vec!["unknown key 'voise' (known sections: engine, switches, punct, voice, llm)"]
        );
    }
}
