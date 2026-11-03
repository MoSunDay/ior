//! User-patch overlay (deep merge) and canonical TOML serialization.
//!
//! Merge semantics: tables merge key by key (recursively); scalars and
//! arrays are replaced wholesale — but only by a value of the same kind,
//! so a type clash fails with the dotted path instead of silently
//! coercing. Serialization is deterministic: scalars before sub-tables,
//! keys in insertion order, 2-space indent per nesting level.

use crate::model::{self, Config};
use crate::parser::is_bare_key;
use crate::schema::{self, Loaded};
use crate::value::{escape_string, kind, Table, Value};

/// Deep-merges `user` over `default`, returning a fresh table. Unknown
/// user keys pass through (the schema step turns them into warnings).
pub fn merge_tables(default: &Table, user: &Table) -> Result<Table, String> {
    merge_section(default, user, "")
}

/// Applies a user patch over the defaults and extracts the typed config.
/// Equivalent to [`apply_with_warnings`] with the warnings dropped.
pub fn apply(default: &Table, user: &Table) -> Result<Config, String> {
    apply_with_warnings(default, user).map(|loaded| loaded.config)
}

/// Like [`apply`] but also returns unknown-key warnings.
pub fn apply_with_warnings(default: &Table, user: &Table) -> Result<Loaded, String> {
    let merged = merge_tables(default, user)?;
    schema::from_table(&merged)
}

/// Canonical TOML text for a Value tree: scalar/array keys first (in
/// insertion order), then every sub-table as a `[dotted.header]` section
/// (also in insertion order), keys indented 2 spaces per level.
pub fn serialize_table(table: &Table) -> String {
    let mut out = String::new();
    let mut path: Vec<String> = Vec::new();
    write_section(&mut out, table, &mut path);
    out
}

/// Canonical TOML text for a [`Config`]; `serialize_table(&to_table(c))`.
pub fn serialize_config(config: &Config) -> String {
    serialize_table(&model::to_table(config))
}

fn merge_section(base: &Table, patch: &Table, path: &str) -> Result<Table, String> {
    let mut out = base.clone();
    for (key, value) in patch.iter() {
        let child_path = if path.is_empty() {
            key.to_string()
        } else {
            format!("{path}.{key}")
        };
        let replacement = match out.get(key) {
            None => value.clone(),
            Some(Value::Table(base_section)) => match value {
                Value::Table(patch_section) => {
                    Value::Table(merge_section(base_section, patch_section, &child_path)?)
                }
                other => {
                    return Err(format!(
                        "{child_path}: expected a table to merge into, got {}",
                        kind(other)
                    ))
                }
            },
            Some(existing) => match (existing, value) {
                (Value::Array(_), Value::Array(_))
                | (Value::Str(_), Value::Str(_))
                | (Value::Int(_), Value::Int(_))
                | (Value::Bool(_), Value::Bool(_)) => value.clone(),
                (base_value, user_value) => {
                    return Err(format!(
                        "{child_path}: cannot override {} with {} (kinds must match; arrays replace wholesale)",
                        kind(base_value),
                        kind(user_value)
                    ))
                }
            },
        };
        out.insert(key.to_string(), replacement);
    }
    Ok(out)
}

fn write_section(out: &mut String, table: &Table, path: &mut Vec<String>) {
    let indent = "  ".repeat(path.len());
    for (key, value) in table.iter() {
        if !matches!(value, Value::Table(_)) {
            out.push_str(&indent);
            out.push_str(&render_key(key));
            out.push_str(" = ");
            out.push_str(&render_inline(value));
            out.push('\n');
        }
    }
    for (key, value) in table.iter() {
        if let Value::Table(section) = value {
            path.push(key.to_string());
            if !out.is_empty() {
                out.push('\n');
            }
            out.push('[');
            out.push_str(&path.join("."));
            out.push_str("]\n");
            write_section(out, section, path);
            path.pop();
        }
    }
}

fn render_key(key: &str) -> String {
    if is_bare_key(key) {
        key.to_string()
    } else {
        format!("\"{}\"", escape_string(key))
    }
}

fn render_inline(value: &Value) -> String {
    match value {
        Value::Str(s) => format!("\"{}\"", escape_string(s)),
        Value::Int(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Array(items) => {
            let rendered: Vec<String> = items.iter().map(render_inline).collect();
            format!("[{}]", rendered.join(", "))
        }
        // `write_section` renders tables as headers, never inline.
        Value::Table(_) => "<table>".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::default_table;
    use crate::parser::parse;
    use crate::value::tables_equivalent;

    #[test]
    fn merge_overrides_scalars_and_keeps_siblings() {
        let user = parse("[engine]\npage_size = 7\n[engine.fuzzy]\nz_zh = true\n")
            .expect("valid user file");
        let merged = merge_tables(&default_table(), &user).expect("merge");
        let engine = merged.get_table("engine").unwrap();
        assert_eq!(engine.get("page_size"), Some(&Value::Int(7)));
        assert_eq!(engine.get("max_candidates"), Some(&Value::Int(50))); // untouched
        assert_eq!(
            engine.get_table("fuzzy").unwrap().get("z_zh"),
            Some(&Value::Bool(true))
        );
        assert_eq!(
            engine.get_table("fuzzy").unwrap().get("c_ch"),
            Some(&Value::Bool(false))
        );
    }

    #[test]
    fn merge_replaces_arrays_wholesale() {
        let mut base = Table::new();
        base.insert(
            "xs".into(),
            Value::Array(vec![Value::Int(1), Value::Int(2)]),
        );
        let user = Table::from_pairs(vec![("xs", Value::Array(vec![Value::Int(9)]))]);
        let merged = merge_tables(&base, &user).expect("merge");
        assert_eq!(
            merged.get("xs"),
            Some(&Value::Array(vec![Value::Int(9)])),
            "no concatenation"
        );
    }

    #[test]
    fn merge_type_mismatch_errors_carry_paths() {
        let user = parse("[engine]\npage_size = \"7\"\n").expect("valid syntax");
        assert_eq!(
            merge_tables(&default_table(), &user).unwrap_err(),
            "engine.page_size: cannot override integer with string (kinds must match; arrays replace wholesale)"
        );

        let user = parse("[engine]\npage_size = [1]\n").expect("valid syntax");
        assert!(merge_tables(&default_table(), &user)
            .unwrap_err()
            .starts_with("engine.page_size: cannot override integer with array"));

        let user = parse("engine = 1\n").expect("valid syntax");
        assert_eq!(
            merge_tables(&default_table(), &user).unwrap_err(),
            "engine: expected a table to merge into, got integer"
        );

        let mut base = Table::new();
        base.insert("xs".into(), Value::Array(vec![Value::Int(1)]));
        let user = Table::from_pairs(vec![("xs", Value::Str("a".into()))]);
        assert!(merge_tables(&base, &user)
            .unwrap_err()
            .starts_with("xs: cannot override array with string"));
    }

    #[test]
    fn merge_keeps_unknown_keys_for_schema_warnings() {
        let user = parse("[engine]\ntypo = true\n[mystery]\nx = 1\n").expect("valid syntax");
        let loaded = apply_with_warnings(&default_table(), &user).expect("merge + schema");
        assert_eq!(
            loaded.warnings,
            vec![
                "unknown key 'engine.typo'".to_string(),
                "unknown key 'mystery' (known sections: engine, switches, punct, voice, llm)"
                    .to_string(),
            ]
        );
    }

    #[test]
    fn apply_empty_patch_yields_defaults() {
        let config = apply(&default_table(), &Table::new()).expect("no patch");
        assert_eq!(config, Config::default());
    }

    #[test]
    fn apply_full_override_end_to_end() {
        let user = parse(
            "[engine]\nenter_action = \"candidate\"\ndouble_pinyin = \"custom\"\n\
             chinese_punct = false\npage_size = 9\nmax_candidates = 30\n\
             [engine.fuzzy]\nan_ang = true\nn_l = true\n\
             [switches]\nascii_mode = true\nfull_shape = false\n\
             [punct]\ncomma = \"，\"\ndash = [\"—\", \"－\"]\n",
        )
        .expect("valid user file");
        let config = apply(&default_table(), &user).expect("valid patch");
        assert_eq!(config.engine.enter_action, model::EnterAction::Candidate);
        assert_eq!(config.engine.double_pinyin, model::DoublePinyin::Custom);
        assert!(!config.engine.chinese_punct);
        assert_eq!(config.engine.page_size, 9);
        assert_eq!(config.engine.max_candidates, 30);
        assert!(config.engine.fuzzy.an_ang && config.engine.fuzzy.n_l);
        // Deep merge: user switches overlay the defaults key by key, so
        // untouched defaults survive alongside the override.
        assert_eq!(
            config.switches,
            vec![
                ("ascii_mode".to_string(), true),
                ("full_shape".to_string(), false),
                ("ascii_punct".to_string(), false),
                ("simplification".to_string(), false),
            ]
        );
        assert_eq!(config.punct.len(), 2);
    }

    #[test]
    fn serialize_config_defaults_parse_back_to_same_config() {
        let text = serialize_config(&Config::default());
        let reparsed = parse(&text).expect("canonical output parses");
        let config = apply(&default_table(), &reparsed).expect("valid");
        assert_eq!(config, Config::default());
        assert!(text.contains("[engine.fuzzy]"));
        assert!(text.contains("[switches]"));
    }

    #[test]
    fn serialize_is_deterministic_and_canonical() {
        let text = serialize_config(&Config::default());
        assert_eq!(text, serialize_config(&Config::default()));
        assert_eq!(text, serialize_table(&default_table()));
        assert!(
            text.starts_with("[engine]\n"),
            "header first, no leading blank"
        );
    }

    #[test]
    fn roundtrip_parse_serialize_parse_is_equivalent() {
        let source = "# comment\n[switches]\nascii_mode = true\nscal = 1\n[engine]\npage_size = 4\n[engine.fuzzy]\nz_zh = true\n";
        let once = parse(source).expect("valid");
        let text = serialize_table(&once);
        let twice = parse(&text).expect("canonical output parses");
        assert!(tables_equivalent(&once, &twice));
        assert_eq!(
            text,
            "[switches]\n  ascii_mode = true\n  scal = 1\n\n[engine]\n  page_size = 4\n\n[engine.fuzzy]\n    z_zh = true\n"
        );
    }

    #[test]
    fn roundtrip_preserves_escapes_and_nested_arrays() {
        let mut nested = Table::new();
        nested.insert("s".into(), Value::Str("a\"b\\c\nd\te中".into()));
        nested.insert("ctl".into(), Value::Str("\u{7}".into()));
        nested.insert(
            "grid".into(),
            Value::Array(vec![
                Value::Array(vec![Value::Str("x".into()), Value::Str("y".into())]),
                Value::Array(vec![]),
            ]),
        );
        let text = serialize_table(&nested);
        assert!(text.contains("\\u{7}"), "control chars must be escaped");
        let back = parse(&text).expect("canonical output parses");
        assert!(tables_equivalent(&nested, &back));
    }
}
