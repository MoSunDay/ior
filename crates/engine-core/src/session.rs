//! Interactive editing session: raw key buffering, candidate listing,
//! selection, commit and learning.
//!
//! A [`Session`] bundles the raw preedit buffer, the engine configuration,
//! the static dictionary and the user dictionary. Spelling transforms are
//! applied at decode time: fuzzy rules via the query-side pipeline
//! (`s.config.fuzzy`), double pinyin via the optional [`DpTable`] on the
//! session — the preedit always shows the letters as typed. [`process_key`]
//! is a pure state transition: it inspects `s.raw` and `s.config`, mutates
//! only through `&mut Session`, and reports what the frontend should do
//! with the key.

use crate::config::{DoublePinyin, EngineConfig, EnterAction, ShiftAction};
use crate::decoder::{self, Candidate};
use crate::dict::Dictionary;
use crate::double_pinyin::{self, DpTable};
use crate::lm::GramModel;
use crate::punct;
use crate::segmenter::best_segmentation;
use crate::user_dict::{self, UserDict};

/// Flat candidate-list length used for digit selection, space commit and
/// paging math (still capped by `config.max_candidates` in `candidates`).
const FLAT_CANDIDATE_LIMIT: usize = 10;

/// Everything an input method needs per client.
#[derive(Debug)]
pub struct Session {
    /// The raw preedit buffer (lowercase letters plus `'` separators).
    pub raw: String,
    /// Engine configuration snapshot for this session.
    pub config: EngineConfig,
    /// Static dictionary (canonical keys; fuzzy matching is query-side).
    pub dict: Dictionary,
    /// Optional character n-gram model; when set, inputs of three or more
    /// syllables use whole-sentence beam search (see [`candidates`]).
    pub lm: Option<GramModel>,
    /// Active double-pinyin table. Filled at construction for the builtin
    /// schemes (`Natural`/`Xiaohe`/`Mspy`, bundled data) and installed by
    /// the host for `Custom` (`double_pinyin::parse_tsv`); `None` with a
    /// non-`Off` config leaves double-pinyin decoding inactive.
    pub dp_table: Option<DpTable>,
    /// Learned entries for this user.
    pub user_dict: UserDict,
    /// False = Chinese mode, true = English (ASCII) mode. Letters are
    /// forwarded untouched while true; Shift toggles it
    /// (see [`process_shift`]).
    pub ascii_mode: bool,
    /// When true, Chinese mode still emits raw ASCII punctuation
    /// (per-session override of `config.chinese_punct`).
    pub ascii_punct: bool,
    /// Current candidate page index into the flat candidate list.
    pub page: usize,
    /// Paired-quote parity for Chinese punctuation.
    pub punct: punct::PunctState,
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

/// Creates a session with the builtin core dictionary (canonical keys).
pub fn new_session(config: EngineConfig) -> Session {
    new_session_with(config, Dictionary::builtin())
}

/// Creates a session over a caller-supplied dictionary (e.g. `.ior`
/// layers merged with [`Dictionary::from_ior_layers`]).
pub fn new_session_with(config: EngineConfig, dict: Dictionary) -> Session {
    let dp_table = double_pinyin::builtin(config.double_pinyin);
    Session {
        raw: String::new(),
        config,
        dict,
        lm: None,
        dp_table,
        user_dict: user_dict::new(),
        ascii_mode: false,
        ascii_punct: false,
        page: 0,
        punct: punct::new(),
    }
}

/// Installs (or clears) the double-pinyin table used for decoding with
/// [`DoublePinyin::Custom`]; the builtin schemes are installed by
/// [`new_session_with`] and need no call here.
pub fn set_double_pinyin(s: &mut Session, table: Option<DpTable>) {
    s.dp_table = table;
}

/// Installs (or clears) the character n-gram model used by whole-sentence
/// decoding. Sessions without one keep the legacy three-step candidate
/// assembly, so callers can enable this only when a `.gram` file is present.
pub fn set_lm(s: &mut Session, lm: Option<GramModel>) {
    s.lm = lm;
}

/// Feeds one key into the session and reports the outcome.
pub fn process_key(s: &mut Session, ch: char) -> ProcessResult {
    match ch {
        'a'..='z' if s.ascii_mode => ProcessResult::Forwarded,
        'a'..='z' => {
            s.raw.push(ch);
            s.page = 0;
            ProcessResult::Editing
        }
        'A'..='Z' => ProcessResult::Forwarded,
        '\'' => {
            if s.raw.is_empty() {
                // No composition: the quote is punctuation, not a separator.
                handle_punct(s, ch)
            } else {
                s.raw.push(ch);
                s.page = 0;
                ProcessResult::Editing
            }
        }
        '0'..='9' => handle_digit(s, ch),
        ' ' => handle_commit_or_passthrough(s),
        '\n' | '\r' => handle_enter(s),
        '\u{8}' | '\u{7f}' => {
            if s.raw.is_empty() {
                ProcessResult::Forwarded
            } else {
                s.raw.pop();
                s.page = 0;
                ProcessResult::Editing
            }
        }
        '\u{1b}' => {
            if s.raw.is_empty() {
                ProcessResult::Forwarded
            } else {
                reset_composition(s);
                ProcessResult::Editing
            }
        }
        _ => handle_punct(s, ch),
    }
}

/// Digit selection: `1`..`9` pick page-local index 0..8, `0` picks 9, both
/// offset by the current page. A digit is forwarded only when there is no
/// preedit; a missing index swallows the key.
fn handle_digit(s: &mut Session, ch: char) -> ProcessResult {
    if s.raw.is_empty() {
        return ProcessResult::Forwarded;
    }
    let offset = match ch.to_digit(10) {
        Some(0) => 9,
        Some(d) => d as usize - 1,
        None => return ProcessResult::Editing,
    };
    select_flat(s, s.page * page_size(s) + offset)
}

/// Space: commit the top candidate, or pass the raw letters through
/// when nothing can be decoded (typed `'` separators are dropped).
fn handle_commit_or_passthrough(s: &mut Session) -> ProcessResult {
    if s.raw.is_empty() {
        return ProcessResult::Forwarded;
    }
    let cands = candidates(s, FLAT_CANDIDATE_LIMIT);
    match cands.first() {
        Some(cand) => commit_candidate(s, cand),
        None => {
            let text = s.raw.replace('\'', "");
            reset_composition(s);
            ProcessResult::Committed(text)
        }
    }
}

/// Enter: commit the typed letters verbatim so Latin text can be
/// entered without leaving Chinese mode; no learning applies.
/// `EnterAction::CommitBest` takes the Space path instead, and an
/// empty composition forwards Enter so applications still receive
/// the newline.
fn handle_enter(s: &mut Session) -> ProcessResult {
    if s.raw.is_empty() {
        return ProcessResult::Forwarded;
    }
    match s.config.enter_action {
        EnterAction::CommitRaw => {
            let text = s.raw.clone();
            reset_composition(s);
            ProcessResult::Committed(text)
        }
        EnterAction::CommitBest => handle_commit_or_passthrough(s),
    }
}

/// Commits `cand`: learn it under the normalized key of the *decode input*
/// (the raw buffer, or its double-pinyin transform when that mode is on)
/// when learning is on, clear the composition, and report the text.
fn commit_candidate(s: &mut Session, cand: &Candidate) -> ProcessResult {
    let key = normalized_key(&decode_input(s));
    if s.config.user_learning {
        if let Some(k) = key {
            user_dict::learn(&mut s.user_dict, &k, &cand.text);
        }
    }
    reset_composition(s);
    ProcessResult::Committed(cand.text.clone())
}

/// Drops the composition: raw buffer and page index. Shared by every path
/// that clears what is being typed (commit, escape, mode switch).
fn reset_composition(s: &mut Session) {
    s.raw.clear();
    s.page = 0;
}

/// Effective page size, never zero (a zero `config.page_size` degrades to
/// one candidate per page rather than dividing by zero).
fn page_size(s: &Session) -> usize {
    s.config.page_size.max(1)
}

/// Number of candidate pages for the current preedit (0 when there are no
/// candidates): flat candidate count rounded up over `config.page_size`.
pub fn page_count(s: &Session) -> usize {
    let n = candidates(s, FLAT_CANDIDATE_LIMIT).len();
    if n == 0 {
        0
    } else {
        n.div_ceil(page_size(s))
    }
}

/// The candidates of page `s.page`, sliced out of the flat list.
pub fn current_page(s: &Session) -> Vec<Candidate> {
    let flat = candidates(s, FLAT_CANDIDATE_LIMIT);
    let start = (s.page * page_size(s)).min(flat.len());
    let end = (start + page_size(s)).min(flat.len());
    flat[start..end].to_vec()
}

/// Advances to the next page, wrapping from the last page back to the
/// first. Returns whether the page index changed.
pub fn next_page(s: &mut Session) -> bool {
    let count = page_count(s);
    if count <= 1 {
        return false;
    }
    let cur = s.page.min(count - 1);
    s.page = (cur + 1) % count;
    s.page != cur
}

/// Goes back one page, wrapping from the first page to the last. Returns
/// whether the page index changed.
pub fn prev_page(s: &mut Session) -> bool {
    let count = page_count(s);
    if count <= 1 {
        return false;
    }
    let cur = s.page.min(count - 1);
    s.page = if cur == 0 { count - 1 } else { cur - 1 };
    s.page != cur
}

/// Commits the flat-list candidate at `flat_index` (learning and page reset
/// included, exactly like the digit/space commit paths). An out-of-range
/// index swallows the key and keeps the composition.
pub fn select_flat(s: &mut Session, flat_index: usize) -> ProcessResult {
    let cands = candidates(s, FLAT_CANDIDATE_LIMIT);
    match cands.get(flat_index) {
        Some(cand) => commit_candidate(s, cand),
        None => ProcessResult::Editing,
    }
}

/// Toggles Chinese/English mode. Switching to English applies
/// `config.shift_action` to a non-empty preedit (commit raw / commit the
/// best candidate, learning included / clear). Switching back to Chinese
/// just reports `Editing`. Either way the composition is cleared.
pub fn process_shift(s: &mut Session) -> ProcessResult {
    s.ascii_mode = !s.ascii_mode;
    let result = if s.ascii_mode && !s.raw.is_empty() {
        match s.config.shift_action {
            ShiftAction::CommitRaw => {
                let text = s.raw.clone();
                reset_composition(s);
                ProcessResult::Committed(text)
            }
            ShiftAction::CommitBest => {
                let cands = candidates(s, FLAT_CANDIDATE_LIMIT);
                match cands.first() {
                    Some(cand) => commit_candidate(s, cand),
                    None => {
                        let text = s.raw.clone();
                        reset_composition(s);
                        ProcessResult::Committed(text)
                    }
                }
            }
            ShiftAction::Clear => {
                reset_composition(s);
                ProcessResult::Editing
            }
        }
    } else {
        // Back to Chinese, or English with nothing pending: plain editing.
        ProcessResult::Editing
    };
    reset_composition(s);
    result
}

/// Flips the per-session "ASCII punctuation even in Chinese mode" flag.
pub fn toggle_ascii_punct(s: &mut Session) {
    s.ascii_punct = !s.ascii_punct;
}

/// Punctuation key: in Chinese mode with `config.chinese_punct` on (and the
/// per-session `ascii_punct` override off) the key commits its Chinese
/// counterpart; with a pending composition the best candidate is committed
/// first (same learning/reset as selection). Everything else passes
/// through, which the frontend reports as a forward.
fn handle_punct(s: &mut Session, ch: char) -> ProcessResult {
    let chinese = !s.ascii_mode && s.config.chinese_punct && !s.ascii_punct;
    match punct::map(&mut s.punct, ch, chinese) {
        punct::PunctOut::Text(t) => {
            if s.raw.is_empty() {
                return ProcessResult::Committed(t);
            }
            let cands = candidates(s, FLAT_CANDIDATE_LIMIT);
            match cands.first() {
                Some(cand) => {
                    let mut text = cand.text.clone();
                    commit_candidate(s, cand);
                    text.push_str(&t);
                    ProcessResult::Committed(text)
                }
                None => {
                    reset_composition(s);
                    ProcessResult::Committed(t)
                }
            }
        }
        punct::PunctOut::Passthrough => ProcessResult::Forwarded,
    }
}

/// Up to `min(limit, config.max_candidates)` candidates for the preedit.
/// Decoding runs on the double-pinyin transform of the raw buffer when a
/// table is active (builtin scheme or `Custom` + installed table; a failed
/// transform falls back to the raw letters); the preedit itself keeps
/// showing `s.raw`.
pub fn candidates(s: &Session, limit: usize) -> Vec<Candidate> {
    let limit = limit.min(s.config.max_candidates);
    let user = if s.config.user_learning {
        Some(&s.user_dict)
    } else {
        None
    };
    let raw = decode_input(s);
    decoder::candidates_with_lm(&s.dict, user, s.lm.as_ref(), &s.config.fuzzy, &raw, limit)
}

/// The string decoding operates on: `s.raw` under full pinyin, or its
/// double-pinyin expansion when a table is active. A transform that fails
/// (odd length, unknown code, invalid syllable) keeps the raw letters so
/// typing stays live.
fn decode_input(s: &Session) -> String {
    match (&s.config.double_pinyin, &s.dp_table) {
        (DoublePinyin::Off, _) | (_, None) => s.raw.clone(),
        (_, Some(table)) => double_pinyin::to_full(table, &s.raw).unwrap_or_else(|| s.raw.clone()),
    }
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

/// Clears the preedit buffer and page (keeps dictionaries, learning data
/// and the mode/punctuation flags).
pub fn reset(s: &mut Session) {
    reset_composition(s);
}

/// Best-segmentation key of `raw`: each `'`-separated run segmented and all
/// syllables joined with `'`. `None` when any run cannot be segmented.
/// (No fuzzy rules here; the decoder probes spelling variants around this
/// key at query time.)
pub fn normalized_key(raw: &str) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    for run in raw.split('\'') {
        parts.push(best_segmentation(run)?.join("'"));
    }
    Some(parts.join("'"))
}
