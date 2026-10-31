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
//! ```
//!
//! # Contract
//!
//! - `ior_engine_new` returns null only on allocation failure; every other
//!   function accepts null (process_key returns -1, preedit/take_commit
//!   return "", candidate_count returns 0, candidate returns null).
//! - `ior_engine_process_key` takes an X11 keysym and returns `0` = forward
//!   to the application, `1` = consumed while editing, `2` = committed (the
//!   host should then call `ior_engine_take_commit`).
//! - Returned `const char*` values are UTF-8, NUL-terminated, never null,
//!   and valid **until the next `ior_*` call on the same engine**. The caller
//!   must never free them and must not retain them across calls.
//! - A pending commit stays available until taken; `take_commit` consumes it
//!   (a second call returns "" until a new commit happens).
//! - The engine is **single-threaded**: fcitx5 drives every input context
//!   from its main loop, so no locking is needed (or provided). Calling
//!   into one engine from two threads concurrently is undefined behavior.
//! - Keysym coverage: BackSpace, Escape, Return/KP_Enter, space, printable
//!   ASCII `0x21..=0x7E`, and keypad digits. Everything else is forwarded.
//!
//! Pointer-taking functions are `unsafe extern "C"` because a non-null but
//! dangling or aliased engine pointer is undefined behavior from the Rust
//! side; passing null itself is always allowed (see the `# Safety` sections).
//!
//! # Unsafe audit
//!
//! Raw-pointer handling is confined to `as_ref`/`as_mut` (null-checked
//! borrows) and `ior_engine_free` (`Box::from_raw` on a pointer produced by
//! `Box::into_raw` in `ior_engine_new`). Nothing else dereferences C memory.

use std::ffi::c_char;

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

/// Creates an engine over the builtin dictionary with the default config.
/// Returns null only on allocation failure.
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
/// # Safety
///
/// `engine` must be null or a live pointer from `ior_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn ior_engine_process_key(engine: *mut IorEngine, keysym: u32) -> i32 {
    match (as_mut(engine), keysym_to_char(keysym)) {
        (Some(engine), Some(ch)) => engine_state::process_key(engine, ch),
        (Some(_), None) => 0,
        (None, _) => -1,
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

/// Number of candidates for the current preedit, `0..=10`.
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CStr;

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

    const NIHAO: [u32; 5] = [0x6E, 0x69, 0x68, 0x61, 0x6F]; // n i h a o

    #[test]
    fn full_flow_nihao() {
        // SAFETY: engine stays live until ior_engine_free below.
        let engine = ior_engine_new();
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
    fn digit_select_commits() {
        // SAFETY: engine stays live until ior_engine_free below.
        let engine = ior_engine_new();
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
        let engine = ior_engine_new();
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
        let engine = ior_engine_new();
        // Uppercase 'A' is outside the engine's lowercase editing alphabet.
        assert_eq!(unsafe { ior_engine_process_key(engine, 0x41) }, 0);
        // Left arrow is not mapped at all.
        assert_eq!(unsafe { ior_engine_process_key(engine, 0xFF51) }, 0);
        type_keys(engine, &NIHAO);
        assert_eq!(unsafe { ior_engine_process_key(engine, 0xFF51) }, 0);
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
        unsafe { ior_engine_reset(std::ptr::null_mut()) };
        unsafe { ior_engine_free(std::ptr::null_mut()) };
    }
}
