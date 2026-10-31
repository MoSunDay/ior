//! Static pinyin-to-hanzi dictionary.
//!
//! A [`Dictionary`] stores one map from a normalized pinyin key (syllables
//! joined by `'`, each syllable fuzzy-normalized with the [`FuzzyConfig`]
//! captured at build time) to `(hanzi, freq)` entries sorted by frequency
//! descending, then hanzi ascending. Because the fuzzy rules are baked into
//! the keys *and* [`Dictionary::lookup`] normalizes its query with the same
//! remembered config, callers never need to apply fuzzy rules themselves.

use crate::config::{normalize_syllable, FuzzyConfig};
use std::collections::HashMap;

/// Pinyin key -> `(hanzi, freq)` entries, freq desc / hanzi asc.
#[derive(Debug, Clone)]
pub struct Dictionary {
    entries: HashMap<String, Vec<(String, u32)>>,
    fuzzy: FuzzyConfig,
}

impl Dictionary {
    /// Parses TSV text: `pinyin<TAB>hanzi<TAB>freq` per line. The frequency
    /// field is optional (default 1). Blank lines and lines starting with
    /// `#` are skipped. Pinyin separators may be `'` or spaces (normalized
    /// to `'`); everything is lowercased and each syllable is fuzzy-normalized
    /// with `fuzzy`. Duplicate `(key, hanzi)` pairs have their frequencies
    /// summed. Returns `Err` with the 1-based line number on malformed lines.
    pub fn from_tsv_with(text: &str, fuzzy: &FuzzyConfig) -> Result<Dictionary, String> {
        let mut entries: HashMap<String, Vec<(String, u32)>> = HashMap::new();
        for (idx, line) in text.lines().enumerate() {
            let lineno = idx + 1;
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let mut fields = trimmed.split('\t');
            let pinyin = fields.next().unwrap_or("");
            let hanzi = fields.next().unwrap_or("");
            let freq = match fields.next() {
                None => 1,
                Some(raw) => raw
                    .parse::<u32>()
                    .map_err(|_| format!("line {lineno}: invalid freq '{raw}'"))?,
            };
            if pinyin.is_empty() || hanzi.is_empty() || fields.next().is_some() {
                return Err(format!(
                    "line {lineno}: expected 'pinyin\\thanzi[\\tfreq]', got '{trimmed}'"
                ));
            }
            let key = normalize_key(fuzzy, pinyin);
            let vals = entries.entry(key).or_default();
            match vals.iter_mut().find(|(h, _)| h == hanzi) {
                Some((_, f)) => *f += freq,
                None => vals.push((hanzi.to_string(), freq)),
            }
        }
        for vals in entries.values_mut() {
            sort_values(vals);
        }
        Ok(Dictionary {
            entries,
            fuzzy: *fuzzy,
        })
    }

    /// The builtin core dictionary (`assets/core.tsv`).
    pub fn builtin(fuzzy: &FuzzyConfig) -> Dictionary {
        Self::from_tsv_with(include_str!("../assets/core.tsv"), fuzzy)
            .expect("builtin core.tsv must be well-formed")
    }

    /// Looks up a joined key (`ni'hao`, `xian`, ...). The key is normalized
    /// with the config remembered at build time before the actual map probe,
    /// so both raw and pre-normalized keys work (normalization is idempotent).
    pub fn lookup(&self, key: &str) -> Option<&[(String, u32)]> {
        self.entries
            .get(&normalize_key(&self.fuzzy, key))
            .map(|v| v.as_slice())
    }

    /// Number of distinct keys.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when there are no keys at all.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Iterates over `(key, entries)` pairs in unspecified order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &[(String, u32)])> + '_ {
        self.entries.iter().map(|(k, v)| (k.as_str(), v.as_slice()))
    }

    /// Unions `other` into `self`, summing frequencies of identical
    /// `(key, hanzi)` pairs and re-sorting. Assumes both dictionaries were
    /// built with the same [`FuzzyConfig`] (fuzziness is part of the key).
    pub fn merge(&mut self, other: &Dictionary) {
        for (key, vals) in other.entries.iter() {
            let entry = self.entries.entry(key.clone()).or_default();
            for (hanzi, freq) in vals {
                match entry.iter_mut().find(|(h, _)| h == hanzi) {
                    Some((_, f)) => *f += *freq,
                    None => entry.push((hanzi.clone(), *freq)),
                }
            }
            sort_values(entry);
        }
    }
}

/// Sorts entries by frequency descending, then hanzi ascending.
fn sort_values(vals: &mut [(String, u32)]) {
    vals.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
}

/// Builds a dictionary key: lowercase, separators (`'` or space) normalized
/// to `'`, each syllable fuzzy-normalized.
fn normalize_key(fuzzy: &FuzzyConfig, key: &str) -> String {
    key.to_lowercase()
        .split(['\'', ' '])
        .filter(|syl| !syl.is_empty())
        .map(|syl| normalize_syllable(fuzzy, syl))
        .collect::<Vec<_>>()
        .join("'")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_fuzzy() -> FuzzyConfig {
        FuzzyConfig::default()
    }

    #[test]
    fn parses_basic_entries() {
        let d = Dictionary::from_tsv_with("ni\t你\t9\nni\t尼\t1\n", &default_fuzzy()).unwrap();
        let vals = d.lookup("ni").unwrap();
        assert_eq!(&vals[0], &("你".to_string(), 9));
        assert_eq!(&vals[1], &("尼".to_string(), 1));
        assert_eq!(d.len(), 1);
        assert!(!d.is_empty());
    }

    #[test]
    fn missing_tab_is_an_error_with_line_number() {
        let err = Dictionary::from_tsv_with("niyou\n", &default_fuzzy()).unwrap_err();
        assert!(err.contains("line 1"), "unexpected error: {err}");
        let err =
            Dictionary::from_tsv_with("ni\t你\nbroken line here\n", &default_fuzzy()).unwrap_err();
        assert!(err.contains("line 2"), "unexpected error: {err}");
    }

    #[test]
    fn invalid_freq_is_an_error() {
        let err = Dictionary::from_tsv_with("ni\t你\tx\n", &default_fuzzy()).unwrap_err();
        assert!(err.contains("line 1"), "unexpected error: {err}");
    }

    #[test]
    fn comments_and_blanks_skipped_freq_defaults_to_one() {
        let text = "# comment\n\n   \nni\t你\n";
        let d = Dictionary::from_tsv_with(text, &default_fuzzy()).unwrap();
        assert_eq!(d.len(), 1);
        assert_eq!(&d.lookup("ni").unwrap()[0], &("你".to_string(), 1));
    }

    #[test]
    fn space_separator_accepted_and_duplicates_summed() {
        let text = "ni hao\t你好\t5\nni hao\t你好\t3\nNI HAO\t你好\t2\n";
        let d = Dictionary::from_tsv_with(text, &default_fuzzy()).unwrap();
        assert_eq!(&d.lookup("ni'hao").unwrap()[0], &("你好".to_string(), 10));
    }

    #[test]
    fn values_sorted_freq_desc_then_hanzi_asc() {
        let text = "a\t甲\t5\na\t乙\t9\na\t丙\t9\n";
        let d = Dictionary::from_tsv_with(text, &default_fuzzy()).unwrap();
        let vals = d.lookup("a").unwrap();
        assert_eq!(vals[0].0, "丙");
        assert_eq!(vals[1].0, "乙");
        assert_eq!(vals[2].0, "甲");
        assert_eq!(vals[2].1, 5);
    }

    #[test]
    fn lookup_miss_returns_none() {
        let d = Dictionary::from_tsv_with("ni\t你\n", &default_fuzzy()).unwrap();
        assert!(d.lookup("xyz").is_none());
        assert!(d.lookup("").is_none());
        assert!(d.lookup("ni'hao").is_none());
    }

    #[test]
    fn fuzzy_keys_and_lookup_agree() {
        let f = FuzzyConfig {
            n_l: true,
            ..FuzzyConfig::default()
        };
        let d = Dictionary::from_tsv_with("li\t里\t5\nni\t你\t9\n", &f).unwrap();
        // both stored under the normalized key, reachable from either spelling
        let vals = d.lookup("li").unwrap();
        assert_eq!(vals[0].0, "你");
        assert_eq!(vals[1].0, "里");
        let vals = d.lookup("ni").unwrap();
        assert_eq!(vals.len(), 2);
    }

    #[test]
    fn merge_sums_and_preserves_missing() {
        let a = Dictionary::from_tsv_with("ni\t你\t5\nshi\t是\t7\n", &default_fuzzy()).unwrap();
        let b = Dictionary::from_tsv_with("ni\t你\t3\nni\t尼\t1\nhao\t好\t2\n", &default_fuzzy())
            .unwrap();
        let mut a = a;
        a.merge(&b);
        let ni = a.lookup("ni").unwrap();
        assert_eq!(&ni[0], &("你".to_string(), 8));
        assert_eq!(&ni[1], &("尼".to_string(), 1));
        assert_eq!(a.lookup("shi").unwrap()[0].1, 7);
        assert_eq!(a.lookup("hao").unwrap()[0].1, 2);
        assert_eq!(a.len(), 3);
    }

    #[test]
    fn builtin_loads_and_answers_core_queries() {
        let d = Dictionary::builtin(&default_fuzzy());
        assert!(d.len() >= 60);
        assert_eq!(d.lookup("ni'hao").unwrap()[0].0, "你好");
        assert!(d.lookup("xian").unwrap().iter().any(|(h, _)| h == "先"));
        assert!(d.lookup("xi'an").unwrap()[0].0 == "西安");
        let iterated = d.iter().count();
        assert_eq!(iterated, d.len());
    }
}
