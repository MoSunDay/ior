//! End-to-end M0 session flows: typing, preedit, candidate selection,
//! passthrough, fuzzy matching, learning and crash-resistance.

use engine_core::config::{default_config, EngineConfig, FuzzyConfig};
use engine_core::dict::Dictionary;
use engine_core::session::{
    self, new_session, new_session_with, preedit, process_key, ProcessResult,
};
use engine_core::user_dict;

#[test]
fn nihao_flow_commits_via_space() {
    let mut s = new_session(default_config());
    for ch in "nihao".chars() {
        assert_eq!(process_key(&mut s, ch), ProcessResult::Editing);
    }
    assert_eq!(preedit(&s), "ni'hao");
    assert_eq!(session::candidates(&s, 10)[0].text, "你好");
    assert_eq!(
        process_key(&mut s, ' '),
        ProcessResult::Committed("你好".to_string())
    );
    assert_eq!(preedit(&s), "");
    assert!(session::candidates(&s, 10).is_empty());
}

#[test]
fn digit_select_takes_second_candidate() {
    let mut s = new_session(default_config());
    for ch in "ni".chars() {
        assert_eq!(process_key(&mut s, ch), ProcessResult::Editing);
    }
    let second = session::candidates(&s, 10)[1].text.clone();
    assert_eq!(process_key(&mut s, '2'), ProcessResult::Committed(second));
}

#[test]
fn zero_beyond_short_list_is_swallowed() {
    let mut s = new_session(default_config());
    for ch in "ni".chars() {
        process_key(&mut s, ch);
    }
    assert!(session::candidates(&s, 10).len() < 10);
    assert_eq!(process_key(&mut s, '0'), ProcessResult::Editing);
    assert_eq!(preedit(&s), "ni");
}

#[test]
fn backspace_pops_one_char() {
    let mut s = new_session(default_config());
    for ch in "ni".chars() {
        process_key(&mut s, ch);
    }
    assert_eq!(process_key(&mut s, '\u{8}'), ProcessResult::Editing);
    assert_eq!(preedit(&s), "n");
}

#[test]
fn esc_clears_preedit() {
    let mut s = new_session(default_config());
    for ch in "ni".chars() {
        process_key(&mut s, ch);
    }
    assert_eq!(process_key(&mut s, '\u{1b}'), ProcessResult::Editing);
    assert_eq!(preedit(&s), "");
    assert!(s.raw.is_empty());
}

#[test]
fn uppercase_forwarded_when_empty() {
    let mut s = new_session(default_config());
    assert_eq!(process_key(&mut s, 'A'), ProcessResult::Forwarded);
    assert!(s.raw.is_empty());
}

#[test]
fn every_ascii_key_is_handled_without_panicking() {
    let mut fresh = new_session(default_config());
    let mut editing = new_session(default_config());
    for ch in "ni".chars() {
        process_key(&mut editing, ch);
    }
    for code in 0u32..=0x7f {
        let ch = char::from_u32(code).expect("ascii char");
        let _ = process_key(&mut fresh, ch);
        let _ = process_key(&mut editing, ch);
    }
}

#[test]
fn undecodable_input_passes_through_verbatim() {
    let mut s = new_session(default_config());
    for ch in "nih".chars() {
        assert_eq!(process_key(&mut s, ch), ProcessResult::Editing);
    }
    assert!(session::candidates(&s, 10).is_empty());
    assert_eq!(
        process_key(&mut s, ' '),
        ProcessResult::Committed("nih".to_string())
    );
    assert_eq!(preedit(&s), "");
}

#[test]
fn fuzzy_n_l_matches_both_sides() {
    let mut cfg = default_config();
    cfg.fuzzy.n_l = true;
    let mut s = new_session(cfg);
    for ch in "li".chars() {
        assert_eq!(process_key(&mut s, ch), ProcessResult::Editing);
    }
    let texts: Vec<String> = session::candidates(&s, 10)
        .iter()
        .map(|c| c.text.clone())
        .collect();
    assert!(texts.contains(&"你".to_string()), "texts: {texts:?}");
    assert!(texts.contains(&"里".to_string()), "texts: {texts:?}");
}

#[test]
fn fuzzy_z_zh_matches_zhe_from_ze() {
    let mut cfg = default_config();
    cfg.fuzzy.z_zh = true;
    let mut s = new_session(cfg);
    for ch in "ze".chars() {
        assert_eq!(process_key(&mut s, ch), ProcessResult::Editing);
    }
    let texts: Vec<String> = session::candidates(&s, 10)
        .iter()
        .map(|c| c.text.clone())
        .collect();
    assert!(texts.contains(&"这".to_string()), "texts: {texts:?}");
}

#[test]
fn commits_are_learned_and_manual_learning_surfaces() {
    let mut s = new_session(default_config());
    for ch in "nihao".chars() {
        process_key(&mut s, ch);
    }
    assert_eq!(
        process_key(&mut s, ' '),
        ProcessResult::Committed("你好".to_string())
    );
    let learned = user_dict::lookup(&s.user_dict, "ni'hao");
    assert!(
        learned.iter().any(|(h, f)| h == "你好" && *f >= 1),
        "learned: {learned:?}"
    );

    user_dict::learn(&mut s.user_dict, "ni'hao", "泥嚎");
    for ch in "nihao".chars() {
        process_key(&mut s, ch);
    }
    let top: Vec<String> = session::candidates(&s, 10)
        .iter()
        .take(3)
        .map(|c| c.text.clone())
        .collect();
    assert!(top.contains(&"泥嚎".to_string()), "top 3: {top:?}");
}

#[test]
fn custom_dictionary_session_works() {
    let dict = Dictionary::from_tsv_with("ma\t妈\t5\nma\t马\t3\n", &FuzzyConfig::default())
        .expect("inline tsv");
    let cfg = EngineConfig::default();
    let mut s = new_session_with(cfg, dict);
    assert_eq!(process_key(&mut s, 'm'), ProcessResult::Editing);
    assert_eq!(process_key(&mut s, 'a'), ProcessResult::Editing);
    let texts: Vec<String> = session::candidates(&s, 10)
        .iter()
        .map(|c| c.text.clone())
        .collect();
    assert_eq!(texts, vec!["妈".to_string(), "马".to_string()]);
    assert_eq!(
        process_key(&mut s, '1'),
        ProcessResult::Committed("妈".to_string())
    );
}
