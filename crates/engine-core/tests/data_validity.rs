//! Data validity: every builtin dictionary key must decompose into valid
//! pinyin syllables and every entry list must be sorted by frequency.
//! Guards against regressions when assets/core.tsv is edited or extended.

use engine_core::{is_syllable, Dictionary};

#[test]
fn builtin_keys_are_valid_syllable_sequences() {
    let dict = Dictionary::builtin();
    assert!(!dict.is_empty());
    let mut checked = 0usize;
    for (key, entries) in dict.iter() {
        assert!(!entries.is_empty(), "empty entries for key {key:?}");
        for syl in key.split('\'') {
            assert!(
                is_syllable(syl),
                "invalid syllable {syl:?} in dictionary key {key:?}"
            );
        }
        checked += 1;
    }
    assert!(
        checked >= 60,
        "builtin dictionary unexpectedly small: {checked}"
    );
}

#[test]
fn builtin_entries_sorted_by_frequency() {
    let dict = Dictionary::builtin();
    for (key, entries) in dict.iter() {
        let freqs: Vec<u32> = entries.iter().map(|(_, f)| *f).collect();
        let mut sorted = freqs.clone();
        sorted.sort_by(|a, b| b.cmp(a));
        assert_eq!(freqs, sorted, "key {key:?} is not freq-desc");
    }
}
