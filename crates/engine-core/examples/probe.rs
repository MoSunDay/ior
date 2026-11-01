use engine_core::config::{DoublePinyin, EngineConfig};
use engine_core::dict::Dictionary;
use engine_core::session::{self, new_session_with, process_key, ProcessResult};

fn read(path: &str) -> Dictionary {
    let bytes = std::fs::read(path).expect("read .ior");
    Dictionary::from_ior_bytes(&bytes).expect("parse .ior")
}

fn type_all(session: &mut session::Session, input: &str) {
    for ch in input.chars() {
        assert_eq!(process_key(session, ch), ProcessResult::Editing);
    }
}

fn show(label: &str, session: &session::Session) {
    let candidates = session::candidates(session, 6);
    let cands: Vec<(&str, &str)> = candidates
        .iter()
        .map(|c| (c.text.as_str(), c.pinyin.as_str()))
        .collect();
    println!("{label}: {cands:?}");
}

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/usr/share/fcitx5/ior/core.ior".to_string());
    let dict = read(&path);
    println!("{path}: {} keys", dict.len());
    for key in ["tian'qi", "jin'tian", "ni'hao", "zhong'guo", "de"] {
        let hits = dict.lookup(key);
        println!(
            "{key}: {:?}",
            hits.map(|v| v.iter().take(3).collect::<Vec<_>>())
        );
    }
    for (label, cfg, input) in [
        ("full tianqi", EngineConfig::default(), "tianqi"),
        ("full jintian", EngineConfig::default(), "jintian"),
        ("full nihao", EngineConfig::default(), "nihao"),
        ("full zhongguo", EngineConfig::default(), "zhongguo"),
        (
            "natural jntm",
            EngineConfig {
                double_pinyin: DoublePinyin::Natural,
                ..EngineConfig::default()
            },
            "jntm",
        ),
        (
            "natural vsgo",
            EngineConfig {
                double_pinyin: DoublePinyin::Natural,
                ..EngineConfig::default()
            },
            "vsgo",
        ),
    ] {
        let mut session = new_session_with(cfg, dict.clone());
        type_all(&mut session, input);
        show(label, &session);
    }
}
