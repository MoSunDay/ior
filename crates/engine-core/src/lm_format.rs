//! Byte-level `.gram` reader primitives (P2/M2b).
//!
//! This module only turns bytes into raw count maps with precise,
//! offset-carrying errors; the authoritative format spec, the packed-key
//! convention and the scoring model live in [`crate::lm`]. Kept separate
//! so `lm.rs` stays focused on the model.

use std::collections::HashMap;
use std::hash::Hash;

use crate::lm::{bi_key, tri_key, GRAM_MAGIC, GRAM_ORDER, GRAM_VERSION};

/// Raw parsed `.gram` counts before [`crate::lm`] wraps them into a model.
#[derive(Debug)]
pub(crate) struct RawGram {
    /// Sum of all unigram counts (denominator of the unigram level).
    pub total: u32,
    /// char code -> count.
    pub unigrams: HashMap<u32, u32>,
    /// packed char pair -> count (see [`bi_key`]).
    pub bigrams: HashMap<u64, u32>,
    /// packed char triple -> count (see [`tri_key`]).
    pub trigrams: HashMap<u64, u32>,
}

/// Fixed header size: magic (6) + version (1) + order (1) + 4 x u32.
const HEADER_BYTES: usize = 24;

/// Parses a whole `.gram` buffer. Every failure names the section (and the
/// key index or byte offset) it was detected at.
pub(crate) fn parse_gram(bytes: &[u8]) -> Result<RawGram, String> {
    let found = bytes.get(0..GRAM_MAGIC.len()).unwrap_or(&[]);
    if found != GRAM_MAGIC.as_slice() {
        return Err(format!(
            "bad magic at offset 0: expected {:?} (\"IOGRAM\"), found {found:?}",
            GRAM_MAGIC.as_slice()
        ));
    }
    let version = *bytes.get(6).ok_or_else(|| truncated(1, 6, bytes.len()))?;
    if version != GRAM_VERSION {
        return Err(format!(
            "unsupported .gram version {version} at offset 6 (expected {GRAM_VERSION})"
        ));
    }
    let order = *bytes.get(7).ok_or_else(|| truncated(1, 7, bytes.len()))?;
    if order != GRAM_ORDER {
        return Err(format!(
            "unsupported .gram order {order} at offset 7 (expected {GRAM_ORDER} = trigram)"
        ));
    }
    if bytes.len() < HEADER_BYTES {
        return Err(truncated(HEADER_BYTES, 0, bytes.len()));
    }
    let (uni_count, pos) = read_u32(bytes, 8)?;
    let (bi_count, pos) = read_u32(bytes, pos)?;
    let (tri_count, pos) = read_u32(bytes, pos)?;
    let (total, pos) = read_u32(bytes, pos)?;

    let (unigrams, pos) = parse_section(bytes, pos, uni_count, "unigram", pack_unigram)?;
    let (bigrams, pos) = parse_section(bytes, pos, bi_count, "bigram", pack_bigram)?;
    let (trigrams, pos) = parse_section(bytes, pos, tri_count, "trigram", pack_trigram)?;
    if pos != bytes.len() {
        return Err(format!(
            "trailing data: {} byte(s) after the trigram section (offset {pos}, buffer is {} byte(s))",
            bytes.len() - pos,
            bytes.len()
        ));
    }
    let sum: u64 = unigrams.values().map(|&count| u64::from(count)).sum();
    if sum != u64::from(total) {
        return Err(format!(
            "total weight mismatch: header says {total}, unigram counts sum to {sum}"
        ));
    }
    Ok(RawGram {
        total,
        unigrams,
        bigrams,
        trigrams,
    })
}

/// Parses one section: `count` index records (`varint(len)` + utf8 key,
/// strictly ascending in byte order, exactly the chars its order demands)
/// followed by `count` varint values zipped by position.
fn parse_section<K, F>(
    bytes: &[u8],
    mut pos: usize,
    count: u32,
    name: &str,
    pack: F,
) -> Result<(HashMap<K, u32>, usize), String>
where
    K: Hash + Eq + Copy,
    F: Fn(&str) -> Result<K, String>,
{
    let mut keys: Vec<K> = Vec::new();
    let mut previous: Option<&str> = None;
    for index in 0..count {
        let no = index + 1;
        let (len, next) =
            read_varint(bytes, pos).map_err(|err| format!("{name} key {no} length: {err}"))?;
        let (key, next) =
            read_sized(bytes, next, len).map_err(|err| format!("{name} key {no}: {err}"))?;
        if let Some(last) = previous {
            if last >= key {
                return Err(format!(
                    "{name} key {no} {key:?}: keys must be distinct and ascending (previous {last:?})"
                ));
            }
        }
        keys.push(pack(key).map_err(|err| format!("{name} key {no} {key:?}: {err}"))?);
        previous = Some(key);
        pos = next;
    }
    let mut map: HashMap<K, u32> = HashMap::new();
    for (index, key) in keys.iter().enumerate() {
        let (value, next) = read_varint(bytes, pos)
            .map_err(|err| format!("{name} key {} value: {err}", index + 1))?;
        map.insert(*key, value);
        pos = next;
    }
    Ok((map, pos))
}

/// Packs a one-char unigram key (validated; error text names the rule).
fn pack_unigram(key: &str) -> Result<u32, String> {
    let mut chars = key.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => Ok(c as u32),
        _ => Err("must hold exactly one char".to_string()),
    }
}

/// Packs a two-char bigram key (validated).
fn pack_bigram(key: &str) -> Result<u64, String> {
    let chars: Vec<char> = key.chars().collect();
    if chars.len() != 2 {
        return Err("must hold exactly two chars".to_string());
    }
    Ok(bi_key(chars[0], chars[1]))
}

/// Packs a three-char trigram key (validated).
fn pack_trigram(key: &str) -> Result<u64, String> {
    let chars: Vec<char> = key.chars().collect();
    if chars.len() != 3 {
        return Err("must hold exactly three chars".to_string());
    }
    Ok(tri_key(chars[0], chars[1], chars[2]))
}

/// Reads one byte at `pos`; returns `(value, next_pos)`.
fn read_u8(bytes: &[u8], pos: usize) -> Result<(u8, usize), String> {
    bytes
        .get(pos)
        .map(|value| (*value, pos + 1))
        .ok_or_else(|| truncated(1, pos, bytes.len()))
}

/// Reads one little-endian u32 at `pos`; returns `(value, next_pos)`.
fn read_u32(bytes: &[u8], pos: usize) -> Result<(u32, usize), String> {
    let raw = bytes
        .get(pos..pos.saturating_add(4))
        .ok_or_else(|| truncated(4, pos, bytes.len()))?;
    let mut buf = [0u8; 4];
    buf.copy_from_slice(raw);
    Ok((u32::from_le_bytes(buf), pos + 4))
}

/// Reads one strict LEB128 u32 varint at `pos` (same rules as `.ior` v2).
fn read_varint(bytes: &[u8], pos: usize) -> Result<(u32, usize), String> {
    let mut result: u32 = 0;
    let mut shift: u32 = 0;
    let mut cursor = pos;
    loop {
        let (byte, next) = read_u8(bytes, cursor)?;
        cursor = next;
        let payload = u32::from(byte & 0x7f);
        if shift >= 28 {
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

/// Reads `len` utf8 bytes at `pos`, validating the length against the buffer.
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

/// Describes a truncated read.
fn truncated(needed: usize, pos: usize, len: usize) -> String {
    format!("truncated data: need {needed} byte(s) at offset {pos}, buffer is {len} byte(s)")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hand-assembled valid .gram: unigrams 你=8 好=8 金=3, bigram 你好=8.
    fn sample() -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(GRAM_MAGIC);
        out.push(GRAM_VERSION);
        out.push(GRAM_ORDER);
        out.extend_from_slice(&3u32.to_le_bytes());
        out.extend_from_slice(&1u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&19u32.to_le_bytes());
        out.extend_from_slice(&[3, 0xe4, 0xbd, 0xa0]); // 你
        out.extend_from_slice(&[3, 0xe5, 0xa5, 0xbd]); // 好
        out.extend_from_slice(&[3, 0xe9, 0x87, 0x91]); // 金
        out.extend_from_slice(&[8, 8, 3]);
        out.extend_from_slice(&[6]); // "你好" utf8 len
        out.extend_from_slice("你好".as_bytes());
        out.extend_from_slice(&[8]);
        out
    }

    #[test]
    fn parses_the_hand_assembled_sample() {
        let gram = parse_gram(&sample()).unwrap();
        assert_eq!(gram.total, 19);
        assert_eq!(gram.unigrams.len(), 3);
        assert_eq!(gram.bigrams.len(), 1);
        assert!(gram.trigrams.is_empty());
        assert_eq!(gram.unigrams[&('你' as u32)], 8);
        assert_eq!(gram.bigrams[&bi_key('你', '好')], 8);
    }

    #[test]
    fn rejects_bad_magic_version_and_order() {
        let mut bytes = sample();
        bytes[0] = b'X';
        assert!(parse_gram(&bytes).unwrap_err().contains("bad magic"));
        let mut bytes = sample();
        bytes[6] = 2;
        assert!(parse_gram(&bytes)
            .unwrap_err()
            .contains("unsupported .gram version 2"));
        let mut bytes = sample();
        bytes[7] = 2;
        assert!(parse_gram(&bytes)
            .unwrap_err()
            .contains("unsupported .gram order 2"));
    }

    #[test]
    fn rejects_mismatched_counts_trailing_and_unsorted_keys() {
        // truncated body
        let mut bytes = sample();
        bytes.truncate(bytes.len() - 1);
        assert!(parse_gram(&bytes).unwrap_err().contains("truncated"));
        // total weight altered
        let mut bytes = sample();
        bytes[23] = 20;
        assert!(parse_gram(&bytes)
            .unwrap_err()
            .contains("total weight mismatch"));
        // trailing junk
        let mut bytes = sample();
        bytes.push(0);
        assert!(parse_gram(&bytes).unwrap_err().contains("trailing data"));
        // unigram keys swapped out of ascending order
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&sample()[..24]);
        bytes.extend_from_slice(&[3, 0xe5, 0xa5, 0xbd]); // 好 first...
        bytes.extend_from_slice(&[3, 0xe4, 0xbd, 0xa0]); // ...then 你: descending
        bytes.extend_from_slice(&[3, 0xe9, 0x87, 0x91]);
        bytes.extend_from_slice(&[8, 8, 3]);
        bytes.extend_from_slice(&[6]);
        bytes.extend_from_slice("你好".as_bytes());
        bytes.extend_from_slice(&[8]);
        assert!(parse_gram(&bytes)
            .unwrap_err()
            .contains("distinct and ascending"));
    }

    #[test]
    fn rejects_wrong_key_widths() {
        // bigram section key holds three chars
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&sample()[..24]);
        bytes.extend_from_slice(&[3, 0xe4, 0xbd, 0xa0]);
        bytes.extend_from_slice(&[3, 0xe5, 0xa5, 0xbd]);
        bytes.extend_from_slice(&[3, 0xe9, 0x87, 0x91]);
        bytes.extend_from_slice(&[8, 8, 3]);
        bytes.extend_from_slice(&[9]); // len 9 = three chars
        bytes.extend_from_slice("你好金".as_bytes());
        bytes.extend_from_slice(&[8]);
        assert!(
            parse_gram(&bytes).unwrap_err().contains("bigram key 1")
                && parse_gram(&bytes)
                    .unwrap_err()
                    .contains("exactly two chars")
        );
    }
}
