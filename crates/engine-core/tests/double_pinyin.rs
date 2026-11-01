//! P1 double-pinyin framework at the session level: a host-loaded table
//! (self-made fixture, NOT the layout of any real scheme) decodes typed
//! code pairs into full pinyin while the preedit keeps showing the typed
//! letters, learning records the transformed key, and every non-`Custom`
//! configuration behaves exactly like plain full pinyin.

use engine_core::config::{default_config, DoublePinyin, EngineConfig};
use engine_core::dict::Dictionary;
use engine_core::double_pinyin::{parse_tsv, to_full};
use engine_core::session::{
    self, new_session_with, next_page, page_count, preedit, process_key, set_double_pinyin,
    ProcessResult,
};
use engine_core::user_dict;

/// Fixture table: `in`->l, `ian`->j, `i`->i, `a`->a, `ao`->q, `uang`->x,
/// zero-initial `a`->o, `e`->w. Invented for these tests.
const FIXTURE: &str = "in\tl\nian\tj\ni\ti\na\ta\nao\tq\nuang\tx\n0a\to\n0e\tw\n";

/// Session over `dict` with double pinyin enabled and the fixture loaded,
/// raw buffer set to `typed`.
fn dp_session(dict: Dictionary, typed: &str) -> engine_core::session::Session {
    let cfg = EngineConfig {
        double_pinyin: DoublePinyin::Custom,
        ..EngineConfig::default()
    };
    let mut s = new_session_with(cfg, dict);
    set_double_pinyin(&mut s, Some(parse_tsv(FIXTURE).expect("fixture table")));
    for ch in typed.chars() {
        assert_eq!(process_key(&mut s, ch), ProcessResult::Editing);
    }
    s
}

#[test]
fn fixture_table_transforms_codes_to_full_pinyin() {
    let table = parse_tsv(FIXTURE).expect("fixture table");
    assert_eq!(to_full(&table, "jltj").as_deref(), Some("jin'tian"));
    assert_eq!(to_full(&table, "oa").as_deref(), Some("a"));
    assert_eq!(to_full(&table, "jlt"), None);
}

#[test]
fn custom_table_decodes_double_pinyin_candidates() {
    // "jltj" = (j,in)(t,ian) -> "jin'tian" -> 今天
    let s = dp_session(Dictionary::builtin(), "jltj");
    let cands = session::candidates(&s, 10);
    assert!(!cands.is_empty(), "must decode through the table");
    assert_eq!(cands[0].text, "今天");
    assert_eq!(cands[0].pinyin, "jin'tian");
}

#[test]
fn preedit_shows_the_typed_codes_not_the_transform() {
    let s = dp_session(Dictionary::builtin(), "jltj");
    assert_eq!(preedit(&s), "jltj", "display keeps the raw buffer");
    // the raw buffer itself is untouched
    assert_eq!(s.raw, "jltj");
}

#[test]
fn commit_learns_the_transformed_key() {
    let mut s = dp_session(Dictionary::builtin(), "jltj");
    assert_eq!(
        process_key(&mut s, ' '),
        ProcessResult::Committed("今天".to_string())
    );
    let learned = user_dict::lookup(&s.user_dict, "jin'tian");
    assert!(
        learned.iter().any(|(h, f)| h == "今天" && *f >= 1),
        "learned: {learned:?}"
    );
    // and nothing was learned under the raw codes
    assert!(user_dict::lookup(&s.user_dict, "jltj").is_empty());
}

#[test]
fn off_mode_keeps_full_pinyin_behavior() {
    // Off with no table: "jltj" is not full pinyin, nothing decodes;
    // regular typing still works as before.
    let mut s = new_session_with(default_config(), Dictionary::builtin());
    for ch in "jltj".chars() {
        process_key(&mut s, ch);
    }
    assert!(session::candidates(&s, 10).is_empty());
    let mut s2 = new_session_with(default_config(), Dictionary::builtin());
    for ch in "nihao".chars() {
        process_key(&mut s2, ch);
    }
    assert_eq!(session::candidates(&s2, 10)[0].text, "你好");
}

#[test]
fn custom_without_table_behaves_like_raw_letters() {
    let cfg = EngineConfig {
        double_pinyin: DoublePinyin::Custom,
        ..EngineConfig::default()
    };
    let mut s = new_session_with(cfg, Dictionary::builtin());
    // no table installed: decoding must not blow up, the raw letters are used
    for ch in "jltj".chars() {
        process_key(&mut s, ch);
    }
    assert!(session::candidates(&s, 10).is_empty());
    // installing and then clearing the table toggles the behavior
    set_double_pinyin(&mut s, Some(parse_tsv(FIXTURE).expect("fixture table")));
    assert_eq!(session::candidates(&s, 10)[0].text, "今天");
    set_double_pinyin(&mut s, None);
    assert!(session::candidates(&s, 10).is_empty());
}

#[test]
fn failed_transform_falls_back_to_the_raw_letters() {
    // "jiaa" cannot transform ((a,a) is not a syllable), but the raw
    // letters segment as jia|a, so the fallback still yields candidates.
    let dict = Dictionary::from_tsv("jia'a\t加阿\t5\njia\t加\t9\na\t阿\t8\n").expect("inline tsv");
    let s = dp_session(dict, "jiaa");
    let texts: Vec<String> = session::candidates(&s, 10)
        .iter()
        .map(|c| c.text.clone())
        .collect();
    assert!(texts.contains(&"加阿".to_string()), "texts: {texts:?}");
    // odd-length codes also fall back and simply decode nothing
    let s = dp_session(Dictionary::builtin(), "jlt");
    assert!(session::candidates(&s, 10).is_empty());
}

#[test]
fn paging_and_digit_selection_work_under_double_pinyin() {
    // ten "jin" entries: typed "jl" decodes to "jin" with two pages.
    let tsv = "jin\t一\t1\njin\t二\t2\njin\t三\t3\njin\t四\t4\njin\t五\t5\njin\t六\t6\njin\t七\t7\njin\t八\t8\njin\t九\t9\njin\t十\t10\n";
    let dict = Dictionary::from_tsv(tsv).expect("inline tsv");
    let mut s = dp_session(dict, "jl");
    assert_eq!(session::candidates(&s, 10).len(), 10);
    assert_eq!(page_count(&s), 2);
    assert!(next_page(&mut s));
    // '1' on page 1 = flat index 5 = the sixth entry (freq 5)
    assert_eq!(
        process_key(&mut s, '1'),
        ProcessResult::Committed("五".to_string())
    );
}
