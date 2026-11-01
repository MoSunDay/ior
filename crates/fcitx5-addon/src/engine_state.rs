//! Internal engine state behind the C ABI ([`crate::ffi`]).
//!
//! [`IorEngine`] owns an engine-core [`Session`] plus the C-string buffers
//! that back the pointers handed out by the FFI layer. The contract there is
//! "valid until the next `ior_*` call on the same engine", which this module
//! upholds by only ever replacing buffers wholesale: an old pointer stays
//! readable until a later call swaps the owning `CString` out.
//!
//! The candidate buffer mirrors the session's **current page** (at most
//! `config.page_size` entries), so what the frontend displays is exactly
//! what digit selection and page-local selection act on.
//!
//! Plain data struct + free functions only, no methods, no inheritance.

use std::ffi::CString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use engine_core::config::{self as core_config, EngineConfig, FuzzyConfig};
use engine_core::dict::Dictionary;
use engine_core::lm::{self, GramModel};
use engine_core::session::{self, ProcessResult, Session};
use engine_core::user_dict::{self, UserDict};
use ior_config as user_config;

/// Opaque engine handed to C. Boxed by [`crate::ffi::ior_engine_new`] and
/// freed by [`crate::ffi::ior_engine_free`]; C never touches its layout.
pub struct IorEngine {
    /// Core state machine (raw buffer, dictionaries, learning data, page,
    /// mode flags).
    pub session: Session,
    /// Display preedit, refreshed after every state change.
    pub preedit_buf: CString,
    /// Pending commit text; survives until `take_commit` is called.
    pub commit_buf: CString,
    /// True once the pending commit has been handed to the caller.
    pub commit_taken: bool,
    /// Candidate texts of the **current page**, in display order.
    pub candidates_buf: Vec<CString>,
}

/// Builds an engine at client start: user config (`$XDG_CONFIG_HOME/
/// fcitx5/ior/config.toml`, all defaults when absent or invalid), the
/// `core.ior` dictionary when installed (user data dir first, then the
/// system share dir), an optional `core.gram` language model from those
/// same data dirs, and freshly computed display buffers.
pub fn new_engine() -> IorEngine {
    new_engine_with_lm(startup_config(), startup_dictionary(), startup_lm())
}

/// Builds an engine over caller-supplied config + dictionary without a
/// language model. Tests use this to stay independent of host data.
pub fn new_engine_with(engine_config: EngineConfig, dict: Dictionary) -> IorEngine {
    new_engine_with_lm(engine_config, dict, None)
}

/// Builds an engine over caller-supplied config, dictionary and optional
/// language model, then computes the initial display buffers.
pub fn new_engine_with_lm(
    engine_config: EngineConfig,
    dict: Dictionary,
    lm_model: Option<GramModel>,
) -> IorEngine {
    let mut session = session::new_session_with(engine_config, dict);
    session::set_lm(&mut session, lm_model);
    let mut engine = IorEngine {
        session,
        preedit_buf: CString::new("").expect("empty CString"),
        commit_buf: CString::new("").expect("empty CString"),
        commit_taken: false,
        candidates_buf: Vec::new(),
    };
    refresh(&mut engine);
    engine
}

/// Engine config for a fresh client: the user's TOML over the defaults.
pub fn startup_config() -> EngineConfig {
    let Some(path) = config_path(
        std::env::var("XDG_CONFIG_HOME").ok().as_deref(),
        std::env::var("HOME").ok().as_deref(),
    ) else {
        return core_config::default_config();
    };
    match read_user_config(&path) {
        Some(config) => to_engine_config(&config),
        None => core_config::default_config(),
    }
}

/// Dictionary for a fresh client: installed `core.ior` when present and
/// loadable, the builtin table otherwise.
pub fn startup_dictionary() -> Dictionary {
    let paths = dict_paths(
        std::env::var("XDG_DATA_HOME").ok().as_deref(),
        std::env::var("HOME").ok().as_deref(),
    );
    for path in paths {
        match fs::read(&path) {
            Ok(bytes) => match Dictionary::from_ior_bytes(&bytes) {
                Ok(dict) => return dict,
                Err(err) => eprintln!("ior: {}: {err}", path.display()),
            },
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => eprintln!("ior: {}: {err}", path.display()),
        }
    }
    Dictionary::builtin()
}

/// Optional language model for a fresh client: `core.gram` next to an
/// installed `core.ior` (user data dir first, then system share dir).
/// Missing files mean "no LM"; unreadable or invalid files are reported
/// and skipped so a bad model can never prevent the engine from starting.
pub fn startup_lm() -> Option<GramModel> {
    let paths = lm_paths(
        std::env::var("XDG_DATA_HOME").ok().as_deref(),
        std::env::var("HOME").ok().as_deref(),
    );
    read_lm(&paths)
}

/// First readable, parseable `.gram` in `paths`; diagnostics to stderr.
fn read_lm(paths: &[PathBuf]) -> Option<GramModel> {
    for path in paths {
        match fs::read(path) {
            Ok(bytes) => match lm::from_gram_bytes(&bytes) {
                Ok(model) => return Some(model),
                Err(err) => eprintln!("ior: {}: {err}", path.display()),
            },
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => eprintln!("ior: {}: {err}", path.display()),
        }
    }
    None
}

/// `$XDG_CONFIG_HOME/fcitx5/ior/config.toml`, falling back to
/// `$HOME/.config/fcitx5/ior/config.toml` (pure resolution).
fn config_path(xdg_config_home: Option<&str>, home: Option<&str>) -> Option<PathBuf> {
    let base = match xdg_config_home {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => home.map(|h| Path::new(h).join(".config"))?,
    };
    Some(base.join("fcitx5").join("ior").join("config.toml"))
}

/// Dictionary lookup order: the user data dir first (lets a user drop in
/// an updated `core.ior` without touching /usr), then the system share
/// dir the install script targets.
fn dict_paths(xdg_data_home: Option<&str>, home: Option<&str>) -> Vec<PathBuf> {
    data_paths(xdg_data_home, home, "core.ior")
}

/// Language-model lookup order, mirroring [`dict_paths`] with `core.gram`.
fn lm_paths(xdg_data_home: Option<&str>, home: Option<&str>) -> Vec<PathBuf> {
    data_paths(xdg_data_home, home, "core.gram")
}

/// User data dir entry followed by the system share dir entry.
fn data_paths(xdg_data_home: Option<&str>, home: Option<&str>, file: &str) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let user_base = match xdg_data_home {
        Some(dir) if !dir.is_empty() => Some(PathBuf::from(dir)),
        _ => home.map(|h| Path::new(h).join(".local").join("share")),
    };
    if let Some(base) = user_base {
        paths.push(base.join("fcitx5").join("ior").join(file));
    }
    paths.push(Path::new("/usr/share/fcitx5/ior").join(file));
    paths
}

/// Reads and validates one user config file; `None` (after a one-line
/// stderr diagnostic, which fcitx5 shows in its journal) on any error.
fn read_user_config(path: &Path) -> Option<user_config::Config> {
    let text = fs::read_to_string(path).ok()?;
    match user_config::apply_with_warnings(
        &user_config::default_table(),
        &user_config::parse(&text).ok()?,
    ) {
        Ok(loaded) => {
            for warning in loaded.warnings {
                eprintln!("ior: {}: {warning}", path.display());
            }
            Some(loaded.config)
        }
        Err(err) => {
            eprintln!("ior: {}: {err}", path.display());
            None
        }
    }
}

/// Maps the user-facing config model onto engine-core's runtime config.
fn to_engine_config(cfg: &user_config::Config) -> EngineConfig {
    let shift_action = match cfg.engine.shift_action {
        user_config::ShiftAction::Commit => core_config::ShiftAction::CommitRaw,
        user_config::ShiftAction::Temp => core_config::ShiftAction::CommitBest,
        user_config::ShiftAction::Drop => core_config::ShiftAction::Clear,
    };
    let enter_action = match cfg.engine.enter_action {
        user_config::EnterAction::Raw => core_config::EnterAction::CommitRaw,
        user_config::EnterAction::Candidate => core_config::EnterAction::CommitBest,
    };
    let double_pinyin = match cfg.engine.double_pinyin {
        user_config::DoublePinyin::Off => core_config::DoublePinyin::Off,
        user_config::DoublePinyin::Natural => core_config::DoublePinyin::Natural,
        user_config::DoublePinyin::Xiaohe => core_config::DoublePinyin::Xiaohe,
        user_config::DoublePinyin::Mspy => core_config::DoublePinyin::Mspy,
        user_config::DoublePinyin::Custom => core_config::DoublePinyin::Custom,
    };
    let fuzzy = &cfg.engine.fuzzy;
    EngineConfig {
        fuzzy: FuzzyConfig {
            z_zh: fuzzy.z_zh,
            c_ch: fuzzy.c_ch,
            s_sh: fuzzy.s_sh,
            n_l: fuzzy.n_l,
            f_h: fuzzy.f_h,
            an_ang: fuzzy.an_ang,
            en_eng: fuzzy.en_eng,
            in_ing: fuzzy.in_ing,
        },
        double_pinyin,
        user_learning: cfg.engine.user_learning,
        page_size: cfg.engine.page_size,
        max_candidates: cfg.engine.max_candidates,
        chinese_punct: cfg.engine.chinese_punct,
        shift_action,
        enter_action,
    }
}

/// Feeds one mapped character to the session and refreshes the display
/// buffers. Returns the FFI status code:
/// `0` forwarded, `1` editing, `2` committed (see `take_commit`).
pub fn process_key(engine: &mut IorEngine, ch: char) -> i32 {
    // English mode: letters belong to the application, buffers stay empty.
    if engine.session.ascii_mode && ch.is_ascii_alphabetic() {
        return 0;
    }
    let result = session::process_key(&mut engine.session, ch);
    apply_result(engine, result)
}

/// Runs the core Shift transition (Chinese <-> English mode switch,
/// applying `shift_action` to a pending preedit) and refreshes buffers.
pub fn process_shift(engine: &mut IorEngine) -> i32 {
    let result = session::process_shift(&mut engine.session);
    apply_result(engine, result)
}

/// Clears the editing buffer (and page) and refreshes the display buffers.
pub fn reset(engine: &mut IorEngine) {
    session::reset(&mut engine.session);
    refresh(engine);
}

/// Advances one candidate page (wrapping). Returns whether the page
/// changed; buffers are refreshed either way so they stay in sync.
pub fn page_next(engine: &mut IorEngine) -> bool {
    let changed = session::next_page(&mut engine.session);
    refresh(engine);
    changed
}

/// Goes back one candidate page (wrapping). Returns whether the page
/// changed; buffers are refreshed either way so they stay in sync.
pub fn page_prev(engine: &mut IorEngine) -> bool {
    let changed = session::prev_page(&mut engine.session);
    refresh(engine);
    changed
}

/// Packed `(current_page + 1) << 16 | page_count` (0 when there are no
/// candidates): a single C-call snapshot of the paging state.
pub fn page_info(engine: &IorEngine) -> u32 {
    let count = session::page_count(&engine.session);
    if count == 0 {
        0
    } else {
        ((engine.session.page as u32 + 1) << 16) | count as u32
    }
}

/// Commits the candidate at `page_local_index` of the **current page**
/// (candidate clicks: what the user sees is what gets committed, no matter
/// which page is shown). Returns true when something was committed; the
/// composition is reset and learning applies, exactly like digit selection.
pub fn select_page_local(engine: &mut IorEngine, page_local_index: u32) -> bool {
    let size = engine.session.config.page_size.max(1);
    // A page-local index only addresses what the current page displays.
    if page_local_index as usize >= size {
        return false;
    }
    let flat_index = engine.session.page * size + page_local_index as usize;
    let result = session::select_flat(&mut engine.session, flat_index);
    apply_result(engine, result) == 2
}

/// Whether the session is in English (ASCII) mode.
pub fn ascii_mode(engine: &IorEngine) -> bool {
    engine.session.ascii_mode
}

/// Forces the Chinese/English mode. Changing the mode routes through the
/// core Shift transition, so a pending preedit is committed/cleared per
/// `shift_action` (the commit text lands in the pending-commit buffer).
/// Returns the (new) mode.
pub fn set_ascii_mode(engine: &mut IorEngine, enable: bool) -> bool {
    if enable != engine.session.ascii_mode {
        process_shift(engine);
    }
    engine.session.ascii_mode
}

// ---------------------------------------------------------------------------
// User dictionary persistence. This is the only place in the crate that
// touches the filesystem: engine-core produces/consumes bytes, the host
// hands us a path. Status codes map 1:1 to ior_engine_userdb_* (ffi.rs).
// ---------------------------------------------------------------------------

/// Writes the learned dictionary to `path` as a deterministic snapshot —
/// saving *is* compaction: the whole dictionary is rewritten, no log is
/// kept on disk. Returns 0 = written, 2 = I/O error (e.g. missing parent
/// directory).
pub fn save_userdb(engine: &IorEngine, path: &Path) -> i32 {
    let bytes = user_dict::to_snapshot(&engine.session.user_dict);
    match fs::write(path, bytes) {
        Ok(()) => 0,
        Err(_) => 2,
    }
}

/// Reads a userdb file and merges it into the live dictionary
/// ([`user_dict::absorb`], frequencies add up). The format is sniffed: a
/// leading `# ior userdb` header means snapshot, `L\t` lines mean
/// append-only log; comment-only or empty files load as an empty history.
/// Only the user dictionary changes — preedit, commit buffer and mode are
/// untouched. Returns 0 = merged, 1 = file does not exist (a normal cold
/// start, the engine keeps its state), 2 = other I/O error, 3 = format
/// error.
pub fn load_userdb(engine: &mut IorEngine, path: &Path) -> i32 {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return 1,
        Err(_) => return 2,
    };
    let text = match std::str::from_utf8(&bytes) {
        Ok(text) => text,
        Err(_) => return 3,
    };
    let loaded = match parse_userdb(text) {
        Some(loaded) => loaded,
        None => return 3,
    };
    user_dict::absorb(&mut engine.session.user_dict, &loaded);
    refresh(engine); // candidates may change; preedit/commit stay identical
    0
}

/// Ages the learned frequencies by whole halvings of `half_life_secs`
/// ([`user_dict::decay`]); `half_life_secs == 0` is legal and leaves
/// everything unchanged. Always returns 0.
pub fn decay_userdb(engine: &mut IorEngine, elapsed_secs: u32, half_life_secs: u32) -> i32 {
    user_dict::decay(
        &mut engine.session.user_dict,
        u64::from(elapsed_secs),
        u64::from(half_life_secs),
    );
    refresh(engine);
    0
}

/// Parses userdb text by sniffing the format: `# ior userdb` first line ->
/// snapshot, any `L\t` line -> append-only log, otherwise snapshot (so an
/// empty or comment-only file yields an empty dictionary). `None` on any
/// format error.
fn parse_userdb(text: &str) -> Option<UserDict> {
    let has_header = text
        .lines()
        .find(|line| !line.trim().is_empty())
        .is_some_and(|line| line.starts_with("# ior userdb"));
    let has_log_records = text.lines().any(|line| line.starts_with("L\t"));
    if has_header {
        user_dict::from_snapshot(text.as_bytes()).ok()
    } else if has_log_records {
        user_dict::replay(text).ok()
    } else {
        user_dict::from_snapshot(text.as_bytes()).ok()
    }
}

/// Applies a core [`ProcessResult`] to the display buffers and maps it to
/// the FFI status code (`0`/`1`/`2`, see `take_commit`).
fn apply_result(engine: &mut IorEngine, result: ProcessResult) -> i32 {
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

/// Recomputes preedit and current-page candidate buffers from the session
/// state. The commit buffer is untouched: a pending commit survives reset
/// and is only replaced by the next commit or consumed by `take_commit`.
pub fn refresh(engine: &mut IorEngine) {
    engine.preedit_buf = to_cstring(&session::preedit(&engine.session));
    engine.candidates_buf = session::current_page(&engine.session)
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
    use engine_core::dict::Dictionary;

    /// Ten entries for `ma` (frequency order) so paging has two full pages.
    const MA_TSV: &str = "ma\t一\t1\nma\t二\t2\nma\t三\t3\nma\t四\t4\nma\t五\t5\nma\t六\t6\nma\t七\t7\nma\t八\t8\nma\t九\t9\nma\t十\t10\n";

    /// Engine over the ten-entry dictionary with preedit `raw`.
    fn ma_engine(raw: &str) -> IorEngine {
        let dict = Dictionary::from_tsv(MA_TSV).expect("tsv");
        let mut engine = IorEngine {
            session: session::new_session_with(core_config::default_config(), dict),
            preedit_buf: to_cstring(""),
            commit_buf: to_cstring(""),
            commit_taken: false,
            candidates_buf: Vec::new(),
        };
        engine.session.raw = raw.to_string();
        refresh(&mut engine);
        engine
    }

    fn candidates(engine: &IorEngine) -> Vec<&str> {
        engine
            .candidates_buf
            .iter()
            .map(|c| c.to_str().expect("utf-8"))
            .collect()
    }

    #[test]
    fn refresh_exposes_only_the_current_page() {
        let mut engine = ma_engine("ma");
        assert_eq!(candidates(&engine), ["十", "九", "八", "七", "六"]);
        assert!(page_next(&mut engine));
        assert_eq!(candidates(&engine), ["五", "四", "三", "二", "一"]);
        assert!(page_prev(&mut engine));
        assert_eq!(candidates(&engine), ["十", "九", "八", "七", "六"]);
    }

    #[test]
    fn page_info_packs_page_and_count() {
        let mut engine = ma_engine("ma");
        assert_eq!(page_info(&engine), (1 << 16) | 2);
        page_next(&mut engine);
        assert_eq!(page_info(&engine), (2 << 16) | 2);
        // Empty preedit: no candidates, no pages.
        let idle = new_engine();
        assert_eq!(page_info(&idle), 0);
        // Fewer candidates than a page still report one page.
        let mut small = ma_engine("ma");
        small.session.raw.clear();
        refresh(&mut small);
        assert_eq!(page_info(&small), 0);
    }

    #[test]
    fn select_page_local_commits_displayed_candidate() {
        let mut engine = ma_engine("ma");
        page_next(&mut engine);
        assert!(select_page_local(&mut engine, 1));
        assert_eq!(engine.commit_buf.to_str(), Ok("四"));
        assert_eq!(engine.preedit_buf.to_str(), Ok(""));
        assert!(engine.candidates_buf.is_empty());
        // Invalid page-local index: nothing committed, composition kept.
        let mut engine = ma_engine("ma");
        assert!(!select_page_local(&mut engine, 7));
        assert_eq!(engine.preedit_buf.to_str(), Ok("ma"));
    }

    #[test]
    fn ascii_mode_forwards_letters_and_clears_buffers() {
        let mut engine = ma_engine("ma");
        assert_eq!(process_shift(&mut engine), 2); // CommitRaw("ma")
        assert_eq!(engine.commit_buf.to_str(), Ok("ma"));
        assert!(ascii_mode(&engine));
        assert_eq!(engine.preedit_buf.to_str(), Ok(""));
        assert!(engine.candidates_buf.is_empty());
        assert_eq!(process_key(&mut engine, 'n'), 0);
        assert_eq!(process_key(&mut engine, 'A'), 0);
        assert_eq!(process_key(&mut engine, ','), 0);
        assert_eq!(process_shift(&mut engine), 1); // back to Chinese
        assert!(!ascii_mode(&engine));
        assert_eq!(process_key(&mut engine, 'm'), 1);
    }

    #[test]
    fn set_ascii_mode_is_idempotent_and_commits_pending_raw() {
        let mut engine = ma_engine("ma");
        assert!(set_ascii_mode(&mut engine, true));
        assert_eq!(engine.commit_buf.to_str(), Ok("ma")); // CommitRaw default
                                                          // Re-setting the mode is a no-op: the one pending commit survives
                                                          // (taken exactly once) and no phantom commit appears.
        assert!(set_ascii_mode(&mut engine, true));
        assert!(take_commit(&mut engine));
        assert!(!take_commit(&mut engine));
        assert!(!set_ascii_mode(&mut engine, false));
        assert!(!ascii_mode(&engine));
    }

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
    #[test]
    fn config_path_prefers_xdg_then_home() {
        assert_eq!(
            config_path(Some("/xdg"), Some("/home/u")),
            Some(PathBuf::from("/xdg/fcitx5/ior/config.toml"))
        );
        assert_eq!(
            config_path(None, Some("/home/u")),
            Some(PathBuf::from("/home/u/.config/fcitx5/ior/config.toml"))
        );
        assert_eq!(config_path(None, None), None);
        // an empty XDG_CONFIG_HOME falls back to HOME
        assert_eq!(
            config_path(Some(""), Some("/home/u")),
            Some(PathBuf::from("/home/u/.config/fcitx5/ior/config.toml"))
        );
    }

    #[test]
    fn dict_path_prefers_user_data_dir_then_system() {
        assert_eq!(
            dict_paths(Some("/xdg"), Some("/home/u")),
            vec![
                PathBuf::from("/xdg/fcitx5/ior/core.ior"),
                PathBuf::from("/usr/share/fcitx5/ior/core.ior"),
            ]
        );
        assert_eq!(
            dict_paths(None, Some("/home/u")),
            vec![
                PathBuf::from("/home/u/.local/share/fcitx5/ior/core.ior"),
                PathBuf::from("/usr/share/fcitx5/ior/core.ior"),
            ]
        );
        assert_eq!(
            dict_paths(None, None),
            vec![PathBuf::from("/usr/share/fcitx5/ior/core.ior")]
        );
    }

    #[test]
    fn lm_path_mirrors_dictionary_lookup() {
        assert_eq!(
            lm_paths(Some("/xdg"), Some("/home/u")),
            vec![
                PathBuf::from("/xdg/fcitx5/ior/core.gram"),
                PathBuf::from("/usr/share/fcitx5/ior/core.gram"),
            ]
        );
        assert_eq!(
            lm_paths(None, None),
            vec![PathBuf::from("/usr/share/fcitx5/ior/core.gram")]
        );
    }

    #[test]
    fn read_lm_skips_missing_and_broken_files() {
        let dir = std::env::temp_dir().join(format!("ior-lm-test-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("mkdir");
        let broken = dir.join("broken.gram");
        let good = dir.join("good.gram");
        fs::write(&broken, b"not a gram").expect("write");

        // Golden 你好=8 / 金=3 model (header + 3 unigrams + 1 bigram).
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"IOGRAM");
        bytes.extend_from_slice(&[1, 3]);
        for count in [3u32, 1, 0, 19] {
            bytes.extend_from_slice(&count.to_le_bytes());
        }
        for key in ["你", "好", "金"] {
            bytes.push(key.len() as u8);
            bytes.extend_from_slice(key.as_bytes());
        }
        for value in [8u8, 8, 3] {
            bytes.push(value);
        }
        bytes.push(6);
        bytes.extend_from_slice("你好".as_bytes());
        bytes.push(8);
        fs::write(&good, bytes).expect("write");

        let paths = vec![dir.join("missing.gram"), broken, good];
        let model = read_lm(&paths).expect("loads the valid model after bad paths");
        assert_eq!(model.total_weight(), 19);
        assert_eq!(model.counts(), (3, 1, 0));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn user_toml_maps_onto_engine_config() {
        let text = "[engine]\nshift_action = \"none\"\ndouble_pinyin = \"flypy\"\n\
                    page_size = 7\nmax_candidates = 12\nchinese_punct = false\n\
                    user_learning = false\n[engine.fuzzy]\nz_zh = true\nn_l = true\n";
        let loaded = user_config::apply(
            &user_config::default_table(),
            &user_config::parse(text).expect("parse"),
        )
        .expect("apply");
        let cfg = to_engine_config(&loaded);
        assert_eq!(cfg.shift_action, core_config::ShiftAction::Clear);
        assert_eq!(cfg.double_pinyin, core_config::DoublePinyin::Xiaohe);
        assert_eq!(cfg.page_size, 7);
        assert_eq!(cfg.max_candidates, 12);
        assert!(!cfg.chinese_punct);
        assert!(!cfg.user_learning);
        assert!(cfg.fuzzy.z_zh && cfg.fuzzy.n_l);
        assert!(!cfg.fuzzy.an_ang);
    }

    #[test]
    fn startup_config_survives_missing_or_broken_files() {
        let dir = std::env::temp_dir().join(format!("ior-cfg-test-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("mkdir");
        let path = dir.join("config.toml");
        // missing file -> defaults
        assert!(read_user_config(&path).is_none());
        // broken TOML -> None, no panic
        fs::write(&path, "not toml at all\n").expect("write");
        assert!(read_user_config(&path).is_none());
        // unknown key -> warning but a usable config
        fs::write(&path, "[engine]\nunknown_knob = 1\n").expect("write");
        assert!(read_user_config(&path).is_some());
        fs::remove_dir_all(&dir).ok();
    }
}
