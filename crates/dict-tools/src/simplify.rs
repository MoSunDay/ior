//! Traditional -> simplified conversion for compiled dictionaries.
//!
//! The conversion tables and algorithm live in the shared `t2s` crate
//! (vendored OpenCC data, Apache-2.0); this module keeps the
//! `format::Entry` glue used by `compile --simplify`, which stores
//! simplified hanzi in the resulting `.ior` (and the trained `.gram`).

pub use t2s::{bundled, from_sources, simplify_text, Simplifier};

use crate::format::Entry;

/// Converts every entry in place and returns how many hanzi strings changed.
pub fn simplify_entries(entries: &mut [Entry], simplifier: &Simplifier) -> usize {
    let mut changed = 0;
    for entry in entries {
        let simplified = simplify_text(simplifier, &entry.hanzi);
        if simplified != entry.hanzi {
            entry.hanzi = simplified;
            changed += 1;
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(pinyin: &str, hanzi: &str) -> Entry {
        Entry {
            pinyin: pinyin.to_string(),
            hanzi: hanzi.to_string(),
            freq: 1,
        }
    }

    #[test]
    fn entries_are_rewritten_and_counted() {
        let s = bundled().unwrap();
        let mut entries = vec![entry("tian'qi", "天氣"), entry("ni", "你")];
        assert_eq!(simplify_entries(&mut entries, &s), 1);
        assert_eq!(entries[0].hanzi, "天气");
        assert_eq!(entries[1].hanzi, "你");
    }
}
