//! The ior configuration data model: field names and defaults mirror
//! `engine_core::config::EngineConfig` so addon wiring stays a
//! field-by-field copy.

use crate::value::{Table, Value};

/// `engine.enter_action`: what Enter does to the un-committed preedit.
/// Mirrors engine-core's `EnterAction`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EnterAction {
    /// `"raw"`: commit the typed letters verbatim
    /// (engine-core `EnterAction::CommitRaw`).
    #[default]
    Raw,
    /// `"candidate"`: commit the top candidate, the Space path
    /// (engine-core `EnterAction::CommitBest`).
    Candidate,
}

impl EnterAction {
    /// Canonical TOML string.
    pub fn as_str(self) -> &'static str {
        match self {
            EnterAction::Raw => "raw",
            EnterAction::Candidate => "candidate",
        }
    }

    /// Parses one of `"raw"` | `"candidate"`.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "raw" => Some(EnterAction::Raw),
            "candidate" => Some(EnterAction::Candidate),
            _ => None,
        }
    }
}

/// `engine.double_pinyin`: mirrors engine-core's `DoublePinyin`. The
/// builtin schemes decode through the tables converted from the rime
/// schema algebra (`data/double-pinyin/*.tsv`); `custom` decodes via a
/// host-installed table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DoublePinyin {
    /// `"off"`: full pinyin, typed letters segmented as-is.
    #[default]
    Off,
    /// `"natural"`: 自然碼雙拼.
    Natural,
    /// `"xiaohe"`: 小鶴雙拼.
    Xiaohe,
    /// `"mspy"`: 微軟雙拼.
    Mspy,
    /// `"custom"`: decode via the host-installed `DpTable`.
    Custom,
}

impl DoublePinyin {
    /// Canonical TOML string.
    pub fn as_str(self) -> &'static str {
        match self {
            DoublePinyin::Off => "off",
            DoublePinyin::Natural => "natural",
            DoublePinyin::Xiaohe => "xiaohe",
            DoublePinyin::Mspy => "mspy",
            DoublePinyin::Custom => "custom",
        }
    }

    /// Parses one of `"off"` | `"natural"` | `"xiaohe"` | `"mspy"` |
    /// `"custom"`.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "off" => Some(DoublePinyin::Off),
            "natural" => Some(DoublePinyin::Natural),
            // rime names the scheme file `flypy`; accept it as an alias
            "xiaohe" | "flypy" => Some(DoublePinyin::Xiaohe),
            "mspy" => Some(DoublePinyin::Mspy),
            "custom" => Some(DoublePinyin::Custom),
            _ => None,
        }
    }
}

/// `voice.hotkey_mode`: how the voice hotkey interacts with recording.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HotkeyMode {
    /// `"toggle"`: press starts recording, the next press cancels it.
    #[default]
    Toggle,
    /// `"hold"`: press starts, release stops and transcribes
    /// (push-to-talk).
    Hold,
}

impl HotkeyMode {
    /// Canonical TOML string.
    pub fn as_str(self) -> &'static str {
        match self {
            HotkeyMode::Toggle => "toggle",
            HotkeyMode::Hold => "hold",
        }
    }

    /// Parses one of `"toggle"` | `"hold"` (push-to-talk alias included).
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "toggle" => Some(HotkeyMode::Toggle),
            "hold" | "push_to_talk" | "push-to-talk" => Some(HotkeyMode::Hold),
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Engine {
    /// Enter behavior on the un-committed preedit (`raw` | `candidate`).
    pub enter_action: EnterAction,
    /// Double-pinyin mode (`off` | `custom`).
    pub double_pinyin: DoublePinyin,
    /// Chinese punctuation mapping enabled.
    pub chinese_punct: bool,
    /// Full-width (全角) input on/off (half-width by default).
    pub full_width: bool,
    /// Commits feed the user dictionary.
    pub user_learning: bool,
    /// Candidate page size.
    pub page_size: usize,
    /// Hard cap on candidates per request.
    pub max_candidates: usize,
    /// Keysym names that page candidates backwards while composing.
    pub prev_page_keys: Vec<String>,
    /// Keysym names that page candidates forwards while composing.
    pub next_page_keys: Vec<String>,
    /// `[engine.fuzzy]` toggles.
    pub fuzzy: Fuzzy,
}

impl Default for Engine {
    /// Same defaults as `engine_core::config::EngineConfig::default()`.
    fn default() -> Self {
        Engine {
            enter_action: EnterAction::default(),
            double_pinyin: DoublePinyin::default(),
            chinese_punct: false,
            full_width: false,
            user_learning: true,
            page_size: 5,
            max_candidates: 50,
            prev_page_keys: vec!["Page_Up".into(), "minus".into()],
            next_page_keys: vec!["Page_Down".into(), "equal".into()],
            fuzzy: Fuzzy::default(),
        }
    }
}

/// The `[voice]` section: voice-input capture controls. Recording and
/// ASR live in the `voice` crate; this only carries user settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Voice {
    /// Voice input on/off (the hotkey is dead while false).
    pub enabled: bool,
    /// Hotkey spec, e.g. `"ctrl+grave"` (alias `"ctrl+\`"`); parsed by
    /// `voice::hotkey::parse_hotkey` in the addon.
    pub hotkey: String,
    /// Hotkey semantics (`toggle` | `hold`).
    pub hotkey_mode: HotkeyMode,
    /// Capture command: `"auto"` probes pw-record / parec / arecord, any
    /// other value is an argv split on spaces (test stubs use `cat`).
    pub capture_command: String,
    /// Capture sample rate in Hz (s16le mono).
    pub sample_rate: u32,
    /// Hard cap on one utterance; auto-stop transcribes afterwards.
    pub max_duration_ms: u32,
}

impl Default for Voice {
    fn default() -> Self {
        Voice {
            enabled: true,
            hotkey: "ctrl+grave".to_string(),
            hotkey_mode: HotkeyMode::default(),
            capture_command: "auto".to_string(),
            sample_rate: 16_000,
            max_duration_ms: 60_000,
        }
    }
}

/// The `[llm.asr]` section: the remote transcription service (node02,
/// Qwen-first, OpenAI-compatible `/v1/audio/transcriptions`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlmAsr {
    /// Service URL, `http://` only (see docs/NODE02-DEPLOY.md).
    pub endpoint: String,
    /// Model id, Qwen family preferred (default `Qwen/Qwen3-ASR-4B`).
    pub model: String,
    /// Environment variable holding the API key; the key itself never
    /// enters the config file (unset = no Authorization header).
    pub api_key_env: String,
    /// ISO language hint passed to the service (`"zh"`).
    pub language: String,
    /// Per-request network timeout.
    pub timeout_ms: u32,
}

impl Default for LlmAsr {
    fn default() -> Self {
        LlmAsr {
            endpoint: "http://node02:8000/v1/audio/transcriptions".to_string(),
            model: "Qwen/Qwen3-ASR-4B".to_string(),
            api_key_env: "IOR_ASR_API_KEY".to_string(),
            language: "zh".to_string(),
            timeout_ms: 5_000,
        }
    }
}

/// The `[llm]` section: optional transcript polish plus the nested
/// `[llm.asr]` transcription settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Llm {
    /// Run the transcript through the polish endpoint before commit.
    pub enabled: bool,
    /// OpenAI-compatible chat completions URL on node02.
    pub endpoint: String,
    /// Polish model id.
    pub model: String,
    /// Environment variable holding the API key (see `LlmAsr::api_key_env`).
    pub api_key_env: String,
    /// `[llm.asr]`: the transcription service itself.
    pub asr: LlmAsr,
}

impl Default for Llm {
    fn default() -> Self {
        Llm {
            enabled: false,
            endpoint: "http://node02:8001/v1/chat/completions".to_string(),
            model: "Qwen/Qwen2.5-7B-Instruct".to_string(),
            api_key_env: "IOR_LLM_API_KEY".to_string(),
            asr: LlmAsr::default(),
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
    /// `[voice]`: voice-input controls.
    pub voice: Voice,
    /// `[llm]` + `[llm.asr]`: remote ASR and optional polish.
    pub llm: Llm,
}

impl Default for Config {
    /// Full defaults; `switches` seeds the switch names the roadmap P4
    /// lists (ascii_mode / full_shape / ascii_punct / simplification).
    fn default() -> Self {
        Config {
            engine: Engine::default(),
            switches: default_switches(),
            punct: Vec::new(),
            voice: Voice::default(),
            llm: Llm::default(),
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
        "enter_action".into(),
        Value::Str(config.engine.enter_action.as_str().to_string()),
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
    engine.insert(
        "prev_page_keys".into(),
        Value::Array(
            config
                .engine
                .prev_page_keys
                .iter()
                .cloned()
                .map(Value::Str)
                .collect(),
        ),
    );
    engine.insert(
        "next_page_keys".into(),
        Value::Array(
            config
                .engine
                .next_page_keys
                .iter()
                .cloned()
                .map(Value::Str)
                .collect(),
        ),
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

    let mut voice = Table::new();
    voice.insert("enabled".into(), Value::Bool(config.voice.enabled));
    voice.insert("hotkey".into(), Value::Str(config.voice.hotkey.clone()));
    voice.insert(
        "hotkey_mode".into(),
        Value::Str(config.voice.hotkey_mode.as_str().to_string()),
    );
    voice.insert(
        "capture_command".into(),
        Value::Str(config.voice.capture_command.clone()),
    );
    voice.insert(
        "sample_rate".into(),
        Value::Int(i64::from(config.voice.sample_rate)),
    );
    voice.insert(
        "max_duration_ms".into(),
        Value::Int(i64::from(config.voice.max_duration_ms)),
    );

    let mut asr = Table::new();
    asr.insert(
        "endpoint".into(),
        Value::Str(config.llm.asr.endpoint.clone()),
    );
    asr.insert("model".into(), Value::Str(config.llm.asr.model.clone()));
    asr.insert(
        "api_key_env".into(),
        Value::Str(config.llm.asr.api_key_env.clone()),
    );
    asr.insert(
        "language".into(),
        Value::Str(config.llm.asr.language.clone()),
    );
    asr.insert(
        "timeout_ms".into(),
        Value::Int(i64::from(config.llm.asr.timeout_ms)),
    );

    let mut llm = Table::new();
    llm.insert("enabled".into(), Value::Bool(config.llm.enabled));
    llm.insert("endpoint".into(), Value::Str(config.llm.endpoint.clone()));
    llm.insert("model".into(), Value::Str(config.llm.model.clone()));
    llm.insert(
        "api_key_env".into(),
        Value::Str(config.llm.api_key_env.clone()),
    );
    llm.insert("asr".into(), Value::Table(asr));

    let mut root = Table::new();
    root.insert("engine".into(), Value::Table(engine));
    root.insert("switches".into(), Value::Table(switches));
    root.insert("punct".into(), Value::Table(punct));
    root.insert("voice".into(), Value::Table(voice));
    root.insert("llm".into(), Value::Table(llm));
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
        assert_eq!(c.engine.max_candidates, 50);
        assert!(c.engine.user_learning);
        assert!(!c.engine.chinese_punct); // half-width symbols by default
        assert_eq!(c.engine.double_pinyin, DoublePinyin::Off);
        assert_eq!(c.engine.prev_page_keys, ["Page_Up", "minus"]);
        assert_eq!(c.engine.next_page_keys, ["Page_Down", "equal"]);
        assert_eq!(c.engine.fuzzy, Fuzzy::default());
        assert_eq!(c.switches, default_switches());
        assert!(c.punct.is_empty());
        assert_eq!(c.voice, Voice::default());
        assert_eq!(c.llm, Llm::default());
    }

    #[test]
    fn enum_strings_roundtrip() {
        for (mode, text) in [
            (DoublePinyin::Off, "off"),
            (DoublePinyin::Natural, "natural"),
            (DoublePinyin::Xiaohe, "xiaohe"),
            (DoublePinyin::Mspy, "mspy"),
            (DoublePinyin::Custom, "custom"),
        ] {
            assert_eq!(DoublePinyin::parse(text), Some(mode));
            assert_eq!(mode.as_str(), text);
        }
        assert_eq!(DoublePinyin::parse("flypy"), Some(DoublePinyin::Xiaohe));
        assert_eq!(DoublePinyin::parse("shuangpin"), None);
    }

    #[test]
    fn to_table_covers_every_field() {
        let table = to_table(&Config::default());
        let engine = table.get_table("engine").expect("engine");
        assert_eq!(engine.len(), 9);
        for key in [
            "enter_action",
            "double_pinyin",
            "chinese_punct",
            "user_learning",
            "page_size",
            "max_candidates",
            "prev_page_keys",
            "next_page_keys",
            "fuzzy",
        ] {
            assert!(engine.get(key).is_some(), "missing engine.{key}");
        }
        assert_eq!(engine.get_table("fuzzy").map(Table::len), Some(8));
        assert_eq!(table.get_table("switches").map(Table::len), Some(4));
        assert_eq!(table.get_table("punct").map(Table::len), Some(0));
    }

    #[test]
    fn to_table_covers_voice_and_llm() {
        let table = to_table(&Config::default());
        let voice = table.get_table("voice").expect("voice");
        assert_eq!(voice.len(), 6);
        assert_eq!(str_of(voice.get("hotkey")), Some("ctrl+grave"));
        assert_eq!(str_of(voice.get("hotkey_mode")), Some("toggle"));
        assert_eq!(int_of(voice.get("sample_rate")), Some(16_000));
        let llm = table.get_table("llm").expect("llm");
        assert_eq!(llm.len(), 5);
        assert_eq!(bool_of(llm.get("enabled")), Some(false));
        let asr = llm.get_table("asr").expect("llm.asr");
        assert_eq!(asr.len(), 5);
        assert_eq!(str_of(asr.get("model")), Some("Qwen/Qwen3-ASR-4B"));
        assert_eq!(
            str_of(asr.get("endpoint")),
            Some("http://node02:8000/v1/audio/transcriptions")
        );
    }

    #[test]
    fn hotkey_mode_strings_roundtrip() {
        assert_eq!(HotkeyMode::default(), HotkeyMode::Toggle);
        assert_eq!(HotkeyMode::parse("toggle"), Some(HotkeyMode::Toggle));
        assert_eq!(HotkeyMode::parse("hold"), Some(HotkeyMode::Hold));
        assert_eq!(HotkeyMode::parse("push-to-talk"), Some(HotkeyMode::Hold));
        assert_eq!(HotkeyMode::parse("click"), None);
        assert_eq!(HotkeyMode::Hold.as_str(), "hold");
    }

    fn str_of(value: Option<&Value>) -> Option<&str> {
        match value {
            Some(Value::Str(s)) => Some(s),
            _ => None,
        }
    }

    fn int_of(value: Option<&Value>) -> Option<i64> {
        match value {
            Some(Value::Int(n)) => Some(*n),
            _ => None,
        }
    }

    fn bool_of(value: Option<&Value>) -> Option<bool> {
        match value {
            Some(Value::Bool(b)) => Some(*b),
            _ => None,
        }
    }
}
