//! Whole-sentence beam search decoding (P2/M2b).
//!
//! For inputs of [`MIN_BEAM_SYLLABLES`] syllables or more, when the caller
//! supplies a [`GramModel`] (see [`crate::decoder::candidates_with_lm`]),
//! the decoder routes here instead of the M0 three-step assembly. The
//! heavy lifting lives in [`crate::beam_search`]:
//!
//! 1. the input's `'`-separated runs are segmented
//!    ([`crate::segmenter`]) and the fewest-segment splits are enumerated
//!    (up to [`BeamParams::max_segmentations`]);
//! 2. a **word lattice** is built over each split's syllables: for every
//!    start position and word length, dictionary and user-dictionary hits
//!    are probed through the same query-side spelling variants as the
//!    legacy path (fuzzy matching stays query-side);
//! 3. a beam search walks the lattice: each state is
//!    `(syllables consumed, hanzi produced, last two chars, score)`,
//!    scored by `LM log-prob (Stupid Backoff) + word frequency + user
//!    frequency`, plus a [`BeamParams::complete_bonus`] for states that
//!    consumed the whole input;
//! 4. complete states rank first (score desc), then — only if the limit
//!    is not exhausted — the best partial states (useful when trailing
//!    syllables have no coverage at all); finally the legacy
//!    [`crate::decoder::candidates`] output is appended, deduplicated, so
//!    beam results are never worse than the old pipeline.
//!
//! The search is pure: every round consumes a `Vec` of states and produces
//! new vectors (bucket per consumed-syllable position, sorted + deduped +
//! truncated); nothing mutates shared state.

use std::collections::BTreeMap;

use crate::beam_search::{build_lattice, search};
use crate::config::FuzzyConfig;
use crate::decoder::{segmentation_combos, Candidate};
use crate::dict::Dictionary;
use crate::lm::GramModel;
use crate::spelling;
use crate::user_dict::UserDict;

/// Minimum syllable count that routes from the legacy assembly to beam
/// search (1–2 syllable inputs keep the M0 ordering guarantees).
pub const MIN_BEAM_SYLLABLES: usize = 3;

/// Tunables of the sentence decoder. Defaults live in
/// [`BeamParams::defaults`]; no engine-config fields are added for them.
#[derive(Debug, Clone, Copy)]
pub struct BeamParams {
    /// States kept per consumed-syllable position each round.
    pub beam_width: usize,
    /// Multiplier of the LM log-probability sum.
    pub lm_weight: f64,
    /// Multiplier of `ln(dict freq)` per chosen word.
    pub freq_weight: f64,
    /// Multiplier of the user-frequency term; a user hit contributes
    /// `user_weight * ln(USER_BOOST + user freq)` so learned words outrank
    /// any static entry, mirroring the legacy `USER_BOOST` contract.
    pub user_weight: f64,
    /// Flat bonus for states that consumed every syllable.
    pub complete_bonus: f64,
    /// Longest word (in syllables) probed per lattice slot.
    pub max_word_syllables: usize,
    /// Segmentations of the raw input explored (fewest segments first).
    pub max_segmentations: usize,
    /// Word candidates kept per lattice slot (best first).
    pub candidates_per_slot: usize,
}

impl BeamParams {
    /// Hand-tuned defaults (P2): beam 8, LM 1.0, freq/user 0.5, bonus 2.0.
    pub const fn defaults() -> Self {
        BeamParams {
            beam_width: 8,
            lm_weight: 1.0,
            freq_weight: 0.5,
            user_weight: 0.5,
            complete_bonus: 2.0,
            max_word_syllables: 6,
            max_segmentations: 4,
            candidates_per_slot: 12,
        }
    }
}

impl Default for BeamParams {
    fn default() -> Self {
        Self::defaults()
    }
}

/// Beam-decodes `raw` when the gate passes: input is full pinyin (not 简拼),
/// segments into at least [`MIN_BEAM_SYLLABLES`] syllables, and the params
/// allow a non-empty beam. `None` means "not my case": the caller falls
/// back to the legacy assembly. When `Some`, the result already merges the
/// legacy candidates (deduplicated) under the beam's ranking.
pub fn candidates(
    dict: &Dictionary,
    user: Option<&UserDict>,
    lm: &GramModel,
    fuzzy: &FuzzyConfig,
    raw: &str,
    params: &BeamParams,
    limit: usize,
) -> Option<Vec<Candidate>> {
    if params.beam_width == 0 || spelling::initials_prefix(raw).is_some() {
        return None;
    }
    let combos = segmentation_combos(raw);
    let syllables = combos.first()?;
    if syllables.len() < MIN_BEAM_SYLLABLES {
        return None;
    }
    // text -> (score, pinyin, complete); the best entry per text wins
    let mut ranked: BTreeMap<String, (f64, String, bool)> = BTreeMap::new();
    let combos_taken = params.max_segmentations.min(combos.len());
    for combo in combos.iter().take(combos_taken) {
        let lattice = build_lattice(dict, user, fuzzy, combo, params);
        let (complete, partials) = search(combo, &lattice, lm, params);
        let key = combo.join("'");
        for (text, score) in complete {
            upsert(&mut ranked, &text, score, &key, true, params.complete_bonus);
        }
        for (text, (score, pos)) in partials {
            let key = combo[..pos].join("'");
            upsert(
                &mut ranked,
                &text,
                score,
                &key,
                false,
                params.complete_bonus,
            );
        }
    }
    let mut entries: Vec<(f64, bool, String, String)> = ranked
        .into_iter()
        .map(|(text, (score, pinyin, complete))| (score, complete, text, pinyin))
        .collect();
    // complete candidates always precede partial ones; within a group the
    // (bonus-included) score decides, then the text for determinism
    entries.sort_by(|a, b| {
        b.1.cmp(&a.1)
            .then_with(|| b.0.total_cmp(&a.0))
            .then_with(|| a.2.cmp(&b.2))
    });
    let mut out: Vec<Candidate> = entries
        .into_iter()
        .take(limit)
        .map(|(_, _, text, pinyin)| Candidate { text, pinyin })
        .collect();
    if out.len() < limit {
        for legacy in crate::decoder::candidates_with(dict, user, fuzzy, raw, limit) {
            if out.len() >= limit {
                break;
            }
            if !out.iter().any(|c| c.text == legacy.text) {
                out.push(legacy);
            }
        }
    }
    Some(out)
}

/// Keeps the entry with the higher bonus-adjusted score.
fn upsert(
    ranked: &mut BTreeMap<String, (f64, String, bool)>,
    text: &str,
    score: f64,
    pinyin: &str,
    complete: bool,
    complete_bonus: f64,
) {
    let adjusted = score + if complete { complete_bonus } else { 0.0 };
    match ranked.get(text) {
        Some((best, _, best_complete)) => {
            let best_adj = *best + if *best_complete { complete_bonus } else { 0.0 };
            if adjusted > best_adj {
                ranked.insert(text.to_string(), (score, pinyin.to_string(), complete));
            }
        }
        None => {
            ranked.insert(text.to_string(), (score, pinyin.to_string(), complete));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::beam_search::tests::{fixture_dict, fixture_gram};

    #[test]
    fn defaults_are_the_documented_tuning() {
        let p = BeamParams::defaults();
        assert_eq!(p.beam_width, 8);
        assert_eq!(p.max_word_syllables, 6);
        assert_eq!(p.max_segmentations, 4);
        assert_eq!(p.candidates_per_slot, 12);
        assert!((p.lm_weight - 1.0).abs() < f64::EPSILON);
        assert!((p.complete_bonus - 2.0).abs() < f64::EPSILON);
        let d = BeamParams::default();
        assert_eq!(
            (d.beam_width, d.max_word_syllables, d.max_segmentations),
            (p.beam_width, p.max_word_syllables, p.max_segmentations)
        );
    }

    #[test]
    fn gate_rejects_short_initial_and_unsegmentable_input() {
        let dict = fixture_dict();
        let gram = fixture_gram();
        let fuzzy = FuzzyConfig::default();
        let params = BeamParams::defaults();
        assert!(candidates(&dict, None, &gram, &fuzzy, "ni'hao", &params, 10).is_none());
        assert!(candidates(&dict, None, &gram, &fuzzy, "nh", &params, 10).is_none());
        assert!(candidates(&dict, None, &gram, &fuzzy, "zzzz", &params, 10).is_none());
        let gated = candidates(
            &dict,
            None,
            &gram,
            &fuzzy,
            "jin'tian'tian'qi'bu'cuo",
            &params,
            10,
        );
        assert!(gated.is_some());
        // a zero beam width disables the path entirely
        let disabled = BeamParams {
            beam_width: 0,
            ..BeamParams::defaults()
        };
        assert!(candidates(
            &dict,
            None,
            &gram,
            &fuzzy,
            "jintiantianqibucuo",
            &disabled,
            10
        )
        .is_none());
    }

    #[test]
    fn limit_zero_returns_an_empty_list_not_none() {
        let dict = fixture_dict();
        let gram = fixture_gram();
        let out = candidates(
            &dict,
            None,
            &gram,
            &FuzzyConfig::default(),
            "jintiantianqibucuo",
            &BeamParams::defaults(),
            0,
        )
        .unwrap();
        assert!(out.is_empty());
    }
}
