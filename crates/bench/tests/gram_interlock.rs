//! Cross-crate `.gram` interlock (P2/M2b): dict-tools writes, engine-core
//! reads. Any drift in the format — constants, header layout, section
//! order, key sorting — fails here before it ships.

use dict_tools::format::{normalize, parse_tsv, to_tsv, Entry};
use dict_tools::gram::{bigrams, total_weight, train_counts, unigrams};
use engine_core::config::{default_config, FuzzyConfig};
use engine_core::decoder;
use engine_core::dict::Dictionary;
use engine_core::lm;
use engine_core::session::{self, new_session_with, process_key, set_lm};

/// Constants must be identical on both sides of the crate boundary.
#[test]
fn constants_match_across_crates() {
    assert_eq!(dict_tools::gram::GRAM_MAGIC, lm::GRAM_MAGIC);
    assert_eq!(dict_tools::gram::GRAM_VERSION, lm::GRAM_VERSION);
    assert_eq!(dict_tools::gram::GRAM_ORDER, lm::GRAM_ORDER);
}

/// dict-tools bytes must parse in engine-core and carry the exact counts
/// that were trained (golden 你好=8 / 金=3 sample, 47 bytes).
#[test]
fn dict_tools_bytes_parse_in_engine_core() {
    let entries = vec![
        Entry {
            pinyin: "ni'hao".into(),
            hanzi: "你好".into(),
            freq: 8,
        },
        Entry {
            pinyin: "jin".into(),
            hanzi: "金".into(),
            freq: 3,
        },
    ];
    let counts = train_counts(&entries);
    let bytes = counts.to_gram_bytes();
    assert_eq!(bytes.len(), 47);
    assert_eq!(total_weight(&counts), 19);
    assert_eq!(unigrams(&counts).len(), 3);
    assert_eq!(bigrams(&counts)["你好"], 8);

    let model = lm::from_gram_bytes(&bytes).expect("engine-core accepts dict-tools bytes");
    // unigram p(你) = ln(8/19); sequence_logp over "你" matches
    let want = (8.0_f64 / 19.0).ln();
    let got = model.sequence_logp("你");
    assert!((want - got).abs() < 1e-12, "want {want}, got {got}");
}

/// Counts above `u32::MAX` are divided by one common divisor on write so
/// both crates can store them, while ratios (and therefore scores) survive.
#[test]
fn oversized_training_scales_consistently_across_crates() {
    let entries = vec![
        Entry {
            pinyin: "a".into(),
            hanzi: "今天".into(),
            freq: u32::MAX,
        },
        Entry {
            pinyin: "b".into(),
            hanzi: "今天".into(),
            freq: 5,
        },
    ];
    let counts = train_counts(&entries);
    assert!(total_weight(&counts) > u64::from(u32::MAX));
    let bytes = counts.to_gram_bytes();
    let header = u32::from_le_bytes(bytes[20..24].try_into().expect("total weight"));
    assert!(header > 0);
    assert!(u64::from(header) <= u64::from(u32::MAX));

    let model = lm::from_gram_bytes(&bytes).expect("scaled bytes parse");
    assert_eq!(model.total_weight(), header);
    assert_eq!(model.counts(), (2, 1, 0));
    let ni = model.logp(None, None, '今');
    let tian = model.logp(None, None, '天');
    assert!((ni - tian).abs() < 1e-12, "equal counts must stay equal");
    assert!(ni.is_finite() && ni < 0.0);
}

/// End-to-end: compile a fixture TSV, train a model, load both into
/// engine-core, and check that beam search picks the fluent sentence.
#[test]
fn trained_model_decodes_whole_sentences() {
    let tsv = "jin\t今\t300\njin\t金\t500\ntian\t天\t900\nqi\t气\t400\nbu\t不\t800\n\
        bu\t补\t200\ncuo\t错\t100\njin'tian\t今天\t4500\njin'tian\t金田\t800\n\
        tian'qi\t天气\t3800\nbu'cuo\t不错\t2900\nbu'cuo\t补错\t500\nhen\t很\t600\n\
        hao\t好\t700\nhen'hao\t很好\t3200\nni\t你\t5000\nni'hao\t你好\t8000\n";
    let entries = normalize(parse_tsv(tsv).expect("fixture tsv"));
    let dict = Dictionary::from_tsv(&to_tsv(&entries)).expect("dict");
    let model = lm::from_gram_bytes(&train_counts(&entries).to_gram_bytes()).expect("gram model");

    let candidates = decoder::candidates_with_lm(
        &dict,
        None,
        Some(&model),
        &FuzzyConfig::default(),
        "jintian'tianqi'bu'cuo",
        10,
    );
    let top = candidates
        .first()
        .map(|c| c.text.clone())
        .unwrap_or_default();
    assert_eq!(top, "今天天气不错");

    // The session path must route its candidates through the installed LM.
    let typed = "jintian'tianqi'bu'cuo";
    let mut s = new_session_with(default_config(), dict.clone());
    for ch in typed.chars() {
        let _ = process_key(&mut s, ch);
    }
    let legacy = session::candidates(&s, 10)
        .first()
        .map(|c| c.text.clone())
        .unwrap_or_default();
    assert_ne!(legacy, "今天天气不错", "fixture must exercise beam routing");
    set_lm(&mut s, Some(model));
    let with_lm = session::candidates(&s, 10)
        .first()
        .map(|c| c.text.clone())
        .unwrap_or_default();
    assert_eq!(with_lm, "今天天气不错");
}

/// Engine-core must reject the same damage dict-tools rejects.
#[test]
fn engine_rejects_corrupt_gram_bytes() {
    let entries = vec![Entry {
        pinyin: "ni'hao".into(),
        hanzi: "你好".into(),
        freq: 8,
    }];
    let mut bytes = train_counts(&entries).to_gram_bytes();
    bytes[20] ^= 0xff; // corrupt total weight
    assert!(lm::from_gram_bytes(&bytes).is_err());
    bytes[20] ^= 0xff;
    bytes.push(0); // trailing byte
    assert!(lm::from_gram_bytes(&bytes).is_err());
}
