//! Sentence staging: prefix-span picks buffer instead of committing, a
//! full-coverage pick flushes the composed word (learned under the
//! composed key), and every clearing path handles the staged buffer.

use engine_core::config::default_config;
use engine_core::dict::Dictionary;

/// Minimal dictionary exercising the staging flow: a full-input hit for
/// `panjie`, span keys `pan`/`jie`, and 姐 leading the `jie` group.
const TSV: &str = "pan'jie\t盘结\t9\npan\t盼\t5\npan\t盘\t4\njie\t姐\t7\njie\t接\t3\n";

fn staged_session() -> engine_core::session::Session {
    let cfg = engine_core::config::EngineConfig {
        chinese_punct: true,
        ..default_config()
    };
    new_session_with(cfg, Dictionary::from_tsv(TSV).expect("inline tsv"))
}
use engine_core::session::{
    self, delete_flat, new_session_with, process_key, select_flat, DeleteOutcome, ProcessResult,
};
use engine_core::user_dict;

fn stage_pick(s: &mut engine_core::session::Session, text: &str) {
    let idx = session::candidates(s, 50)
        .iter()
        .position(|c| c.text == text)
        .unwrap_or_else(|| panic!("candidate {text}"));
    assert_eq!(select_flat(s, idx), ProcessResult::Editing, "staged {text}");
}

fn typed(s: &mut engine_core::session::Session, keys: &str) {
    for ch in keys.chars() {
        assert_eq!(process_key(s, ch), ProcessResult::Editing);
    }
}

/// The motivating case: `panjie` has no 盼姐 word, so the user picks
/// 盼 then 姐 span-by-span — the pair commits as ONE 盼姐 and learns
/// under `pan'jie`, so the word resurfaces next round without paging.
#[test]
fn span_by_span_picks_compose_one_learnable_word() {
    let mut s = staged_session();
    typed(&mut s, "panjie");
    stage_pick(&mut s, "盼");
    assert_eq!(s.raw, "jie");
    assert_eq!(session::preedit(&s), "盼jie");
    // 姐 covers the remainder fully: one composed commit
    let idx = session::candidates(&s, 50)
        .iter()
        .position(|c| c.text == "姐")
        .expect("full candidate 姐");
    match select_flat(&mut s, idx) {
        ProcessResult::Committed(text) => assert_eq!(text, "盼姐"),
        other => panic!("full pick must commit, got {other:?}"),
    }
    assert!(s.raw.is_empty() && s.staged.is_empty());
    // learned under the composed key AND the span keys
    assert!(user_dict::lookup(&s.user_dict, "pan'jie")
        .iter()
        .any(|(h, _)| h == "盼姐"));
    assert!(user_dict::lookup(&s.user_dict, "pan")
        .iter()
        .any(|(h, _)| h == "盼"));
    // next round the word surfaces for the full input — no paging for it
    typed(&mut s, "panjie");
    assert!(
        session::candidates(&s, 50)
            .iter()
            .take(10)
            .any(|c| c.text == "盼姐"),
        "learned 盼姐 should surface within the first two pages"
    );
}

/// Backspace below the span boundary undoes the newest staged pick and
/// restores its span key to the raw buffer for re-selection.
#[test]
fn backspace_unstages_the_newest_pick() {
    let mut s = staged_session();
    typed(&mut s, "panjie");
    stage_pick(&mut s, "盼");
    for _ in 0..3 {
        // erase "jie"
        assert_eq!(process_key(&mut s, '\u{8}'), ProcessResult::Editing);
    }
    assert!(s.raw.is_empty());
    // one more backspace unstages: `pan` returns as raw letters
    assert_eq!(process_key(&mut s, '\u{8}'), ProcessResult::Editing);
    assert_eq!(s.raw, "pan");
    assert!(s.staged.is_empty());
    assert_eq!(session::preedit(&s), "pan");
    // re-picking from the bare `pan` input is full coverage now, so it
    // commits directly — staging only applies with a remainder left over
    let idx = session::candidates(&s, 50)
        .iter()
        .position(|c| c.text == "盼")
        .expect("candidate 盼 for pan");
    assert_eq!(
        select_flat(&mut s, idx),
        ProcessResult::Committed("盼".into())
    );
    assert!(s.raw.is_empty() && s.staged.is_empty());
}

/// Enter (raw commit) carries the staged hanzi out verbatim ahead of the
/// unconverted letters; nothing is learned for the raw tail.
#[test]
fn enter_raw_commits_staged_then_verbatim_letters() {
    let mut s = staged_session();
    typed(&mut s, "panjie");
    stage_pick(&mut s, "盼");
    assert_eq!(
        process_key(&mut s, '\n'),
        ProcessResult::Committed("盼jie".to_string())
    );
    assert!(s.raw.is_empty() && s.staged.is_empty());
}

/// The default Shift action (Clear) drops the staged picks with the
/// rest of the composition.
#[test]
fn shift_clear_drops_staged_picks() {
    let mut s = staged_session();
    typed(&mut s, "panjie");
    stage_pick(&mut s, "盼");
    assert_eq!(session::process_shift(&mut s), ProcessResult::Editing);
    assert!(s.raw.is_empty() && s.staged.is_empty());
}

/// A punctuation flush carries the staged hanzi out with it when the
/// top candidate of the remainder is another staged span.
#[test]
fn punctuation_carries_the_staged_prefix_out() {
    let mut s = staged_session();
    typed(&mut s, "panjie");
    stage_pick(&mut s, "盼");
    // remainder `jie`'s top candidate: whatever it is, the punct path
    // settles it and the committed text starts with 盼
    let r = process_key(&mut s, ',');
    match r {
        ProcessResult::Committed(text) => {
            assert!(text.starts_with('盼'), "{text}");
            assert!(text.ends_with('，'), "{text}");
        }
        other => panic!("punct must commit, got {other:?}"),
    }
    assert!(s.raw.is_empty() && s.staged.is_empty());
}

/// The composed learning key mirrors deletion: a word learned through
/// staging deletes under `pan'jie` when the full input is typed again.
#[test]
fn composed_word_deletes_under_the_composed_key() {
    let mut s = staged_session();
    typed(&mut s, "panjie");
    stage_pick(&mut s, "盼");
    // finish with the top candidate of `jie` that covers it fully
    let idx = session::candidates(&s, 50)
        .iter()
        .position(|c| c.text == "姐")
        .expect("full candidate 姐");
    match select_flat(&mut s, idx) {
        ProcessResult::Committed(text) => assert_eq!(text, "盼姐"),
        other => panic!("full pick must commit, got {other:?}"),
    }
    assert!(user_dict::lookup(&s.user_dict, "pan'jie")
        .iter()
        .any(|(h, _)| h == "盼姐"));

    // retype: the learned word now leads, and deletes under pan'jie
    typed(&mut s, "panjie");
    let cands = session::candidates(&s, 50);
    let learned = cands
        .iter()
        .position(|c| c.text == "盼姐")
        .expect("learned 盼姐 surfaces for the full input");
    assert!(learned < 5, "user word leads: {learned}");
    assert_eq!(delete_flat(&mut s, learned), DeleteOutcome::Deleted);
    assert!(user_dict::lookup(&s.user_dict, "pan'jie").is_empty());
}
