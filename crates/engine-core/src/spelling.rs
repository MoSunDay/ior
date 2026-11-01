//! Spelling-transform pipeline (P1/M2a).
//!
//! Position in the engine: the typed string (`session::raw`) is first
//! mapped through a **spelling transform** and only then decoded:
//!
//! ```text
//! raw input ──► double_pinyin::to_full (mode-dependent)
//!          ──► spelling::syllable_variants / key_variants (fuzzy expansion)
//!          ──► segmenter (syllable graph) ──► decoder (candidates)
//!          └─► spelling::initials_prefix (简拼) ──► dict::lookup_initials
//! ```
//!
//! Dictionary keys are canonical full pinyin (no fuzzy baking since P1);
//! every alternative spelling of what the user typed is generated here and
//! probed against those canonical keys. P2 (whole-sentence decoding) will
//! consume the same `key_variants`/`syllable_variants` surface when walking
//! the syllable lattice, so this module is the stable seam between raw
//! keystrokes and the syllable graph.

use crate::config::{normalize_syllable, FuzzyConfig};

/// Recommended cap for whole-key variant expansion (see [`key_variants`]).
pub const KEY_VARIANT_CAP: usize = 64;

/// Letters that may open a pinyin syllable (the initials `b p m f d t n l
/// g k h j q x zh ch sh r z c s y w`, reduced to their first letters).
/// `h` can form a syllable on its own, but for 简拼 input it counts as an
/// initial letter like everywhere else.
const INITIAL_LETTERS: [char; 20] = [
    'b', 'p', 'm', 'f', 'd', 't', 'n', 'l', 'g', 'k', 'h', 'j', 'q', 'x', 'r', 'z', 'c', 's', 'y',
    'w',
];

/// Fuzzy initial pairs as `(light, heavy)`: with a rule on, the canonical
/// form holds `light` (see [`normalize_syllable`]) and the equivalent
/// `heavy` spelling is produced as a variant — and vice versa, because
/// normalizing either member of a pair yields the same canonical syllable.
const INITIAL_RULES: [(&str, &str); 5] = [
    ("z", "zh"),
    ("c", "ch"),
    ("s", "sh"),
    ("n", "l"),
    ("f", "h"),
];

/// Fuzzy final pairs as `(light, heavy)`, same convention as
/// [`INITIAL_RULES`]: the canonical form keeps `an`/`en`/`in`, the variant
/// expands to `ang`/`eng`/`ing`.
const FINAL_RULES: [(&str, &str); 3] = [("an", "ang"), ("en", "eng"), ("in", "ing")];

/// All spellings equivalent to `syl` under `fuzzy`.
///
/// The syllable is first normalized with [`normalize_syllable`]; the
/// canonical result is then expanded independently at the initial position
/// (z/zh, c/ch, s/sh, n/l, f/h) and the final position (an/ang, en/eng,
/// in/ing) and the two dimensions are combined. The returned list always
/// contains the canonical form itself, is deduped and sorted
/// lexicographically, so the output is deterministic. With all rules off
/// the result is exactly the canonical syllable. No syllable validation is
/// done here: variants are probe strings, invalid ones simply miss.
pub fn syllable_variants(fuzzy: &FuzzyConfig, syl: &str) -> Vec<String> {
    let canonical = normalize_syllable(fuzzy, syl);
    // Initial-position expansions of the canonical syllable.
    let mut initials: Vec<String> = vec![canonical.clone()];
    for (index, (light, heavy)) in INITIAL_RULES.iter().enumerate() {
        if !initial_rule_on(fuzzy, index) {
            continue;
        }
        if let Some(rest) = canonical.strip_prefix(light) {
            initials.push(format!("{heavy}{rest}"));
        }
    }
    // Final-position expansions, kept as (light, heavy) rewrite pairs to
    // apply on top of each initial variant.
    let mut finals: Vec<Option<(&str, &str)>> = vec![None];
    for (index, (light, heavy)) in FINAL_RULES.iter().enumerate() {
        if !final_rule_on(fuzzy, index) || !canonical.ends_with(light) {
            continue;
        }
        finals.push(Some((light, heavy)));
    }
    let mut out: Vec<String> = Vec::with_capacity(initials.len() * finals.len());
    for init in &initials {
        for rewrite in &finals {
            let mut s = init.clone();
            if let Some((light, heavy)) = rewrite {
                if let Some(head) = s.strip_suffix(light) {
                    s = format!("{head}{heavy}");
                }
            }
            out.push(s);
        }
    }
    out.sort();
    out.dedup();
    out
}

/// All joined keys equivalent to `key` (syllables separated by `'`):
/// every syllable is expanded with [`syllable_variants`] and the per-syllable
/// classes are combined into a cartesian product. When the product would
/// exceed `cap` it is truncated lexicographically at each syllable step,
/// which keeps the result deterministic; `cap` is honored as an upper bound
/// for the returned length. `cap == 0` yields an empty list.
pub fn key_variants(fuzzy: &FuzzyConfig, key: &str, cap: usize) -> Vec<String> {
    if cap == 0 {
        return Vec::new();
    }
    let mut acc: Vec<String> = vec![String::new()];
    for syl in key.split('\'') {
        let variants = syllable_variants(fuzzy, syl);
        let mut next: Vec<String> = Vec::with_capacity(acc.len() * variants.len());
        for prefix in &acc {
            for variant in &variants {
                if prefix.is_empty() {
                    next.push(variant.clone());
                } else {
                    next.push(format!("{prefix}'{variant}"));
                }
            }
        }
        next.sort();
        next.dedup();
        next.truncate(cap);
        acc = next;
    }
    acc
}

/// 简拼 (simplified pinyin) detection: `Some(raw)` when the input consists
/// only of initial letters (no `'` separator, no vowels) and is at least
/// two letters long. A single letter is deliberately *not* 简拼 — the
/// ambiguity against full pinyin and association is too high, so it stays
/// with the regular full-pinyin path.
pub fn initials_prefix(raw: &str) -> Option<String> {
    let chars: Vec<char> = raw.chars().collect();
    if chars.len() < 2 || !chars.iter().all(|c| INITIAL_LETTERS.contains(c)) {
        return None;
    }
    Some(raw.to_string())
}

/// Whether the initial rule at `INITIAL_RULES[index]` is enabled.
fn initial_rule_on(fuzzy: &FuzzyConfig, index: usize) -> bool {
    match index {
        0 => fuzzy.z_zh,
        1 => fuzzy.c_ch,
        2 => fuzzy.s_sh,
        3 => fuzzy.n_l,
        _ => fuzzy.f_h,
    }
}

/// Whether the final rule at `FINAL_RULES[index]` is enabled.
fn final_rule_on(fuzzy: &FuzzyConfig, index: usize) -> bool {
    match index {
        0 => fuzzy.an_ang,
        1 => fuzzy.en_eng,
        _ => fuzzy.in_ing,
    }
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
    fn no_rules_yield_the_canonical_form_only() {
        let f = FuzzyConfig::default();
        assert_eq!(syllable_variants(&f, "zhang"), vec!["zhang"]);
        assert_eq!(syllable_variants(&f, "li"), vec!["li"]);
        assert_eq!(syllable_variants(&f, ""), vec![""]);
    }

    #[test]
    fn n_l_expands_both_directions() {
        let f = with(|f| f.n_l = true);
        // either spelling normalizes to "ni" and expands to {li, ni}
        assert_eq!(syllable_variants(&f, "li"), vec!["li", "ni"]);
        assert_eq!(syllable_variants(&f, "ni"), vec!["li", "ni"]);
        assert_eq!(syllable_variants(&f, "lv"), vec!["lv", "nv"]);
    }

    #[test]
    fn z_zh_expands_both_directions() {
        let f = with(|f| f.z_zh = true);
        assert_eq!(syllable_variants(&f, "zhe"), vec!["ze", "zhe"]);
        assert_eq!(syllable_variants(&f, "ze"), vec!["ze", "zhe"]);
        // an untouched initial stays single
        assert_eq!(syllable_variants(&f, "ni"), vec!["ni"]);
    }

    #[test]
    fn f_h_and_finals_pairwise() {
        let f = with(|f| f.f_h = true);
        assert_eq!(syllable_variants(&f, "hao"), vec!["fao", "hao"]);
        let f = with(|f| f.an_ang = true);
        assert_eq!(syllable_variants(&f, "zhang"), vec!["zhan", "zhang"]);
        assert_eq!(syllable_variants(&f, "san"), vec!["san", "sang"]);
        let f = with(|f| f.in_ing = true);
        assert_eq!(syllable_variants(&f, "jin"), vec!["jin", "jing"]);
    }

    #[test]
    fn initial_and_final_dimensions_combine() {
        let f = with(|f| {
            f.z_zh = true;
            f.an_ang = true;
        });
        // "zhuang" -> canonical "zuan" -> {z,zh} x {uan,uang}
        assert_eq!(
            syllable_variants(&f, "zhuang"),
            vec!["zhuan", "zhuang", "zuan", "zuang"]
        );
        let f = with(|f| {
            f.n_l = true;
            f.in_ing = true;
        });
        assert_eq!(
            syllable_variants(&f, "lin"),
            vec!["lin", "ling", "nin", "ning"]
        );
    }

    #[test]
    fn key_variants_multiply_and_dedupe() {
        let f = with(|f| f.n_l = true);
        assert_eq!(key_variants(&f, "li'hao", 64), vec!["li'hao", "ni'hao"]);
        // no rules: the key maps to itself
        assert_eq!(
            key_variants(&FuzzyConfig::default(), "ni'hao", 64),
            vec!["ni'hao"]
        );
        // cap 0 -> nothing
        assert!(key_variants(&f, "li", 0).is_empty());
    }

    #[test]
    fn key_variants_cap_truncates_deterministically() {
        let f = with(|f| {
            f.z_zh = true;
            f.an_ang = true;
        });
        // each syllable of "zhang'zhang" has 4 variants -> 16 combinations
        let capped = key_variants(&f, "zhang'zhang", 4);
        assert_eq!(capped.len(), 4);
        assert_eq!(capped, {
            let mut all = key_variants(&f, "zhang'zhang", 64);
            all.sort();
            all.truncate(4);
            all
        });
        // lexicographic truncation: the smallest spellings survive
        // (`'` sorts below letters, so bare "zan" pairs come first)
        assert_eq!(capped[0], "zan'zan");
    }

    #[test]
    fn initials_prefix_accepts_only_initial_letter_runs() {
        assert_eq!(initials_prefix("nh"), Some("nh".to_string()));
        assert_eq!(initials_prefix("jt"), Some("jt".to_string()));
        assert_eq!(initials_prefix("zhgs"), Some("zhgs".to_string()));
        assert_eq!(initials_prefix("n"), None, "single letter is not 简拼");
        assert_eq!(initials_prefix(""), None);
        assert_eq!(initials_prefix("ni"), None, "i is not an initial letter");
        assert_eq!(initials_prefix("ma"), None);
        assert_eq!(initials_prefix("n'h"), None, "separators are excluded");
        assert_eq!(initials_prefix("Ni"), None, "uppercase is excluded");
        assert_eq!(initials_prefix("n2"), None);
    }
}
