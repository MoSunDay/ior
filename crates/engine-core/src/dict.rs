//! Static pinyin-to-hanzi dictionary.
//!
//! A [`Dictionary`] stores one map from a **canonical** pinyin key
//! (syllables joined by `'`, lowercased, separators folded) to
//! `(hanzi, freq)` entries sorted by frequency descending, then hanzi
//! ascending. Since P1 no fuzzy rules are baked into the keys: fuzzy
//! matching happens at query time in [`crate::spelling`]/[`crate::decoder`],
//! which probe every spelling variant of the typed input against the
//! canonical keys kept here.
//!
//! A secondary **initials index** (`BTreeMap<String, Vec<String>>`) maps the
//! first letters of each syllable of a key (`ni'hao` -> `nh`, `xi'an` -> `xa`)
//! to the keys sharing them, powering simplified-pinyin (简拼) lookups via
//! [`Dictionary::lookup_initials`].

use std::collections::{BTreeMap, HashMap};

/// Magic bytes at the start of every `.ior` file (matches dict-tools).
const IOR_MAGIC: &[u8; 4] = b"IORD";

/// `.ior` format version 2: grouped key-index layout (matches dict-tools).
const IOR_VERSION_V2: u8 = 2;

/// `.ior` format version 1: legacy flat per-entry layout (matches
/// dict-tools files built before the v2 switch; still readable).
const IOR_VERSION_V1: u8 = 1;

/// One raw `(pinyin, hanzi, freq)` triple as stored in a `.ior` buffer,
/// before canonical key normalization.
struct RawEntry {
    pinyin: String,
    hanzi: String,
    freq: u32,
}

/// Parses a `.ior` binary (v2 grouped layout or legacy v1 flat layout)
/// into raw triples. Byte-for-byte compatible with `dict_tools::format`:
/// little-endian counts, LEB128 u32 varints in v2, and
/// magic/version/truncation/utf8/count/trailing-byte validation with
/// readable errors.
fn parse_ior(bytes: &[u8]) -> Result<Vec<RawEntry>, String> {
    if bytes.get(0..IOR_MAGIC.len()) != Some(IOR_MAGIC.as_slice()) {
        let found = bytes.get(0..IOR_MAGIC.len()).unwrap_or(&[]);
        return Err(format!(
            "bad magic: expected {:?} (\"IORD\"), found {found:?}",
            IOR_MAGIC
        ));
    }
    let (version, pos) = read_u8(bytes, IOR_MAGIC.len())?;
    match version {
        IOR_VERSION_V1 => parse_ior_v1(bytes, pos),
        IOR_VERSION_V2 => parse_ior_v2(bytes, pos),
        other => Err(format!(
            "unsupported version {other}: expected {IOR_VERSION_V1} or {IOR_VERSION_V2}"
        )),
    }
}

/// Legacy v1: `entry_count u32`, then per entry `pinyin_len u32 | pinyin |
/// hanzi_len u32 | hanzi | freq u32`.
fn parse_ior_v1(bytes: &[u8], pos: usize) -> Result<Vec<RawEntry>, String> {
    let (count, mut pos) = read_u32(bytes, pos)?;
    let mut entries = Vec::new();
    for index in 0..count {
        let entry_no = index + 1;
        let (pinyin, next) =
            read_u32_field(bytes, pos).map_err(|err| format!("entry {entry_no} pinyin: {err}"))?;
        let (hanzi, next) =
            read_u32_field(bytes, next).map_err(|err| format!("entry {entry_no} hanzi: {err}"))?;
        let (freq, next) =
            read_u32(bytes, next).map_err(|err| format!("entry {entry_no} freq: {err}"))?;
        pos = next;
        entries.push(RawEntry {
            pinyin: pinyin.to_string(),
            hanzi: hanzi.to_string(),
            freq,
        });
    }
    expect_no_trailing(bytes, pos)?;
    Ok(entries)
}

/// v2: `entry_count u32 | key_count u32`, then a key index of
/// `varint(key_len) key varint(values_len)` records, then the value area:
/// per key, `varint(hanzi_len) hanzi varint(freq)` records.
fn parse_ior_v2(bytes: &[u8], pos: usize) -> Result<Vec<RawEntry>, String> {
    let (entry_count, next) = read_u32(bytes, pos)?;
    let (key_count, mut pos) = read_u32(bytes, next)?;
    let mut keys: Vec<(String, u32)> = Vec::new();
    for index in 0..key_count {
        let key_no = index + 1;
        let (key, values_len, next) =
            read_key_record(bytes, pos).map_err(|err| format!("key {key_no}: {err}"))?;
        pos = next;
        keys.push((key, values_len));
    }
    let mut entries = Vec::new();
    for (index, (key, values_len)) in keys.iter().enumerate() {
        let key_no = index + 1;
        for value_no in 0..*values_len {
            let (hanzi, freq, next) = read_value_record(bytes, pos)
                .map_err(|err| format!("key {key_no} value {}: {err}", value_no + 1))?;
            pos = next;
            entries.push(RawEntry {
                pinyin: key.clone(),
                hanzi,
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
    expect_no_trailing(bytes, pos)?;
    Ok(entries)
}

/// One key-index record: `varint(len) key-utf8 varint(values_len)`.
fn read_key_record(bytes: &[u8], pos: usize) -> Result<(String, u32, usize), String> {
    let (len, after_len) = read_varint(bytes, pos)?;
    let (key, after_key) = read_sized(bytes, after_len, len)?;
    let (values_len, after_count) = read_varint(bytes, after_key)?;
    Ok((key.to_string(), values_len, after_count))
}

/// One value record: `varint(len) hanzi-utf8 varint(freq)`.
fn read_value_record(bytes: &[u8], pos: usize) -> Result<(String, u32, usize), String> {
    let (len, after_len) = read_varint(bytes, pos)?;
    let (hanzi, after_hanzi) = read_sized(bytes, after_len, len)?;
    let (freq, after_freq) = read_varint(bytes, after_hanzi)?;
    Ok((hanzi.to_string(), freq, after_freq))
}

/// Reject trailing bytes after the last record.
fn expect_no_trailing(bytes: &[u8], pos: usize) -> Result<(), String> {
    if pos != bytes.len() {
        return Err(format!(
            "{} trailing byte(s) after last record (at offset {pos})",
            bytes.len() - pos
        ));
    }
    Ok(())
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
    let raw = bytes
        .get(pos..pos.saturating_add(4))
        .ok_or_else(|| truncated(4, pos, bytes.len()))?;
    let mut buf = [0u8; 4];
    buf.copy_from_slice(raw);
    Ok((u32::from_le_bytes(buf), pos + 4))
}

/// Read one u32-length-prefixed utf8 field (v1 layout).
fn read_u32_field(bytes: &[u8], pos: usize) -> Result<(&str, usize), String> {
    let (len, pos) = read_u32(bytes, pos)?;
    read_sized(bytes, pos, len)
}

/// Read one LEB128 u32 varint at `pos` (v2 layout); strict about the
/// 5-byte / 4-payload-bit u32 limit.
fn read_varint(bytes: &[u8], pos: usize) -> Result<(u32, usize), String> {
    let mut result: u32 = 0;
    let mut shift: u32 = 0;
    let mut cursor = pos;
    loop {
        let Some(&byte) = bytes.get(cursor) else {
            return Err(format!(
                "truncated varint starting at offset {pos}: buffer is {} byte(s)",
                bytes.len()
            ));
        };
        cursor += 1;
        let payload = u32::from(byte & 0x7f);
        if shift >= 28 {
            // Fifth (final) group: only four payload bits fit into a u32.
            if byte > 0x0f {
                return Err(format!("varint at offset {pos} exceeds u32 range"));
            }
            return Ok((result | (payload << shift), cursor));
        }
        result |= payload << shift;
        shift += 7;
        if byte & 0x80 == 0 {
            return Ok((result, cursor));
        }
    }
}

/// Read `len` utf8 bytes at `pos`, validating the length against the buffer.
fn read_sized(bytes: &[u8], pos: usize, len: u32) -> Result<(&str, usize), String> {
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

/// Pinyin key -> `(hanzi, freq)` entries, freq desc / hanzi asc, plus the
/// simplified-pinyin (initials) index over the same keys.
#[derive(Debug, Clone)]
pub struct Dictionary {
    entries: HashMap<String, Vec<(String, u32)>>,
    /// First letters of every syllable (`ni'hao` -> `nh`) -> canonical keys
    /// sharing them, each list deduped and lexicographically sorted.
    initials: BTreeMap<String, Vec<String>>,
}

impl Dictionary {
    /// Parses TSV text: `pinyin<TAB>hanzi<TAB>freq` per line. The frequency
    /// field is optional (default 1). Blank lines and lines starting with
    /// `#` are skipped. Pinyin separators may be `'` or spaces (normalized
    /// to `'`); everything is lowercased. Keys stay **canonical** — no fuzzy
    /// rules are applied here (fuzzy matching is query-side, see
    /// [`crate::spelling`]). Duplicate `(key, hanzi)` pairs have their
    /// frequencies summed. Returns `Err` with the 1-based line number on
    /// malformed lines.
    pub fn from_tsv(text: &str) -> Result<Dictionary, String> {
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
            let key = normalize_key(pinyin);
            let vals = entries.entry(key).or_default();
            match vals.iter_mut().find(|(h, _)| h == hanzi) {
                Some((_, f)) => *f += freq,
                None => vals.push((hanzi.to_string(), freq)),
            }
        }
        for vals in entries.values_mut() {
            sort_values(vals);
        }
        Ok(dictionary_from_parts(entries))
    }

    /// The builtin core dictionary (`assets/core.tsv`).
    pub fn builtin() -> Dictionary {
        Self::from_tsv(include_str!("../assets/core.tsv"))
            .expect("builtin core.tsv must be well-formed")
    }

    /// Loads a `.ior` binary dictionary (v2 grouped layout or legacy v1
    /// flat layout, byte-identical to what `dict-tools` writes). Keys are
    /// normalized exactly like [`Dictionary::from_tsv`] — lowercase,
    /// separators folded to `'`, no fuzzy rules — and duplicate
    /// `(key, hanzi)` frequencies are summed, so `.ior` and TSV sources of
    /// the same data behave identically.
    pub fn from_ior_bytes(bytes: &[u8]) -> Result<Dictionary, String> {
        let raw = parse_ior(bytes)?;
        Ok(dictionary_from_raw(raw))
    }

    /// Loads and overlays `.ior` layers in the given order (base first,
    /// then domain, then user dictionaries). Semantically equivalent to
    /// loading each layer with [`Dictionary::from_ior_bytes`] and
    /// [`Dictionary::merge`]-ing them in order: frequencies of identical
    /// `(key, hanzi)` pairs add up. An empty layer list yields an empty
    /// dictionary.
    pub fn from_ior_layers(layers: &[&[u8]]) -> Result<Dictionary, String> {
        let mut combined: Option<Dictionary> = None;
        for bytes in layers {
            let layer = Self::from_ior_bytes(bytes)?;
            combined = match combined {
                Some(mut acc) => {
                    acc.merge(&layer);
                    Some(acc)
                }
                None => Some(layer),
            };
        }
        Ok(combined.unwrap_or_else(|| dictionary_from_raw(Vec::new())))
    }

    /// Looks up a joined key (`ni'hao`, `xian`, ...) after folding it to
    /// the canonical form (lowercase, separators collapsed), so both raw
    /// and pre-normalized keys work. Fuzzy spellings are **not** matched
    /// here: the caller expands them with [`crate::spelling::key_variants`].
    pub fn lookup(&self, key: &str) -> Option<&[(String, u32)]> {
        self.entries.get(&normalize_key(key)).map(|v| v.as_slice())
    }

    /// Simplified-pinyin (简拼) probe: up to `limit` canonical keys whose
    /// initials string (`ni'hao` -> `nh`) starts with `prefix`, in key
    /// lexicographic order. An empty prefix matches nothing.
    pub fn lookup_initials(&self, prefix: &str, limit: usize) -> Vec<&str> {
        if prefix.is_empty() || limit == 0 {
            return Vec::new();
        }
        let end = format!("{prefix}\u{10FFFF}");
        let mut keys: Vec<&str> = self
            .initials
            .range(prefix.to_string()..end)
            .flat_map(|(_, keys)| keys.iter().map(String::as_str))
            .collect();
        keys.sort_unstable();
        keys.truncate(limit);
        keys
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
    /// `(key, hanzi)` pairs, re-sorting and rebuilding the initials index.
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
        self.initials = build_initials_index(&self.entries);
    }
}

/// Sorts entries by frequency descending, then hanzi ascending.
fn sort_values(vals: &mut [(String, u32)]) {
    vals.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
}

/// Builds a dictionary from raw `.ior` triples: same key normalization and
/// duplicate handling as `Dictionary::from_tsv` (canonical keys, no fuzzy).
fn dictionary_from_raw(raw: Vec<RawEntry>) -> Dictionary {
    let mut entries: HashMap<String, Vec<(String, u32)>> = HashMap::new();
    for record in raw {
        let key = normalize_key(&record.pinyin);
        let vals = entries.entry(key).or_default();
        match vals.iter_mut().find(|(h, _)| *h == record.hanzi) {
            Some((_, freq)) => *freq = freq.saturating_add(record.freq),
            None => vals.push((record.hanzi, record.freq)),
        }
    }
    for vals in entries.values_mut() {
        sort_values(vals);
    }
    dictionary_from_parts(entries)
}

/// Wraps a filled entry map into a [`Dictionary`] with a fresh initials
/// index (shared by the TSV and `.ior` loading paths).
fn dictionary_from_parts(entries: HashMap<String, Vec<(String, u32)>>) -> Dictionary {
    let initials = build_initials_index(&entries);
    Dictionary { entries, initials }
}

/// First letter of every syllable of a canonical key: `ni'hao` -> `nh`,
/// `xi'an` -> `xa`. Syllables are never empty after normalization.
fn initials_of_key(key: &str) -> String {
    key.split('\'')
        .filter_map(|syl| syl.chars().next())
        .collect()
}

/// Groups canonical keys by their initials string; each list is deduped
/// and sorted lexicographically so `lookup_initials` is deterministic.
fn build_initials_index(
    entries: &HashMap<String, Vec<(String, u32)>>,
) -> BTreeMap<String, Vec<String>> {
    let mut index: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for key in entries.keys() {
        index
            .entry(initials_of_key(key))
            .or_default()
            .push(key.clone());
    }
    for keys in index.values_mut() {
        keys.sort();
        keys.dedup();
    }
    index
}

/// Builds a dictionary key in canonical form: lowercase, separators
/// (`'` or space) normalized to `'`, empty segments dropped. No fuzzy
/// rules are applied — that is query-side work ([`crate::spelling`]).
fn normalize_key(key: &str) -> String {
    key.to_lowercase()
        .split(['\'', ' '])
        .filter(|syl| !syl.is_empty())
        .collect::<Vec<_>>()
        .join("'")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_basic_entries() {
        let d = Dictionary::from_tsv("ni\t你\t9\nni\t尼\t1\n").unwrap();
        let vals = d.lookup("ni").unwrap();
        assert_eq!(&vals[0], &("你".to_string(), 9));
        assert_eq!(&vals[1], &("尼".to_string(), 1));
        assert_eq!(d.len(), 1);
        assert!(!d.is_empty());
    }

    #[test]
    fn missing_tab_is_an_error_with_line_number() {
        let err = Dictionary::from_tsv("niyou\n").unwrap_err();
        assert!(err.contains("line 1"), "unexpected error: {err}");
        let err = Dictionary::from_tsv("ni\t你\nbroken line here\n").unwrap_err();
        assert!(err.contains("line 2"), "unexpected error: {err}");
    }

    #[test]
    fn invalid_freq_is_an_error() {
        let err = Dictionary::from_tsv("ni\t你\tx\n").unwrap_err();
        assert!(err.contains("line 1"), "unexpected error: {err}");
    }

    #[test]
    fn comments_and_blanks_skipped_freq_defaults_to_one() {
        let text = "# comment\n\n   \nni\t你\n";
        let d = Dictionary::from_tsv(text).unwrap();
        assert_eq!(d.len(), 1);
        assert_eq!(&d.lookup("ni").unwrap()[0], &("你".to_string(), 1));
    }

    #[test]
    fn space_separator_accepted_and_duplicates_summed() {
        let text = "ni hao\t你好\t5\nni hao\t你好\t3\nNI HAO\t你好\t2\n";
        let d = Dictionary::from_tsv(text).unwrap();
        assert_eq!(&d.lookup("ni'hao").unwrap()[0], &("你好".to_string(), 10));
    }

    #[test]
    fn values_sorted_freq_desc_then_hanzi_asc() {
        let text = "a\t甲\t5\na\t乙\t9\na\t丙\t9\n";
        let d = Dictionary::from_tsv(text).unwrap();
        let vals = d.lookup("a").unwrap();
        assert_eq!(vals[0].0, "丙");
        assert_eq!(vals[1].0, "乙");
        assert_eq!(vals[2].0, "甲");
        assert_eq!(vals[2].1, 5);
    }

    #[test]
    fn lookup_miss_returns_none() {
        let d = Dictionary::from_tsv("ni\t你\n").unwrap();
        assert!(d.lookup("xyz").is_none());
        assert!(d.lookup("").is_none());
        assert!(d.lookup("ni'hao").is_none());
    }

    #[test]
    fn keys_stay_canonical_fuzzy_is_query_side() {
        // P1: the dictionary keeps one key per spelling; fuzzy expansion is
        // done by `spelling::key_variants` at lookup time (decoder probes
        // every variant), not by merging keys at build time.
        let d = Dictionary::from_tsv("li\t里\t5\nni\t你\t9\n").unwrap();
        assert_eq!(d.len(), 2, "li and ni must stay distinct keys");
        assert_eq!(d.lookup("li").unwrap(), [("里".to_string(), 5)]);
        assert_eq!(d.lookup("ni").unwrap(), [("你".to_string(), 9)]);
    }

    #[test]
    fn initials_index_groups_canonical_keys() {
        let d =
            Dictionary::from_tsv("ni'hao\t你好\t5\nxi'an\t西安\t4\njin'tian\t今天\t3\n").unwrap();
        assert_eq!(d.lookup_initials("nh", 10), vec!["ni'hao"]);
        assert_eq!(d.lookup_initials("xa", 10), vec!["xi'an"]);
        assert_eq!(d.lookup_initials("jt", 10), vec!["jin'tian"]);
    }

    #[test]
    fn lookup_initials_prefix_range_and_limit() {
        let text = "ni'hao\t你好\t5\nni'men\t你们\t4\njin'tian\t今天\t3\n";
        let d = Dictionary::from_tsv(text).unwrap();
        // prefix "n" sees every key whose initials start with n, sorted by key
        assert_eq!(d.lookup_initials("n", 10), vec!["ni'hao", "ni'men"]);
        assert_eq!(d.lookup_initials("n", 1), vec!["ni'hao"]);
        assert!(d.lookup_initials("n", 0).is_empty());
        // no match, empty prefix and empty result set
        assert!(d.lookup_initials("z", 10).is_empty());
        assert!(d.lookup_initials("", 10).is_empty());
    }

    #[test]
    fn merge_rebuilds_the_initials_index() {
        let mut a = Dictionary::from_tsv("ni'hao\t你好\t5\n").unwrap();
        assert!(a.lookup_initials("nm", 10).is_empty());
        let b = Dictionary::from_tsv("ni'men\t你们\t4\n").unwrap();
        a.merge(&b);
        assert_eq!(a.lookup_initials("nm", 10), vec!["ni'men"]);
        assert_eq!(a.lookup_initials("nh", 10), vec!["ni'hao"]);
    }

    #[test]
    fn merge_sums_and_preserves_missing() {
        let a = Dictionary::from_tsv("ni\t你\t5\nshi\t是\t7\n").unwrap();
        let b = Dictionary::from_tsv("ni\t你\t3\nni\t尼\t1\nhao\t好\t2\n").unwrap();
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
        let d = Dictionary::builtin();
        assert!(d.len() >= 60);
        assert_eq!(d.lookup("ni'hao").unwrap()[0].0, "你好");
        assert!(d.lookup("xian").unwrap().iter().any(|(h, _)| h == "先"));
        assert!(d.lookup("xi'an").unwrap()[0].0 == "西安");
        let iterated = d.iter().count();
        assert_eq!(iterated, d.len());
    }
    // ---- `.ior` binary loading (v2 + legacy v1) -------------------------

    /// Encode `(key, [(hanzi, freq)])` groups as a hand-built v2 buffer.
    fn v2_bytes(groups: &[(&str, &[(&str, u32)])]) -> Vec<u8> {
        fn varint(out: &mut Vec<u8>, mut value: u32) {
            loop {
                let mut byte = (value & 0x7f) as u8;
                value >>= 7;
                if value != 0 {
                    byte |= 0x80;
                }
                out.push(byte);
                if value == 0 {
                    return;
                }
            }
        }
        let total: usize = groups.iter().map(|(_, vals)| vals.len()).sum();
        let mut out = Vec::new();
        out.extend_from_slice(b"IORD");
        out.push(2);
        out.extend_from_slice(&(total as u32).to_le_bytes());
        out.extend_from_slice(&(groups.len() as u32).to_le_bytes());
        for &(key, vals) in groups {
            varint(&mut out, key.len() as u32);
            out.extend_from_slice(key.as_bytes());
            varint(&mut out, vals.len() as u32);
        }
        for &(_, vals) in groups {
            for &(hanzi, freq) in vals {
                varint(&mut out, hanzi.len() as u32);
                out.extend_from_slice(hanzi.as_bytes());
                varint(&mut out, freq);
            }
        }
        out
    }

    /// Encode flat entries as a hand-built legacy v1 buffer.
    fn v1_bytes(entries: &[(&str, &str, u32)]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"IORD");
        out.push(1);
        out.extend_from_slice(&(entries.len() as u32).to_le_bytes());
        for (pinyin, hanzi, freq) in entries {
            for field in [pinyin, hanzi] {
                out.extend_from_slice(&(field.len() as u32).to_le_bytes());
                out.extend_from_slice(field.as_bytes());
            }
            out.extend_from_slice(&freq.to_le_bytes());
        }
        out
    }

    #[test]
    fn from_ior_bytes_v2_groups_values_and_sorts() {
        let bytes = v2_bytes(&[("ni", &[("你", 9u32), ("尼", 1)]), ("hao", &[("好", 4)])]);
        let d = Dictionary::from_ior_bytes(&bytes).unwrap();
        assert_eq!(d.len(), 2);
        let ni = d.lookup("ni").unwrap();
        assert_eq!(&ni[0], &("你".to_string(), 9));
        assert_eq!(&ni[1], &("尼".to_string(), 1));
        assert_eq!(&d.lookup("hao").unwrap()[0], &("好".to_string(), 4));
    }

    #[test]
    fn from_ior_bytes_reads_legacy_v1() {
        let bytes = v1_bytes(&[("ni", "你", 9), ("ni", "尼", 1), ("hao", "好", 4)]);
        let d = Dictionary::from_ior_bytes(&bytes).unwrap();
        assert_eq!(d.len(), 2);
        assert_eq!(d.lookup("ni").unwrap().len(), 2);
        assert_eq!(&d.lookup("ni").unwrap()[0], &("你".to_string(), 9));
    }

    #[test]
    fn from_ior_bytes_sums_duplicate_pairs_inside_a_key() {
        // A crafted v2 file can repeat a hanzi under one key; freq adds up.
        let bytes = v2_bytes(&[("ni", &[("你", 5), ("你", 3)])]);
        let d = Dictionary::from_ior_bytes(&bytes).unwrap();
        assert_eq!(d.lookup("ni").unwrap(), &[("你".to_string(), 8)]);
    }

    #[test]
    fn from_ior_bytes_rejects_corrupt_input() {
        let good = v2_bytes(&[("ni", &[("你", 9)])]);
        let mut bad_magic = good.clone();
        bad_magic[0] = b'X';
        assert!(Dictionary::from_ior_bytes(&bad_magic)
            .unwrap_err()
            .contains("magic"));
        let mut bad_version = good.clone();
        bad_version[4] = 7;
        assert!(Dictionary::from_ior_bytes(&bad_version)
            .unwrap_err()
            .contains("unsupported version"));
        let mut count_mismatch = good;
        count_mismatch[5..9].copy_from_slice(&9u32.to_le_bytes());
        assert!(Dictionary::from_ior_bytes(&count_mismatch)
            .unwrap_err()
            .contains("mismatch"));
        let mut truncated = v2_bytes(&[("ni", &[("你", 9)]), ("hao", &[("好", 4)])]);
        truncated.truncate(truncated.len() - 3);
        assert!(Dictionary::from_ior_bytes(&truncated).is_err());
        let mut trailing = v2_bytes(&[("ni", &[("你", 9)])]);
        trailing.push(0);
        assert!(Dictionary::from_ior_bytes(&trailing)
            .unwrap_err()
            .contains("trailing"));
    }

    #[test]
    fn from_ior_bytes_keeps_canonical_keys_like_from_tsv() {
        // P1: `.ior` loading no longer applies fuzzy rules either; the same
        // bytes and the same TSV text produce the same canonical dictionary.
        let bytes = v2_bytes(&[("li", &[("里", 5)]), ("ni", &[("你", 9)])]);
        let d = Dictionary::from_ior_bytes(&bytes).unwrap();
        let tsv = Dictionary::from_tsv("li\t里\t5\nni\t你\t9\n").unwrap();
        assert_eq!(d.len(), tsv.len());
        assert_eq!(d.lookup("li").unwrap(), tsv.lookup("li").unwrap());
        assert_eq!(d.lookup("ni").unwrap(), tsv.lookup("ni").unwrap());
        assert_eq!(d.lookup("ni").unwrap().len(), 1);
    }

    #[test]
    fn from_ior_layers_overlays_in_order_and_sums() {
        let base = v2_bytes(&[("ni", &[("你", 5)]), ("shi", &[("是", 7)])]);
        let domain = v2_bytes(&[("ni", &[("你", 3), ("尼", 1)])]);
        let user = v2_bytes(&[("hao", &[("好", 2)])]);
        let d = Dictionary::from_ior_layers(&[&base, &domain, &user]).unwrap();
        let ni = d.lookup("ni").unwrap();
        assert_eq!(&ni[0], &("你".to_string(), 8)); // 5 + 3 across layers
        assert_eq!(&ni[1], &("尼".to_string(), 1));
        assert_eq!(&d.lookup("shi").unwrap()[0], &("是".to_string(), 7));
        assert_eq!(&d.lookup("hao").unwrap()[0], &("好".to_string(), 2));
        assert_eq!(d.len(), 3);
    }

    #[test]
    fn from_ior_layers_accepts_empty_and_propagates_errors() {
        let empty = Dictionary::from_ior_layers(&[]).unwrap();
        assert!(empty.is_empty());
        let bad = vec![b"IORX".as_slice()];
        assert!(Dictionary::from_ior_layers(&bad).is_err());
    }
}
