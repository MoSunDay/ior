use engine_core::config::{DoublePinyin, EngineConfig};
use engine_core::dict::Dictionary;
use engine_core::lm::{self, GramModel};
use engine_core::session::{self, new_session_with, process_key, ProcessResult};

fn read(path: &str) -> Dictionary {
    let bytes = std::fs::read(path).expect("read .ior");
    Dictionary::from_ior_bytes(&bytes).expect("parse .ior")
}

fn read_lm(path: &str) -> GramModel {
    let bytes = std::fs::read(path).expect("read .gram");
    lm::from_gram_bytes(&bytes).expect("parse .gram")
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
    let lm_path = std::env::args().nth(2);
    let model = lm_path.as_deref().map(read_lm);
    println!(
        "{path}: {} keys{}",
        dict.len(),
        lm_path
            .as_deref()
            .map(|path| format!(", LM {path}"))
            .unwrap_or_default()
    );
    for key in ["tian'qi", "jin'tian", "ni'hao", "zhong'guo", "de"] {
        let hits = dict.lookup(key);
        println!(
            "{key}: {:?}",
            hits.map(|v| v.iter().take(3).collect::<Vec<_>>())
        );
    }
    for (label, cfg, input) in [
        ("partial n", EngineConfig::default(), "n"),
        ("partial ni", EngineConfig::default(), "ni"),
        ("partial nih", EngineConfig::default(), "nih"),
        ("partial niha", EngineConfig::default(), "niha"),
        ("partial nihao", EngineConfig::default(), "nihao"),
        ("full tianqi", EngineConfig::default(), "tianqi"),
        ("full jintian", EngineConfig::default(), "jintian"),
        ("full nihao", EngineConfig::default(), "nihao"),
        ("full zhongguo", EngineConfig::default(), "zhongguo"),
        (
            "full sentence 1",
            EngineConfig::default(),
            "jintian'tianqi'bu'cuo",
        ),
        (
            "full sentence 2",
            EngineConfig::default(),
            "wo'xiang'qu'bei'jing",
        ),
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
        if let Some(model) = model.as_ref() {
            session::set_lm(&mut session, Some(model.clone()));
        }
        show(label, &session);
    }
}
