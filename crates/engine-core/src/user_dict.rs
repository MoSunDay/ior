//! User learning dictionary: pinyin -> hanzi frequency counts.
//!
//! Keys are plain joined keys (lowercase, separators normalized to `'`); no
//! fuzzy rules are applied here. Frequencies start at 1 on first selection
//! and grow by 1 per commit, so a couple of manual selections quickly float a
//! phrase to the top of the candidate list.
//!
//! # Persistence, merging and aging
//!
//! The text formats live in [`crate::user_dict_store`] and are re-exported
//! here so callers get the whole userdb surface as `user_dict::*`:
//! deterministic snapshots ([`to_snapshot`]/[`from_snapshot`]), an
//! append-only learn log ([`log_line`]/[`replay`]), compaction
//! ([`compact`]), cloud-backup aliases ([`export_tsv`]/[`import_tsv`]),
//! history merging ([`absorb`]) and time-based aging ([`decay`]).
//! engine-core itself never touches files: it only produces and consumes
//! bytes; reading/writing them belongs to the host (addon) layer.

use std::collections::HashMap;

pub use crate::user_dict_store::{
    compact, export_tsv, from_snapshot, import_tsv, log_line, replay, to_snapshot, SNAPSHOT_HEADER,
};

/// Learned `(hanzi, freq)` entries per normalized pinyin key,
/// kept sorted by frequency descending (hanzi ascending on ties).
#[derive(Debug, Default, Clone)]
pub struct UserDict {
    pub(crate) entries: HashMap<String, Vec<(String, u32)>>,
}

/// Creates an empty user dictionary.
pub fn new() -> UserDict {
    UserDict::default()
}

/// Records one use of `hanzi` under `pinyin`. The key is normalized
/// (lowercased, `'`/space separators collapsed to `'`); the frequency is
/// incremented (inserted as 1 when unseen) and the entry list re-sorted.
pub fn learn(u: &mut UserDict, pinyin: &str, hanzi: &str) {
    add(u, &normalize_pinyin(pinyin), hanzi, 1);
}

/// Adds `freq` to the `(key, hanzi)` entry (inserting it when unseen,
/// saturating on overflow) and restores the sort invariant. `key` must
/// already be normalized; shared by [`learn`], [`absorb`], snapshot
/// parsing and compaction.
pub(crate) fn add(u: &mut UserDict, key: &str, hanzi: &str, freq: u32) {
    let vals = u.entries.entry(key.to_string()).or_default();
    match vals.iter_mut().find(|(h, _)| h == hanzi) {
        Some((_, f)) => *f = f.saturating_add(freq),
        None => vals.push((hanzi.to_string(), freq)),
    }
    sort_entries(vals);
}

/// Merges `other` into `u` with learn semantics: matching entries have
/// their frequencies added, entries unknown to `u` are inserted with their
/// carried frequency, and the sort invariant is kept. Hosts use this to
/// layer a loaded history file on top of the live dictionary (`u` and
/// `other` must be distinct dictionaries — the types make aliasing
/// unrepresentable at the call site).
pub fn absorb(u: &mut UserDict, other: &UserDict) {
    for (key, vals) in &other.entries {
        for (hanzi, freq) in vals {
            add(u, key, hanzi, *freq);
        }
    }
}

/// Ages every frequency towards zero by whole halvings of the half-life:
/// `halvings = elapsed_secs / half_life_secs` (capped at 31 shifts), each
/// frequency is shifted right by that many bits, entries that reach zero
/// are dropped, and keys left empty disappear. Sorting inside each
/// surviving key is restored. `half_life_secs == 0` or `elapsed_secs == 0`
/// changes nothing (a zero half-life is defined as "no decay").
///
/// The half-life is quantized to whole halvings: any elapsed time below one
/// half-life leaves frequencies untouched, and the aging itself is pure
/// integer arithmetic (no floating point), so it is exact and
/// deterministic.
pub fn decay(u: &mut UserDict, elapsed_secs: u64, half_life_secs: u64) {
    if half_life_secs == 0 || elapsed_secs == 0 {
        return;
    }
    let halvings = (elapsed_secs / half_life_secs).min(31) as u32;
    let mut dead_keys: Vec<String> = Vec::new();
    for (key, vals) in u.entries.iter_mut() {
        for (_, freq) in vals.iter_mut() {
            *freq >>= halvings;
        }
        vals.retain(|(_, freq)| *freq > 0);
        if vals.is_empty() {
            dead_keys.push(key.clone());
        } else {
            sort_entries(vals);
        }
    }
    for key in dead_keys {
        u.entries.remove(&key);
    }
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

/// Sorts one key's entries by the invariant order: frequency descending,
/// hanzi ascending on ties.
pub(crate) fn sort_entries(vals: &mut [(String, u32)]) {
    vals.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
}

/// Lowercases and normalizes separators to `'`.
pub(crate) fn normalize_pinyin(pinyin: &str) -> String {
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

    #[test]
    fn absorb_sums_and_inserts_entries() {
        let mut u = new();
        learn(&mut u, "ni'hao", "你好");
        let mut hist = new();
        learn(&mut hist, "ni'hao", "你好");
        learn(&mut hist, "ni'hao", "泥嚎");
        learn(&mut hist, "ni'hao", "泥嚎");
        learn(&mut hist, "go", "够");
        absorb(&mut u, &hist);
        // 你好 1+1=2, 泥嚎 carried in at 2; the tie sorts hanzi ascending.
        assert_eq!(lookup(&u, "ni'hao")[0], ("你好".to_string(), 2));
        assert_eq!(lookup(&u, "ni'hao")[1], ("泥嚎".to_string(), 2));
        assert_eq!(lookup(&u, "go"), vec![("够".to_string(), 1)]);
        // Absorb is pure merging: the source dictionary is untouched.
        assert_eq!(lookup(&hist, "ni'hao").len(), 2);
    }

    #[test]
    fn decay_halves_once_and_drops_zero_entries() {
        let mut u = new();
        for _ in 0..5 {
            learn(&mut u, "k", "五");
        }
        learn(&mut u, "k", "一"); // 1 >> 1 == 0 -> dropped
        learn(&mut u, "solo", "独"); // key left empty -> key disappears
        decay(&mut u, 86_400, 86_400); // one half-life elapsed
        assert_eq!(lookup(&u, "k"), vec![("五".to_string(), 2)]);
        assert!(lookup(&u, "solo").is_empty());
        assert_eq!(len(&u), 1);
    }

    #[test]
    fn decay_applies_whole_halvings_only() {
        let mut u = new();
        for _ in 0..100 {
            learn(&mut u, "k", "百");
        }
        for _ in 0..8 {
            learn(&mut u, "k", "八");
        }
        // Five half-lives: 100 -> 3, 8 -> 0 (dropped). Re-sort by freq.
        decay(&mut u, 5 * 60, 60);
        assert_eq!(lookup(&u, "k"), vec![("百".to_string(), 3)]);
        // Below one half-life: quantized to zero halvings, nothing changes.
        let mut v = new();
        learn(&mut v, "k", "一");
        decay(&mut v, 59, 60);
        assert_eq!(lookup(&v, "k"), vec![("一".to_string(), 1)]);
    }

    #[test]
    fn decay_with_zero_half_life_or_elapsed_is_noop() {
        let mut u = new();
        learn(&mut u, "k", "一");
        decay(&mut u, 10_000, 0); // half_life 0 is legal: no decay
        assert_eq!(lookup(&u, "k"), vec![("一".to_string(), 1)]);
        decay(&mut u, 0, 1); // nothing elapsed
        assert_eq!(lookup(&u, "k"), vec![("一".to_string(), 1)]);
        // 31+ halvings still saturate safely instead of shifting UB-ish.
        decay(&mut u, u64::MAX, 1);
        assert!(is_empty(&u));
    }
}
