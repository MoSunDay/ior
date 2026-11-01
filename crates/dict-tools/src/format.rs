//! Dictionary formats for ior: the TSV source format and the `.ior` binary.
//!
//! TSV: one entry per line, `pinyin<TAB>hanzi<TAB>freq`. `freq` is optional
//! (default 1). `#` comments and blank lines are skipped. Pinyin is stored
//! lowercase with `'` as the single syllable separator.
//!
//! `.ior` binary v2 (current, little-endian): 13-byte header
//! `MAGIC "IORD" | version u8 = 2 | entry_count u32 | key_count u32`, then a
//! key index of `key_count` records `varint(key_len) + key utf8 +
//! varint(values_len)` with keys deduplicated in byte order ascending, then
//! the value area: for each key (index order) `values_len` records
//! `varint(hanzi_len) + hanzi utf8 + varint(freq)` ordered by freq
//! descending, hanzi ascending. Varints are LEB128 u32 ([`crate::varint`]).
//! `serialize` emits v2; `deserialize` also reads the legacy v1 layout
//! (MAGIC, version 1, entry_count u32, then per entry `pinyin_len u32 |
//! pinyin utf8 | hanzi_len u32 | hanzi utf8 | freq u32`) so existing files
//! keep loading.

use crate::varint;
use std::collections::BTreeMap;

/// One dictionary entry: pinyin key, hanzi candidate, usage frequency.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub pinyin: String,
    pub hanzi: String,
    pub freq: u32,
}

/// Magic bytes at the start of every `.ior` file.
pub const MAGIC: &[u8; 4] = b"IORD";

/// Current `.ior` format version (grouped key-index layout).
pub const VERSION: u8 = 2;

/// Legacy `.ior` format version (flat per-entry layout), still readable.
pub const VERSION_V1: u8 = 1;

/// Canonical syllable separator used inside stored pinyin strings.
const SEPARATOR: char = '\'';

/// Size of the fixed `.ior` v2 header: magic (4) + version (1) +
/// entry_count (4) + key_count (4).
const HEADER_BYTES: usize = 13;

/// Minimum bytes one v1-serialized entry can take: two length fields + freq.
const MIN_ENTRY_BYTES: usize = 12;

/// Lowercase pinyin and canonicalize separators to `'` (collapses repeats,
/// drops leading/trailing separators).
pub fn canonical_pinyin(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut pending_separator = false;
    for ch in raw.to_lowercase().chars() {
        if ch == ' ' || ch == SEPARATOR {
            if !out.is_empty() {
                pending_separator = true;
            }
        } else {
            if pending_separator {
                out.push(SEPARATOR);
                pending_separator = false;
            }
            out.push(ch);
        }
    }
    out
}

/// Parse TSV text into entries. Errors carry 1-based line numbers.
pub fn parse_tsv(text: &str) -> Result<Vec<Entry>, String> {
    let mut entries = Vec::new();
    for (index, raw_line) in text.lines().enumerate() {
        let lineno = index + 1;
        let trimmed = raw_line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = raw_line.split('\t').map(str::trim).collect();
        if fields.len() < 2 {
            return Err(format!(
                "line {lineno}: missing hanzi field, expected 'pinyin<TAB>hanzi<TAB>freq'"
            ));
        }
        if fields.len() > 3 {
            return Err(format!(
                "line {lineno}: {} fields, expected at most 'pinyin<TAB>hanzi<TAB>freq'",
                fields.len()
            ));
        }
        let pinyin = canonical_pinyin(fields[0]);
        let hanzi = fields[1].to_string();
        if pinyin.is_empty() {
            return Err(format!("line {lineno}: empty pinyin"));
        }
        if hanzi.is_empty() {
            return Err(format!("line {lineno}: empty hanzi"));
        }
        let freq = if fields.len() == 3 {
            fields[2]
                .parse::<u32>()
                .map_err(|err| format!("line {lineno}: invalid freq '{}': {err}", fields[2]))?
        } else {
            1
        };
        entries.push(Entry {
            pinyin,
            hanzi,
            freq,
        });
    }
    Ok(entries)
}

/// Render entries as TSV lines (always including the freq column).
pub fn to_tsv(entries: &[Entry]) -> String {
    let mut out = String::new();
    for entry in entries {
        out.push_str(&entry.pinyin);
        out.push('\t');
        out.push_str(&entry.hanzi);
        out.push('\t');
        out.push_str(&entry.freq.to_string());
        out.push('\n');
    }
    out
}

/// Serialize entries to the `.ior` v2 binary format.
///
/// Entries are grouped under their pinyin key: the key index is emitted in
/// byte order ascending with keys deduplicated, so files are canonical even
/// if the caller forgot to run [`normalize`] first (values inside each key
/// keep their input order — call [`normalize`] for the canonical
/// freq-desc/hanzi-asc ordering). No deduplication of identical
/// `(key, hanzi)` pairs is performed here.
pub fn serialize(entries: &[Entry]) -> Vec<u8> {
    let mut grouped: BTreeMap<&str, Vec<(&str, u32)>> = BTreeMap::new();
    for entry in entries {
        grouped
            .entry(entry.pinyin.as_str())
            .or_default()
            .push((&entry.hanzi, entry.freq));
    }
    let entry_total =
        u32::try_from(entries.len()).expect("more than 2^32 entries cannot be serialized");
    let key_total = u32::try_from(grouped.len()).expect("more than 2^32 keys cannot be serialized");
    let mut out = Vec::with_capacity(HEADER_BYTES + entries.len() * MIN_ENTRY_BYTES);
    out.extend_from_slice(MAGIC);
    out.push(VERSION);
    out.extend_from_slice(&entry_total.to_le_bytes());
    out.extend_from_slice(&key_total.to_le_bytes());
    for (key, values) in &grouped {
        append_varint_field(&mut out, key);
        varint::write_u32(
            &mut out,
            u32::try_from(values.len()).expect("more than 2^32 values under one key"),
        );
    }
    for values in grouped.values() {
        for (hanzi, freq) in values {
            append_varint_field(&mut out, hanzi);
            varint::write_u32(&mut out, *freq);
        }
    }
    out
}

/// Append one varint-length-prefixed utf8 field.
fn append_varint_field(out: &mut Vec<u8>, value: &str) {
    let len = u32::try_from(value.len()).expect("string field longer than 4 GiB");
    varint::write_u32(out, len);
    out.extend_from_slice(value.as_bytes());
}

/// Deserialize a `.ior` binary (v2 grouped layout or legacy v1 flat
/// layout), validating magic, version, truncation, utf8, count consistency,
/// and the absence of trailing bytes.
pub fn deserialize(bytes: &[u8]) -> Result<Vec<Entry>, String> {
    if bytes.get(0..MAGIC.len()) != Some(MAGIC.as_slice()) {
        let found = bytes.get(0..MAGIC.len()).unwrap_or(&[]);
        return Err(format!(
            "bad magic: expected {:?} (\"IORD\"), found {found:?}",
            MAGIC
        ));
    }
    let (version, pos) = read_u8(bytes, MAGIC.len())?;
    match version {
        VERSION_V1 => deserialize_v1(bytes, pos),
        VERSION => deserialize_v2(bytes, pos),
        other => Err(format!(
            "unsupported version {other}: expected {VERSION_V1} or {VERSION}"
        )),
    }
}

/// Parse the legacy v1 flat entry layout starting after the version byte.
fn deserialize_v1(bytes: &[u8], pos: usize) -> Result<Vec<Entry>, String> {
    let (count, mut pos) = read_u32(bytes, pos)?;
    let capacity = usize::try_from(count)
        .unwrap_or(usize::MAX)
        .min(bytes.len() / MIN_ENTRY_BYTES + 1);
    let mut entries = Vec::with_capacity(capacity);
    for index in 0..count {
        let entry_no = index + 1;
        let (pinyin, next) =
            read_string(bytes, pos).map_err(|err| format!("entry {entry_no} pinyin: {err}"))?;
        let (hanzi, next) =
            read_string(bytes, next).map_err(|err| format!("entry {entry_no} hanzi: {err}"))?;
        let (freq, next) =
            read_u32(bytes, next).map_err(|err| format!("entry {entry_no} freq: {err}"))?;
        pos = next;
        entries.push(Entry {
            pinyin: pinyin.to_string(),
            hanzi: hanzi.to_string(),
            freq,
        });
    }
    if pos != bytes.len() {
        return Err(format!(
            "{} trailing byte(s) after last entry (at offset {pos})",
            bytes.len() - pos
        ));
    }
    Ok(entries)
}

/// Parse the v2 key-index layout starting after the version byte.
fn deserialize_v2(bytes: &[u8], pos: usize) -> Result<Vec<Entry>, String> {
    let (entry_count, next) = read_u32(bytes, pos)?;
    let (key_count, mut pos) = read_u32(bytes, next)?;
    let capacity = usize::try_from(key_count)
        .unwrap_or(usize::MAX)
        .min(bytes.len() / 2 + 1);
    let mut keys: Vec<(String, u32)> = Vec::with_capacity(capacity);
    for index in 0..key_count {
        let key_no = index + 1;
        let (key, next) =
            read_varint_string(bytes, pos).map_err(|err| format!("key {key_no}: {err}"))?;
        let (values_len, next) = varint::read_u32(bytes, next)
            .map_err(|err| format!("key {key_no} values count: {err}"))?;
        pos = next;
        keys.push((key.to_string(), values_len));
    }
    let mut entries = Vec::new();
    for (index, (key, values_len)) in keys.iter().enumerate() {
        let key_no = index + 1;
        for value_no in 0..*values_len {
            let (hanzi, next) = read_varint_string(bytes, pos)
                .map_err(|err| format!("key {key_no} value {value_no} hanzi: {err}"))?;
            let (freq, next) = varint::read_u32(bytes, next)
                .map_err(|err| format!("key {key_no} value {value_no} freq: {err}"))?;
            pos = next;
            entries.push(Entry {
                pinyin: key.clone(),
                hanzi: hanzi.to_string(),
                freq,
            });
        }
    }
    let stored = usize::try_from(entry_count).unwrap_or(usize::MAX);
    if stored != entries.len() {
        return Err(format!(
            "entry count mismatch: header says {entry_count}, value area holds {}",
            entries.len()
        ));
    }
    if pos != bytes.len() {
        return Err(format!(
            "{} trailing byte(s) after last value (at offset {pos})",
            bytes.len() - pos
        ));
    }
    Ok(entries)
}

/// Read one byte at `pos`; returns `(value, next_pos)`.
fn read_u8(bytes: &[u8], pos: usize) -> Result<(u8, usize), String> {
    bytes
        .get(pos)
        .map(|value| (*value, pos + 1))
        .ok_or_else(|| truncated(1, pos, bytes.len()))
}

/// Read one little-endian u32 at `pos`; returns `(value, next_pos)`.
fn read_u32(bytes: &[u8], pos: usize) -> Result<(u32, usize), String> {
    let end = pos
        .checked_add(4)
        .ok_or_else(|| truncated(4, pos, bytes.len()))?;
    let raw = bytes
        .get(pos..end)
        .ok_or_else(|| truncated(4, pos, bytes.len()))?;
    let mut buf = [0u8; 4];
    buf.copy_from_slice(raw);
    Ok((u32::from_le_bytes(buf), end))
}

/// Read one u32-length-prefixed utf8 string at `pos` (v1 field layout);
/// returns `(&str, next_pos)`.
fn read_string(bytes: &[u8], pos: usize) -> Result<(&str, usize), String> {
    let (len, pos) = read_u32(bytes, pos)?;
    read_sized_string(bytes, pos, len)
}

/// Read one varint-length-prefixed utf8 string at `pos` (v2 field layout);
/// returns `(&str, next_pos)`.
fn read_varint_string(bytes: &[u8], pos: usize) -> Result<(&str, usize), String> {
    let (len, pos) = varint::read_u32(bytes, pos)?;
    read_sized_string(bytes, pos, len)
}

/// Read `len` utf8 bytes at `pos`, validating the length against the buffer.
fn read_sized_string(bytes: &[u8], pos: usize, len: u32) -> Result<(&str, usize), String> {
    let len = usize::try_from(len).map_err(|_| format!("string length {len} out of range"))?;
    let end = pos
        .checked_add(len)
        .ok_or_else(|| format!("string length {len} overflows buffer offset {pos}"))?;
    let raw = bytes.get(pos..end).ok_or_else(|| {
        format!(
            "truncated string: need {len} byte(s) at offset {pos}, buffer is {} byte(s)",
            bytes.len()
        )
    })?;
    std::str::from_utf8(raw)
        .map(|value| (value, end))
        .map_err(|err| format!("invalid utf-8 at offset {pos}: {err}"))
}

/// Describe a truncated read.
fn truncated(needed: usize, pos: usize, len: usize) -> String {
    format!("truncated data: need {needed} byte(s) at offset {pos}, buffer is {len} byte(s)")
}

/// Deduplicate entries by (pinyin, hanzi), summing freq, then sort by
/// (pinyin asc, freq desc, hanzi asc) so output is deterministic.
pub fn normalize(entries: Vec<Entry>) -> Vec<Entry> {
    let mut totals: BTreeMap<(String, String), u32> = BTreeMap::new();
    for entry in entries {
        let key = (canonical_pinyin(&entry.pinyin), entry.hanzi);
        let total = totals.entry(key).or_insert(0);
        *total = total.saturating_add(entry.freq);
    }
    let mut merged: Vec<Entry> = totals
        .into_iter()
        .map(|((pinyin, hanzi), freq)| Entry {
            pinyin,
            hanzi,
            freq,
        })
        .collect();
    merged.sort_by(|a, b| {
        a.pinyin
            .cmp(&b.pinyin)
            .then_with(|| b.freq.cmp(&a.freq))
            .then_with(|| a.hanzi.cmp(&b.hanzi))
    });
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    // Hanzi via escapes to keep this source file ASCII.
    const KAI: &str = "\u{5f00}\u{53d1}"; // kai fa
    const CHENG: &str = "\u{7a0b}\u{5e8f}"; // cheng xu
    const CE: &str = "\u{6d4b}\u{8bd5}"; // ce shi
    const JIA: &str = "\u{7532}";
    const YI: &str = "\u{4e59}";

    fn entry(pinyin: &str, hanzi: &str, freq: u32) -> Entry {
        Entry {
            pinyin: pinyin.to_string(),
            hanzi: hanzi.to_string(),
            freq,
        }
    }

    #[test]
    fn parse_tsv_skips_comments_and_blanks_and_canonicalizes() {
        let text = "# header comment\n\nkai'fa\tKAI\t500\nNi Hao\tNI\n";
        let entries = parse_tsv(text).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].pinyin, "kai'fa");
        assert_eq!(entries[0].freq, 500);
        assert_eq!(entries[1].pinyin, "ni'hao");
        assert_eq!(entries[1].freq, 1);
    }

    #[test]
    fn canonical_pinyin_collapses_and_trims_separators() {
        assert_eq!(canonical_pinyin("  Ni   hao  "), "ni'hao");
        assert_eq!(canonical_pinyin("'kai'fa'"), "kai'fa");
        assert_eq!(canonical_pinyin("shu ju ku"), "shu'ju'ku");
    }

    #[test]
    fn parse_tsv_reports_line_numbers() {
        assert!(parse_tsv("kai'fa\tKAI\t500\nbroken-line\n")
            .unwrap_err()
            .contains("line 2"));
        assert!(parse_tsv("kai'fa\tKAI\t-1").unwrap_err().contains("line 1"));
        assert!(parse_tsv("kai'fa\tKAI\t5\textra")
            .unwrap_err()
            .contains("line 1"));
        assert!(parse_tsv("\tKAI\t5").unwrap_err().contains("empty pinyin"));
        assert!(parse_tsv("kai'fa\t\t5")
            .unwrap_err()
            .contains("empty hanzi"));
    }

    #[test]
    fn normalize_dedups_sums_and_sorts_deterministically() {
        let text = format!("b\t{YI}\t10\na\t{YI}\t5\nb\t{YI}\t3\na\t{JIA}\t7\nb\t{YI}\t0\n");
        let merged = normalize(parse_tsv(&text).unwrap());
        assert_eq!(
            merged,
            vec![entry("a", JIA, 7), entry("a", YI, 5), entry("b", YI, 13),]
        );
    }

    #[test]
    fn roundtrip_tsv_serialize_deserialize_tsv() {
        let text = format!("# it sample\nkai'fa\t{KAI}\t500\ncheng'xu\t{CHENG}\t400\n");
        let entries = normalize(parse_tsv(&text).unwrap());
        let bytes = serialize(&entries);
        let loaded = deserialize(&bytes).unwrap();
        assert_eq!(loaded, entries);
        assert_eq!(to_tsv(&loaded), to_tsv(&entries));
        assert_eq!(parse_tsv(&to_tsv(&loaded)).unwrap(), loaded);
    }

    #[test]
    fn serialize_header_layout() {
        let bytes = serialize(&[]);
        assert_eq!(bytes.len(), HEADER_BYTES);
        assert_eq!(&bytes[..4], MAGIC.as_slice());
        assert_eq!(bytes[4], VERSION);
        assert_eq!(&bytes[5..9], &0u32.to_le_bytes()); // entry_count
        assert_eq!(&bytes[9..13], &0u32.to_le_bytes()); // key_count
        assert_eq!(deserialize(&bytes).unwrap(), vec![]);
    }

    /// Hand-encode the expected v2 layout for a two-key, three-entry set.
    #[test]
    fn serialize_v2_byte_layout_is_exact() {
        // normalize orders keys ascending, values freq desc / hanzi asc.
        let entries =
            normalize(parse_tsv(&format!("a\t{YI}\t5\na\t{JIA}\t7\nb\t{YI}\t1\n")).unwrap());
        let bytes = serialize(&entries);
        let mut expected = Vec::new();
        expected.extend_from_slice(MAGIC);
        expected.push(2);
        expected.extend_from_slice(&3u32.to_le_bytes()); // entry_count
        expected.extend_from_slice(&2u32.to_le_bytes()); // key_count
                                                         // key index: "a" (2 values), "b" (1 value)
        expected.extend_from_slice(&[1, b'a', 2]);
        expected.extend_from_slice(&[1, b'b', 1]);
        // values for "a": JIA(3B) freq 7, then YI(3B) freq 5
        expected.push(3);
        expected.extend_from_slice(JIA.as_bytes());
        expected.extend_from_slice(&[7]);
        expected.push(3);
        expected.extend_from_slice(YI.as_bytes());
        expected.extend_from_slice(&[5]);
        // values for "b": YI freq 1
        expected.push(3);
        expected.extend_from_slice(YI.as_bytes());
        expected.extend_from_slice(&[1]);
        assert_eq!(bytes, expected);
        assert_eq!(deserialize(&expected).unwrap(), entries);
    }

    /// Keys are grouped and written in byte order even for unsorted input.
    #[test]
    fn serialize_groups_unsorted_input_canonically() {
        let entries = vec![entry("b", YI, 2), entry("a", JIA, 1), entry("b", JIA, 9)];
        let bytes = serialize(&entries);
        let reloaded = deserialize(&bytes).unwrap();
        assert_eq!(
            reloaded,
            vec![entry("a", JIA, 1), entry("b", YI, 2), entry("b", JIA, 9),]
        );
    }

    /// Multi-byte varint paths: strings longer than 127 bytes and big freqs.
    #[test]
    fn serialize_v2_roundtrips_long_fields_and_large_freqs() {
        let long_hanzi: String = "\u{56fe}".repeat(60); // 180 utf8 bytes
                                                        // keys already in byte order so serialize's grouping is identity
        let entries = vec![
            entry("a", "b", 1),
            entry("chang'de'ci'yu", &long_hanzi, u32::MAX),
        ];
        let bytes = serialize(&entries);
        let reloaded = deserialize(&bytes).unwrap();
        assert_eq!(reloaded, entries);
    }

    /// Build a legacy v1 buffer byte by byte (u32 length prefixes).
    /// Header: magic (4) + version (1) + entry_count (4).
    fn v1_bytes(entries: &[Entry]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(MAGIC);
        out.push(VERSION_V1);
        out.extend_from_slice(&u32::try_from(entries.len()).unwrap().to_le_bytes());
        for entry in entries {
            for field in [&entry.pinyin, &entry.hanzi] {
                out.extend_from_slice(&u32::try_from(field.len()).unwrap().to_le_bytes());
                out.extend_from_slice(field.as_bytes());
            }
            out.extend_from_slice(&entry.freq.to_le_bytes());
        }
        out
    }

    #[test]
    fn deserialize_reads_legacy_v1_layout() {
        let entries = vec![entry("kai'fa", KAI, 500), entry("ce'shi", CE, 7)];
        let bytes = v1_bytes(&entries);
        assert_eq!(bytes[4], VERSION_V1);
        assert_eq!(deserialize(&bytes).unwrap(), entries);
    }

    #[test]
    fn deserialize_rejects_bad_magic_and_version() {
        let bytes = serialize(&[entry("a", "b", 1)]);
        let mut bad = bytes.clone();
        bad[0] = b'X';
        assert!(deserialize(&bad).unwrap_err().contains("magic"));
        let mut wrong_version = bytes;
        wrong_version[4] = VERSION + 1;
        assert!(deserialize(&wrong_version)
            .unwrap_err()
            .contains("unsupported version"));
        let mut v0 = serialize(&[entry("a", "b", 1)]);
        v0[4] = 0;
        assert!(deserialize(&v0)
            .unwrap_err()
            .contains("unsupported version"));
    }

    #[test]
    fn deserialize_rejects_truncation_and_trailing_bytes() {
        let bytes = serialize(&[entry("a", "b", 1), entry("c", "d", 2)]);
        for cut in 0..bytes.len() {
            assert!(
                deserialize(&bytes[..cut]).is_err(),
                "truncation at {cut} bytes should fail"
            );
        }
        let mut extra = bytes.clone();
        extra.push(0);
        assert!(deserialize(&extra).unwrap_err().contains("trailing"));
        // v1 buffers must reject trailing bytes too.
        let v1 = v1_bytes(&[entry("a", "b", 1)]);
        let mut v1_extra = v1.clone();
        v1_extra.push(0);
        assert!(deserialize(&v1_extra).unwrap_err().contains("trailing"));
    }

    #[test]
    fn deserialize_v2_rejects_entry_count_mismatch() {
        let bytes = serialize(&[entry("a", "b", 1)]);
        let mut mismatched = bytes.clone();
        // entry_count lives at offset 5..9; claim two entries instead of one.
        mismatched[5..9].copy_from_slice(&2u32.to_le_bytes());
        let err = deserialize(&mismatched).unwrap_err();
        assert!(err.contains("mismatch"), "unexpected error: {err}");
    }

    #[test]
    fn deserialize_rejects_invalid_utf8_with_entry_context() {
        // Corrupt a hand-built v1 buffer: hanzi starts at 9 + 4 + 1 + 4.
        let mut corrupt = v1_bytes(&[entry("a", "\u{4f60}", 2)]);
        let hanzi_start = 9 + 4 + 1 + 4; // v1 header + pinyin len + pinyin + hanzi len
        corrupt[hanzi_start + 1] = 0xFF;
        let err = deserialize(&corrupt).unwrap_err();
        assert!(err.contains("utf-8"), "unexpected error: {err}");
        assert!(err.contains("entry 1"), "missing entry context: {err}");
    }

    #[test]
    fn deserialize_v2_rejects_invalid_utf8_with_key_context() {
        let bytes = serialize(&[entry("a", "\u{4f60}", 2)]);
        let mut corrupt = bytes;
        // Layout: header (13) | key len 1 | 'a' | values len 1 | hanzi len 3
        // | hanzi (3 bytes) | freq. Corrupt the second hanzi byte.
        corrupt[18] = 0xFF;
        let err = deserialize(&corrupt).unwrap_err();
        assert!(err.contains("utf-8"), "unexpected error: {err}");
        assert!(err.contains("key 1"), "missing key context: {err}");
    }

    #[test]
    fn merge_via_normalize_sums_and_canonicalizes() {
        let a = parse_tsv(&format!("kai'fa\t{KAI}\t500\nce'shi\t{CE}\t420\n")).unwrap();
        let b = parse_tsv(&format!("KAI FA\t{KAI}\t100\n")).unwrap();
        let mut combined = a;
        combined.extend(b);
        let merged = normalize(combined);
        assert_eq!(merged.len(), 2);
        let kai = merged
            .iter()
            .find(|e| e.pinyin == "kai'fa")
            .expect("kai'fa entry");
        assert_eq!(kai.freq, 600);
        assert_eq!(kai.hanzi, KAI);
    }
}
