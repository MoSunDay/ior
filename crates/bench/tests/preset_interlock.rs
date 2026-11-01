//! Cross-crate interlock for rime preset-vocabulary compilation: dict-tools
//! encodes `essay.txt` phrases + rime weights, writes `.ior` v2, and
//! engine-core must read back the exact candidates. The 200-value key also
//! exercises multi-byte `values_len` varints on real writer output.

use dict_tools::format::{normalize, serialize, Entry};
use dict_tools::preset::{PresetVocab, WEIGHT_SCALE};
use engine_core::dict::Dictionary;

fn entry(pinyin: &str, hanzi: &str, freq: u32) -> Entry {
    Entry {
        pinyin: pinyin.to_string(),
        hanzi: hanzi.to_string(),
        freq,
    }
}

#[test]
fn engine_reads_preset_encoded_phrases() {
    let base = vec![
        entry("jin", "今", 100 * WEIGHT_SCALE),
        entry("tian", "天", 100 * WEIGHT_SCALE),
        entry("qi", "氣", 100 * WEIGHT_SCALE),
        entry("ni", "你", 100 * WEIGHT_SCALE),
        entry("hao", "好", 100 * WEIGHT_SCALE),
    ];
    let vocab = PresetVocab::parse("今天\t500\n天氣\t300\n你好\t200\n").unwrap();
    let mut all = base;
    all.extend(vocab.encode(&all, 0, 0.0));
    let entries = normalize(all);
    let bytes = serialize(&entries);

    let dict = Dictionary::from_ior_bytes(&bytes).expect("engine reads dict-tools bytes");
    let today = dict.lookup("jin'tian").expect("jin'tian");
    assert_eq!(today[0], ("今天".to_string(), 500 * WEIGHT_SCALE));
    assert_eq!(
        dict.lookup("tian'qi").expect("tian'qi")[0].0,
        "天氣".to_string()
    );
    assert_eq!(dict.lookup("ni'hao").expect("ni'hao")[0].0, "你好");
    // Unencoded essay phrases stay absent.
    assert!(dict.lookup("ni'tian").is_none());
}

#[test]
fn engine_reads_keys_with_more_than_127_values() {
    let mut values: Vec<(String, u32)> = (0..200)
        .map(|index| (format!("候{index:03}"), 200 - index))
        .collect();
    values.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let entries: Vec<Entry> = values
        .into_iter()
        .map(|(hanzi, freq)| Entry {
            pinyin: "hou".to_string(),
            hanzi,
            freq,
        })
        .collect();
    let bytes = serialize(&normalize(entries));
    let dict = Dictionary::from_ior_bytes(&bytes).expect("200-value key parses");
    let hits = dict.lookup("hou").expect("hou");
    assert_eq!(hits.len(), 200);
    assert_eq!(hits[0].0, "候000");
    assert_eq!(hits[0].1, 200);
    assert_eq!(hits[199].1, 1);
}
