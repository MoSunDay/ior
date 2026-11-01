//! Prefix index for unfinished pinyin input.
//!
//! Keys are stored sorted by their separator-free spelling so a query can
//! binary-search the range for `nih` and find `ni'hao`, `ni'hai`, ... without
//! walking the whole dictionary. This is the fallback used only when the
//! regular full segmentation has no candidates (see `decoder`).

use std::collections::HashMap;

/// Sorts canonical keys by separator-free spelling. Equal flat spellings
/// (e.g. `xian` and `xi'an`) are tie-broken by the canonical key.
pub(crate) fn build(entries: &HashMap<String, Vec<(String, u32)>>) -> Vec<String> {
    let mut keys: Vec<String> = entries.keys().cloned().collect();
    keys.sort_by(|a, b| flat_cmp(a, b).then_with(|| a.cmp(b)));
    keys
}

/// Up to `limit` highest-frequency entries whose key starts with `prefix`
/// after separators are removed. One entry per key, deduped by hanzi.
pub(crate) fn lookup(
    keys: &[String],
    entries: &HashMap<String, Vec<(String, u32)>>,
    prefix: &str,
    limit: usize,
) -> Vec<(String, String, u32)> {
    if limit == 0 {
        return Vec::new();
    }
    let flat: String = prefix.chars().filter(|ch| *ch != '\'').collect();
    if flat.is_empty() || !flat.bytes().all(|byte| byte.is_ascii_lowercase()) {
        return Vec::new();
    }
    let start = keys.partition_point(|key| flat_cmp(key, &flat) == std::cmp::Ordering::Less);
    let mut hits: Vec<(u32, &str, &str)> = Vec::new();
    for key in &keys[start..] {
        if !flat_starts_with(key, &flat) {
            break;
        }
        if let Some((hanzi, freq)) = entries.get(key).and_then(|vals| vals.first()) {
            hits.push((*freq, key.as_str(), hanzi.as_str()));
        }
    }
    hits.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| a.1.cmp(b.1))
            .then_with(|| a.2.cmp(b.2))
    });
    let mut out: Vec<(String, String, u32)> = Vec::new();
    for (freq, key, hanzi) in hits {
        if out.iter().any(|(_, text, _)| text == hanzi) {
            continue;
        }
        out.push((key.to_string(), hanzi.to_string(), freq));
        if out.len() == limit {
            break;
        }
    }
    out
}

/// Compares two pinyin keys byte-by-byte while ignoring `'` separators.
fn flat_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let mut left = a.bytes().filter(|byte| *byte != b'\'');
    let mut right = b.bytes().filter(|byte| *byte != b'\'');
    loop {
        match (left.next(), right.next()) {
            (Some(l), Some(r)) => match l.cmp(&r) {
                std::cmp::Ordering::Equal => {}
                other => return other,
            },
            (Some(_), None) => return std::cmp::Ordering::Greater,
            (None, Some(_)) => return std::cmp::Ordering::Less,
            (None, None) => return std::cmp::Ordering::Equal,
        }
    }
}

/// Whether `key`'s separator-free spelling starts with `prefix`.
fn flat_starts_with(key: &str, prefix: &str) -> bool {
    let mut wanted = prefix.bytes();
    for byte in key.bytes().filter(|byte| *byte != b'\'') {
        match wanted.next() {
            Some(expected) if expected == byte => {}
            Some(_) => return false,
            None => return true,
        }
    }
    wanted.next().is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dictionary(tsv: &str) -> HashMap<String, Vec<(String, u32)>> {
        let mut entries = HashMap::new();
        for line in tsv.lines() {
            let mut fields = line.split('\t');
            let key = fields.next().unwrap().to_string();
            let text = fields.next().unwrap().to_string();
            let freq: u32 = fields.next().unwrap().parse().unwrap();
            entries
                .entry(key)
                .or_insert_with(Vec::new)
                .push((text, freq));
        }
        for vals in entries.values_mut() {
            vals.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        }
        entries
    }

    #[test]
    fn prefix_matches_ignore_apostrophes_and_rank_by_frequency() {
        let entries = dictionary(
            "ni\t你\t100\nna\t那\t90\nni'hao\t你好\t200\nni'hai\t你害\t80\nxian\t先\t70\nxi'an\t西安\t60\n",
        );
        let keys = build(&entries);
        assert_eq!(
            lookup(&keys, &entries, "n", 2),
            vec![
                ("ni'hao".to_string(), "你好".to_string(), 200),
                ("ni".to_string(), "你".to_string(), 100),
            ]
        );
        assert_eq!(
            lookup(&keys, &entries, "nih", 10),
            vec![
                ("ni'hao".to_string(), "你好".to_string(), 200),
                ("ni'hai".to_string(), "你害".to_string(), 80),
            ]
        );
        assert_eq!(lookup(&keys, &entries, "ni'h", 10).len(), 2);
        assert!(lookup(&keys, &entries, "NI", 10).is_empty());
        assert!(lookup(&keys, &entries, "", 10).is_empty());
        assert!(lookup(&keys, &entries, "n", 0).is_empty());
    }

    #[test]
    fn flat_collisions_are_both_reachable() {
        let entries = dictionary("xian\t先\t70\nxi'an\t西安\t60\n");
        let keys = build(&entries);
        let hits = lookup(&keys, &entries, "xian", 10);
        assert_eq!(hits.len(), 2);
        assert!(hits.iter().any(|(_, text, _)| text == "先"));
        assert!(hits.iter().any(|(_, text, _)| text == "西安"));
    }
}
