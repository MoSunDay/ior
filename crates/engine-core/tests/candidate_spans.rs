//! rime-aligned candidate richness: prefix-span waterfall and partial
//! commits (`nihao` -> 你好, then `ni` homophones; picking 你 keeps `hao`
//! typed), plus the raised flat-candidate window.

use engine_core::config::default_config;
use engine_core::decoder::candidates;
use engine_core::dict::Dictionary;
use engine_core::session::{
    self, new_session_with, page_count, process_key, select_flat, ProcessResult,
};
use engine_core::user_dict;

/// `xi'an` (typed with the separator) segments as [xi, an]: the whole-phrase
/// hit comes first, then the span-1 homophones of `xi` — rime's translator
/// lists start-anchored words by descending span length.
#[test]
fn prefix_span_candidates_follow_whole_phrase_matches() {
    let d = Dictionary::builtin();
    let cands = candidates(&d, None, "xi'an", 50);
    assert_eq!(cands[0].text, "西安");
    let xi = cands
        .iter()
        .find(|c| c.text == "西")
        .expect("span-1 hit 西");
    assert_eq!(xi.pinyin, "xi");
    assert!(cands.iter().any(|c| c.text == "系"));
    // full-span entries always precede the shorter spans
    let xi_pos = cands.iter().position(|c| c.text == "西").unwrap();
    assert!(xi_pos > cands.iter().position(|c| c.text == "西安").unwrap());
}

/// Multi-syllable prefixes appear between the full span and single
/// syllables, longest span first (时间表 -> 时间 -> 是).
#[test]
fn multi_syllable_prefix_spans_come_longest_first() {
    let tsv = "shi'jian'biao\t时间表\t9\nshi'jian\t时间\t8\nshi\t是\t7\njian\t见\t6\nbiao\t表\t5\n";
    let d = Dictionary::from_tsv(tsv).expect("inline tsv");
    let cands = candidates(&d, None, "shijianbiao", 50);
    let texts: Vec<&str> = cands.iter().map(|c| c.text.as_str()).collect();
    let full = texts.iter().position(|t| *t == "时间表").unwrap();
    let span2 = texts.iter().position(|t| *t == "时间").unwrap();
    let span1 = texts.iter().position(|t| *t == "是").unwrap();
    assert!(full < span2 && span2 < span1, "texts: {texts:?}");
    assert_eq!(cands[span2].pinyin, "shi'jian");
    assert_eq!(cands[span1].pinyin, "shi");
}

/// Committing a prefix-span candidate commits its hanzi and leaves the
/// uncovered letters typed (rime: pick the word, keep typing the rest);
/// learning records the word under its span key.
#[test]
fn partial_span_commit_keeps_remainder_typed() {
    let mut s = new_session_with(default_config(), Dictionary::builtin());
    for ch in "nihao".chars() {
        assert_eq!(process_key(&mut s, ch), ProcessResult::Editing);
    }
    let cands = session::candidates(&s, 50);
    let idx = cands
        .iter()
        .position(|c| c.text == "你")
        .expect("span candidate 你");
    assert_eq!(
        select_flat(&mut s, idx),
        ProcessResult::Committed("你".to_string())
    );
    assert_eq!(s.raw, "hao");
    // the remainder keeps decoding on its own
    assert_eq!(
        process_key(&mut s, ' '),
        ProcessResult::Committed("好".to_string())
    );
    assert!(s.raw.is_empty());
    // learned under the span key, so it resurfaces early next round
    assert!(user_dict::lookup(&s.user_dict, "ni")
        .iter()
        .any(|(h, _)| h == "你"));
}

/// A typed `'` separator at the commit boundary is consumed with the
/// covered span; the remainder starts clean.
#[test]
fn partial_commit_swallows_the_boundary_separator() {
    let mut s = new_session_with(default_config(), Dictionary::builtin());
    for ch in "ni'hao".chars() {
        assert_eq!(process_key(&mut s, ch), ProcessResult::Editing);
    }
    let idx = session::candidates(&s, 50)
        .iter()
        .position(|c| c.text == "你")
        .expect("span candidate 你");
    assert_eq!(
        select_flat(&mut s, idx),
        ProcessResult::Committed("你".to_string())
    );
    assert_eq!(s.raw, "hao");
}

/// Learned span words surface at the head of their span group (user-first
/// ordering, same rule as whole-input matches).
#[test]
fn learned_span_words_lead_their_span_group() {
    let tsv = "shi'jian\t时间\t8\nshi\t是\t7\nshi\t事\t6\n";
    let dict = Dictionary::from_tsv(tsv).expect("inline tsv");
    let mut s = new_session_with(default_config(), dict);
    for _ in 0..3 {
        for ch in "shijian".chars() {
            process_key(&mut s, ch);
        }
        let idx = session::candidates(&s, 50)
            .iter()
            .position(|c| c.text == "事")
            .expect("span candidate 事");
        assert_eq!(
            select_flat(&mut s, idx),
            ProcessResult::Committed("事".to_string())
        );
        // clear the leftover "jian"
        reset_via_escape(&mut s);
    }
    // now the learned 事 precede the dictionary hit 是 within the span
    for ch in "shijian".chars() {
        process_key(&mut s, ch);
    }
    let cands = session::candidates(&s, 50);
    let shi_group: Vec<&str> = cands
        .iter()
        .filter(|c| c.pinyin == "shi")
        .map(|c| c.text.as_str())
        .collect();
    assert_eq!(shi_group.first(), Some(&"事"), "group: {shi_group:?}");
}

/// The flat window holds ten pages' worth of candidates (rime menus page
/// far beyond the old hard cap of ten entries).
#[test]
fn flat_window_spans_ten_pages() {
    let tsv = "ma\t一\t1\nma\t二\t2\nma\t三\t3\nma\t四\t4\nma\t五\t5\nma\t六\t6\nma\t七\t7\nma\t八\t8\nma\t九\t9\nma\t十\t10\nma\t甲\t11\nma\t乙\t12\n";
    let dict = Dictionary::from_tsv(tsv).expect("inline tsv");
    let mut s = new_session_with(default_config(), dict);
    for ch in "ma".chars() {
        process_key(&mut s, ch);
    }
    assert_eq!(session::candidates(&s, 50).len(), 12);
    assert_eq!(page_count(&s), 3);
}

fn reset_via_escape(s: &mut engine_core::session::Session) {
    assert_eq!(process_key(s, '\u{1b}'), ProcessResult::Editing);
}

/// Double-pinyin partial commits map covered syllables back to two code
/// letters each: picking a `jin` span candidate from `jltj` (= jin'tian)
/// commits its hanzi and leaves `tj` (= tian) typed. The fixture table is
/// the one from tests/double_pinyin.rs (`in`->l, `ian`->j, ...).
#[test]
fn double_pinyin_partial_commit_trims_code_pairs() {
    use engine_core::config::{DoublePinyin, EngineConfig};
    use engine_core::double_pinyin::parse_tsv;
    use engine_core::session::set_double_pinyin;
    const FIXTURE: &str = "in\tl\nian\tj\ni\ti\na\ta\nao\tq\nuang\tx\n0a\to\n0e\tw\n";
    let cfg = EngineConfig {
        double_pinyin: DoublePinyin::Custom,
        ..EngineConfig::default()
    };
    let tsv = "jin'tian\t今天\t9\njin\t金\t8\njin\t进\t7\ntian\t天\t6\n";
    let mut s = new_session_with(cfg, Dictionary::from_tsv(tsv).expect("inline tsv"));
    set_double_pinyin(&mut s, Some(parse_tsv(FIXTURE).expect("fixture table")));
    for ch in "jltj".chars() {
        assert_eq!(process_key(&mut s, ch), ProcessResult::Editing);
    }
    let cands = session::candidates(&s, 50);
    let jin: Vec<&engine_core::decoder::Candidate> =
        cands.iter().filter(|c| c.pinyin == "jin").collect();
    let pick = jin.first().expect("span candidate for jin");
    let idx = cands.iter().position(|c| c.pinyin == pick.pinyin).unwrap();
    assert_eq!(
        select_flat(&mut s, idx),
        ProcessResult::Committed(pick.text.clone())
    );
    assert_eq!(s.raw, "tj", "one code pair per covered syllable");
}
