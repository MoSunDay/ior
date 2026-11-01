//! `.gram` — character n-gram language model training and (de)serialization
//! (P2/M2b). The authoritative format spec lives in
//! `engine-core/src/lm.rs`; this module is the writer (`train` +
//! [`GramCounts::to_gram_bytes`]) and a validating reader ([`parse_gram`])
//! used by `lminspect`. Both ends are pinned byte-for-byte by golden tests
//! here and by `crates/bench/tests/gram_interlock.rs` across crates.
//!
//! Training signal: the only offline corpus available is the dictionary
//! itself, so counts are **within-word** character n-grams weighted by word
//! frequency (an entry `jin'tian<TAB>今天<TAB>4500` adds 4500 to the
//! unigrams 今/天 and the bigram 今天). Corpus-level models are future
//! data (see the P2 row of `docs/ROADMAP-RIME-PARITY.md`).
//!
//! dict-tools deliberately does not depend on engine-core; the constants
//! below mirror `engine-core::lm` and the interlock test fails if either
//! side drifts.

use std::collections::BTreeMap;

use crate::format::Entry;
use crate::varint::{read_u32, write_u32};

/// Magic bytes at the start of every `.gram` file (mirrors engine-core).
pub const GRAM_MAGIC: &[u8; 6] = b"IOGRAM";

/// `.gram` format version (mirrors engine-core).
pub const GRAM_VERSION: u8 = 1;

/// Model order: 3 = character trigram (mirrors engine-core).
pub const GRAM_ORDER: u8 = 3;

/// Header size in bytes: magic(6) + version(1) + order(1) + 4 x u32 LE.
const HEADER_BYTES: usize = 24;

/// Ceiling applied to per-key counts when serialized (varints are u32).
const COUNT_MAX: u64 = u32::MAX as u64;

/// Character n-gram counts ready to serialize as `.gram`.
///
/// Keys are utf8 strings of exactly 1/2/3 chars, kept in `BTreeMap`s so
/// iteration is already in the on-disk order (ascending utf8 bytes =
/// code-point order). Values accumulate as u64 and saturate at
/// [`COUNT_MAX`] on write.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GramCounts {
    unigrams: BTreeMap<String, u64>,
    bigrams: BTreeMap<String, u64>,
    trigrams: BTreeMap<String, u64>,
}

/// Distinct unigram keys (ascending).
pub fn unigrams(counts: &GramCounts) -> &BTreeMap<String, u64> {
    &counts.unigrams
}

/// Distinct bigram keys (ascending).
pub fn bigrams(counts: &GramCounts) -> &BTreeMap<String, u64> {
    &counts.bigrams
}

/// Distinct trigram keys (ascending).
pub fn trigrams(counts: &GramCounts) -> &BTreeMap<String, u64> {
    &counts.trigrams
}

/// Sum of all unigram counts (saturating u32 on serialization).
pub fn total_weight(counts: &GramCounts) -> u64 {
    counts.unigrams.values().sum()
}

/// Accumulate within-word character n-grams from dictionary entries.
///
/// Every 1/2/3-char window of each entry's hanzi gains the entry's
/// frequency; windows never cross word boundaries. Duplicates must be
/// normalized away by the caller ([`crate::format::normalize`]) or they
/// simply count twice, mirroring how word frequency would double-count.
pub fn train_counts(entries: &[Entry]) -> GramCounts {
    let mut counts = GramCounts::default();
    for entry in entries {
        let weight = u64::from(entry.freq);
        let chars: Vec<char> = entry.hanzi.chars().collect();
        for start in 0..chars.len() {
            for order in 1..=3 {
                let Some(window) = chars.get(start..start + order) else {
                    break;
                };
                let key: String = window.iter().collect();
                let slot = match order {
                    1 => &mut counts.unigrams,
                    2 => &mut counts.bigrams,
                    _ => &mut counts.trigrams,
                };
                *slot.entry(key).or_insert(0) += weight;
            }
        }
    }
    counts
}

impl GramCounts {
    /// Serialize to `.gram` v1 bytes (deterministic: same counts, same
    /// bytes — the maps are already key-sorted).
    pub fn to_gram_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(GRAM_MAGIC);
        out.push(GRAM_VERSION);
        out.push(GRAM_ORDER);
        for count in [
            self.unigrams.len() as u32,
            self.bigrams.len() as u32,
            self.trigrams.len() as u32,
        ] {
            out.extend_from_slice(&count.to_le_bytes());
        }
        out.extend_from_slice(&saturate(total_weight(self)).to_le_bytes());
        for section in [&self.unigrams, &self.bigrams, &self.trigrams] {
            for key in section.keys() {
                write_u32(&mut out, key.len() as u32);
                out.extend_from_slice(key.as_bytes());
            }
            for value in section.values() {
                write_u32(&mut out, saturate(*value));
            }
        }
        out
    }
}

/// Parse and fully validate `.gram` v1 bytes (the `lminspect` reader).
///
/// Rejects the same things the engine reader rejects: bad magic, wrong
/// version/order, header/body count disagreement, mis-ordered or
/// mis-sized keys, invalid utf8, truncated fields, trailing bytes, and a
/// total-weight mismatch.
pub fn parse_gram(bytes: &[u8]) -> Result<GramCounts, String> {
    if bytes.get(0..GRAM_MAGIC.len()).unwrap_or(&[]) != GRAM_MAGIC {
        return Err(format!(
            "bad magic: expected {GRAM_MAGIC:?} at offset 0, buffer is {} byte(s)",
            bytes.len()
        ));
    }
    if bytes.get(6) != Some(&GRAM_VERSION) {
        return Err(format!(
            "unsupported version {:?} (want {GRAM_VERSION})",
            bytes.get(6)
        ));
    }
    if bytes.get(7) != Some(&GRAM_ORDER) {
        return Err(format!(
            "unsupported order {:?} (want {GRAM_ORDER})",
            bytes.get(7)
        ));
    }
    if bytes.len() < HEADER_BYTES {
        return Err(format!(
            "truncated header: need {HEADER_BYTES} byte(s), buffer is {} byte(s)",
            bytes.len()
        ));
    }
    let read_count =
        |pos: usize| u32::from_le_bytes(bytes[pos..pos + 4].try_into().expect("slice is 4 bytes"));
    let uni_count = read_count(8) as usize;
    let bi_count = read_count(12) as usize;
    let tri_count = read_count(16) as usize;
    let total = read_count(20);
    let mut pos = HEADER_BYTES;
    let mut counts = GramCounts::default();
    for (order, count) in [(1usize, uni_count), (2, bi_count), (3, tri_count)] {
        let (section, next) = parse_section(bytes, pos, count, order)?;
        pos = next;
        match order {
            1 => counts.unigrams = section,
            2 => counts.bigrams = section,
            _ => counts.trigrams = section,
        }
    }
    if pos != bytes.len() {
        return Err(format!(
            "trailing data: {} byte(s) after the trigram section (offset {pos}, buffer is {} byte(s))",
            bytes.len() - pos,
            bytes.len()
        ));
    }
    let sum = total_weight(&counts);
    if sum != u64::from(total) {
        return Err(format!(
            "total weight mismatch: header says {total}, unigram counts sum to {sum}"
        ));
    }
    Ok(counts)
}

/// Parse one section body: `count` index records then `count` values.
fn parse_section(
    bytes: &[u8],
    mut pos: usize,
    count: usize,
    order: usize,
) -> Result<(BTreeMap<String, u64>, usize), String> {
    let mut keys: Vec<String> = Vec::with_capacity(count);
    for no in 0..count {
        let (len, next) =
            read_u32(bytes, pos).map_err(|err| format!("{order}-gram key {no}: {err}"))?;
        if len == 0 || len as usize > bytes.len() {
            return Err(format!("{order}-gram key {no}: bad length {len}"));
        }
        pos = next;
        let end = pos + len as usize;
        let raw = bytes.get(pos..end).ok_or_else(|| {
            format!(
                "{order}-gram key {no}: truncated (offset {pos}, buffer is {} byte(s))",
                bytes.len()
            )
        })?;
        let key = std::str::from_utf8(raw)
            .map_err(|_| format!("{order}-gram key {no}: invalid utf8 at offset {pos}"))?;
        if key.chars().count() != order {
            return Err(format!(
                "{order}-gram key {no} {key:?}: expected {order} char(s), found {}",
                key.chars().count()
            ));
        }
        if let Some(last) = keys.last() {
            if key.as_bytes() <= last.as_bytes() {
                return Err(format!(
                    "{order}-gram key {no} {key:?}: keys must be distinct and ascending (previous {last:?})"
                ));
            }
        }
        keys.push(key.to_string());
        pos = end;
    }
    let mut section = BTreeMap::new();
    for key in keys {
        let (value, next) =
            read_u32(bytes, pos).map_err(|err| format!("{order}-gram value {key:?}: {err}"))?;
        section.insert(key, u64::from(value));
        pos = next;
    }
    Ok((section, pos))
}

/// Saturating u64 -> u32.
fn saturate(value: u64) -> u32 {
    value.min(COUNT_MAX) as u32
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

    /// Golden bytes for 你好=8, 金=3: 24-byte header, three sorted unigram
    /// keys, their values, then the lone bigram. Any format change on
    /// either side of the crates must update this and the interlock test.
    #[test]
    fn golden_bytes() {
        let counts = train_counts(&[entry("ni'hao", "你好", 8), entry("jin", "金", 3)]);
        let bytes = counts.to_gram_bytes();
        let mut want = Vec::new();
        want.extend_from_slice(b"IOGRAM");
        want.extend_from_slice(&[GRAM_VERSION, GRAM_ORDER]);
        want.extend_from_slice(&3u32.to_le_bytes());
        want.extend_from_slice(&1u32.to_le_bytes());
        want.extend_from_slice(&0u32.to_le_bytes());
        want.extend_from_slice(&19u32.to_le_bytes()); // 8 + 8 + 3
        want.extend_from_slice(&[3]);
        want.extend_from_slice("你".as_bytes());
        want.extend_from_slice(&[3]);
        want.extend_from_slice("好".as_bytes());
        want.extend_from_slice(&[3]);
        want.extend_from_slice("金".as_bytes());
        want.extend_from_slice(&[8, 8, 3]); // unigram values, index order
        want.extend_from_slice(&[6]);
        want.extend_from_slice("你好".as_bytes());
        want.extend_from_slice(&[8]); // bigram value
        assert_eq!(bytes, want);
        assert_eq!(bytes.len(), 47);
        let back = parse_gram(&bytes).expect("round trip");
        assert_eq!(back, counts);
    }

    #[test]
    fn windows_never_cross_word_boundaries() {
        let counts = train_counts(&[entry("a", "今天", 10), entry("b", "天", 5)]);
        assert_eq!(unigrams(&counts)["今"], 10);
        assert_eq!(unigrams(&counts)["天"], 15);
        assert_eq!(bigrams(&counts)["今天"], 10);
        assert!(trigrams(&counts).is_empty());
        assert_eq!(total_weight(&counts), 25);
    }

    #[test]
    fn trigram_counts_every_window() {
        let counts = train_counts(&[entry("a'b'c", "今天天", 2)]);
        assert_eq!(unigrams(&counts)["今"], 2);
        assert_eq!(bigrams(&counts)["天天"], 2);
        assert_eq!(trigrams(&counts)["今天天"], 2);
        assert!(trigrams(&counts).get("天天今").is_none());
    }

    #[test]
    fn counts_saturate_instead_of_overflowing() {
        let big = u32::MAX;
        let counts = train_counts(&[entry("a", "今天", big), entry("b", "今天", 5)]);
        assert_eq!(unigrams(&counts)["今"], u64::from(big) + 5);
        // total saturates at u32::MAX inside the header
        let bytes = counts.to_gram_bytes();
        let total = u32::from_le_bytes(bytes[20..24].try_into().expect("4 bytes"));
        assert_eq!(total, u32::MAX);
    }

    #[test]
    fn parse_rejects_damage() {
        let counts = train_counts(&[entry("ni'hao", "你好", 8), entry("jin", "金", 3)]);
        let good = counts.to_gram_bytes();

        let mut bad_magic = good.clone();
        bad_magic[0] = b'X';
        assert!(parse_gram(&bad_magic).is_err());

        let mut bad_total = good.clone();
        bad_total[20] ^= 1;
        assert!(parse_gram(&bad_total).is_err());

        let truncated = &good[..good.len() - 1];
        assert!(parse_gram(truncated).is_err());

        let mut trailing = good.clone();
        trailing.push(0);
        assert!(parse_gram(&trailing).is_err());

        assert!(parse_gram(&[]).is_err());
    }

    #[test]
    fn empty_model_serializes_as_header_only() {
        let counts = train_counts(&[]);
        let bytes = counts.to_gram_bytes();
        assert_eq!(bytes.len(), HEADER_BYTES);
        assert_eq!(parse_gram(&bytes).expect("round trip"), counts);
    }
}
