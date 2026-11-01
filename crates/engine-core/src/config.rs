//! Engine configuration: fuzzy-pinyin flags and candidate-display settings.

/// Fuzzy-pinyin toggles. Each rule maps a "heavier" spelling onto a
/// "lighter" one so that, for example, typing `li` can still match `ni`
/// entries once the dictionary is built with [`FuzzyConfig`] applied.
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

/// Top-level engine settings.
#[derive(Debug, Clone)]
pub struct EngineConfig {
    /// Fuzzy rules baked into the dictionary at build time.
    pub fuzzy: FuzzyConfig,
    /// Whether committing a candidate feeds the user dictionary.
    pub user_learning: bool,
    /// Candidate page size (used by frontends, not by core decoding).
    pub page_size: usize,
    /// Hard cap on how many candidates are produced per request.
    pub max_candidates: usize,
    /// Chinese punctuation mapping: when true (and the session is in
    /// Chinese mode with `ascii_punct` off), printable ASCII punctuation
    /// commits its Chinese counterpart instead of passing through.
    pub chinese_punct: bool,
    /// How Shift leaving Chinese mode handles the un-committed preedit:
    /// commit it raw, commit the best candidate, or clear it.
    pub shift_action: ShiftAction,
}

impl Default for EngineConfig {
    fn default() -> Self {
        EngineConfig {
            fuzzy: FuzzyConfig::default(),
            user_learning: true,
            page_size: 5,
            max_candidates: 10,
            chinese_punct: true,
            shift_action: ShiftAction::CommitRaw,
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
        assert_eq!(c.max_candidates, 10);
        assert!(c.user_learning);
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
