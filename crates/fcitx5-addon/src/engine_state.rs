//! Internal engine state behind the C ABI ([`crate::ffi`]).
//!
//! [`IorEngine`] owns an engine-core [`Session`] plus the C-string buffers
//! that back the pointers handed out by the FFI layer. The contract there is
//! "valid until the next `ior_*` call on the same engine", which this module
//! upholds by only ever replacing buffers wholesale: an old pointer stays
//! readable until a later call swaps the owning `CString` out.
//!
//! Plain data struct + free functions only, no methods, no inheritance.

use std::ffi::CString;

use engine_core::default_config;
use engine_core::session::{self, ProcessResult, Session};

/// How many candidates the FFI layer reports at most (`0..=10`).
pub const CANDIDATE_LIMIT: usize = 10;

/// Opaque engine handed to C. Boxed by [`crate::ffi::ior_engine_new`] and
/// freed by [`crate::ffi::ior_engine_free`]; C never touches its layout.
pub struct IorEngine {
    /// Core state machine (raw buffer, dictionaries, learning data).
    pub session: Session,
    /// Display preedit, refreshed after every state change.
    pub preedit_buf: CString,
    /// Pending commit text; survives until `take_commit` is called.
    pub commit_buf: CString,
    /// True once the pending commit has been handed to the caller.
    pub commit_taken: bool,
    /// Candidate texts for the current preedit, in display order.
    pub candidates_buf: Vec<CString>,
}

/// Builds an engine around a default-config session over the builtin
/// dictionary, with freshly computed display buffers.
pub fn new_engine() -> IorEngine {
    let mut engine = IorEngine {
        session: session::new_session(default_config()),
        preedit_buf: CString::new("").expect("empty CString"),
        commit_buf: CString::new("").expect("empty CString"),
        commit_taken: false,
        candidates_buf: Vec::new(),
    };
    refresh(&mut engine);
    engine
}

/// Feeds one mapped character to the session and refreshes the display
/// buffers. Returns the FFI status code:
/// `0` forwarded, `1` editing, `2` committed (see `take_commit`).
pub fn process_key(engine: &mut IorEngine, ch: char) -> i32 {
    let result = session::process_key(&mut engine.session, ch);
    refresh(engine);
    match result {
        ProcessResult::Forwarded => 0,
        ProcessResult::Editing => 1,
        ProcessResult::Committed(text) => {
            engine.commit_buf = to_cstring(&text);
            engine.commit_taken = false;
            2
        }
    }
}

/// Clears the editing buffer and refreshes the display buffers.
pub fn reset(engine: &mut IorEngine) {
    session::reset(&mut engine.session);
    refresh(engine);
}

/// Recomputes preedit and candidate buffers from the session state.
/// The commit buffer is untouched: a pending commit survives reset and is
/// only replaced by the next commit or consumed by `take_commit`.
pub fn refresh(engine: &mut IorEngine) {
    engine.preedit_buf = to_cstring(&session::preedit(&engine.session));
    engine.candidates_buf = session::candidates(&engine.session, CANDIDATE_LIMIT)
        .iter()
        .map(|cand| to_cstring(&cand.text))
        .collect();
}

/// Hands out the pending commit, if any. Returns true when `commit_buf`
/// holds text that has not been taken yet; a second call without an
/// intervening commit always reports "nothing pending".
pub fn take_commit(engine: &mut IorEngine) -> bool {
    let pending = !engine.commit_taken && !engine.commit_buf.as_bytes().is_empty();
    engine.commit_taken = true;
    pending
}

/// NUL-terminates a Rust string for C. Engine output never contains NUL,
/// so the fallback (strip NULs) is unreachable in practice but keeps the
/// conversion total instead of panicking across the FFI boundary.
fn to_cstring(s: &str) -> CString {
    CString::new(s).unwrap_or_else(|_| {
        let cleaned: String = s.chars().filter(|c| *c != '\0').collect();
        CString::new(cleaned).expect("NUL bytes already removed")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn committed_then_reset_keeps_pending_commit() {
        let mut engine = new_engine();
        for ch in "ni".chars() {
            assert_eq!(process_key(&mut engine, ch), 1);
        }
        assert_eq!(process_key(&mut engine, ' '), 2);
        reset(&mut engine); // e.g. focus loss before the host took the text
        assert!(take_commit(&mut engine));
        assert!(!take_commit(&mut engine));
    }

    #[test]
    fn buffers_track_session_state() {
        let mut engine = new_engine();
        assert_eq!(engine.preedit_buf.to_str(), Ok(""));
        assert!(engine.candidates_buf.is_empty());
        assert_eq!(process_key(&mut engine, 'n'), 1);
        assert_eq!(engine.preedit_buf.to_str(), Ok("n"));
        process_key(&mut engine, 'i');
        assert_eq!(engine.preedit_buf.to_str(), Ok("ni"));
        assert!(!engine.candidates_buf.is_empty());
    }
}
