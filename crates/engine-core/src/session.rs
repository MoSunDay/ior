//! Interactive editing session: raw key buffering, candidate listing,
//! selection, commit and learning.
//!
//! A [`Session`] bundles the raw preedit buffer, the engine configuration,
//! the static dictionary (fuzzy rules already baked into its keys) and the
//! user dictionary. [`process_key`] is a pure state transition: it inspects
//! `s.raw` and `s.config`, mutates only through `&mut Session`, and reports
//! what the frontend should do with the key.

use crate::config::EngineConfig;
use crate::decoder::{self, Candidate};
use crate::dict::Dictionary;
use crate::segmenter::best_segmentation;
use crate::user_dict::{self, UserDict};

/// Flat candidate-list length used for digit selection and space commit.
const FLAT_CANDIDATE_LIMIT: usize = 10;

/// Everything an input method needs per client.
#[derive(Debug)]
pub struct Session {
    /// The raw preedit buffer (lowercase letters plus `'` separators).
    pub raw: String,
    /// Engine configuration snapshot for this session.
    pub config: EngineConfig,
    /// Static dictionary (fuzzy rules baked in at build time).
    pub dict: Dictionary,
    /// Learned entries for this user.
    pub user_dict: UserDict,
}

/// What the frontend should do with a key event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessResult {
    /// Key is not for us: forward it to the application untouched.
    Forwarded,
    /// Key consumed; the preedit/candidate list changed.
    Editing,
    /// Key consumed; commit this text to the application.
    Committed(String),
}

/// Creates a session with the builtin core dictionary built under
/// `config.fuzzy`.
pub fn new_session(config: EngineConfig) -> Session {
    let dict = Dictionary::builtin(&config.fuzzy);
    new_session_with(config, dict)
}

/// Creates a session over a caller-supplied dictionary (e.g. a merged or
/// fuzzy-built one). Pass a dictionary built with the same `config.fuzzy`
/// for consistent keys.
pub fn new_session_with(config: EngineConfig, dict: Dictionary) -> Session {
    Session {
        raw: String::new(),
        config,
        dict,
        user_dict: user_dict::new(),
    }
}

/// Feeds one key into the session and reports the outcome.
pub fn process_key(s: &mut Session, ch: char) -> ProcessResult {
    match ch {
        'a'..='z' => {
            s.raw.push(ch);
            ProcessResult::Editing
        }
        'A'..='Z' => ProcessResult::Forwarded,
        '\'' => {
            if s.raw.is_empty() {
                ProcessResult::Forwarded
            } else {
                s.raw.push(ch);
                ProcessResult::Editing
            }
        }
        '0'..='9' => handle_digit(s, ch),
        ' ' | '\n' | '\r' => handle_commit_or_passthrough(s),
        '\u{8}' | '\u{7f}' => {
            if s.raw.is_empty() {
                ProcessResult::Forwarded
            } else {
                s.raw.pop();
                ProcessResult::Editing
            }
        }
        '\u{1b}' => {
            if s.raw.is_empty() {
                ProcessResult::Forwarded
            } else {
                s.raw.clear();
                ProcessResult::Editing
            }
        }
        _ => ProcessResult::Forwarded,
    }
}

/// Digit selection: `1`..`9` pick index 0..8, `0` picks index 9. A digit is
/// forwarded only when there is no preedit; a missing index swallows the key.
fn handle_digit(s: &mut Session, ch: char) -> ProcessResult {
    if s.raw.is_empty() {
        return ProcessResult::Forwarded;
    }
    let idx = match ch.to_digit(10) {
        Some(0) => 9,
        Some(d) => d as usize - 1,
        None => return ProcessResult::Editing,
    };
    let cands = candidates(s, FLAT_CANDIDATE_LIMIT);
    match cands.get(idx) {
        Some(cand) => commit_candidate(s, cand),
        None => ProcessResult::Editing,
    }
}

/// Space/enter: commit the top candidate, or pass the raw letters through
/// verbatim when nothing can be decoded.
fn handle_commit_or_passthrough(s: &mut Session) -> ProcessResult {
    if s.raw.is_empty() {
        return ProcessResult::Forwarded;
    }
    let cands = candidates(s, FLAT_CANDIDATE_LIMIT);
    match cands.first() {
        Some(cand) => commit_candidate(s, cand),
        None => {
            let text = s.raw.clone();
            s.raw.clear();
            ProcessResult::Committed(text)
        }
    }
}

/// Commits `cand`: learn it under the normalized key of the current raw
/// input (when learning is on), clear the buffer, and report the text.
fn commit_candidate(s: &mut Session, cand: &Candidate) -> ProcessResult {
    let key = normalized_key(&s.raw);
    if s.config.user_learning {
        if let Some(k) = key {
            user_dict::learn(&mut s.user_dict, &k, &cand.text);
        }
    }
    s.raw.clear();
    ProcessResult::Committed(cand.text.clone())
}

/// Up to `min(limit, config.max_candidates)` candidates for the preedit.
pub fn candidates(s: &Session, limit: usize) -> Vec<Candidate> {
    let limit = limit.min(s.config.max_candidates);
    let user = if s.config.user_learning {
        Some(&s.user_dict)
    } else {
        None
    };
    decoder::candidates(&s.dict, user, &s.raw, limit)
}

/// Preedit string for display: each `'`-separated run is shown as its best
/// segmentation (as-typed letters), e.g. `nihao` -> `ni'hao`. Runs that
/// cannot be segmented yet leave the raw string unchanged.
pub fn preedit(s: &Session) -> String {
    let mut parts: Vec<String> = Vec::new();
    for run in s.raw.split('\'') {
        match best_segmentation(run) {
            Some(segs) => parts.push(segs.join("'")),
            None => return s.raw.clone(),
        }
    }
    parts.join("'")
}

/// Clears the preedit buffer (keeps dictionaries and learning data).
pub fn reset(s: &mut Session) {
    s.raw.clear();
}

/// Best-segmentation key of `raw`: each `'`-separated run segmented and all
/// syllables joined with `'`. `None` when any run cannot be segmented.
/// (No fuzzy rules here; those live inside the dictionary.)
pub fn normalized_key(raw: &str) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    for run in raw.split('\'') {
        parts.push(best_segmentation(run)?.join("'"));
    }
    Some(parts.join("'"))
}
