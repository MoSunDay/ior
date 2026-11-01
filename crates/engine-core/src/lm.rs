//! # `.gram` — character n-gram language model (P2/M2b)
//!
//! A `.gram` file holds character unigram/bigram/trigram counts distilled
//! from a pinyin→hanzi word list (dict-tools `train`: **within-word**
//! character n-grams, weighted by word frequency — the only real signal
//! available offline; corpus-level models are future data, see the P2 row
//! of `docs/ROADMAP-RIME-PARITY.md`). engine-core parses it into a
//! [`GramModel`] (zero I/O: bytes in, model out) and [`crate::beam`] scores
//! whole-sentence candidates with **Stupid Backoff** — trigram → bigram →
//! unigram, fixed penalty [`BACKOFF_PENALTY`] = 0.4 per backoff level, no
//! Katz re-normalization.
//!
//! ## Format (version 1 — this module doc is the authoritative spec)
//!
//! The writer is `dict_tools::gram` (`train` + `GramCounts::to_gram_bytes`);
//! both ends are pinned byte-for-byte by `crates/bench/tests/gram_interlock.rs`
//! and by golden-byte tests on each side. Multi-byte integers are
//! little-endian; varints are strict LEB128 u32 (`.ior` v2 /
//! `dict_tools::varint` encoding).
//!
//! ```text
//! header (24 bytes):
//!   0..6    magic "IOGRAM"
//!   6       version u8 = 1
//!   7       order  u8 = 3 (character trigram model)
//!   8..12   unigram_count  u32 LE  (distinct keys in the unigram section)
//!   12..16  bigram_count   u32 LE
//!   16..20  trigram_count  u32 LE
//!   20..24  total_weight   u32 LE  (sum of all unigram counts)
//!
//! body: three sections in order — unigram, bigram, trigram. Each section:
//!   index area  `count` records of `varint(len) utf8-key`, keys distinct
//!               and strictly ascending in utf8 byte order, each key holds
//!               exactly as many chars as the section's order;
//!   value area  `count` varints — the count of the i-th index key
//!               (summed source word frequency, saturating u32).
//! ```
//!
//! Readers reject bad magic, unsupported version/order, header counts that
//! disagree with the body, mis-ordered or mis-sized keys, invalid utf8,
//! truncated fields and trailing bytes; every error carries the offset or
//! key index where it was detected ([`from_gram_bytes`]).
//!
//! Keys are stored packed (see [`bi_key`]/[`tri_key`]) so scoring does no
//! string allocation; UTF-8 byte order equals code-point order, so the
//! packed keys preserve the on-disk ordering.

use std::collections::HashMap;

use crate::lm_format::{parse_gram, RawGram};

/// Magic bytes at the start of every `.gram` file (matches dict-tools).
pub const GRAM_MAGIC: &[u8; 6] = b"IOGRAM";

/// `.gram` format version understood (and written) by this pair of crates.
pub const GRAM_VERSION: u8 = 1;

/// Model order: 3 = character trigram with bigram/unigram backoff.
pub const GRAM_ORDER: u8 = 3;

/// Stupid Backoff multiplier per backoff level (trigram→bigram→unigram).
pub const BACKOFF_PENALTY: f64 = 0.4;

/// Squared backoff penalty (two levels skipped: trigram context and bigram).
const BACKOFF_PENALTY2: f64 = BACKOFF_PENALTY * BACKOFF_PENALTY;

/// Pseudo-count credited to characters never seen in training, so unseen
/// hanzi score very low but finite (no `-inf`/NaN ever enters a beam).
const OOV_COUNT: f64 = 0.5;

/// Bits occupied by one char code inside packed n-gram keys (21 = full
/// Unicode range).
const CHAR_BITS: u32 = 21;

/// Packs a char pair into a bigram map key.
pub(crate) fn bi_key(a: char, b: char) -> u64 {
    ((a as u64) << CHAR_BITS) | b as u64
}

/// Packs a char triple into a trigram map key.
pub(crate) fn tri_key(a: char, b: char, c: char) -> u64 {
    ((a as u64) << (2 * CHAR_BITS)) | ((b as u64) << CHAR_BITS) | c as u64
}

/// A parsed character-trigram model with Stupid Backoff scoring.
///
/// Immutable once parsed; scoring methods take everything they need as
/// arguments and allocate nothing.
#[derive(Debug, Clone)]
pub struct GramModel {
    /// Unigram count sum as f64 (denominator; at least 1.0).
    total: f64,
    /// Raw `total_weight` from the header.
    total_raw: u32,
    /// char code -> count.
    unigrams: HashMap<u32, u32>,
    /// [`bi_key`] -> count.
    bigrams: HashMap<u64, u32>,
    /// [`tri_key`] -> count.
    trigrams: HashMap<u64, u32>,
}

/// Parses a `.gram` buffer (as written by `dict-tools train`) into a model.
pub fn from_gram_bytes(bytes: &[u8]) -> Result<GramModel, String> {
    let RawGram {
        total,
        unigrams,
        bigrams,
        trigrams,
    } = parse_gram(bytes)?;
    Ok(GramModel {
        total: f64::from(total).max(1.0),
        total_raw: total,
        unigrams,
        bigrams,
        trigrams,
    })
}

impl GramModel {
    /// Model order (always [`GRAM_ORDER`] for v1 files).
    pub fn order(&self) -> u8 {
        GRAM_ORDER
    }

    /// Distinct-key counts per section: `(unigrams, bigrams, trigrams)`.
    pub fn counts(&self) -> (usize, usize, usize) {
        (self.unigrams.len(), self.bigrams.len(), self.trigrams.len())
    }

    /// Sum of all unigram counts (the header `total_weight`).
    pub fn total_weight(&self) -> u32 {
        self.total_raw
    }

    /// log S(next | prev2 prev1) under Stupid Backoff:
    ///
    /// - trigram `c(abc)/c(ab)` when the trigram and its context exist;
    /// - else `0.4 · c(bc)/c(b)` when the bigram and its context exist;
    /// - else `0.4² · c(c)/N` (the leading `0.4`s are skipped only when
    ///   the context itself is absent, i.e. at sentence start).
    ///
    /// Unseen characters score `0.4^k · OOV_COUNT/N` — low but finite.
    pub fn logp(&self, prev2: Option<char>, prev1: Option<char>, next: char) -> f64 {
        if let (Some(a), Some(b)) = (prev2, prev1) {
            if let (Some(&abc), Some(&ab)) = (
                self.trigrams.get(&tri_key(a, b, next)),
                self.bigrams.get(&bi_key(a, b)),
            ) {
                return (f64::from(abc) / f64::from(ab)).ln();
            }
        }
        if let Some(b) = prev1 {
            if let (Some(&bc), Some(&b_count)) = (
                self.bigrams.get(&bi_key(b, next)),
                self.unigrams.get(&(b as u32)),
            ) {
                return BACKOFF_PENALTY.ln() + (f64::from(bc) / f64::from(b_count)).ln();
            }
        }
        let penalty = if prev1.is_some() {
            BACKOFF_PENALTY2.ln()
        } else {
            0.0
        };
        let count = match self.unigrams.get(&(next as u32)) {
            Some(&seen) => f64::from(seen),
            None => OOV_COUNT,
        };
        penalty + (count / self.total).ln()
    }

    /// Total log score of a whole hanzi string under [`GramModel::logp`].
    pub fn sequence_logp(&self, text: &str) -> f64 {
        let mut total = 0.0;
        let (mut prev2, mut prev1) = (None, None);
        for c in text.chars() {
            total += self.logp(prev2, prev1, c);
            prev2 = prev1;
            prev1 = Some(c);
        }
        total
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test-side mini writer implementing the module-doc spec (unigram +
    /// bigram sections, single-byte counts; the production writer is
    /// dict-tools `gram`, byte-compat pinned by the cross-crate test in
    /// crates/bench/tests/gram_interlock.rs).
    fn gram_bytes(unigrams: &[(&str, u32)], bigrams: &[(&str, u32)]) -> Vec<u8> {
        let mut sorted: Vec<(&str, u32)> = unigrams.to_vec();
        sorted.sort_unstable();
        let mut out = Vec::new();
        out.extend_from_slice(GRAM_MAGIC);
        out.push(GRAM_VERSION);
        out.push(GRAM_ORDER);
        out.extend_from_slice(&(unigrams.len() as u32).to_le_bytes());
        out.extend_from_slice(&(bigrams.len() as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        let total: u32 = unigrams.iter().map(|&(_, c)| c).sum();
        out.extend_from_slice(&total.to_le_bytes());
        for (key, _) in &sorted {
            out.push(key.len() as u8);
            out.extend_from_slice(key.as_bytes());
        }
        for (_, count) in &sorted {
            out.push(*count as u8);
        }
        for (key, count) in bigrams {
            out.push(key.len() as u8);
            out.extend_from_slice(key.as_bytes());
            out.push(*count as u8);
        }
        out
    }

    #[test]
    fn parses_and_counts_a_minimal_model() {
        let bytes = gram_bytes(&[("你", 8), ("好", 8), ("金", 3)], &[("你好", 8)]);
        let model = from_gram_bytes(&bytes).unwrap();
        assert_eq!(model.order(), 3);
        assert_eq!(model.counts(), (3, 1, 0));
        assert_eq!(model.total_weight(), 19);
    }

    #[test]
    fn backoff_levels_order_sensibly() {
        // 今=10 天=30 气=5; bigram 今天=8 (no trigram section here: the
        // trigram level is exercised in trigram_level_beats_backoff_levels
        // and end-to-end through dict-tools-trained models)
        let bytes = gram_bytes(&[("今", 10), ("天", 30), ("气", 5)], &[("今天", 8)]);
        let model = from_gram_bytes(&bytes).unwrap();
        // sentence start: unigram level, no penalty
        let start = model.logp(None, None, '天');
        assert!((start - (30.0_f64 / 45.0).ln()).abs() < 1e-12);
        // bigram level: 0.4 * 8/10
        let bigram = model.logp(None, Some('今'), '天');
        assert!((bigram - (0.4_f64 * 0.8).ln()).abs() < 1e-12);
        // backoff to unigram: 0.16 * 30/45
        let backed = model.logp(None, Some('天'), '天');
        assert!((backed - (0.16_f64 * 30.0_f64 / 45.0).ln()).abs() < 1e-12);
        // unseen char: 0.16 * 0.5/45, finite and lowest
        let oov = model.logp(None, Some('天'), '错');
        assert!(oov < backed && oov.is_finite());
        // sequence scoring equals the per-char sum (今 first, then 天|今, 天|今天)
        let first = model.logp(None, None, '今');
        let seq = model.sequence_logp("今天天");
        assert!((seq - (first + bigram + backed)).abs() < 1e-12);
    }

    #[test]
    fn empty_model_scores_finite() {
        let bytes = gram_bytes(&[], &[]);
        let model = from_gram_bytes(&bytes).unwrap();
        assert_eq!(model.counts(), (0, 0, 0));
        assert_eq!(model.total_weight(), 0);
        assert!(model.sequence_logp("你好").is_finite());
    }

    #[test]
    fn trigram_level_beats_backoff_levels() {
        // hand-built trigram section appended after the spec's two:
        // 今=10 天=30, bigram 今天=8, trigram 今天天=2
        let mut out = gram_bytes(&[("今", 10), ("天", 30)], &[("今天", 8)]);
        out[16] = 1; // header trigram_count: 0 -> 1 (total stays 10+30)
        out.push("今天天".len() as u8);
        out.extend_from_slice("今天天".as_bytes());
        out.push(2);
        let model = from_gram_bytes(&out).unwrap();
        assert_eq!(model.counts(), (2, 1, 1));
        // trigram 今天天=2 over context 今天=8, no backoff penalty
        let tri = model.logp(Some('今'), Some('天'), '天');
        assert!((tri - (2.0_f64 / 8.0).ln()).abs() < 1e-12);
        // unseen trigram and unseen bigram back off to the unigram level
        let back = model.logp(Some('天'), Some('天'), '天');
        assert!((back - (0.16_f64 * 30.0 / 40.0).ln()).abs() < 1e-12);
    }
}
