//! The ior configuration data model: field names and defaults mirror
//! `engine_core::config::EngineConfig` so addon wiring stays a
//! field-by-field copy.

use crate::value::{Table, Value};

/// `engine.shift_action`: what Shift does to the un-committed preedit
/// when switching Chinese -> English.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ShiftAction {
    /// `"commit"`: commit the raw typed buffer verbatim
    /// (engine-core `ShiftAction::CommitRaw`).
    #[default]
    Commit,
    /// `"temp"`: commit the best candidate, raw text as fallback
    /// (engine-core `ShiftAction::CommitBest`).
    Temp,
    /// `"none"`: drop the composition (engine-core `ShiftAction::Clear`).
    Drop,
}

impl ShiftAction {
    /// Canonical TOML string.
    pub fn as_str(self) -> &'static str {
        match self {
            ShiftAction::Commit => "commit",
            ShiftAction::Temp => "temp",
            ShiftAction::Drop => "none",
        }
    }

    /// Parses one of `"commit"` | `"temp"` | `"none"`.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "commit" => Some(ShiftAction::Commit),
            "temp" => Some(ShiftAction::Temp),
            "none" => Some(ShiftAction::Drop),
            _ => None,
        }
    }
}

/// `engine.double_pinyin`: mirrors engine-core's `DoublePinyin`. The real
/// scheme tables (自然码/小鹤/微软) are imported as data files in P1+;
/// this enum deliberately names the *mode* only — a scheme picker
/// (e.g. `"flypy"` / `"ziranma"`) joins here once the data lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DoublePinyin {
    /// `"off"`: full pinyin, typed letters segmented as-is.
    #[default]
    Off,
    /// `"custom"`: decode via the host-installed `DpTable`.
    Custom,
}

impl DoublePinyin {
    /// Canonical TOML string.
    pub fn as_str(self) -> &'static str {
        match self {
            DoublePinyin::Off => "off",
            DoublePinyin::Custom => "custom",
        }
    }

    /// Parses one of `"off"` | `"custom"`.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "off" => Some(DoublePinyin::Off),
            "custom" => Some(DoublePinyin::Custom),
            _ => None,
        }
    }
}

/// `engine.fuzzy.*`: query-side fuzzy-pinyin toggles, same field names as
/// engine-core `FuzzyConfig` (all off by default).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Fuzzy {
    /// `zh` initial collapses to `z`.
    pub z_zh: bool,
    /// `ch` initial collapses to `c`.
    pub c_ch: bool,
    /// `sh` initial collapses to `s`.
    pub s_sh: bool,
    /// initial `l` collapses to `n`.
    pub n_l: bool,
    /// initial `h` collapses to `f`.
    pub f_h: bool,
    /// final `ang` collapses to `an`.
    pub an_ang: bool,
    /// final `eng` collapses to `en`.
    pub en_eng: bool,
    /// final `ing` collapses to `in`.
    pub in_ing: bool,
}

/// The `[engine]` section (mirrors `engine_core::config::EngineConfig`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Engine {
    /// Shift behavior on Chinese -> English (`commit` | `temp` | `none`).
    pub shift_action: ShiftAction,
    /// Double-pinyin mode (`off` | `custom`).
    pub double_pinyin: DoublePinyin,
    /// Chinese punctuation mapping enabled.
    pub chinese_punct: bool,
    /// Commits feed the user dictionary.
    pub user_learning: bool,
    /// Candidate page size.
    pub page_size: usize,
    /// Hard cap on candidates per request.
    pub max_candidates: usize,
    /// `[engine.fuzzy]` toggles.
    pub fuzzy: Fuzzy,
}

impl Default for Engine {
    /// Same defaults as `engine_core::config::EngineConfig::default()`.
    fn default() -> Self {
        Engine {
            shift_action: ShiftAction::default(),
            double_pinyin: DoublePinyin::default(),
            chinese_punct: true,
            user_learning: true,
            page_size: 5,
            max_candidates: 10,
            fuzzy: Fuzzy::default(),
        }
    }
}

/// One `punct.<name>` entry: a single character, or an ordered list of
/// alternatives (placeholder until the P5 punct system reads it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PunctValue {
    /// `punct.<name> = "，"`
    Char(String),
    /// `punct.<name> = ["—", "～"]`
    Alternatives(Vec<String>),
}

/// The whole `ior.toml` model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// The `[engine]` section.
    pub engine: Engine,
    /// `[switches]`: dynamic bool switch table (any name accepted; the
    /// addon maps the ones it knows, e.g. ascii_mode/full_shape).
    pub switches: Vec<(String, bool)>,
    /// `[punct]`: character mapping placeholder (any name accepted).
    pub punct: Vec<(String, PunctValue)>,
}

impl Default for Config {
    /// Full defaults; `switches` seeds the switch names the roadmap P4
    /// lists (ascii_mode / full_shape / ascii_punct / simplification).
    fn default() -> Self {
        Config {
            engine: Engine::default(),
            switches: default_switches(),
            punct: Vec::new(),
        }
    }
}

/// The default switch table (shared by `Config::default` and
/// [`crate::schema::from_table`] when no `[switches]` section exists).
pub fn default_switches() -> Vec<(String, bool)> {
    vec![
        ("ascii_mode".to_string(), false),
        ("full_shape".to_string(), false),
        ("ascii_punct".to_string(), false),
        ("simplification".to_string(), false),
    ]
}

/// Renders a [`Config`] back into a Value tree with canonical key order
/// (engine scalars, then engine.fuzzy, switches, punct).
pub fn to_table(config: &Config) -> Table {
    let mut engine = Table::new();
    engine.insert(
        "shift_action".into(),
        Value::Str(config.engine.shift_action.as_str().to_string()),
    );
    engine.insert(
        "double_pinyin".into(),
        Value::Str(config.engine.double_pinyin.as_str().to_string()),
    );
    engine.insert(
        "chinese_punct".into(),
        Value::Bool(config.engine.chinese_punct),
    );
    engine.insert(
        "user_learning".into(),
        Value::Bool(config.engine.user_learning),
    );
    engine.insert(
        "page_size".into(),
        Value::Int(config.engine.page_size as i64),
    );
    engine.insert(
        "max_candidates".into(),
        Value::Int(config.engine.max_candidates as i64),
    );
    let mut fuzzy = Table::new();
    for (name, on) in [
        ("z_zh", config.engine.fuzzy.z_zh),
        ("c_ch", config.engine.fuzzy.c_ch),
        ("s_sh", config.engine.fuzzy.s_sh),
        ("n_l", config.engine.fuzzy.n_l),
        ("f_h", config.engine.fuzzy.f_h),
        ("an_ang", config.engine.fuzzy.an_ang),
        ("en_eng", config.engine.fuzzy.en_eng),
        ("in_ing", config.engine.fuzzy.in_ing),
    ] {
        fuzzy.insert(name.into(), Value::Bool(on));
    }
    engine.insert("fuzzy".into(), Value::Table(fuzzy));

    let mut switches = Table::new();
    for (name, on) in &config.switches {
        switches.insert(name.clone(), Value::Bool(*on));
    }
    let mut punct = Table::new();
    for (name, value) in &config.punct {
        let rendered = match value {
            PunctValue::Char(s) => Value::Str(s.clone()),
            PunctValue::Alternatives(items) => {
                Value::Array(items.iter().cloned().map(Value::Str).collect())
            }
        };
        punct.insert(name.clone(), rendered);
    }

    let mut root = Table::new();
    root.insert("engine".into(), Value::Table(engine));
    root.insert("switches".into(), Value::Table(switches));
    root.insert("punct".into(), Value::Table(punct));
    root
}

/// The full default configuration as a Value tree (the merge base and the
/// text a deployer writes out).
pub fn default_table() -> Table {
    to_table(&Config::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_mirror_engine_core() {
        let c = Config::default();
        assert_eq!(c.engine.page_size, 5);
        assert_eq!(c.engine.max_candidates, 10);
        assert!(c.engine.user_learning);
        assert!(c.engine.chinese_punct);
        assert_eq!(c.engine.shift_action, ShiftAction::Commit);
        assert_eq!(c.engine.double_pinyin, DoublePinyin::Off);
        assert_eq!(c.engine.fuzzy, Fuzzy::default());
        assert_eq!(c.switches, default_switches());
        assert!(c.punct.is_empty());
    }

    #[test]
    fn enum_strings_roundtrip() {
        for (action, text) in [
            (ShiftAction::Commit, "commit"),
            (ShiftAction::Temp, "temp"),
            (ShiftAction::Drop, "none"),
        ] {
            assert_eq!(ShiftAction::parse(text), Some(action));
            assert_eq!(action.as_str(), text);
        }
        assert_eq!(ShiftAction::parse("raw"), None);
        for (mode, text) in [(DoublePinyin::Off, "off"), (DoublePinyin::Custom, "custom")] {
            assert_eq!(DoublePinyin::parse(text), Some(mode));
            assert_eq!(mode.as_str(), text);
        }
        assert_eq!(DoublePinyin::parse("flypy"), None);
    }

    #[test]
    fn to_table_covers_every_field() {
        let table = to_table(&Config::default());
        let engine = table.get_table("engine").expect("engine");
        assert_eq!(engine.len(), 7);
        for key in [
            "shift_action",
            "double_pinyin",
            "chinese_punct",
            "user_learning",
            "page_size",
            "max_candidates",
            "fuzzy",
        ] {
            assert!(engine.get(key).is_some(), "missing engine.{key}");
        }
        assert_eq!(engine.get_table("fuzzy").map(Table::len), Some(8));
        assert_eq!(table.get_table("switches").map(Table::len), Some(4));
        assert_eq!(table.get_table("punct").map(Table::len), Some(0));
    }
}
