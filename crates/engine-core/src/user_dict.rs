//! User learning dictionary: pinyin -> hanzi frequency counts.
//!
//! Keys are plain joined keys (lowercase, separators normalized to `'`); no
//! fuzzy rules are applied here. Frequencies start at 1 on first selection
//! and grow by 1 per commit, so a couple of manual selections quickly float a
//! phrase to the top of the candidate list.

use std::collections::HashMap;

/// Learned `(hanzi, freq)` entries per normalized pinyin key,
/// kept sorted by frequency descending (hanzi ascending on ties).
#[derive(Debug, Default, Clone)]
pub struct UserDict {
    entries: HashMap<String, Vec<(String, u32)>>,
}

/// Creates an empty user dictionary.
pub fn new() -> UserDict {
    UserDict::default()
}

/// Records one use of `hanzi` under `pinyin`. The key is normalized
/// (lowercased, `'`/space separators collapsed to `'`); the frequency is
/// incremented (inserted as 1 when unseen) and the entry list re-sorted.
pub fn learn(u: &mut UserDict, pinyin: &str, hanzi: &str) {
    let key = normalize_pinyin(pinyin);
    let vals = u.entries.entry(key).or_default();
    match vals.iter_mut().find(|(h, _)| h == hanzi) {
        Some((_, f)) => *f += 1,
        None => vals.push((hanzi.to_string(), 1)),
    }
    vals.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
}

/// Returns a clone of the entries stored for `key` (empty when unknown).
pub fn lookup(u: &UserDict, key: &str) -> Vec<(String, u32)> {
    u.entries.get(key).cloned().unwrap_or_default()
}

/// Number of distinct learned keys.
pub fn len(u: &UserDict) -> usize {
    u.entries.len()
}

/// True when nothing has been learned yet.
pub fn is_empty(u: &UserDict) -> bool {
    u.entries.is_empty()
}

/// Lowercases and normalizes separators to `'`.
fn normalize_pinyin(pinyin: &str) -> String {
    pinyin
        .to_lowercase()
        .split(['\'', ' '])
        .filter(|syl| !syl.is_empty())
        .collect::<Vec<_>>()
        .join("'")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn learn_increments_and_orders() {
        let mut u = new();
        assert!(is_empty(&u));
        learn(&mut u, "ni'hao", "你好");
        learn(&mut u, "ni hao", "泥嚎"); // space separator normalizes too
        learn(&mut u, "NI'HAO", "你好"); // case-insensitive
        assert_eq!(len(&u), 1);
        let vals = lookup(&u, "ni'hao");
        assert_eq!(vals[0], ("你好".to_string(), 2));
        assert_eq!(vals[1], ("泥嚎".to_string(), 1));
    }

    #[test]
    fn higher_freq_sorts_first() {
        let mut u = new();
        learn(&mut u, "go", "乙");
        learn(&mut u, "go", "乙");
        learn(&mut u, "go", "甲");
        let vals = lookup(&u, "go");
        assert_eq!(vals[0], ("乙".to_string(), 2));
        assert_eq!(vals[1], ("甲".to_string(), 1));
    }

    #[test]
    fn unknown_key_is_empty() {
        let mut u = new();
        learn(&mut u, "ni", "你");
        assert!(lookup(&u, "hao").is_empty());
        assert!(lookup(&u, "").is_empty());
        assert_eq!(len(&u), 1);
        assert!(!is_empty(&u));
    }
}
