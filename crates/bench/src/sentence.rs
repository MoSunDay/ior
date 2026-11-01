//! Whole-sentence decode scenario (P2/M2b): trains a `.gram` model with
//! dict-tools, loads it with engine-core, then times beam-search
//! decoding of long inputs. This doubles as a live cross-crate format
//! interlock — if either end of the `.gram` contract drifts, the bench
//! panics instead of printing numbers.

use dict_tools::format as dt_format;
use dict_tools::gram as dt_gram;
use engine_core::config::FuzzyConfig;
use engine_core::decoder;
use engine_core::dict::Dictionary;
use engine_core::lm;

use crate::synth;

/// Fixture word list (mirrors the engine-core beam tests, plus one
/// 3-char word so the trigram section is exercised end-to-end).
const SENTENCE_TSV: &str = "jin\t今\t300\njin\t金\t500\ntian\t天\t900\nqi\t气\t400\n\
    bu\t不\t800\nbu\t补\t200\ncuo\t错\t100\njin'tian\t今天\t4500\njin'tian\t金田\t800\n\
    tian'qi\t天气\t3800\nbu'cuo\t不错\t2900\nbu'cuo\t补错\t500\nhen\t很\t600\nhao\t好\t700\n\
    hen'hao\t很好\t3200\nni\t你\t5000\nni'hao\t你好\t8000\nbu'ke'qi\t不客气\t1200\n";

/// Timed `candidates_with_lm` calls per input.
const SENTENCE_RUNS: usize = 2_000;

/// Decode calls discarded before timing.
const SENTENCE_WARMUP: usize = 100;

/// Long inputs (≥ 8 syllables) routed to beam search by
/// `MIN_BEAM_SYLLABLES`.
const SENTENCE_INPUTS: &[&str] = &[
    "jintian'tianqi'bu'cuo'hen'hao",  // 8 syllables, 4 words
    "ni'hao'ni'hao'jin'tian'tian'qi", // 8 syllables, repetition
];

/// Train + load + time. Returns report rows; panics on any contract
/// mismatch between dict-tools and engine-core.
pub fn scenario() -> Vec<(String, String)> {
    let entries = dt_format::parse_tsv(SENTENCE_TSV).expect("fixture tsv");
    let entries = dt_format::normalize(entries);
    let dict = Dictionary::from_tsv(&dt_format::to_tsv(&entries)).expect("dict from fixture");

    let counts = dt_gram::train_counts(&entries);
    let gram_bytes = counts.to_gram_bytes();
    let (parse_secs, model) = timed_parse(&gram_bytes);

    let mut rows = vec![
        (
            "fixture entries / gram bytes".to_string(),
            format!("{} entries, {} bytes", entries.len(), gram_bytes.len()),
        ),
        (
            "gram keys (1/2/3-gram)".to_string(),
            format!(
                "{}/{}/{}",
                dt_gram::unigrams(&counts).len(),
                dt_gram::bigrams(&counts).len(),
                dt_gram::trigrams(&counts).len()
            ),
        ),
        (
            "from_gram_bytes parse".to_string(),
            format!("{} us", synth::micros(parse_secs)),
        ),
    ];
    for raw in SENTENCE_INPUTS {
        rows.extend(decode_rows(&dict, &model, raw));
    }
    rows
}

/// Time only the `.gram` parse (single shot: it is a startup cost).
fn timed_parse(bytes: &[u8]) -> (f64, lm::GramModel) {
    let (secs, model) =
        crate::timed(|| lm::from_gram_bytes(bytes).expect("engine-core parses dict-tools bytes"));
    (secs, model)
}

/// Time `candidates_with_lm` for one input and sanity-check the winner.
fn decode_rows(dict: &Dictionary, model: &lm::GramModel, raw: &str) -> Vec<(String, String)> {
    let fuzzy = FuzzyConfig::default();
    let decode = || decoder::candidates_with_lm(dict, None, Some(model), &fuzzy, raw, 10);

    let first = decode();
    let winner = first.first().map(|c| c.text.clone()).unwrap_or_default();
    assert!(
        winner.contains('今'),
        "beam search lost the fluent chain for {raw}: top = {winner:?}"
    );

    let mut samples = Vec::with_capacity(SENTENCE_RUNS);
    for round in 0..SENTENCE_WARMUP + SENTENCE_RUNS {
        let (secs, candidates) = crate::timed(decode);
        std::hint::black_box(candidates.len());
        if round >= SENTENCE_WARMUP {
            samples.push(secs);
        }
    }
    vec![
        (
            format!("input \"{raw}\""),
            format!("{} candidates, top {winner:?}", first.len()),
        ),
        (
            "  P50 / P99".to_string(),
            format!(
                "{} / {} us",
                synth::micros(synth::median(&samples)),
                synth::micros(synth::percentile(&samples, 99.0))
            ),
        ),
    ]
}
