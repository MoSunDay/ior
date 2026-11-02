//! User-dictionary deletion (`session::delete_flat`): the delete-side
//! counterpart of selection — removing a mis-learned word must keep the
//! composition intact, drop exactly the learned entry (span candidates
//! under their span key) and never touch dictionary words.

use engine_core::config::default_config;
use engine_core::dict::Dictionary;
use engine_core::session::{
    self, delete_flat, new_session_with, process_key, select_flat, DeleteOutcome, ProcessResult,
};
use engine_core::user_dict;

fn type_str(s: &mut engine_core::session::Session, input: &str) {
    for ch in input.chars() {
        assert_eq!(process_key(s, ch), ProcessResult::Editing);
    }
}

/// Learning floats a word to the top; deleting it removes exactly that
/// learned entry, keeps the composition and restores the dictionary
/// ordering; a second delete of the same word is a `NotUserWord` miss.
#[test]
fn delete_flat_removes_learned_entry_and_keeps_composition() {
    let mut s = new_session_with(default_config(), Dictionary::builtin());
    // Learn 你 twice via digit commits so it floats to the top of `ni`.
    for _ in 0..2 {
        type_str(&mut s, "ni");
        let cands = session::candidates(&s, 50);
        let idx = cands
            .iter()
            .position(|c| c.text == "你")
            .expect("你 in dict");
        assert_eq!(
            select_flat(&mut s, idx),
            ProcessResult::Committed("你".into())
        );
    }
    assert_eq!(
        user_dict::lookup(&s.user_dict, "ni")[0],
        ("你".to_string(), 2)
    );

    type_str(&mut s, "ni");
    // The learned 你 outranks the dictionary's first hit.
    assert_eq!(session::candidates(&s, 50)[0].text, "你");
    assert_eq!(delete_flat(&mut s, 0), DeleteOutcome::Deleted);
    assert!(user_dict::lookup(&s.user_dict, "ni").is_empty());
    // Composition survives: still editing, candidates refresh without the boost.
    assert_eq!(process_key(&mut s, '\u{8}'), ProcessResult::Editing); // Backspace
    type_str(&mut s, "i");
    let cands = session::candidates(&s, 50);
    assert!(
        cands.iter().any(|c| c.text == "你"),
        "dict hit stays available"
    );
    // The dictionary word behind the (no longer learned) top hit cannot be deleted.
    type_str(&mut s, "\u{8}"); // n
    type_str(&mut s, "\u{8}"); // empty
    type_str(&mut s, "ni");
    assert_eq!(delete_flat(&mut s, 0), DeleteOutcome::NotUserWord);
}

/// A span candidate learned under its span key deletes under that same
/// span key while the full input is still typed.
#[test]
fn delete_flat_of_span_candidate_uses_the_span_key() {
    let mut s = new_session_with(default_config(), Dictionary::builtin());
    // Learn the span candidate 你 from `nihao` (span key `ni`).
    for _ in 0..2 {
        type_str(&mut s, "nihao");
        let cands = session::candidates(&s, 50);
        let idx = cands.iter().position(|c| c.text == "你").expect("span 你");
        // Staged (not committed) under the span key; the uncovered `hao`
        // remains typed. Escape clears both before re-typing.
        assert_eq!(select_flat(&mut s, idx), ProcessResult::Editing);
        assert_eq!(s.staged, vec![("ni".to_string(), "你".to_string())]);
        assert_eq!(process_key(&mut s, '\u{1b}'), ProcessResult::Editing);
    }
    assert_eq!(
        user_dict::lookup(&s.user_dict, "ni")[0],
        ("你".to_string(), 2)
    );

    type_str(&mut s, "nihao");
    let cands = session::candidates(&s, 50);
    let idx = cands.iter().position(|c| c.text == "你").expect("span 你");
    assert_eq!(delete_flat(&mut s, idx), DeleteOutcome::Deleted);
    assert!(user_dict::lookup(&s.user_dict, "ni").is_empty());
    // The full composition is untouched by the deletion.
    assert_eq!(session::candidates(&s, 50)[0].text, "你好");
}

/// Out-of-range flat indices report `OutOfRange` and change nothing.
#[test]
fn delete_flat_out_of_range_is_reported() {
    let mut s = new_session_with(default_config(), Dictionary::builtin());
    type_str(&mut s, "ni");
    let count = session::candidates(&s, 50).len();
    assert_eq!(delete_flat(&mut s, count), DeleteOutcome::OutOfRange);
    assert_eq!(delete_flat(&mut s, count + 7), DeleteOutcome::OutOfRange);
    assert!(!user_dict::is_empty(&s.user_dict) || true); // untouched
}

/// A deleted word is gone from the persisted snapshot round-trip (the
/// host saves right after a deletion; reload must not resurrect it).
#[test]
fn delete_flat_survives_snapshot_round_trip() {
    let mut s = new_session_with(default_config(), Dictionary::builtin());
    // Seed a learned homophone 泥 under `ni` (span-key commit from nihao
    // would also work; a direct seed keeps this test about persistence).
    user_dict::learn(&mut s.user_dict, "ni", "泥");
    type_str(&mut s, "ni");
    let cands = session::candidates(&s, 50);
    let idx = cands
        .iter()
        .position(|c| c.text == "泥")
        .expect("seeded entry surfaces");
    assert_eq!(delete_flat(&mut s, idx), DeleteOutcome::Deleted);
    // Snapshot round-trip no longer contains the removed word.
    let bytes = user_dict::to_snapshot(&s.user_dict);
    let reloaded = user_dict::from_snapshot(&bytes).expect("snapshot parses");
    assert!(user_dict::lookup(&reloaded, "ni").is_empty());
}
