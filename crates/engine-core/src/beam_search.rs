//! Word lattice and beam-walk for the sentence decoder (P2/M2b).
//!
//! [`crate::beam`] owns the gate, the ranking merge and the tunables; this
//! module builds the per-segmentation **word lattice** (dictionary +
//! user-dictionary candidates per `(start, word length)` slot, probed
//! through the query-side spelling pipeline) and walks it with a
//! bucket-per-position beam: each round consumes the finished states of
//! one position and appends successors into later buckets, so states at
//! different consumed lengths never crowd each other out of the beam.

use std::collections::BTreeMap;

use crate::beam::BeamParams;
use crate::config::FuzzyConfig;
use crate::decoder::{dict_lookup_variants, user_lookup_variants, USER_BOOST};
use crate::dict::Dictionary;
use crate::lm::GramModel;
use crate::user_dict::UserDict;

/// One word candidate at a lattice slot (0 = not present).
struct WordCand {
    hanzi: String,
    dict_freq: u32,
    user_freq: u32,
}

/// Word candidates per `(start, word length in syllables)`; a start
/// position with no candidates has an empty map.
pub(crate) struct Lattice {
    slots: Vec<BTreeMap<usize, Vec<WordCand>>>,
}

/// One beam state: a partial decoding of the syllable sequence.
struct BeamState {
    pos: usize,
    text: String,
    last2: Option<char>,
    last1: Option<char>,
    score: f64,
}

/// Outcome of [`search`]: complete decodings `(hanzi, score)` best-first,
/// plus the best partial prefix decodings `hanzi -> (score, position)`.
pub(crate) type SearchOutcome = (Vec<(String, f64)>, BTreeMap<String, (f64, usize)>);

/// Probes every `(start, length)` key through the spelling pipeline and
/// keeps the merged user+dictionary candidates. Additionally the
/// whole-sentence key at `(0, n)` is always probed so long dict phrases
/// and learned user sentences are not cut off by `max_word_syllables`.
pub(crate) fn build_lattice(
    dict: &Dictionary,
    user: Option<&UserDict>,
    fuzzy: &FuzzyConfig,
    syllables: &[String],
    params: &BeamParams,
) -> Lattice {
    let count = syllables.len();
    let mut slots: Vec<BTreeMap<usize, Vec<WordCand>>> =
        (0..count).map(|_| BTreeMap::new()).collect();
    for start in 0..count {
        let longest = params.max_word_syllables.min(count - start);
        for len in 1..=longest {
            let key = syllables[start..start + len].join("'");
            let words = slot_words(dict, user, fuzzy, &key, params.candidates_per_slot);
            if !words.is_empty() {
                slots[start].insert(len, words);
            }
        }
    }
    if count > params.max_word_syllables {
        let key = syllables.join("'");
        let words = slot_words(dict, user, fuzzy, &key, params.candidates_per_slot);
        if !words.is_empty() {
            slots[0].insert(count, words);
        }
    }
    Lattice { slots }
}

/// User hits first (frequency desc), then dictionary hits not already
/// present; a hanzi found in both keeps its maximum dictionary frequency.
fn slot_words(
    dict: &Dictionary,
    user: Option<&UserDict>,
    fuzzy: &FuzzyConfig,
    key: &str,
    cap: usize,
) -> Vec<WordCand> {
    let mut out: Vec<WordCand> = Vec::new();
    if let Some(u) = user {
        for (hanzi, freq) in user_lookup_variants(u, fuzzy, key) {
            out.push(WordCand {
                hanzi,
                dict_freq: 0,
                user_freq: freq,
            });
        }
    }
    for (hanzi, freq) in dict_lookup_variants(dict, fuzzy, key) {
        match out.iter_mut().find(|w| w.hanzi == hanzi) {
            Some(existing) => existing.dict_freq = existing.dict_freq.max(freq),
            None => out.push(WordCand {
                hanzi,
                dict_freq: freq,
                user_freq: 0,
            }),
        }
    }
    out.truncate(cap);
    out
}

/// Walks the lattice position by position. Returns the complete states
/// (every syllable consumed) and the best partial states (prefix decodings)
/// seen along the way, both best-score-first.
pub(crate) fn search(
    syllables: &[String],
    lattice: &Lattice,
    lm: &GramModel,
    params: &BeamParams,
) -> SearchOutcome {
    let count = syllables.len();
    let mut buckets: Vec<Vec<BeamState>> = (0..=count).map(|_| Vec::new()).collect();
    buckets[0].push(BeamState {
        pos: 0,
        text: String::new(),
        last2: None,
        last1: None,
        score: 0.0,
    });
    let mut partials: BTreeMap<String, (f64, usize)> = BTreeMap::new();
    for pos in 0..=count {
        let bucket = prune(std::mem::take(&mut buckets[pos]), params.beam_width);
        if bucket.is_empty() {
            continue;
        }
        if pos == count {
            let complete = bucket
                .into_iter()
                .map(|state| (state.text, state.score))
                .collect();
            return (complete, partials);
        }
        for state in &bucket {
            if !state.text.is_empty() {
                match partials.get(&state.text) {
                    Some((best, _)) if *best >= state.score => {}
                    _ => {
                        partials.insert(state.text.clone(), (state.score, state.pos));
                    }
                }
            }
            for (len, words) in &lattice.slots[pos] {
                let Some(end) = pos.checked_add(*len) else {
                    continue;
                };
                if end > count {
                    continue;
                }
                for word in words {
                    buckets[end].push(expand(state, end, word, lm, params));
                }
            }
        }
    }
    (Vec::new(), partials)
}

/// Sorts by score descending (text ascending on exact ties), drops states
/// that produced the same text, and truncates to the beam width.
fn prune(mut bucket: Vec<BeamState>, width: usize) -> Vec<BeamState> {
    bucket.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.text.cmp(&b.text))
    });
    bucket.dedup_by(|a, b| a.text == b.text);
    bucket.truncate(width);
    bucket
}

/// Produces the successor of `state` after consuming `word` ending at
/// syllable position `end`.
fn expand(
    state: &BeamState,
    end: usize,
    word: &WordCand,
    lm: &GramModel,
    params: &BeamParams,
) -> BeamState {
    let (lm_score, last2, last1) = append_logp(lm, params, state.last2, state.last1, &word.hanzi);
    let mut text = state.text.clone();
    text.push_str(&word.hanzi);
    BeamState {
        pos: end,
        text,
        last2,
        last1,
        score: state.score + lm_score + freq_term(word, params),
    }
}

/// LM score of appending `hanzi` to the context `(prev2, prev1)`, plus the
/// resulting two-char context.
fn append_logp(
    lm: &GramModel,
    params: &BeamParams,
    prev2: Option<char>,
    prev1: Option<char>,
    hanzi: &str,
) -> (f64, Option<char>, Option<char>) {
    let mut total = 0.0;
    let (mut ctx2, mut ctx1) = (prev2, prev1);
    for c in hanzi.chars() {
        total += params.lm_weight * lm.logp(ctx2, ctx1, c);
        ctx2 = ctx1;
        ctx1 = Some(c);
    }
    (total, ctx2, ctx1)
}

/// Frequency term of one chosen word: dictionary frequency plus the user
/// boost (learned words outrank static ones, as in the legacy decoder).
fn freq_term(word: &WordCand, params: &BeamParams) -> f64 {
    let mut score = params.freq_weight * (word.dict_freq.max(1) as f64).ln();
    if word.user_freq > 0 {
        score += params.user_weight * ((USER_BOOST + word.user_freq) as f64).ln();
    }
    score
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::dict::Dictionary;
    use crate::lm;

    /// Shared fixture: a small realistic word list covering the sentence
    /// `jin'tian'tian'qi'bu'cuo` with real words and single-char decoys.
    pub(crate) const FIXTURE_TSV: &str = "jin\t今\t300\njin\t金\t500\ntian\t天\t900\nqi\t气\t400\n\
        bu\t不\t800\nbu\t补\t200\ncuo\t错\t100\njin'tian\t今天\t4500\njin'tian\t金田\t800\n\
        tian'qi\t天气\t3800\nbu'cuo\t不错\t2900\nbu'cuo\t补错\t500\nhen\t很\t600\nhao\t好\t700\n\
        hen'hao\t很好\t3200\nni\t你\t5000\nni'hao\t你好\t8000\n";

    pub(crate) fn fixture_dict() -> Dictionary {
        Dictionary::from_tsv(FIXTURE_TSV).unwrap()
    }

    /// Hand-assembled .gram trained (offline, by hand) from FIXTURE_TSV:
    /// within-word char n-grams weighted by word frequency.
    pub(crate) fn fixture_gram() -> lm::GramModel {
        let unigrams: &[(&str, u32)] = &[
            ("今", 4800),  // 今300 + 今天4500
            ("金", 1300),  // 金500 + 金田800
            ("天", 9200),  // 天900 + 今天4500 + 天气3800
            ("气", 4200),  // 气400 + 天气3800
            ("不", 3700),  // 不800 + 不错2900
            ("补", 700),   // 补200 + 补错500
            ("错", 3500),  // 错100 + 不错2900 + 补错500
            ("田", 800),   // 金田800
            ("很", 3800),  // 很600 + 很好3200
            ("好", 11900), // 好700 + 你好8000 + 很好3200
            ("你", 13000), // 你5000 + 你好8000
        ];
        let bigrams: &[(&str, u32)] = &[
            ("今天", 4500),
            ("天气", 3800),
            ("不错", 2900),
            ("补错", 500),
            ("金田", 800),
            ("很好", 3200),
            ("你好", 8000),
        ];
        let mut out = Vec::new();
        out.extend_from_slice(lm::GRAM_MAGIC);
        out.push(lm::GRAM_VERSION);
        out.push(lm::GRAM_ORDER);
        out.extend_from_slice(&(unigrams.len() as u32).to_le_bytes());
        out.extend_from_slice(&(bigrams.len() as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        let total: u32 = unigrams.iter().map(|&(_, c)| c).sum();
        out.extend_from_slice(&total.to_le_bytes());
        let mut sorted: Vec<(&str, u32)> = unigrams.to_vec();
        sorted.sort_unstable();
        for (key, _) in &sorted {
            push_varint(&mut out, key.len() as u32);
            out.extend_from_slice(key.as_bytes());
        }
        for (_, count) in &sorted {
            push_varint(&mut out, *count);
        }
        let mut sorted_bigrams: Vec<(&str, u32)> = bigrams.to_vec();
        sorted_bigrams.sort_unstable();
        for (key, _) in &sorted_bigrams {
            push_varint(&mut out, key.len() as u32);
            out.extend_from_slice(key.as_bytes());
        }
        for (_, count) in &sorted_bigrams {
            push_varint(&mut out, *count);
        }
        lm::from_gram_bytes(&out).unwrap()
    }

    /// LEB128 u32 varint writer (mirrors the .gram spec; test-side only).
    fn push_varint(out: &mut Vec<u8>, value: u32) {
        let mut remaining = value;
        loop {
            let mut byte = (remaining & 0x7f) as u8;
            remaining >>= 7;
            if remaining != 0 {
                byte |= 0x80;
            }
            out.push(byte);
            if remaining == 0 {
                return;
            }
        }
    }

    #[test]
    fn lattice_slots_cover_prefixes_and_the_full_key() {
        let dict = fixture_dict();
        let params = BeamParams::defaults();
        let syllables: Vec<String> = ["jin", "tian", "tian", "qi", "bu", "cuo"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let lattice = build_lattice(&dict, None, &FuzzyConfig::default(), &syllables, &params);
        assert!(lattice.slots[0].contains_key(&1)); // 今/金
        assert!(lattice.slots[0].contains_key(&2)); // 今天/金田
        assert!(lattice.slots[2].contains_key(&2)); // 天气
        assert!(lattice.slots[5].contains_key(&1)); // 错
                                                    // no whole-sentence dictionary entry: the (0, 6) slot stays empty
        assert!(!lattice.slots[0].contains_key(&6));
        // but a learned user sentence is probed there (n == max_word_syllables)
        let mut user = crate::user_dict::new();
        crate::user_dict::learn(&mut user, "jin'tian'tian'qi'bu'cuo", "今天天气不错");
        let lattice = build_lattice(
            &dict,
            Some(&user),
            &FuzzyConfig::default(),
            &syllables,
            &params,
        );
        assert!(lattice.slots[0].contains_key(&6));
    }

    #[test]
    fn search_finds_the_word_segmentation() {
        let dict = fixture_dict();
        let params = BeamParams::defaults();
        let syllables: Vec<String> = ["jin", "tian", "tian", "qi", "bu", "cuo"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let lattice = build_lattice(&dict, None, &FuzzyConfig::default(), &syllables, &params);
        let (complete, partials) = search(&syllables, &lattice, &fixture_gram(), &params);
        assert!(complete.iter().any(|(text, _)| text == "今天天气不错"));
        assert!(partials.contains_key("今天"));
    }

    #[test]
    fn slot_words_merge_user_and_dict_frequencies() {
        let dict = fixture_dict();
        let mut user = crate::user_dict::new();
        for _ in 0..3 {
            crate::user_dict::learn(&mut user, "bu'cuo", "不错");
        }
        let words = slot_words(&dict, Some(&user), &FuzzyConfig::default(), "bu'cuo", 12);
        let cuo: Vec<&WordCand> = words.iter().filter(|w| w.hanzi == "不错").collect();
        assert_eq!(cuo.len(), 1, "user and dict hits merge by hanzi");
        assert_eq!(cuo[0].dict_freq, 2900);
        assert_eq!(cuo[0].user_freq, 3);
        // user entries lead the slot
        assert_eq!(words[0].hanzi, "不错");
    }
}
