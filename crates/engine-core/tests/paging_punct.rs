//! P5-lite integration coverage: candidate paging, Shift mode switching and
//! Chinese punctuation, all through the public `process_key`-level API.

use engine_core::config::{default_config, EngineConfig, EnterAction};
use engine_core::dict::Dictionary;
use engine_core::session::{
    self, current_page, new_session_with, next_page, page_count, prev_page, process_key,
    process_shift, select_flat, toggle_ascii_punct, ProcessResult,
};
use engine_core::user_dict;

/// Ten one-syllable entries so the flat candidate list for `ma` is exactly
/// `MA_WORDS` (dictionary order: highest frequency first).
const MA_TSV: &str = "ma\t一\t1\nma\t二\t2\nma\t三\t3\nma\t四\t4\nma\t五\t5\nma\t六\t6\nma\t七\t7\nma\t八\t8\nma\t九\t9\nma\t十\t10\n";
const MA_WORDS: [&str; 10] = ["十", "九", "八", "七", "六", "五", "四", "三", "二", "一"];

/// Session over the ten-entry dictionary, preedit set to `raw`.
fn chinese_punct_config() -> EngineConfig {
    let mut cfg = default_config();
    cfg.chinese_punct = true;
    cfg
}

fn ma_session(raw: &str, config: EngineConfig) -> engine_core::session::Session {
    let dict = Dictionary::from_tsv(MA_TSV).expect("inline tsv");
    let mut s = new_session_with(config, dict);
    for ch in raw.chars() {
        assert_eq!(process_key(&mut s, ch), ProcessResult::Editing);
    }
    s
}

#[test]
fn page_math_ten_candidates_two_pages() {
    let s = ma_session("ma", default_config());
    let flat = session::candidates(&s, 10);
    assert_eq!(flat.len(), 10);
    assert_eq!(page_count(&s), 2);
    assert_eq!(current_page(&s).len(), 5);
}

#[test]
fn second_page_shows_sixth_to_tenth_candidate() {
    let mut s = ma_session("ma", default_config());
    assert!(next_page(&mut s));
    assert_eq!(s.page, 1);
    let page1: Vec<String> = current_page(&s).iter().map(|c| c.text.clone()).collect();
    assert_eq!(page1, MA_WORDS[5..]);
    // Wrapping: one more step goes back to page 0.
    assert!(next_page(&mut s));
    assert_eq!(s.page, 0);
}

#[test]
fn second_page_digit_selects_sixth_candidate() {
    let mut s = ma_session("ma", default_config());
    assert!(next_page(&mut s));
    // '1' on page 1 = flat index 5 = the first entry of the second page.
    assert_eq!(
        process_key(&mut s, '1'),
        ProcessResult::Committed("五".to_string())
    );
    assert_eq!(page_count(&s), 0);
    assert!(s.raw.is_empty());
    assert_eq!(s.page, 0);
}

#[test]
fn prev_page_wraps_to_last_page() {
    let mut s = ma_session("ma", default_config());
    assert!(prev_page(&mut s));
    assert_eq!(s.page, 1);
    assert!(prev_page(&mut s));
    assert_eq!(s.page, 0);
    // No candidates: paging is a no-op.
    let mut idle = ma_session("", default_config());
    assert!(!next_page(&mut idle));
    assert!(!prev_page(&mut idle));
}

#[test]
fn editing_keys_reset_the_page() {
    let mut s = ma_session("ma", default_config());
    assert!(next_page(&mut s));
    assert_eq!(process_key(&mut s, '\u{8}'), ProcessResult::Editing);
    assert_eq!(s.page, 0, "backspace must reset the page");

    let mut s = ma_session("ma", default_config());
    assert!(next_page(&mut s));
    assert_eq!(process_key(&mut s, 'y'), ProcessResult::Editing);
    assert_eq!(s.page, 0, "letters must reset the page");

    let mut s = ma_session("ma", default_config());
    assert!(next_page(&mut s));
    assert_eq!(process_key(&mut s, '\''), ProcessResult::Editing);
    assert_eq!(s.page, 0, "apostrophe must reset the page");
}

#[test]
fn select_flat_commits_and_learns() {
    let mut s = ma_session("ma", default_config());
    assert_eq!(
        select_flat(&mut s, 7),
        ProcessResult::Committed("三".to_string())
    );
    assert!(s.raw.is_empty());
    assert_eq!(s.page, 0);
    let learned = user_dict::lookup(&s.user_dict, "ma");
    assert!(
        learned.iter().any(|(h, f)| h == "三" && *f >= 1),
        "learned: {learned:?}"
    );
    // Out-of-range indices swallow the key and keep the composition.
    let mut s = ma_session("ma", default_config());
    assert_eq!(select_flat(&mut s, 10), ProcessResult::Editing);
    assert_eq!(s.raw, "ma");
}

#[test]
fn shift_to_english_commits_like_enter() {
    // Default enter_action = CommitRaw: switching Chinese -> English with
    // a pending composition behaves exactly like Enter — the typed
    // letters go out verbatim, nothing is learned.
    let mut s = ma_session("ni'hao", default_config());
    assert_eq!(
        process_shift(&mut s),
        ProcessResult::Committed("ni'hao".to_string())
    );
    assert!(s.ascii_mode);
    assert!(s.raw.is_empty());
    assert_eq!(s.page, 0);
    // Back to Chinese: plain Editing, buffer stays empty.
    assert_eq!(process_shift(&mut s), ProcessResult::Editing);
    assert!(!s.ascii_mode);
}

#[test]
fn shift_with_enter_candidate_learns_top_candidate() {
    // enter_action = "candidate": the switch takes the Space path and
    // commits the top candidate (learning included).
    let mut cfg = default_config();
    cfg.enter_action = EnterAction::CommitBest;
    let mut s = ma_session("ma", cfg.clone());
    assert_eq!(
        process_shift(&mut s),
        ProcessResult::Committed("十".to_string())
    );
    let learned = user_dict::lookup(&s.user_dict, "ma");
    assert!(
        learned.iter().any(|(h, _)| h == "十"),
        "learned: {learned:?}"
    );
    // Undecodable input falls back to the raw text.
    let mut s = ma_session("mh", cfg);
    assert!(session::candidates(&s, 10).is_empty());
    assert_eq!(
        process_shift(&mut s),
        ProcessResult::Committed("mh".to_string())
    );
}

#[test]
fn shift_with_empty_buffer_stays_quiet() {
    // Nothing pending: the toggle neither commits nor leaks anything.
    let mut s = ma_session("", default_config());
    assert_eq!(process_shift(&mut s), ProcessResult::Editing);
    assert!(s.ascii_mode);
    assert!(s.raw.is_empty());
    assert_eq!(page_count(&s), 0);
}

#[test]
fn ascii_mode_forwards_letters_and_punct() {
    let mut s = ma_session("ma", default_config());
    process_shift(&mut s); // -> English mode, raw committed
    assert_eq!(process_key(&mut s, 'n'), ProcessResult::Forwarded);
    assert_eq!(process_key(&mut s, ','), ProcessResult::Forwarded);
    assert_eq!(process_key(&mut s, ' '), ProcessResult::Forwarded);
    assert_eq!(process_key(&mut s, '7'), ProcessResult::Forwarded);
    assert!(s.raw.is_empty());
}

#[test]
fn punct_idle_commits_chinese_counterpart() {
    let mut s = ma_session("", chinese_punct_config());
    assert_eq!(
        process_key(&mut s, ','),
        ProcessResult::Committed("，".to_string())
    );
    assert_eq!(
        process_key(&mut s, '.'),
        ProcessResult::Committed("。".to_string())
    );
    // Quotes alternate through the session state.
    assert_eq!(
        process_key(&mut s, '"'),
        ProcessResult::Committed("“".to_string())
    );
    assert_eq!(
        process_key(&mut s, '"'),
        ProcessResult::Committed("”".to_string())
    );
    // Unmapped printable ASCII still passes through.
    assert_eq!(process_key(&mut s, '`'), ProcessResult::Forwarded);
}

#[test]
fn punct_with_raw_commits_first_candidate_then_punct() {
    let mut s = ma_session("ma", chinese_punct_config());
    assert_eq!(
        process_key(&mut s, ','),
        ProcessResult::Committed("十，".to_string())
    );
    assert!(s.raw.is_empty());
    assert_eq!(page_count(&s), 0);
    let learned = user_dict::lookup(&s.user_dict, "ma");
    assert!(
        learned.iter().any(|(h, _)| h == "十"),
        "learned: {learned:?}"
    );
}

#[test]
fn punct_without_candidates_commits_punct_only() {
    // "mh" cannot be segmented: no candidates, so only the punctuation is
    // committed and the dead preedit is dropped.
    let mut s = ma_session("mh", chinese_punct_config());
    assert!(session::candidates(&s, 10).is_empty());
    assert_eq!(
        process_key(&mut s, '.'),
        ProcessResult::Committed("。".to_string())
    );
    assert!(s.raw.is_empty());
}

#[test]
fn ascii_punct_override_and_config_switch_forward_punct() {
    // Per-session override: Chinese mode but ASCII punctuation.
    let mut s = ma_session("", chinese_punct_config());
    toggle_ascii_punct(&mut s);
    assert!(s.ascii_punct);
    assert_eq!(process_key(&mut s, ','), ProcessResult::Forwarded);
    toggle_ascii_punct(&mut s);
    assert_eq!(
        process_key(&mut s, ','),
        ProcessResult::Committed("，".to_string())
    );
    // Config switch off: same passthrough behavior.
    let mut cfg = default_config();
    cfg.chinese_punct = false;
    let mut s = ma_session("", cfg);
    assert_eq!(process_key(&mut s, ','), ProcessResult::Forwarded);
}
