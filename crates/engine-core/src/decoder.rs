//! Candidate decoding: spelling pipeline -> segmentation -> lookup -> dedupe.
//!
//! The assembly (unchanged shape since M0, now fed by the P1 spelling
//! pipeline):
//!  1. split the raw input on `'` and enumerate segmentations of each run
//!     (fewest syllables first, then lexicographic);
//!  2. user-dict hits for the best segmentation come first (their frequency
//!     is boosted by `USER_BOOST` for display ordering; since they are pushed
//!     before any dictionary match, the boost is realized as position);
//!  3. whole-phrase dictionary matches follow, one segmentation at a time;
//!  4. a composed candidate is built from the top single-syllable entry of
//!     each segment of the best segmentation;
//!  5. single-syllable inputs need no extra handling: step 3 already appends
//!     every (non-top included) dictionary entry for that syllable;
//!  6. 简拼 candidates (pure initial-letter input) are appended after all
//!     full-pinyin ones, at most one per matching key.
//!
//! Fuzzy matching is **query-side** since P1: dictionary keys are canonical,
//! and every segmentation key is expanded with
//! [`spelling::key_variants`] before probing the dictionary or the user
//! dictionary. Hits found through several variants are merged — the same
//! hanzi keeps its highest frequency, never a sum.
//!
//! P2 (M2b) adds a second decoding path: [`candidates_with_lm`] routes
//! inputs of ≥ [`crate::beam::MIN_BEAM_SYLLABLES`] syllables to
//! [`crate::beam`] (word lattice + trigram LM + beam search) when the
//! caller supplies a [`crate::lm::GramModel`]. The three-step assembly
//! above stays the path for 1–2 syllable inputs, 简拼, inputs without a
//! full segmentation, and callers without a model — and it is merged in
//! as the tail of beam results, so beam output is never thinner than the
//! legacy output. The lattice probes dictionaries through the very same
//! variant helpers used here ([`dict_lookup_variants`],
//! [`user_lookup_variants`]).

use crate::beam::{self, BeamParams};
use crate::config::FuzzyConfig;
use crate::dict::Dictionary;
use crate::lm::GramModel;
use crate::segmenter::segmentations;
use crate::spelling;
use crate::user_dict::{self, UserDict};
use std::collections::HashSet;

/// Frequency boost conceptually added to user-dict entries so they sort
/// ahead of any static-dictionary candidate.
pub const USER_BOOST: u32 = 10_000;

/// Guard for multi-run inputs: cap on enumerated segmentation combinations.
const MAX_COMBOS: usize = 4096;

/// Cap on the whole-key spelling-variant expansion ([`spelling::key_variants`]).
const VARIANT_CAP: usize = 64;

/// How many keys the 简拼 probe may fetch from the dictionary's initials
/// index; one candidate is shown per key so short spellings never flood
/// the list.
const INITIALS_KEY_LIMIT: usize = 32;

/// One candidate: the hanzi text plus the pinyin key it was produced from
/// (syllables joined with `'`, as segmented from the typed letters).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// Hanzi (or composed hanzi string) to display/commit.
    pub text: String,
    /// Joined pinyin key of the segmentation that produced this candidate.
    pub pinyin: String,
}

/// Produces up to `limit` candidates for the raw (typed) input `raw` with
/// the default (empty) fuzzy configuration. Kept as the compatibility entry
/// point; see [`candidates_with`].
pub fn candidates(
    dict: &Dictionary,
    user: Option<&UserDict>,
    raw: &str,
    limit: usize,
) -> Vec<Candidate> {
    candidates_with(dict, user, &FuzzyConfig::default(), raw, limit)
}

/// Produces up to `limit` candidates for the raw (typed) input `raw`,
/// which may contain `'` separators between syllable runs, expanding every
/// segmentation key through the spelling pipeline configured by `fuzzy`.
/// Full-pinyin candidates come first (user-learned entries boosted ahead of
/// dictionary hits); 简拼 candidates are appended when `raw` is a pure
/// initial-letter run. Returns an empty `Vec` when nothing matches.
pub fn candidates_with(
    dict: &Dictionary,
    user: Option<&UserDict>,
    fuzzy: &FuzzyConfig,
    raw: &str,
    limit: usize,
) -> Vec<Candidate> {
    if limit == 0 {
        return Vec::new();
    }
    let mut out: Vec<Candidate> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let combos = segmentation_combos(raw);

    if !combos.is_empty() {
        let best_key = combos[0].join("'");

        // (a) user-learned entries for the best segmentation, boosted by
        // USER_BOOST: they are pushed first, so they always precede static
        // hits. Probed over spelling variants, merged by highest frequency.
        if let Some(u) = user {
            for (hanzi, _freq) in user_lookup_variants(u, fuzzy, &best_key) {
                push_unique(&mut out, &mut seen, hanzi, best_key.clone(), limit);
            }
        }

        // (b) whole-phrase matches, one segmentation at a time, each key
        // probed over its spelling variants.
        for combo in &combos {
            let key = combo.join("'");
            for (hanzi, _freq) in dict_lookup_variants(dict, fuzzy, &key) {
                push_unique(&mut out, &mut seen, hanzi, key.clone(), limit);
            }
            if out.len() >= limit {
                return out;
            }
        }

        // (c) composed candidate: top hanzi per segment of the best
        // segmentation (first spelling variant that has an entry).
        let mut composed = String::new();
        let mut complete = true;
        for seg in &combos[0] {
            match top_hanzi_any_variant(dict, fuzzy, seg) {
                Some(hanzi) => composed.push_str(&hanzi),
                None => {
                    complete = false;
                    break;
                }
            }
        }
        if complete {
            push_unique(&mut out, &mut seen, composed, best_key, limit);
        }
    }

    // (d) 简拼 candidates: pure initial-letter input, appended after every
    // full-pinyin candidate, at most one per matching key (freq desc).
    if let Some(prefix) = spelling::initials_prefix(raw) {
        let mut picks: Vec<(u32, String, String)> = Vec::new();
        for key in dict.lookup_initials(&prefix, INITIALS_KEY_LIMIT) {
            if let Some((hanzi, freq)) = dict.lookup(key).and_then(|vals| vals.first()) {
                picks.push((*freq, hanzi.clone(), key.to_string()));
            }
        }
        picks.sort_by(|a, b| {
            b.0.cmp(&a.0)
                .then_with(|| a.1.cmp(&b.1))
                .then_with(|| a.2.cmp(&b.2))
        });
        for (_freq, hanzi, key) in picks {
            push_unique(&mut out, &mut seen, hanzi, key, limit);
        }
    }
    out
}

/// Produces up to `limit` candidates like [`candidates_with`], routing to
/// whole-sentence beam search ([`crate::beam`]) when `lm` is `Some` and the
/// input covers ≥ [`crate::beam::MIN_BEAM_SYLLABLES`] full-pinyin
/// syllables. Without a model — or on short/简拼/unsegmentable inputs —
/// this is exactly the legacy three-step assembly, so the M0 ordering
/// guarantees hold wherever beam search does not apply. Hosts that own a
/// `.gram` file call this instead of [`candidates_with`] (see
/// [`crate::lm::from_gram_bytes`]).
pub fn candidates_with_lm(
    dict: &Dictionary,
    user: Option<&UserDict>,
    lm: Option<&GramModel>,
    fuzzy: &FuzzyConfig,
    raw: &str,
    limit: usize,
) -> Vec<Candidate> {
    if limit > 0 {
        if let Some(model) = lm {
            let beam = beam::candidates(
                dict,
                user,
                model,
                fuzzy,
                raw,
                &BeamParams::defaults(),
                limit,
            );
            if let Some(candidates) = beam {
                return candidates;
            }
        }
    }
    candidates_with(dict, user, fuzzy, raw, limit)
}

/// Probes every spelling variant of `key` in the dictionary and merges the
/// hits: the same hanzi keeps its **highest** frequency across variants
/// (frequencies are never summed across spellings), the result re-sorted
/// freq descending / hanzi ascending.
pub(crate) fn dict_lookup_variants(
    dict: &Dictionary,
    fuzzy: &FuzzyConfig,
    key: &str,
) -> Vec<(String, u32)> {
    let mut merged: Vec<(String, u32)> = Vec::new();
    for variant in spelling::key_variants(fuzzy, key, VARIANT_CAP) {
        if let Some(entries) = dict.lookup(&variant) {
            for (hanzi, freq) in entries {
                match merged.iter_mut().find(|(h, _)| h == hanzi) {
                    Some((_, best)) => *best = (*best).max(*freq),
                    None => merged.push((hanzi.clone(), *freq)),
                }
            }
        }
    }
    merged.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    merged
}

/// Same variant probing and max-frequency merge for user-dict entries.
pub(crate) fn user_lookup_variants(
    user: &UserDict,
    fuzzy: &FuzzyConfig,
    key: &str,
) -> Vec<(String, u32)> {
    let mut merged: Vec<(String, u32)> = Vec::new();
    for variant in spelling::key_variants(fuzzy, key, VARIANT_CAP) {
        for (hanzi, freq) in user_dict::lookup(user, &variant) {
            match merged.iter_mut().find(|(h, _)| *h == hanzi) {
                Some((_, best)) => *best = (*best).max(freq),
                None => merged.push((hanzi, freq)),
            }
        }
    }
    merged.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    merged
}

/// Top hanzi of the first spelling variant of `syl` that has an entry.
fn top_hanzi_any_variant(dict: &Dictionary, fuzzy: &FuzzyConfig, syl: &str) -> Option<String> {
    for variant in spelling::syllable_variants(fuzzy, syl) {
        if let Some((hanzi, _)) = dict.lookup(&variant).and_then(|vals| vals.first()) {
            return Some(hanzi.clone());
        }
    }
    None
}

/// All segmentation combinations of the `'`-separated runs of `raw`, ordered
/// by fewest total segments first, then lexicographically. Empty when any
/// run cannot be segmented.
pub(crate) fn segmentation_combos(raw: &str) -> Vec<Vec<String>> {
    let mut combos: Vec<Vec<String>> = vec![Vec::new()];
    for run in raw.split('\'') {
        let segs = segmentations(run);
        if segs.is_empty() {
            return Vec::new();
        }
        let mut next: Vec<Vec<String>> = Vec::with_capacity(combos.len() * segs.len());
        for combo in &combos {
            for seg in &segs {
                let mut merged = combo.clone();
                merged.extend(seg.iter().cloned());
                next.push(merged);
            }
        }
        next.sort_by(|a, b| compare_combos(a, b));
        next.dedup();
        next.truncate(MAX_COMBOS);
        combos = next;
    }
    combos
}

/// Same ordering as the segmenter: fewest segments first, then lexicographic.
fn compare_combos(a: &[String], b: &[String]) -> std::cmp::Ordering {
    a.len().cmp(&b.len()).then_with(|| a.cmp(b))
}

/// Appends `(text, pinyin)` unless `text` was already produced or the
/// `limit` is reached.
fn push_unique(
    out: &mut Vec<Candidate>,
    seen: &mut HashSet<String>,
    text: String,
    pinyin: String,
    limit: usize,
) {
    if out.len() < limit && seen.insert(text.clone()) {
        out.push(Candidate { text, pinyin });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::FuzzyConfig;
    use crate::user_dict;

    #[test]
    fn top_phrase_match_wins() {
        let d = Dictionary::builtin();
        let cands = candidates(&d, None, "nihao", 10);
        assert_eq!(cands[0].text, "你好");
        assert_eq!(cands[0].pinyin, "ni'hao");
        assert!(!cands.is_empty());
    }

    #[test]
    fn xian_mixes_whole_and_split_matches() {
        let d = Dictionary::builtin();
        let cands = candidates(&d, None, "xian", 10);
        let texts: Vec<&str> = cands.iter().map(|c| c.text.as_str()).collect();
        assert!(texts.contains(&"现"));
        assert!(texts.contains(&"先"));
        assert!(texts.contains(&"西安"), "texts: {texts:?}");
        // the one-syllable segmentation comes first
        assert_eq!(cands[0].pinyin, "xian");
    }

    #[test]
    fn unsegmentable_input_yields_nothing() {
        let d = Dictionary::builtin();
        assert!(candidates(&d, None, "nih", 10).is_empty());
        assert!(candidates(&d, None, "", 10).is_empty());
        assert!(candidates(&d, None, "ni'zh", 10).is_empty());
    }

    #[test]
    fn limit_is_respected() {
        let d = Dictionary::builtin();
        assert_eq!(candidates(&d, None, "shi", 1).len(), 1);
        assert!(candidates(&d, None, "shi", 0).is_empty());
        assert!(candidates(&d, None, "shi", 10).len() > 1);
    }

    #[test]
    fn user_entries_come_first_and_boost_wins() {
        let d = Dictionary::builtin();
        let mut u = user_dict::new();
        user_dict::learn(&mut u, "ni'hao", "泥嚎");
        let cands = candidates(&d, Some(&u), "nihao", 10);
        assert_eq!(cands[0].text, "泥嚎");
        assert!(cands.iter().any(|c| c.text == "你好"));
    }

    #[test]
    fn composed_candidate_from_best_segmentation() {
        let d = Dictionary::builtin();
        // "wo'de" has no phrase entry in core.tsv, but both syllables do, so
        // the composed candidate is built from the top entry of each segment.
        let cands = candidates(&d, None, "wo'de", 10);
        let texts: Vec<&str> = cands.iter().map(|c| c.text.as_str()).collect();
        assert!(texts.contains(&"我的"), "texts: {texts:?}");
    }

    // ---- P1: query-side fuzzy expansion --------------------------------

    #[test]
    fn fuzzy_input_hits_both_spellings() {
        let d = Dictionary::from_tsv("li\t里\t5000\nni\t你\t9000\n").unwrap();
        let f = FuzzyConfig {
            n_l: true,
            ..FuzzyConfig::default()
        };
        // typing "li" matches both the li and the ni entries
        let cands = candidates_with(&d, None, &f, "li", 10);
        let texts: Vec<&str> = cands.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(texts, vec!["你", "里"], "texts: {texts:?}");
        // and typing "ni" reaches exactly the same set
        let cands = candidates_with(&d, None, &f, "ni", 10);
        let texts: Vec<&str> = cands.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(texts, vec!["你", "里"], "texts: {texts:?}");
    }

    #[test]
    fn fuzzy_hits_keep_their_own_frequency_not_a_sum() {
        // 里 freq 5000 reached via variant "li", 你 freq 9000 via variant
        // "ni": the merged order uses each hanzi's own best frequency and
        // never adds them up across spellings.
        let d = Dictionary::from_tsv("li\t里\t5000\nni\t你\t9000\n").unwrap();
        let f = FuzzyConfig {
            n_l: true,
            ..FuzzyConfig::default()
        };
        let cands = candidates_with(&d, None, &f, "li", 10);
        assert_eq!(cands[0].text, "你");
        assert_eq!(cands[1].text, "里");
    }

    #[test]
    fn fuzzy_phrase_probe_uses_key_variants() {
        let d = Dictionary::from_tsv("zhi'dao\t知道\t10\nzi'dao\t字典\t9\n").unwrap();
        let f = FuzzyConfig {
            z_zh: true,
            ..FuzzyConfig::default()
        };
        // typing "zi'dao" expands to {zi,zhi}'dao and finds both phrases
        let cands = candidates_with(&d, None, &f, "zi'dao", 10);
        let texts: Vec<&str> = cands.iter().map(|c| c.text.as_str()).collect();
        assert!(
            texts.contains(&"知道") && texts.contains(&"字典"),
            "texts: {texts:?}"
        );
        assert_eq!(cands[0].text, "知道", "freq desc across variants");
        // without the rule nothing extra is probed
        let cands = candidates_with(&d, None, &FuzzyConfig::default(), "zi'dao", 10);
        assert_eq!(cands.len(), 1);
        assert_eq!(cands[0].text, "字典");
    }

    #[test]
    fn fuzzy_user_entries_are_probed_over_variants() {
        let d = Dictionary::from_tsv("ni'hao\t你好\t100\n").unwrap();
        let mut u = user_dict::new();
        user_dict::learn(&mut u, "li'hao", "理号");
        let f = FuzzyConfig {
            n_l: true,
            ..FuzzyConfig::default()
        };
        // typing "ni'hao" finds the user entry learned under "li'hao"
        let cands = candidates_with(&d, Some(&u), &f, "ni'hao", 10);
        assert_eq!(cands[0].text, "理号", "user entries still come first");
        assert!(cands.iter().any(|c| c.text == "你好"));
    }

    #[test]
    fn candidates_wrapper_uses_no_fuzzy_rules() {
        let d = Dictionary::from_tsv("li\t里\t5000\nni\t你\t9000\n").unwrap();
        // the compatibility wrapper behaves exactly like an off config
        let cands = candidates(&d, None, "li", 10);
        let texts: Vec<&str> = cands.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(texts, vec!["里"]);
    }

    // ---- P1: 简拼 (initials) candidates ---------------------------------

    #[test]
    fn initials_input_matches_keys_by_first_letters() {
        let d = Dictionary::from_tsv("jin'tian\t今天\t4500\njin\t进\t100\n").unwrap();
        let cands = candidates_with(&d, None, &FuzzyConfig::default(), "jt", 10);
        assert_eq!(cands.len(), 1, "one candidate per key: {cands:?}");
        assert_eq!(cands[0].text, "今天");
        assert_eq!(cands[0].pinyin, "jin'tian", "pinyin carries the full key");
    }

    #[test]
    fn initials_candidates_are_appended_not_substituted() {
        // A pure initial run can never segment into full syllables (no
        // syllable consists of initial letters only), so 简拼 picks are by
        // construction the only — and therefore trailing — candidates.
        // This pins that contract plus the freq-desc order among them.
        let d = Dictionary::from_tsv("na\t拿\t100\nni'hao\t你好\t50\nni'men\t你们\t900\n").unwrap();
        let cands = candidates_with(&d, None, &FuzzyConfig::default(), "nh", 10);
        let texts: Vec<&str> = cands.iter().map(|c| c.text.as_str()).collect();
        // "nh" prefixes the initials of ni'hao only (ni'men is "nm");
        // the 拿 entry (initials "n") does not match the prefix.
        assert_eq!(texts, vec!["你好"]);
        assert_eq!(cands[0].pinyin, "ni'hao");
    }

    #[test]
    fn initials_candidates_respect_the_limit() {
        let d = Dictionary::from_tsv("ni'hao\t你好\t50\nni'men\t你们\t900\n").unwrap();
        assert_eq!(
            candidates_with(&d, None, &FuzzyConfig::default(), "nm", 1).len(),
            1
        );
        assert!(candidates_with(&d, None, &FuzzyConfig::default(), "nm", 0).is_empty());
    }

    #[test]
    fn initials_needs_a_pure_initial_letter_run() {
        let d = Dictionary::builtin();
        // "ni" is a full syllable (i is not an initial letter): no 简拼 path,
        // and the full-pinyin path is untouched
        let cands = candidates_with(&d, None, &FuzzyConfig::default(), "ni", 10);
        assert_eq!(cands[0].text, "你");
        // separators disqualify 简拼 even when both runs are initials
        assert!(candidates_with(&d, None, &FuzzyConfig::default(), "n'h", 10).is_empty());
        // single letter is not 简拼 either
        assert!(candidates_with(&d, None, &FuzzyConfig::default(), "n", 10).is_empty());
    }

    #[test]
    fn builtin_jt_finds_jintian() {
        let d = Dictionary::builtin();
        let cands = candidates_with(&d, None, &FuzzyConfig::default(), "jt", 10);
        assert_eq!(cands[0].text, "今天");
        assert_eq!(cands[0].pinyin, "jin'tian");
    }

    // ---- P2 (M2b): beam-search integration -----------------------------

    fn lm_fixture() -> (Dictionary, crate::lm::GramModel) {
        (
            crate::beam_search::tests::fixture_dict(),
            crate::beam_search::tests::fixture_gram(),
        )
    }

    #[test]
    fn without_a_model_the_legacy_path_is_untouched() {
        let d = Dictionary::builtin();
        for raw in ["ni", "nihao", "jintiantianqibucuo", "jt", "nh"] {
            assert_eq!(
                candidates_with_lm(&d, None, None, &FuzzyConfig::default(), raw, 10),
                candidates_with(&d, None, &FuzzyConfig::default(), raw, 10),
                "no model must mean byte-identical legacy output for {raw}"
            );
        }
    }

    #[test]
    fn short_inputs_with_a_model_keep_the_legacy_ordering() {
        let (d, gram) = lm_fixture();
        // 1-2 syllables never enter the beam: 你 / 你好 stay first
        assert_eq!(
            candidates_with_lm(&d, None, Some(&gram), &FuzzyConfig::default(), "ni", 10)[0].text,
            "你"
        );
        assert_eq!(
            candidates_with_lm(&d, None, Some(&gram), &FuzzyConfig::default(), "ni'hao", 10)[0]
                .text,
            "你好"
        );
        // 简拼 keeps its own path even though it has >= 3 letters
        let jianpin = candidates_with_lm(&d, None, Some(&gram), &FuzzyConfig::default(), "jt", 10);
        assert_eq!(jianpin[0].text, "今天");
    }

    #[test]
    fn long_input_with_a_model_decodes_the_whole_sentence() {
        let (d, gram) = lm_fixture();
        let cands = candidates_with_lm(
            &d,
            None,
            Some(&gram),
            &FuzzyConfig::default(),
            "jintian'tianqi'bu'cuo",
            10,
        );
        assert_eq!(cands[0].text, "今天天气不错");
        assert_eq!(cands[0].pinyin, "jin'tian'tian'qi'bu'cuo");
        // the unseparated spelling decodes identically
        let glued = candidates_with_lm(
            &d,
            None,
            Some(&gram),
            &FuzzyConfig::default(),
            "jintiantianqibucuo",
            10,
        );
        assert_eq!(glued[0].text, "今天天气不错");
    }

    #[test]
    fn user_learning_lifts_a_sentence_variant_above_the_default() {
        let (d, gram) = lm_fixture();
        let mut u = user_dict::new();
        for _ in 0..3 {
            user_dict::learn(&mut u, "bu'cuo", "补错");
        }
        let cands = candidates_with_lm(
            &d,
            Some(&u),
            Some(&gram),
            &FuzzyConfig::default(),
            "jintian'tianqi'bu'cuo",
            10,
        );
        assert_eq!(cands[0].text, "今天天气补错");
        assert!(cands.iter().any(|c| c.text == "今天天气不错"));
    }

    #[test]
    fn a_broken_model_is_rejected_at_parse_time_not_decode_time() {
        let err = crate::lm::from_gram_bytes(b"NOPE").unwrap_err();
        assert!(err.contains("bad magic"));
        // callers that ignore the error simply decode without a model
        let (d, _) = lm_fixture();
        assert_eq!(
            candidates_with_lm(&d, None, None, &FuzzyConfig::default(), "nihao", 10)[0].text,
            "你好"
        );
    }
}
