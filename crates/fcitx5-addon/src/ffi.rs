//! C ABI bridge between the fcitx5 C++ shell and engine-core.
//!
//! # Symbols
//!
//! All exported symbols are prefixed `ior_` and follow the contract declared
//! in `cpp/ioraddon.h` (the C header mirrors this file; keep both in sync):
//!
//! ```c
//! typedef struct IorEngineC IorEngineC; // opaque
//! IorEngineC* ior_engine_new(void);
//! void        ior_engine_free(IorEngineC*);
//! void        ior_engine_reset(IorEngineC*);
//! int32_t     ior_engine_process_key(IorEngineC*, uint32_t keysym);
//! const char* ior_engine_preedit(const IorEngineC*);
//! uint32_t    ior_engine_candidate_count(const IorEngineC*);
//! const char* ior_engine_candidate(const IorEngineC*, uint32_t index);
//! const char* ior_engine_take_commit(IorEngineC*);
//! int32_t     ior_engine_page_next(IorEngineC*);        // ABI v2
//! int32_t     ior_engine_page_prev(IorEngineC*);        // ABI v2
//! uint32_t    ior_engine_page_info(const IorEngineC*);  // ABI v2
//! int32_t     ior_engine_page_key_direction(const IorEngineC*, uint32_t keysym); // v2
//! const char* ior_engine_select(IorEngineC*, uint32_t page_local_index); // v2
//! uint32_t    ior_engine_ascii_mode(const IorEngineC*); // ABI v2
//! uint32_t    ior_engine_set_ascii_mode(IorEngineC*, uint32_t enable);   // v2
//! int32_t     ior_engine_userdb_save(IorEngineC*, const char* path);     // v2
//! int32_t     ior_engine_userdb_load(IorEngineC*, const char* path);     // v2
//! int32_t     ior_engine_userdb_decay(IorEngineC*, uint32_t elapsed_secs,
//!                                     uint32_t half_life_secs);          // v2
//! uint32_t   ior_engine_abi_version(void);
//! ```
//!
//! 19 exported symbols in total, plus the 10 `ior_engine_voice_*`
//! symbols in [`crate::ffi_voice`], the 2 full-width symbols in
//! [`crate::ffi_width`] and the 1 deletion symbol in
//! [`crate::ffi_userdict`] (32 overall; ABI stays at version 2,
//! additive-only).
//!
//! # Contract
//!
//! - `ior_engine_new` returns null only on allocation failure; every other
//!   function accepts null (process_key returns -1, preedit/take_commit/
//!   select return "", candidate_count/page_info/ascii_mode return 0,
//!   page_next/page_prev/page_key_direction return 0, candidate returns
//!   null).
//! - `ior_engine_process_key` takes an X11 keysym and returns `0` = forward
//!   to the application, `1` = consumed while editing, `2` = committed (the
//!   host should then call `ior_engine_take_commit`).
//! - `ior_engine_candidate*` expose the **current page** only (at most
//!   `page_size` entries); paging is cyclic through `ior_engine_page_next` /
//!   `ior_engine_page_prev`. `ior_engine_page_info` packs
//!   `(current_page + 1) << 16 | page_count` (0 when there are no
//!   candidates).
//! - `ior_engine_select` commits the candidate at a **page-local** index of
//!   the current page (candidate clicks: what is displayed is what is
//!   committed, regardless of which page is shown) and returns the
//!   committed text ("" for an invalid index); the pending commit is set
//!   and the composition is reset. `ior_engine_candidate_delete` (in
//!   [`crate::ffi_userdict`]) removes the learned entry behind the same
//!   page-local index instead of committing, keeping the composition.
//! - `ior_engine_ascii_mode` / `ior_engine_set_ascii_mode` report/force the
//!   Chinese(0)/English(1) mode. Changing the mode routes through the core
//!   Shift transition, so a pending preedit is handled per `shift_action`;
//!   re-setting the current mode is a no-op. Returns the new mode.
//! - Returned `const char*` values are UTF-8, NUL-terminated, never null,
//!   and valid **until the next `ior_*` call on the same engine**. The caller
//!   must never free them and must not retain them across calls.
//! - A pending commit stays available until taken; `take_commit` consumes it
//!   (a second call returns "" until a new commit happens).
//! - The engine is **single-threaded**: fcitx5 drives every input context
//!   from its main loop, so no locking is needed (or provided). Calling
//!   into one engine from two threads concurrently is undefined behavior.
//! - Keysym coverage: BackSpace, Escape, Return/KP_Enter, space, printable
//!   ASCII `0x21..=0x7E`, keypad digits, `Page_Up`/`Page_Down` (paging) and
//!   `Shift_L`/`Shift_R` (mode toggle, **triggered on press**; hosts that
//!   switch modes on Shift release must not also forward Shift here —
//!   pick one of the two mechanisms). Everything else is forwarded.
//! - `ior_engine_userdb_save/load/decay` persist and age the learned
//!   dictionary (pure v2 additions, no ABI bump). `save` rewrites `path` as
//!   a compacted snapshot: 0 = written, 2 = I/O error, -1 = null engine or
//!   null path. `load` sniffs the file format (snapshot header / append-only
//!   `L\t` log) and only **merges** it into the user dictionary — preedit,
//!   commit buffer and mode are untouched: 0 = merged, 1 = file does not
//!   exist (non-fatal cold start, the engine keeps its state), 2 = I/O
//!   error, 3 = format error, -1 = null. `decay` ages frequencies by whole
//!   halvings of `half_life_secs` (`0` is legal and changes nothing):
//!   0 = ok, -1 = null. Paths are NUL-terminated; on unix the bytes are
//!   passed to the OS verbatim.
//!
//! Pointer-taking functions are `unsafe extern "C"` because a non-null but
//! dangling or aliased engine pointer is undefined behavior from the Rust
//! side; passing null itself is always allowed (see the `# Safety` sections).
//!
//! # Unsafe audit
//!
//! Raw-pointer handling is confined to `as_ref`/`as_mut` (null-checked
//! borrows), `ior_engine_free` (`Box::from_raw` on a pointer produced by
//! `Box::into_raw` in `ior_engine_new`) and `c_path` (`CStr::from_ptr` on
//! caller-owned NUL-terminated path strings). Nothing else dereferences C
//! memory.

use std::ffi::{c_char, CStr};
use std::path::PathBuf;

use crate::engine_state::{self, IorEngine};

/// Permanently valid empty C string, returned for idle states and null
/// engines so callers never have to null-check text pointers.
static EMPTY_NUL: &[u8] = b"\0";

/// X11 keysyms for the editing keys we translate.
const XK_BACKSPACE: u32 = 0xFF08;
const XK_ESCAPE: u32 = 0xFF1B;
const XK_RETURN: u32 = 0xFF0D;
const XK_KP_ENTER: u32 = 0xFF8D;
const XK_KP_0: u32 = 0xFFB0;
const XK_KP_1: u32 = 0xFFB1;
const XK_KP_9: u32 = 0xFFB9;
const XK_PAGE_UP: u32 = 0xFF55;
const XK_PAGE_DOWN: u32 = 0xFF56;
const XK_SHIFT_L: u32 = 0xFFE1;
const XK_SHIFT_R: u32 = 0xFFE2;

/// Translates an X11 keysym to the character engine-core understands.
/// `None` means "not for us", which the caller reports as a forward.
pub fn keysym_to_char(keysym: u32) -> Option<char> {
    match keysym {
        XK_BACKSPACE => Some('\u{8}'),
        XK_ESCAPE => Some('\u{1b}'),
        XK_RETURN | XK_KP_ENTER => Some('\n'),
        0x20 => Some(' '),
        // Printable ASCII maps to itself (letters, digits, punctuation).
        0x21..=0x7E => Some((keysym as u8) as char),
        // Keypad digits KP_1..KP_9, KP_0 map to their ASCII digits.
        XK_KP_1..=XK_KP_9 => Some((keysym - XK_KP_1 + u32::from(b'1')) as u8 as char),
        XK_KP_0 => Some('0'),
        _ => None,
    }
}

/// Null-checked immutable borrow of an engine pointer.
fn as_ref<'a>(ptr: *const IorEngine) -> Option<&'a IorEngine> {
    if ptr.is_null() {
        None
    } else {
        // SAFETY: callers pass either null or a pointer from ior_engine_new
        // that has not been freed; both sides of that contract live here.
        Some(unsafe { &*ptr })
    }
}

/// Null-checked mutable borrow of an engine pointer.
fn as_mut<'a>(ptr: *mut IorEngine) -> Option<&'a mut IorEngine> {
    if ptr.is_null() {
        None
    } else {
        // SAFETY: same contract as as_ref, plus exclusive access, which the
        // single-threaded fcitx5 main loop guarantees.
        Some(unsafe { &mut *ptr })
    }
}

/// Pointer to the shared empty string.
fn empty_ptr() -> *const c_char {
    EMPTY_NUL.as_ptr().cast()
}

/// Null-checked C string -> path. On unix the raw bytes are preserved
/// verbatim (paths need not be UTF-8); elsewhere a lossy UTF-8 conversion
/// is the best `std` offers without platform code.
fn c_path(ptr: *const c_char) -> Option<PathBuf> {
    if ptr.is_null() {
        return None;
    }
    // SAFETY: callers of the userdb functions pass a live NUL-terminated
    // string (or null, handled above); the borrow ends before any other
    // ior_* call can touch it.
    let bytes = unsafe { CStr::from_ptr(ptr) }.to_bytes();
    Some(bytes_to_path(bytes))
}

/// Converts C-string bytes into a path; the unix branch keeps the bytes
/// exact, the fallback lossily decodes them.
fn bytes_to_path(bytes: &[u8]) -> PathBuf {
    #[cfg(unix)]
    fn inner(bytes: &[u8]) -> PathBuf {
        use std::os::unix::ffi::OsStrExt;
        PathBuf::from(std::ffi::OsStr::from_bytes(bytes))
    }
    #[cfg(not(unix))]
    fn inner(bytes: &[u8]) -> PathBuf {
        PathBuf::from(String::from_utf8_lossy(bytes).into_owned())
    }
    inner(bytes)
}

/// ABI version of this C interface. Hosts may call it right after
/// `dlopen`/load to refuse mismatched shells. Bump on any breaking change.
/// v2 adds paging (`page_next`/`page_prev`/`page_info`/`select`), the mode
/// pair (`ascii_mode`/`set_ascii_mode`), page-scoped candidate exposure and
/// the Shift/Page keysyms; candidate exposure changed from "flat 10" to
/// "current page of `page_size`". The `userdb_save`/`userdb_load`/
/// `userdb_decay` trio is a purely additive v2 change (same version, three
/// new symbols).
#[no_mangle]
pub extern "C" fn ior_engine_abi_version() -> u32 {
    2
}

#[no_mangle]
pub extern "C" fn ior_engine_new() -> *mut IorEngine {
    Box::into_raw(Box::new(engine_state::new_engine()))
}

/// Destroys an engine. Null-safe; the pointer must not be used afterwards.
///
/// # Safety
///
/// `engine` must be null or a live pointer from `ior_engine_new` that has
/// not been passed to `ior_engine_free` yet. The pointer is consumed.
#[no_mangle]
pub unsafe extern "C" fn ior_engine_free(engine: *mut IorEngine) {
    if !engine.is_null() {
        // SAFETY: engine comes from Box::into_raw in ior_engine_new and is
        // consumed exactly once here (callers must not reuse the pointer).
        drop(unsafe { Box::from_raw(engine) });
    }
}

/// Clears the preedit buffer (focus loss, mode switch). Null-safe.
///
/// # Safety
///
/// `engine` must be null or a live pointer from `ior_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn ior_engine_reset(engine: *mut IorEngine) {
    if let Some(engine) = as_mut(engine) {
        engine_state::reset(engine);
    }
}

/// Feeds one X11 keysym to the engine. Returns 0 = forward to the app,
/// 1 = consumed (editing), 2 = committed (call ior_engine_take_commit),
/// -1 = null engine. Keysyms outside the mapped set return 0.
///
/// `Shift_L`/`Shift_R` toggle the mode on **press** (returned like any
/// consumed key, 1 or 2); `Page_Up`/`Page_Down` page the candidate list and
/// always report consumed while a composition is active.
///
/// # Safety
///
/// `engine` must be null or a live pointer from `ior_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn ior_engine_process_key(engine: *mut IorEngine, keysym: u32) -> i32 {
    let Some(engine) = as_mut(engine) else {
        return -1;
    };
    // Keysyms with engine semantics but no character translation.
    match keysym {
        XK_SHIFT_L | XK_SHIFT_R => return engine_state::process_shift(engine),
        XK_PAGE_UP => {
            engine_state::page_prev(engine);
            return 1;
        }
        XK_PAGE_DOWN => {
            engine_state::page_next(engine);
            return 1;
        }
        _ => {}
    }
    match keysym_to_char(keysym) {
        Some(ch) => engine_state::process_key(engine, ch),
        None => 0,
    }
}

/// Current preedit for display (UTF-8, "" when idle, never null). Valid
/// until the next ior_* call on this engine.
///
/// # Safety
///
/// `engine` must be null or a live pointer from `ior_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn ior_engine_preedit(engine: *const IorEngine) -> *const c_char {
    match as_ref(engine) {
        Some(engine) => engine.preedit_buf.as_ptr(),
        None => empty_ptr(),
    }
}

/// Number of candidates on the **current page** (at most the configured
/// page size, 5 by default; use `ior_engine_page_next`/`prev` to move).
///
/// # Safety
///
/// `engine` must be null or a live pointer from `ior_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn ior_engine_candidate_count(engine: *const IorEngine) -> u32 {
    as_ref(engine).map_or(0, |engine| engine.candidates_buf.len() as u32)
}

/// Candidate text at `index`, or null when out of range. Valid until the
/// next ior_* call on this engine.
///
/// # Safety
///
/// `engine` must be null or a live pointer from `ior_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn ior_engine_candidate(
    engine: *const IorEngine,
    index: u32,
) -> *const c_char {
    as_ref(engine)
        .and_then(|engine| engine.candidates_buf.get(index as usize))
        .map_or_else(std::ptr::null, |text| text.as_ptr())
}

/// Takes the pending commit text ("" if none). The pending commit is
/// consumed: a second call returns "" until a new commit happens. Valid
/// until the next ior_* call on this engine.
///
/// # Safety
///
/// `engine` must be null or a live pointer from `ior_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn ior_engine_take_commit(engine: *mut IorEngine) -> *const c_char {
    let engine = match as_mut(engine) {
        Some(engine) => engine,
        None => return empty_ptr(),
    };
    if engine_state::take_commit(engine) {
        engine.commit_buf.as_ptr()
    } else {
        empty_ptr()
    }
}

/// Advances one candidate page (wrapping from the last page to the first).
/// Returns 1 when the page changed, 0 when it did not (single page, no
/// candidates) or the engine is null. Candidate pointers handed out before
/// this call are invalidated, so the host should re-read the list.
///
/// # Safety
///
/// `engine` must be null or a live pointer from `ior_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn ior_engine_page_next(engine: *mut IorEngine) -> i32 {
    as_mut(engine).map_or(0, |engine| engine_state::page_next(engine) as i32)
}

/// Goes back one candidate page (wrapping from the first page to the
/// last). Returns 1 when the page changed, otherwise 0 (same rules as
/// `ior_engine_page_next`).
///
/// # Safety
///
/// `engine` must be null or a live pointer from `ior_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn ior_engine_page_prev(engine: *mut IorEngine) -> i32 {
    as_mut(engine).map_or(0, |engine| engine_state::page_prev(engine) as i32)
}

/// Paging snapshot: `(current_page + 1) << 16 | page_count`, i.e. a 1-based
/// page number in the high half and the total page count in the low half.
/// Returns 0 when there are no candidates (or the engine is null).
///
/// # Safety
///
/// `engine` must be null or a live pointer from `ior_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn ior_engine_page_info(engine: *const IorEngine) -> u32 {
    as_ref(engine).map_or(0, engine_state::page_info)
}

/// -1 = configured prev-page key, 1 = next-page key, 0 = neither
/// (also for a null engine). Pure query: no state change.
///
/// # Safety
///
/// `engine` must be null or a live pointer from `ior_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn ior_engine_page_key_direction(
    engine: *const IorEngine,
    keysym: u32,
) -> i32 {
    as_ref(engine).map_or(0, |engine| engine_state::page_key_direction(engine, keysym))
}

/// Commits the candidate at `page_local_index` of the **current page**
/// (candidate clicks: what the panel shows is what gets committed, on every
/// page). Returns the committed text (UTF-8, "" for an invalid index or a
/// null engine); the text is also pending in `ior_engine_take_commit` and
/// the composition is reset, exactly like a digit selection.
///
/// # Safety
///
/// `engine` must be null or a live pointer from `ior_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn ior_engine_select(
    engine: *mut IorEngine,
    page_local_index: u32,
) -> *const c_char {
    let Some(engine) = as_mut(engine) else {
        return empty_ptr();
    };
    if engine_state::select_page_local(engine, page_local_index) {
        engine.commit_buf.as_ptr()
    } else {
        empty_ptr()
    }
}

/// Current mode: 0 = Chinese, 1 = English (ASCII). Null engine reads as 0.
///
/// # Safety
///
/// `engine` must be null or a live pointer from `ior_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn ior_engine_ascii_mode(engine: *const IorEngine) -> u32 {
    as_ref(engine).map_or(0, |engine| engine_state::ascii_mode(engine) as u32)
}

/// Forces the mode (0 = Chinese, 1 = English); returns the resulting mode.
/// A change runs the core Shift transition, so a pending preedit is
/// committed/cleared per `shift_action` (the commit text lands in the
/// pending-commit buffer). Setting the current mode is a no-op.
///
/// # Safety
///
/// `engine` must be null or a live pointer from `ior_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn ior_engine_set_ascii_mode(engine: *mut IorEngine, enable: u32) -> u32 {
    as_mut(engine).map_or(0, |engine| {
        engine_state::set_ascii_mode(engine, enable != 0) as u32
    })
}

/// Saves the learned user dictionary to `path` (NUL-terminated) as a
/// deterministic snapshot — saving is compaction, the file is rewritten
/// wholesale. Returns 0 = written, 2 = I/O error (unwritable directory,
/// permissions, ...), -1 = null engine or null path.
///
/// # Safety
///
/// `engine` must be null or a live pointer from `ior_engine_new`;
/// `path` must be null or a live NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn ior_engine_userdb_save(
    engine: *mut IorEngine,
    path: *const c_char,
) -> i32 {
    let (Some(engine), Some(path)) = (as_ref(engine), c_path(path)) else {
        return -1;
    };
    engine_state::save_userdb(engine, &path)
}

/// Loads a userdb file and **merges** it into the learned dictionary
/// (frequencies add up). The format is sniffed: a `# ior userdb` header
/// means snapshot, `L\t` lines mean append-only log. Preedit, pending
/// commit and mode are untouched. Returns 0 = merged, 1 = file does not
/// exist (non-fatal: a fresh install simply starts empty), 2 = I/O error,
/// 3 = format error, -1 = null engine or null path.
///
/// # Safety
///
/// `engine` must be null or a live pointer from `ior_engine_new`;
/// `path` must be null or a live NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn ior_engine_userdb_load(
    engine: *mut IorEngine,
    path: *const c_char,
) -> i32 {
    let (Some(engine), Some(path)) = (as_mut(engine), c_path(path)) else {
        return -1;
    };
    engine_state::load_userdb(engine, &path)
}

/// Ages the learned frequencies by whole halvings of `half_life_secs`
/// (`elapsed_secs / half_life_secs` right-shifts; entries reaching zero are
/// dropped). `half_life_secs == 0` is legal and defined as "no decay".
/// Returns 0 = applied, -1 = null engine.
///
/// # Safety
///
/// `engine` must be null or a live pointer from `ior_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn ior_engine_userdb_decay(
    engine: *mut IorEngine,
    elapsed_secs: u32,
    half_life_secs: u32,
) -> i32 {
    let Some(engine) = as_mut(engine) else {
        return -1;
    };
    engine_state::decay_userdb(engine, elapsed_secs, half_life_secs)
}

/// Test shim: `ior_engine_abi_version` is safe to call directly.
#[cfg(test)]
fn unsafe_or_safe_abi() -> u32 {
    ior_engine_abi_version()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::{CStr, CString};

    /// Reads a live engine-owned C string. Tests keep to the documented
    /// contract: the pointer is used before any other ior_* call.
    fn text(ptr: *const c_char) -> String {
        assert!(!ptr.is_null());
        // SAFETY: pointers come from this module's CString buffers (or the
        // static EMPTY_NUL) and are read immediately after being returned.
        unsafe { CStr::from_ptr(ptr) }
            .to_string_lossy()
            .into_owned()
    }

    fn type_keys(engine: *mut IorEngine, keysyms: &[u32]) {
        for ks in keysyms {
            // SAFETY: engine is live for the whole test.
            assert_eq!(unsafe { ior_engine_process_key(engine, *ks) }, 1);
        }
    }

    /// Reads candidate `index` of the current page.
    fn candidate_text(engine: *mut IorEngine, index: u32) -> String {
        // SAFETY: engine is live; the pointer is read before the next call.
        text(unsafe { ior_engine_candidate(engine, index) })
    }

    const NIHAO: [u32; 5] = [0x6E, 0x69, 0x68, 0x61, 0x6F]; // n i h a o

    /// Host-independent engine: builtin dictionary + default config, so
    /// installing a real `core.ior` cannot change FFI test candidates.
    fn test_engine() -> *mut IorEngine {
        let engine = engine_state::new_engine_with(
            engine_core::config::EngineConfig::default(),
            engine_core::dict::Dictionary::builtin(),
        );
        Box::into_raw(Box::new(engine))
    }

    #[test]
    fn abi_version_is_two() {
        assert_eq!(unsafe_or_safe_abi(), 2);
    }

    #[test]
    fn full_flow_nihao() {
        // SAFETY: engine stays live until ior_engine_free below.
        let engine = test_engine();
        assert!(!engine.is_null());
        assert_eq!(text(unsafe { ior_engine_preedit(engine) }), "");

        type_keys(engine, &NIHAO);
        assert_eq!(text(unsafe { ior_engine_preedit(engine) }), "ni'hao");
        let count = unsafe { ior_engine_candidate_count(engine) };
        assert!(count >= 1, "need at least one candidate, got {count}");
        assert!(count <= 10);
        assert_eq!(text(unsafe { ior_engine_candidate(engine, 0) }), "你好");

        assert_eq!(unsafe { ior_engine_process_key(engine, 0x20) }, 2);
        assert_eq!(text(unsafe { ior_engine_take_commit(engine) }), "你好");
        assert_eq!(text(unsafe { ior_engine_preedit(engine) }), "");
        assert_eq!(text(unsafe { ior_engine_take_commit(engine) }), "");
        assert_eq!(unsafe { ior_engine_candidate_count(engine) }, 0);

        unsafe { ior_engine_free(engine) };
    }

    #[test]
    fn partial_input_exposes_prefix_candidates() {
        // SAFETY: each engine stays live until its own free call below.
        let engine = test_engine();
        type_keys(engine, &[0x6E]); // n
        assert_eq!(text(unsafe { ior_engine_preedit(engine) }), "n");
        assert!(
            unsafe { ior_engine_candidate_count(engine) } >= 1,
            "bare initial should have candidates"
        );
        unsafe { ior_engine_free(engine) };

        let engine = test_engine();
        type_keys(engine, &[0x6E, 0x69, 0x68]); // n i h
        assert_eq!(text(unsafe { ior_engine_preedit(engine) }), "nih");
        assert!(unsafe { ior_engine_candidate_count(engine) } >= 1);
        assert_eq!(candidate_text(engine, 0), "你好");
        unsafe { ior_engine_free(engine) };
    }

    #[test]
    fn digit_select_commits() {
        // SAFETY: engine stays live until ior_engine_free below.
        let engine = test_engine();
        type_keys(engine, &[0x6E, 0x69]); // n i
        let count = unsafe { ior_engine_candidate_count(engine) };
        assert!(count >= 2, "need a second candidate, got {count}");
        assert_eq!(unsafe { ior_engine_process_key(engine, 0x32) }, 2); // '2'
        let commit = text(unsafe { ior_engine_take_commit(engine) });
        assert!(!commit.is_empty());
        // Decodes as UTF-8 without loss (no replacement characters).
        assert!(!commit.contains(char::REPLACEMENT_CHARACTER));
        unsafe { ior_engine_free(engine) };
    }

    #[test]
    fn backspace_and_escape_edit() {
        // SAFETY: engine stays live until ior_engine_free below.
        let engine = test_engine();
        type_keys(engine, &NIHAO);
        assert_eq!(unsafe { ior_engine_process_key(engine, 0xFF08) }, 1);
        let after_bs = text(unsafe { ior_engine_preedit(engine) });
        assert!(after_bs.len() < "ni'hao".len());
        assert!(after_bs.starts_with("ni'ha"));

        assert_eq!(unsafe { ior_engine_process_key(engine, 0xFF1B) }, 1);
        assert_eq!(text(unsafe { ior_engine_preedit(engine) }), "");
        unsafe { ior_engine_free(engine) };
    }

    #[test]
    fn forward_and_unknown_keysyms() {
        // SAFETY: engine stays live until ior_engine_free below.
        let engine = test_engine();
        // Uppercase 'A' is outside the engine's lowercase editing alphabet.
        assert_eq!(unsafe { ior_engine_process_key(engine, 0x41) }, 0);
        // Left arrow is not mapped at all.
        assert_eq!(unsafe { ior_engine_process_key(engine, 0xFF51) }, 0);
        type_keys(engine, &NIHAO);
        assert_eq!(unsafe { ior_engine_process_key(engine, 0xFF51) }, 0);
        unsafe { ior_engine_free(engine) };
    }

    #[test]
    fn shift_keysym_toggles_mode_and_commits_raw() {
        // SAFETY: engine stays live until ior_engine_free below.
        let engine = test_engine();
        type_keys(engine, &NIHAO);
        assert_eq!(unsafe { ior_engine_ascii_mode(engine) }, 0);
        // Shift press triggers the toggle; the pending preedit is committed
        // raw (default shift_action) in the same stroke.
        assert_eq!(unsafe { ior_engine_process_key(engine, XK_SHIFT_L) }, 2);
        assert_eq!(text(unsafe { ior_engine_take_commit(engine) }), "nihao");
        assert_eq!(unsafe { ior_engine_ascii_mode(engine) }, 1);
        assert_eq!(text(unsafe { ior_engine_preedit(engine) }), "");
        // English mode: letters and punctuation are forwarded.
        assert_eq!(unsafe { ior_engine_process_key(engine, 0x6E) }, 0);
        assert_eq!(unsafe { ior_engine_process_key(engine, 0x2C) }, 0);
        // Shift again: back to Chinese, plain editing.
        assert_eq!(unsafe { ior_engine_process_key(engine, XK_SHIFT_R) }, 1);
        assert_eq!(unsafe { ior_engine_ascii_mode(engine) }, 0);
        unsafe { ior_engine_free(engine) };
    }

    #[test]
    fn page_keysyms_and_page_info() {
        // SAFETY: engine stays live until ior_engine_free below.
        let engine = test_engine();
        // No candidates: page_info is 0 and paging is a no-op.
        assert_eq!(unsafe { ior_engine_page_info(engine) }, 0);
        assert_eq!(unsafe { ior_engine_page_next(engine) }, 0);
        type_keys(engine, &[0x73, 0x68, 0x69]); // s h i (4 candidates)
        assert_eq!(
            unsafe { ior_engine_page_info(engine) },
            (1 << 16) | 1,
            "one page of four candidates"
        );
        assert_eq!(unsafe { ior_engine_candidate_count(engine) }, 4);
        // Single page: paging keys are consumed but change nothing.
        assert_eq!(unsafe { ior_engine_process_key(engine, XK_PAGE_DOWN) }, 1);
        assert_eq!(unsafe { ior_engine_page_info(engine) }, (1 << 16) | 1);
        assert_eq!(unsafe { ior_engine_process_key(engine, XK_PAGE_UP) }, 1);
        assert_eq!(unsafe { ior_engine_page_info(engine) }, (1 << 16) | 1);
        unsafe { ior_engine_free(engine) };
    }

    #[test]
    fn select_commits_page_local_index() {
        // SAFETY: engine stays live until ior_engine_free below.
        let engine = test_engine();
        type_keys(engine, &[0x73, 0x68, 0x69]); // s h i
                                                // Capture the expected text before selecting: the select call
                                                // resets the composition and with it the candidate list.
        let expected = candidate_text(engine, 1);
        assert_eq!(
            text(unsafe { ior_engine_select(engine, 1) }),
            expected,
            "select must commit the displayed candidate"
        );
        assert_eq!(text(unsafe { ior_engine_preedit(engine) }), "");
        // Page-local indices beyond the page size are invalid: empty text,
        // composition kept.
        type_keys(engine, &[0x73, 0x68, 0x69]);
        assert_eq!(text(unsafe { ior_engine_select(engine, 9) }), "");
        assert_eq!(text(unsafe { ior_engine_preedit(engine) }), "shi");
        unsafe { ior_engine_free(engine) };
    }

    #[test]
    fn set_ascii_mode_round_trip() {
        // SAFETY: engine stays live until ior_engine_free below.
        let engine = test_engine();
        type_keys(engine, &[0x6E, 0x69]); // n i
        assert_eq!(unsafe { ior_engine_set_ascii_mode(engine, 1) }, 1);
        assert_eq!(text(unsafe { ior_engine_take_commit(engine) }), "ni");
        // Re-setting the current mode is a no-op (no phantom commit).
        assert_eq!(unsafe { ior_engine_set_ascii_mode(engine, 1) }, 1);
        assert_eq!(text(unsafe { ior_engine_take_commit(engine) }), "");
        assert_eq!(unsafe { ior_engine_set_ascii_mode(engine, 0) }, 0);
        assert_eq!(unsafe { ior_engine_ascii_mode(engine) }, 0);
        unsafe { ior_engine_free(engine) };
    }

    #[test]
    fn keypad_digits_map_to_ascii() {
        assert_eq!(keysym_to_char(0xFFB1), Some('1'));
        assert_eq!(keysym_to_char(0xFFB9), Some('9'));
        assert_eq!(keysym_to_char(0xFFB0), Some('0'));
        assert_eq!(keysym_to_char(0xFFBA), None); // KP_Decimal
        assert_eq!(keysym_to_char(0xFF0D), Some('\n'));
        assert_eq!(keysym_to_char(0xFF8D), Some('\n'));
        assert_eq!(keysym_to_char(0x30), Some('0'));
        assert_eq!(keysym_to_char(0x41), Some('A'));
        assert_eq!(keysym_to_char(0xFF52), None); // Up
    }

    #[test]
    fn null_engine_is_safe() {
        // SAFETY: every function documents null tolerance.
        assert_eq!(
            unsafe { ior_engine_process_key(std::ptr::null_mut(), 0x6E) },
            -1
        );
        assert_eq!(text(unsafe { ior_engine_preedit(std::ptr::null()) }), "");
        assert_eq!(unsafe { ior_engine_candidate_count(std::ptr::null()) }, 0);
        assert!(unsafe { ior_engine_candidate(std::ptr::null(), 0) }.is_null());
        assert_eq!(
            text(unsafe { ior_engine_take_commit(std::ptr::null_mut()) }),
            ""
        );
        assert_eq!(unsafe { ior_engine_page_next(std::ptr::null_mut()) }, 0);
        assert_eq!(unsafe { ior_engine_page_prev(std::ptr::null_mut()) }, 0);
        assert_eq!(unsafe { ior_engine_page_info(std::ptr::null()) }, 0);
        assert_eq!(
            unsafe { ior_engine_page_key_direction(std::ptr::null(), 0xFF55) },
            0
        );
        assert_eq!(
            text(unsafe { ior_engine_select(std::ptr::null_mut(), 0) }),
            ""
        );
        assert_eq!(unsafe { ior_engine_ascii_mode(std::ptr::null()) }, 0);
        assert_eq!(
            unsafe { ior_engine_set_ascii_mode(std::ptr::null_mut(), 1) },
            0
        );
        let path = CString::new("/tmp/ior-null-path").expect("path");
        assert_eq!(
            unsafe { ior_engine_userdb_save(std::ptr::null_mut(), path.as_ptr()) },
            -1
        );
        assert_eq!(
            unsafe { ior_engine_userdb_load(std::ptr::null_mut(), path.as_ptr()) },
            -1
        );
        assert_eq!(
            unsafe { ior_engine_userdb_decay(std::ptr::null_mut(), 1, 1) },
            -1
        );
        unsafe { ior_engine_reset(std::ptr::null_mut()) };
        unsafe { ior_engine_free(std::ptr::null_mut()) };
    }
}
