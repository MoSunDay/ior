//! Rime preset vocabulary (usually `essay.txt`) import.
//!
//! Rime's `luna_pinyin` dictionary only ships characters and uncommon
//! words; everyday phrases come from a separate vocabulary file whose
//! lines are `phrase<TAB>weight` (no pinyin). At deployment time Rime
//! encodes each phrase by looking up per-character readings collected
//! from the dictionary, keeping only readings at least 5% of that
//! character's total weight, and enumerating up to 32 code combinations
//! (longest-match first).
//!
//! This module reproduces that encoding as pure functions:
//! [`PresetVocab::parse`] reads the text, [`PresetVocab::encode`] turns
//! it into ordinary [`Entry`] rows. The dictionary weight semantics live
//! in `source.rs`; both use [`WEIGHT_SCALE`] to keep Rime's fractional
//! weights (percentages of small essay weights) in our integer format.

use crate::format::Entry;
use std::collections::{BTreeMap, BTreeSet};

/// Fixed-point scale applied to Rime weights when a preset vocabulary is
/// active. Rime stores `double`; we multiply by 100 so `1%` of a weight-1
/// character stays representable (`1`), while the largest essay weight
/// (about 4.8 million) still fits comfortably in `u32`.
pub const WEIGHT_SCALE: u32 = 100;

/// Rime's `kEncoderDfsLimit`: at most this many codes per phrase.
const DFS_LIMIT: usize = 32;

/// Rime's `kMaxPhraseLength`: hard cap even when the dict asks for more.
const MAX_PHRASE_LEN: usize = 32;

/// Rime's `kMinimalWeight` in `EntryCollector::TranslateWord`: readings
/// below 5% of a character's total weight are not used to encode phrases.
const READING_MIN_RATIO: f64 = 0.05;

/// Parsed `essay.txt`: phrase -> raw (unscaled) weight.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PresetVocab {
    weights: BTreeMap<String, u32>,
}

impl PresetVocab {
    /// Parses `phrase<TAB>weight` lines. Blank lines and `#` comments are
    /// ignored; tabs are preferred, whitespace is accepted as a fallback.
    /// Duplicate phrases keep their maximum weight. Errors carry 1-based
    /// line numbers.
    pub fn parse(text: &str) -> Result<PresetVocab, String> {
        let mut weights: BTreeMap<String, u32> = BTreeMap::new();
        for (index, raw_line) in text.lines().enumerate() {
            let lineno = index + 1;
            let trimmed = raw_line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let fields: Vec<&str> = if raw_line.contains('\t') {
                raw_line.split('\t').map(str::trim).collect()
            } else {
                raw_line.split_whitespace().collect()
            };
            let Some(phrase) = fields.first().filter(|field| !field.is_empty()) else {
                return Err(format!("line {lineno}: empty phrase"));
            };
            let weight = match fields.get(1) {
                Some(field) if !field.is_empty() => parse_weight(field, lineno)?,
                _ => 0,
            };
            let entry = weights.entry((*phrase).to_string()).or_insert(0);
            *entry = (*entry).max(weight);
        }
        Ok(PresetVocab { weights })
    }

    /// Raw essay weight for `phrase`, if present (may be 0).
    pub fn weight(&self, phrase: &str) -> Option<u32> {
        self.weights.get(phrase).copied()
    }

    /// Number of distinct phrases.
    pub fn len(&self) -> usize {
        self.weights.len()
    }

    /// True when no phrases were parsed.
    pub fn is_empty(&self) -> bool {
        self.weights.is_empty()
    }

    /// Encodes essay phrases into dictionary entries using readings from
    /// `base`. Phrases already present in `base` are skipped (Rime's
    /// explicit dictionary entries win); zero-weight phrases are dropped
    /// because `.ior` reserves freq 0 for "absent".
    pub fn encode(
        &self,
        base: &[Entry],
        max_phrase_length: usize,
        min_phrase_weight: f64,
    ) -> Vec<Entry> {
        let readings = readings_index(base);
        let explicit: BTreeSet<&str> = base.iter().map(|entry| entry.hanzi.as_str()).collect();
        let max_len = effective_max_len(max_phrase_length);
        let mut out = Vec::new();
        for (phrase, weight) in &self.weights {
            if *weight == 0 || f64::from(*weight) < min_phrase_weight {
                continue;
            }
            if explicit.contains(phrase.as_str()) {
                continue;
            }
            let chars: Vec<char> = phrase.chars().collect();
            if chars.len() > max_len {
                continue;
            }
            let mut codes = BTreeSet::new();
            let mut current = Vec::new();
            encode_from(&chars, 0, &readings, &mut current, &mut codes);
            if codes.is_empty() {
                continue;
            }
            let freq = weight.saturating_mul(WEIGHT_SCALE);
            for code in codes {
                out.push(Entry {
                    pinyin: code,
                    hanzi: phrase.clone(),
                    freq,
                });
            }
        }
        out
    }
}

fn parse_weight(field: &str, lineno: usize) -> Result<u32, String> {
    let value = field
        .parse::<f64>()
        .map_err(|err| format!("line {lineno}: invalid vocabulary weight '{field}': {err}"))?;
    if !value.is_finite() || value < 0.0 {
        return Err(format!(
            "line {lineno}: vocabulary weight '{field}' must be finite and >= 0"
        ));
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let rounded = value.round() as u32;
    Ok(rounded)
}

fn effective_max_len(requested: usize) -> usize {
    match requested {
        0 => MAX_PHRASE_LEN,
        n => n.min(MAX_PHRASE_LEN),
    }
}

/// Builds the phrase-encoding reading table: single-syllable entries
/// grouped by hanzi, keeping readings >= 5% of that character's total.
fn readings_index(base: &[Entry]) -> BTreeMap<String, Vec<(String, u32)>> {
    let mut grouped: BTreeMap<String, BTreeMap<String, u32>> = BTreeMap::new();
    for entry in base {
        if entry.pinyin.contains('\'') {
            continue;
        }
        let readings = grouped.entry(entry.hanzi.clone()).or_default();
        let weight = readings.entry(entry.pinyin.clone()).or_insert(0);
        *weight = weight.saturating_add(entry.freq);
    }
    let mut words = BTreeMap::new();
    for (hanzi, readings) in grouped {
        let total: u64 = readings.values().map(|w| u64::from(*w)).sum();
        if total == 0 {
            continue;
        }
        #[allow(clippy::cast_precision_loss)]
        let minimum = total as f64 * READING_MIN_RATIO;
        let mut kept: Vec<(String, u32)> = readings
            .into_iter()
            .filter(|(_, weight)| f64::from(*weight) >= minimum)
            .collect();
        if kept.is_empty() {
            continue;
        }
        kept.sort_by(|a, b| a.0.cmp(&b.0));
        words.insert(hanzi, kept);
    }
    words
}

/// Longest-match DFS in Rime's order: at each position try the longest
/// remaining substring first, then shorter ones, emitting joined codes.
fn encode_from(
    chars: &[char],
    start: usize,
    readings: &BTreeMap<String, Vec<(String, u32)>>,
    current: &mut Vec<String>,
    out: &mut BTreeSet<String>,
) {
    if out.len() >= DFS_LIMIT {
        return;
    }
    if start == chars.len() {
        out.insert(current.join("'"));
        return;
    }
    for len in (1..=chars.len() - start).rev() {
        let word: String = chars[start..start + len].iter().collect();
        let Some(codes) = readings.get(&word) else {
            continue;
        };
        for (code, _) in codes {
            current.push(code.clone());
            encode_from(chars, start + len, readings, current, out);
            current.pop();
            if out.len() >= DFS_LIMIT {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(pinyin: &str, hanzi: &str, freq: u32) -> Entry {
        Entry {
            pinyin: pinyin.to_string(),
            hanzi: hanzi.to_string(),
            freq,
        }
    }

    #[test]
    fn reads_essay_lines_and_skips_comments() {
        let vocab = PresetVocab::parse("# c\n\n今天\t110731\n你好\t0\n坏\tx\n").unwrap_err();
        assert!(vocab.contains("line 5"), "{vocab}");
        let vocab = PresetVocab::parse("# c\n\n今天\t110731\n今天\t9\n零\t0\n").unwrap();
        assert_eq!(vocab.len(), 2);
        assert_eq!(vocab.weight("今天"), Some(110731));
        assert_eq!(vocab.weight("零"), Some(0));
        assert_eq!(vocab.weight("无"), None);
    }

    #[test]
    fn encodes_phrases_with_scaled_weights() {
        let base = vec![
            entry("jin", "今", 100),
            entry("tian", "天", 100),
            entry("qi", "氣", 100),
            entry("hao", "好", 100),
            entry("ni", "你", 100),
        ];
        let vocab = PresetVocab::parse("今天\t500\n天氣\t300\n你好\t200\n无名\t9\n").unwrap();
        let mut extra = vocab.encode(&base, 0, 0.0);
        extra.sort_by(|a, b| a.pinyin.cmp(&b.pinyin).then(a.hanzi.cmp(&b.hanzi)));
        assert_eq!(extra.len(), 3);
        assert_eq!(extra[0].pinyin, "jin'tian");
        assert_eq!(extra[0].freq, 50_000);
        assert_eq!(extra[1].pinyin, "ni'hao");
        assert_eq!(extra[2].pinyin, "tian'qi");
        assert_eq!(extra[2].freq, 30_000);
    }

    #[test]
    fn explicit_phrases_win_and_longest_match_is_used() {
        let base = vec![
            entry("jin", "今", 100),
            entry("tian", "天", 100),
            entry("jin'tian", "今天", 7),
            entry("qi", "氣", 100),
        ];
        let vocab = PresetVocab::parse("今天\t500\n天氣\t300\n天氣預報\t100\n").unwrap();
        let extra = vocab.encode(&base, 3, 0.0);
        assert!(extra.iter().all(|e| e.hanzi != "今天"));
        assert!(extra.iter().all(|e| e.hanzi != "天氣預報"), "too long");
        assert_eq!(extra.len(), 1);
        assert_eq!(extra[0].pinyin, "tian'qi");
    }

    #[test]
    fn filters_rare_readings_before_encoding() {
        let base = vec![
            entry("xing", "行", 95),
            entry("hang", "行", 5),
            entry("zou", "走", 100),
        ];
        let vocab = PresetVocab::parse("行走\t10\n").unwrap();
        let extra = vocab.encode(&base, 0, 0.0);
        // 5% is exactly at Rime's threshold, so both readings are used.
        let codes: Vec<&str> = extra.iter().map(|e| e.pinyin.as_str()).collect();
        assert_eq!(codes, vec!["hang'zou", "xing'zou"]);

        let base = vec![
            entry("xing", "行", 96),
            entry("hang", "行", 4),
            entry("zou", "走", 100),
        ];
        let extra = vocab.encode(&base, 0, 0.0);
        let codes: Vec<&str> = extra.iter().map(|e| e.pinyin.as_str()).collect();
        assert_eq!(codes, vec!["xing'zou"]);
    }

    #[test]
    fn zero_and_minimum_weights_are_dropped() {
        let base = vec![entry("ni", "你", 100), entry("hao", "好", 100)];
        let vocab = PresetVocab::parse("你好\t0\n你好啊\t5\n").unwrap();
        assert!(vocab.encode(&base, 0, 0.0).is_empty());
        let extra = vocab.encode(&base, 0, 6.0);
        assert!(extra.is_empty());
    }
}
