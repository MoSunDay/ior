//! Dictionary formats for ior: the TSV source format and the `.ior` binary.
//!
//! TSV: one entry per line, `pinyin<TAB>hanzi<TAB>freq`. `freq` is optional
//! (default 1). `#` comments and blank lines are skipped. Pinyin is stored
//! lowercase with `'` as the single syllable separator.
//!
//! `.ior` binary (little-endian): MAGIC, VERSION (u8), entry_count (u32),
//! then per entry: pinyin_len (u32), pinyin utf8 bytes, hanzi_len (u32),
//! hanzi utf8 bytes, freq (u32).

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

/// Current `.ior` format version.
pub const VERSION: u8 = 1;

/// Canonical syllable separator used inside stored pinyin strings.
const SEPARATOR: char = '\'';

/// Size of the fixed `.ior` header: magic (4) + version (1) + count (4).
const HEADER_BYTES: usize = 9;

/// Minimum bytes one serialized entry can take: two length fields + freq.
const MIN_ENTRY_BYTES: usize = 12;

/// Lowercase pinyin and canonicalize separators to `'` (collapses repeats,
/// drops leading/trailing separators).
fn canonical_pinyin(raw: &str) -> String {
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

/// Serialize entries to the `.ior` binary format.
pub fn serialize(entries: &[Entry]) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER_BYTES + entries.len() * MIN_ENTRY_BYTES);
    out.extend_from_slice(MAGIC);
    out.push(VERSION);
    out.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    for entry in entries {
        append_field(&mut out, &entry.pinyin);
        append_field(&mut out, &entry.hanzi);
        out.extend_from_slice(&entry.freq.to_le_bytes());
    }
    out
}

/// Append one length-prefixed utf8 field.
fn append_field(out: &mut Vec<u8>, value: &str) {
    let len = u32::try_from(value.len()).expect("string field longer than 4 GiB");
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(value.as_bytes());
}

/// Deserialize a `.ior` binary, validating magic, version, truncation,
/// utf8, and the absence of trailing bytes.
pub fn deserialize(bytes: &[u8]) -> Result<Vec<Entry>, String> {
    if bytes.get(0..MAGIC.len()) != Some(MAGIC.as_slice()) {
        let found = bytes.get(0..MAGIC.len()).unwrap_or(&[]);
        return Err(format!(
            "bad magic: expected {:?} (\"IORD\"), found {found:?}",
            MAGIC
        ));
    }
    let (version, pos) = read_u8(bytes, MAGIC.len())?;
    if version != VERSION {
        return Err(format!("unsupported version {version}: expected {VERSION}"));
    }
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

/// Read one length-prefixed utf8 string at `pos`; returns `(&str, next_pos)`.
fn read_string(bytes: &[u8], pos: usize) -> Result<(&str, usize), String> {
    let (len, pos) = read_u32(bytes, pos)?;
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
        assert_eq!(&bytes[5..9], &0u32.to_le_bytes());
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
    }

    #[test]
    fn deserialize_rejects_invalid_utf8_with_entry_context() {
        let bytes = serialize(&[entry("a", "\u{4f60}", 2)]);
        // hanzi starts at HEADER_BYTES + 4 (pinyin len) + 1 (pinyin) + 4 (hanzi len).
        let mut corrupt = bytes;
        let hanzi_start = HEADER_BYTES + 4 + 1 + 4;
        corrupt[hanzi_start + 1] = 0xFF;
        let err = deserialize(&corrupt).unwrap_err();
        assert!(err.contains("utf-8"), "unexpected error: {err}");
        assert!(err.contains("entry 1"), "missing entry context: {err}");
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
