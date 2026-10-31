//! Candidate decoding for M0: segmentation -> dictionary lookup -> dedupe.
//!
//! This is the deliberately simple M0 decoder:
//!  1. split the raw input on `'` and enumerate segmentations of each run
//!     (fewest syllables first, then lexicographic);
//!  2. user-dict hits for the best segmentation come first (their frequency
//!     is boosted by `USER_BOOST` for display ordering; since they are pushed
//!     before any dictionary match, the boost is realized as position);
//!  3. whole-phrase dictionary matches follow, one segmentation at a time;
//!  4. a composed candidate is built from the top single-syllable entry of
//!     each segment of the best segmentation;
//!  5. single-syllable inputs need no extra handling: step 3 already appends
//!     every (non-top included) dictionary entry for that syllable.
//!
//! No fuzzy rules are applied here: fuzziness is baked into the dictionary
//! keys at build time, and [`Dictionary::lookup`] normalizes queries with the
//! config remembered by that dictionary. M2 replaces this module with a
//! trigram language model plus beam search.

use crate::dict::Dictionary;
use crate::segmenter::segmentations;
use crate::user_dict::{self, UserDict};
use std::collections::HashSet;

/// Frequency boost conceptually added to user-dict entries so they sort
/// ahead of any static-dictionary candidate.
pub const USER_BOOST: u32 = 10_000;

/// Guard for multi-run inputs: cap on enumerated segmentation combinations.
const MAX_COMBOS: usize = 4096;

/// One candidate: the hanzi text plus the pinyin key it was produced from
/// (syllables joined with `'`, as segmented from the typed letters).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// Hanzi (or composed hanzi string) to display/commit.
    pub text: String,
    /// Joined pinyin key of the segmentation that produced this candidate.
    pub pinyin: String,
}

/// Produces up to `limit` candidates for the raw (typed) input `raw`,
/// which may contain `'` separators between syllable runs. Returns an empty
/// `Vec` when the input (or any `'`-separated run of it) cannot be segmented
/// into syllables.
pub fn candidates(
    dict: &Dictionary,
    user: Option<&UserDict>,
    raw: &str,
    limit: usize,
) -> Vec<Candidate> {
    if limit == 0 {
        return Vec::new();
    }
    let combos = segmentation_combos(raw);
    if combos.is_empty() {
        return Vec::new();
    }
    let mut out: Vec<Candidate> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let best_key = combos[0].join("'");

    // (a) user-learned entries for the best segmentation, boosted by
    // USER_BOOST: they are pushed first, so they always precede static hits.
    if let Some(u) = user {
        for (hanzi, _freq) in user_dict::lookup(u, &best_key) {
            push_unique(&mut out, &mut seen, hanzi, best_key.clone(), limit);
        }
    }

    // (b) whole-phrase matches, one segmentation at a time.
    for combo in &combos {
        let key = combo.join("'");
        if let Some(entries) = dict.lookup(&key) {
            for (hanzi, _freq) in entries {
                push_unique(&mut out, &mut seen, hanzi.clone(), key.clone(), limit);
            }
        }
        if out.len() >= limit {
            return out;
        }
    }

    // (c) composed candidate: top hanzi per segment of the best segmentation.
    let mut composed = String::new();
    let mut complete = true;
    for seg in &combos[0] {
        match dict.lookup(seg).and_then(|vals| vals.first()) {
            Some((hanzi, _)) => composed.push_str(hanzi),
            None => {
                complete = false;
                break;
            }
        }
    }
    if complete {
        push_unique(&mut out, &mut seen, composed, best_key, limit);
    }
    out
}

/// All segmentation combinations of the `'`-separated runs of `raw`, ordered
/// by fewest total segments first, then lexicographically. Empty when any
/// run cannot be segmented.
fn segmentation_combos(raw: &str) -> Vec<Vec<String>> {
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
        let d = Dictionary::builtin(&FuzzyConfig::default());
        let cands = candidates(&d, None, "nihao", 10);
        assert_eq!(cands[0].text, "你好");
        assert_eq!(cands[0].pinyin, "ni'hao");
        assert!(!cands.is_empty());
    }

    #[test]
    fn xian_mixes_whole_and_split_matches() {
        let d = Dictionary::builtin(&FuzzyConfig::default());
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
        let d = Dictionary::builtin(&FuzzyConfig::default());
        assert!(candidates(&d, None, "nih", 10).is_empty());
        assert!(candidates(&d, None, "", 10).is_empty());
        assert!(candidates(&d, None, "ni'zh", 10).is_empty());
    }

    #[test]
    fn limit_is_respected() {
        let d = Dictionary::builtin(&FuzzyConfig::default());
        assert_eq!(candidates(&d, None, "shi", 1).len(), 1);
        assert!(candidates(&d, None, "shi", 0).is_empty());
        assert!(candidates(&d, None, "shi", 10).len() > 1);
    }

    #[test]
    fn user_entries_come_first_and_boost_wins() {
        let d = Dictionary::builtin(&FuzzyConfig::default());
        let mut u = user_dict::new();
        user_dict::learn(&mut u, "ni'hao", "泥嚎");
        let cands = candidates(&d, Some(&u), "nihao", 10);
        assert_eq!(cands[0].text, "泥嚎");
        assert!(cands.iter().any(|c| c.text == "你好"));
    }

    #[test]
    fn composed_candidate_from_best_segmentation() {
        let d = Dictionary::builtin(&FuzzyConfig::default());
        // "wo'de" has no phrase entry in core.tsv, but both syllables do, so
        // the composed candidate is built from the top entry of each segment.
        let cands = candidates(&d, None, "wo'de", 10);
        let texts: Vec<&str> = cands.iter().map(|c| c.text.as_str()).collect();
        assert!(texts.contains(&"我的"), "texts: {texts:?}");
    }
}
