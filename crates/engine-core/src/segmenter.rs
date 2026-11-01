//! Pinyin segmentation: split a letter-run into valid syllables.
//!
//! [`segmentations`] enumerates every valid split of a lowercase `a`-`z` run
//! into syllables from [`crate::syllable::SYLLABLES`], ordered by fewest
//! segments first, then lexicographically. It is a bottom-up DP over suffixes:
//! `dp[i]` holds the (already sorted, deduped) segmentations of `raw[i..]`,
//! built from `dp[i + len]` for every syllable-length `len` starting at `i`.
//!
//! Construction cannot emit duplicates (each segmentation is uniquely
//! identified by its first syllable plus a distinct tail); an explicit
//! dedup pass keeps that guarantee explicit.

use crate::syllable::is_syllable;

/// Inputs longer than this are rejected outright.
pub const MAX_INPUT_LEN: usize = 64;

/// Longest possible syllable ("chuang"/"shuang"/"zhuang").
const MAX_SYLLABLE_LEN: usize = 6;

/// Guard against pathological exponential blow-up (e.g. "ananana..."):
/// at most this many segmentations are kept per suffix. Ordinary IME input
/// never comes close; truncation keeps the worst case bounded.
const MAX_SEGS_PER_SUFFIX: usize = 1024;

/// Orders two segmentations: fewest segments first, then lexicographic.
fn compare_segs(a: &[String], b: &[String]) -> std::cmp::Ordering {
    a.len().cmp(&b.len()).then_with(|| a.cmp(b))
}

/// All valid syllable splits of `raw` (lowercase `a`-`z` only), ordered by
/// fewest segments first, then lexicographically. Returns an empty `Vec` for
/// empty input, non-`a`-`z` characters, inputs longer than [`MAX_INPUT_LEN`],
/// or when no valid split exists.
pub fn segmentations(raw: &str) -> Vec<Vec<String>> {
    if raw.is_empty() || raw.len() > MAX_INPUT_LEN {
        return Vec::new();
    }
    if !raw.bytes().all(|b| b.is_ascii_lowercase()) {
        return Vec::new();
    }
    let n = raw.len();
    // dp[i] = sorted segmentations of the suffix raw[i..]; dp[n] = one empty split.
    let mut dp: Vec<Vec<Vec<String>>> = vec![Vec::new(); n + 1];
    dp[n] = vec![Vec::new()];
    for i in (0..n).rev() {
        let mut segs: Vec<Vec<String>> = Vec::new();
        let max_len = MAX_SYLLABLE_LEN.min(n - i);
        for len in 1..=max_len {
            let head = &raw[i..i + len];
            if !is_syllable(head) {
                continue;
            }
            for tail in &dp[i + len] {
                let mut seg = Vec::with_capacity(tail.len() + 1);
                seg.push(head.to_string());
                seg.extend(tail.iter().cloned());
                segs.push(seg);
            }
        }
        segs.sort_by(|a, b| compare_segs(a, b));
        segs.dedup();
        segs.truncate(MAX_SEGS_PER_SUFFIX);
        dp[i] = segs;
    }
    dp.into_iter().next().unwrap_or_default()
}

/// The preferred segmentation (fewest segments, then lexicographically first),
/// or `None` when `raw` cannot be split into syllables.
pub fn best_segmentation(raw: &str) -> Option<Vec<String>> {
    segmentations(raw).into_iter().next()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn best_two_syllable() {
        assert_eq!(
            best_segmentation("nihao"),
            Some(vec!["ni".into(), "hao".into()])
        );
    }

    #[test]
    fn xian_prefers_whole_then_split() {
        let segs = segmentations("xian");
        assert_eq!(segs.first(), Some(&vec!["xian".to_string()]));
        assert!(segs.contains(&vec!["xi".to_string(), "an".to_string()]));
        assert_eq!(segs.len(), 2);
    }

    #[test]
    fn unsegmentable_inputs() {
        assert!(segmentations("nih").is_empty());
        assert!(segmentations("").is_empty());
        assert!(segmentations("zh").is_empty());
        assert!(segmentations("Ni").is_empty());
        assert!(segmentations("ni2").is_empty());
    }

    #[test]
    fn too_long_input_rejected() {
        let long = "a".repeat(MAX_INPUT_LEN + 1);
        assert!(segmentations(&long).is_empty());
    }

    #[test]
    fn ordering_and_separators() {
        let segs = segmentations("anan");
        // fewest segments first, then lexicographic ("a" < "an")
        assert_eq!(
            segs,
            vec![
                vec!["a".to_string(), "nan".to_string()],
                vec!["an".to_string(), "an".to_string()],
            ]
        );
        assert_eq!(best_segmentation("xian"), Some(vec!["xian".to_string()]));
    }

    #[test]
    fn houxuanci_prefers_xuan_then_keeps_split() {
        // Regression: without `xuan` in the table the only split of
        // `houxuanci` was `hou|xu|an|ci`, so 候选词 (hou'xuan'ci) was
        // unreachable.
        let segs = segmentations("houxuanci");
        assert_eq!(
            segs.first(),
            Some(&vec![
                "hou".to_string(),
                "xuan".to_string(),
                "ci".to_string()
            ])
        );
        assert!(segs.contains(&vec![
            "hou".to_string(),
            "xu".to_string(),
            "an".to_string(),
            "ci".to_string()
        ]));
    }

    #[test]
    fn apostrophe_runs_are_not_segmented_here() {
        // separator handling lives in the decoder/session, not here
        assert!(segmentations("ni'hao").is_empty());
    }
}
