//! Schema extraction: turn a [`Value`] tree into a typed
//! [`Config`](crate::Config). Wrong values fail with dotted-path errors
//! (`engine.shift_action: expected one of ...`); unknown keys are
//! collected as warnings instead of failing the load.

use crate::model::{
    default_switches, Config, DoublePinyin, Engine, Fuzzy, PunctValue, ShiftAction,
};
use crate::value::{kind, Table, Value};

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
    for (key, value) in table.iter() {
        match key {
            "engine" => engine = parse_engine(value, &mut warnings)?,
            "switches" => switches = Some(parse_switches(value)?),
            "punct" => punct = Some(parse_punct(value)?),
            other => warnings.push(format!(
                "unknown key '{other}' (known sections: engine, switches, punct)"
            )),
        }
    }
    Ok(Loaded {
        config: Config {
            engine,
            switches: switches.unwrap_or_else(default_switches),
            punct: punct.unwrap_or_default(),
        },
        warnings,
    })
}

fn parse_engine(value: &Value, warnings: &mut Vec<String>) -> Result<Engine, String> {
    let table = expect_table(value, "engine")?;
    let mut engine = Engine::default();
    for (key, value) in table.iter() {
        match key {
            "shift_action" => {
                engine.shift_action = parse_enum(
                    value,
                    "engine.shift_action",
                    &["commit", "temp", "none"],
                    ShiftAction::parse,
                )?
            }
            "double_pinyin" => {
                // Scheme values (自然码/小鹤/微软) join this list after the
                // key-table data import in P1+.
                engine.double_pinyin = parse_enum(
                    value,
                    "engine.double_pinyin",
                    &["off", "custom"],
                    DoublePinyin::parse,
                )?
            }
            "chinese_punct" => engine.chinese_punct = expect_bool(value, "engine.chinese_punct")?,
            "user_learning" => engine.user_learning = expect_bool(value, "engine.user_learning")?,
            "page_size" => engine.page_size = expect_count(value, "engine.page_size")?,
            "max_candidates" => {
                engine.max_candidates = expect_count(value, "engine.max_candidates")?
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

/// Parses a string enum with a precise "expected one of ..." error.
fn parse_enum<T>(
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

fn expect_table<'v>(value: &'v Value, path: &str) -> Result<&'v Table, String> {
    match value {
        Value::Table(t) => Ok(t),
        other => Err(format!("{path}: expected a table, got {}", kind(other))),
    }
}

fn expect_str<'v>(value: &'v Value, path: &str) -> Result<&'v str, String> {
    match value {
        Value::Str(s) => Ok(s),
        other => Err(format!("{path}: expected a string, got {}", kind(other))),
    }
}

fn expect_bool(value: &Value, path: &str) -> Result<bool, String> {
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
            .insert("shift_action".into(), Value::Str("raw".into()));
        assert_eq!(
            from_table(&t).unwrap_err(),
            "engine.shift_action: expected one of \"commit\", \"temp\", \"none\", got \"raw\""
        );

        let mut t = default_table();
        t.get_table_mut("engine")
            .unwrap()
            .insert("double_pinyin".into(), Value::Str("flypy".into()));
        let err = from_table(&t).unwrap_err();
        assert!(err.starts_with("engine.double_pinyin: expected one of"));

        let mut t = default_table();
        t.get_table_mut("engine")
            .unwrap()
            .insert("shift_action".into(), Value::Int(1));
        assert_eq!(
            from_table(&t).unwrap_err(),
            "engine.shift_action: expected a string, got integer"
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
                "unknown key 'mystery' (known sections: engine, switches, punct)".to_string(),
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
}
