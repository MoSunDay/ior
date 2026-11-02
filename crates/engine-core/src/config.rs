//! Engine configuration: fuzzy-pinyin flags, double-pinyin mode switch and
//! candidate-display settings.

/// Fuzzy-pinyin toggles. Since P1 these rules are applied on the **query
/// side** (see [`crate::spelling`]): the typed syllable is normalized with
/// [`normalize_syllable`] and every equivalent spelling is probed against
/// the canonical dictionary keys. Dictionary keys themselves never carry
/// fuzzy transforms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FuzzyConfig {
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

/// Double-pinyin mode. `Off` keeps full-pinyin input; the three named
/// schemes decode through the builtin tables bundled from
/// `data/double-pinyin/*.tsv` (converted from the rime schema algebra);
/// `Custom` decodes through a host-loaded [`crate::double_pinyin::DpTable`]
/// (see `session::set_double_pinyin`). Without a usable table the raw
/// letters are used as-is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DoublePinyin {
    /// Full pinyin: the typed letters are segmented as-is.
    #[default]
    Off,
    /// 自然碼雙拼 (`data/double-pinyin/natural.tsv`).
    Natural,
    /// 小鶴雙拼 (`data/double-pinyin/flypy.tsv`).
    Xiaohe,
    /// 微軟雙拼 (`data/double-pinyin/mspy.tsv`).
    Mspy,
    /// Decode via the table installed on the session (fall back to the raw
    /// letters when no table is installed or the transform fails).
    Custom,
}

/// What Shift does to the un-committed preedit when switching from Chinese
/// to English mode (see `session::process_shift`). Three configurable
/// states: commit the raw buffer verbatim, commit the best candidate (raw
/// text when nothing decodes), or just drop the composition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ShiftAction {
    /// Commit the raw (typed) buffer as-is, apostrophes included.
    #[default]
    CommitRaw,
    /// Commit the top candidate (learning applies); raw text as fallback.
    CommitBest,
    /// Drop the composition without committing anything.
    Clear,
}

/// What Enter does to the un-committed preedit while a composition is
/// active (see `session::process_key`). The common IME contract: Space
/// commits the top Chinese candidate while Enter commits the typed
/// letters verbatim, so Latin text can be entered without a mode
/// switch; the legacy "Enter behaves exactly like Space" stays
/// reachable via `CommitBest`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EnterAction {
    /// Commit the raw (typed) buffer as-is, apostrophes included.
    #[default]
    CommitRaw,
    /// Commit the top candidate (learning applies); raw text as
    /// fallback — the same path Space takes.
    CommitBest,
}

/// Top-level engine settings.
#[derive(Debug, Clone)]
pub struct EngineConfig {
    /// Fuzzy rules applied at query time by the spelling pipeline
    /// (dictionary keys stay canonical).
    pub fuzzy: FuzzyConfig,
    /// Double-pinyin mode; `Custom` needs a table on the session
    /// (`session::set_double_pinyin`) to have any effect.
    pub double_pinyin: DoublePinyin,
    /// Whether committing a candidate feeds the user dictionary.
    pub user_learning: bool,
    /// Candidate page size (used by frontends, not by core decoding).
    pub page_size: usize,
    /// Keysym names frontends treat as paging-while-composing keys
    /// (frontend concern, like `page_size`).
    pub prev_page_keys: Vec<String>,
    /// Keysym names frontends treat as paging-while-composing keys
    /// (frontend concern, like `page_size`).
    pub next_page_keys: Vec<String>,
    /// Hard cap on how many candidates are produced per request.
    pub max_candidates: usize,
    /// Chinese punctuation mapping: when true (and the session is in
    /// Chinese mode with `ascii_punct` off), printable ASCII punctuation
    /// commits its Chinese counterpart instead of passing through.
    /// Default false: half-width symbols, like the classic IMEs'
    /// "半角符号" state.
    pub chinese_punct: bool,
    /// Full-width (全角) input: idle digits/space (Chinese mode) and all
    /// printable ASCII (English mode) commit full-width forms. Default
    /// half-width; runtime-toggleable per session.
    pub full_width: bool,
    /// How Shift leaving Chinese mode handles the un-committed preedit:
    /// commit it raw, commit the best candidate, or clear it.
    pub shift_action: ShiftAction,
    /// What Enter does to the un-committed preedit: commit the typed
    /// letters raw, or take the Space path and commit the top candidate.
    pub enter_action: EnterAction,
}

impl Default for EngineConfig {
    fn default() -> Self {
        EngineConfig {
            fuzzy: FuzzyConfig::default(),
            double_pinyin: DoublePinyin::Off,
            user_learning: true,
            page_size: 5,
            prev_page_keys: vec!["Page_Up".into(), "minus".into()],
            next_page_keys: vec!["Page_Down".into(), "equal".into()],
            max_candidates: 50,
            chinese_punct: false,
            full_width: false,
            shift_action: ShiftAction::CommitRaw,
            enter_action: EnterAction::CommitRaw,
        }
    }
}

/// The default engine configuration (no fuzzy matching, learning on).
pub fn default_config() -> EngineConfig {
    EngineConfig::default()
}

/// Applies fuzzy rules to a single syllable, in a fixed order:
/// initials first (`zh->z`, `ch->c`, `sh->s`, `l->n`, `h->f`), then finals
/// (`ang->an`, `eng->en`, `ing->in`). Applied blindly, without checking that
/// the input or the output is a real syllable; the transform is idempotent.
/// Query-side use: [`crate::spelling::syllable_variants`] normalizes the
/// typed syllable with this and then expands the equivalence class in both
/// directions, so either spelling of a fuzzy pair matches the dictionary.
pub fn normalize_syllable(fuzzy: &FuzzyConfig, s: &str) -> String {
    let mut out = s.to_string();
    if fuzzy.z_zh && out.starts_with("zh") {
        out.replace_range(0..2, "z");
    }
    if fuzzy.c_ch && out.starts_with("ch") {
        out.replace_range(0..2, "c");
    }
    if fuzzy.s_sh && out.starts_with("sh") {
        out.replace_range(0..2, "s");
    }
    if fuzzy.n_l && out.starts_with('l') {
        out.replace_range(..1, "n");
    }
    if fuzzy.f_h && out.starts_with('h') {
        out.replace_range(..1, "f");
    }
    if fuzzy.an_ang && out.ends_with("ang") {
        let end = out.len();
        out.replace_range(end - 3.., "an");
    }
    if fuzzy.en_eng && out.ends_with("eng") {
        let end = out.len();
        out.replace_range(end - 3.., "en");
    }
    if fuzzy.in_ing && out.ends_with("ing") {
        let end = out.len();
        out.replace_range(end - 3.., "in");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(field: impl FnOnce(&mut FuzzyConfig)) -> FuzzyConfig {
        let mut f = FuzzyConfig::default();
        field(&mut f);
        f
    }

    #[test]
    fn defaults_are_conservative() {
        let f = FuzzyConfig::default();
        assert!(!f.z_zh && !f.c_ch && !f.s_sh && !f.n_l);
        assert!(!f.f_h && !f.an_ang && !f.en_eng && !f.in_ing);
        assert_eq!(normalize_syllable(&f, "zhang"), "zhang");
        let c = default_config();
        assert_eq!(c.page_size, 5);
        assert_eq!(c.max_candidates, 50);
        assert!(c.user_learning);
        assert_eq!(c.prev_page_keys, ["Page_Up", "minus"]);
        assert_eq!(c.next_page_keys, ["Page_Down", "equal"]);
    }

    #[test]
    fn initial_rules_individually() {
        assert_eq!(
            normalize_syllable(&with(|f| f.z_zh = true), "zhang"),
            "zang"
        );
        assert_eq!(
            normalize_syllable(&with(|f| f.c_ch = true), "chang"),
            "cang"
        );
        assert_eq!(normalize_syllable(&with(|f| f.s_sh = true), "shui"), "sui");
        assert_eq!(normalize_syllable(&with(|f| f.n_l = true), "li"), "ni");
        assert_eq!(normalize_syllable(&with(|f| f.n_l = true), "lv"), "nv");
        assert_eq!(normalize_syllable(&with(|f| f.f_h = true), "hao"), "fao");
        assert_eq!(normalize_syllable(&with(|f| f.f_h = true), "hua"), "fua");
    }

    #[test]
    fn final_rules_individually() {
        assert_eq!(
            normalize_syllable(&with(|f| f.an_ang = true), "zhang"),
            "zhan"
        );
        assert_eq!(normalize_syllable(&with(|f| f.an_ang = true), "ang"), "an");
        assert_eq!(
            normalize_syllable(&with(|f| f.en_eng = true), "feng"),
            "fen"
        );
        assert_eq!(normalize_syllable(&with(|f| f.en_eng = true), "eng"), "en");
        assert_eq!(
            normalize_syllable(&with(|f| f.in_ing = true), "xing"),
            "xin"
        );
        assert_eq!(normalize_syllable(&with(|f| f.in_ing = true), "ing"), "in");
    }

    #[test]
    fn combined_rules_and_idempotence() {
        let f = FuzzyConfig {
            z_zh: true,
            an_ang: true,
            ..FuzzyConfig::default()
        };
        assert_eq!(normalize_syllable(&f, "zhang"), "zan");
        let once = normalize_syllable(&f, "zhang");
        assert_eq!(normalize_syllable(&f, &once), once, "must be idempotent");

        let f = FuzzyConfig {
            n_l: true,
            f_h: true,
            ..FuzzyConfig::default()
        };
        assert_eq!(normalize_syllable(&f, "li"), "ni");
        assert_eq!(normalize_syllable(&f, "hua"), "fua");
    }

    #[test]
    fn applied_blindly_without_validation() {
        let f = FuzzyConfig {
            n_l: true,
            ..FuzzyConfig::default()
        };
        assert_eq!(normalize_syllable(&f, "lxx"), "nxx");
        assert_eq!(
            normalize_syllable(&f, "xxlxx"),
            "xxlxx",
            "only initials change"
        );
        assert_eq!(normalize_syllable(&FuzzyConfig::default(), ""), "");
    }
}
