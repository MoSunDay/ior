//! Voice-input C ABI (additive to ABI v2). Mirrors the declarations in
//! `cpp/ioraddon.h`; see that header and [`crate::ffi`] for the shared
//! contract (null tolerance, string lifetimes, single-thread use).
//!
//! ```c
//! uint32_t ior_engine_voice_hotkey_keysym(const IorEngineC*);
//! uint32_t ior_engine_voice_hotkey_mods(const IorEngineC*);
//! uint32_t ior_engine_voice_hotkey_mode(const IorEngineC*);
//! const char* ior_engine_voice_hotkey_name(const IorEngineC*);
//! uint32_t ior_engine_voice_state(const IorEngineC*);
//! uint32_t ior_engine_voice_toggle(IorEngineC*);
//! uint32_t ior_engine_voice_stop(IorEngineC*);
//! uint32_t ior_engine_voice_cancel(IorEngineC*);
//! uint32_t ior_engine_voice_poll(IorEngineC*);
//! const char* ior_engine_voice_take_text(IorEngineC*);
//! ```
//!
//! `hotkey_*` return the parsed `voice.hotkey` (`(0, 0)` keysym/mods when
//! voice input is disabled — a keysym of 0 never matches a real key).
//! State codes: 0 idle, 1 recording, 2 transcribing. `toggle` is the
//! hotkey press (idle -> recording, active -> cancel), `stop` the
//! `hold`-mode release (recording -> transcribing). `poll` returns 1
//! exactly once per finished transcript, then `take_text` hands it out
//! (UTF-8, NUL-terminated, valid until the next `ior_*` call).

use std::ffi::c_char;

use crate::engine_state::IorEngine;
use crate::voice_state::{self, VoiceRuntime};

/// Permanently valid empty C string (null engines, no transcript).
static EMPTY_NUL: &[u8] = b"\0";

fn as_ref<'a>(ptr: *const IorEngine) -> Option<&'a IorEngine> {
    if ptr.is_null() {
        None
    } else {
        // SAFETY: callers pass null or a live pointer from ior_engine_new,
        // same contract as crate::ffi.
        Some(unsafe { &*ptr })
    }
}

fn as_mut<'a>(ptr: *mut IorEngine) -> Option<&'a mut IorEngine> {
    if ptr.is_null() {
        None
    } else {
        // SAFETY: same as as_ref plus exclusive access, guaranteed by the
        // single-threaded fcitx5 main loop.
        Some(unsafe { &mut *ptr })
    }
}

fn empty_ptr() -> *const c_char {
    EMPTY_NUL.as_ptr().cast()
}

/// # Safety
///
/// `engine` must be null or a live pointer from `ior_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn ior_engine_voice_hotkey_keysym(engine: *const IorEngine) -> u32 {
    as_ref(engine).map_or(0, |e| voice_state::hotkey(&e.voice).0)
}

/// # Safety
///
/// `engine` must be null or a live pointer from `ior_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn ior_engine_voice_hotkey_mods(engine: *const IorEngine) -> u32 {
    as_ref(engine).map_or(0, |e| voice_state::hotkey(&e.voice).1)
}

/// Hotkey mode: 0 = toggle, 1 = hold (0 when disabled or null).
///
/// # Safety
///
/// `engine` must be null or a live pointer from `ior_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn ior_engine_voice_hotkey_mode(engine: *const IorEngine) -> u32 {
    as_ref(engine).map_or(0, |e| u32::from(e.voice.settings.hold_mode))
}

/// Configured hotkey as text (e.g. `ctrl+\``) for UI hints.
///
/// # Safety
///
/// `engine` must be null or a live pointer from `ior_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn ior_engine_voice_hotkey_name(engine: *const IorEngine) -> *const c_char {
    match as_ref(engine) {
        Some(engine) if engine.voice.settings.enabled => store_hotkey_label(&engine.voice),
        _ => empty_ptr(),
    }
}

/// The label lives in the runtime; this helper only reborrows it as a C
/// pointer (valid until the next `ior_*` call, per the crate contract).
fn store_hotkey_label(runtime: &VoiceRuntime) -> *const c_char {
    runtime.settings.hotkey_label.as_ptr().cast()
}

/// # Safety
///
/// `engine` must be null or a live pointer from `ior_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn ior_engine_voice_state(engine: *const IorEngine) -> u32 {
    as_ref(engine).map_or(0, |e| voice_state::phase(&e.voice))
}

/// Hotkey press; returns the new state (0 when null or disabled).
///
/// # Safety
///
/// `engine` must be null or a live pointer from `ior_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn ior_engine_voice_toggle(engine: *mut IorEngine) -> u32 {
    as_mut(engine).map_or(0, |e| voice_state::toggle(&mut e.voice))
}

/// `hold`-mode release: stop recording and transcribe.
///
/// # Safety
///
/// `engine` must be null or a live pointer from `ior_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn ior_engine_voice_stop(engine: *mut IorEngine) -> u32 {
    as_mut(engine).map_or(0, |e| voice_state::stop_and_transcribe(&mut e.voice))
}

/// Cancels recording/transcription; returns the new state.
///
/// # Safety
///
/// `engine` must be null or a live pointer from `ior_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn ior_engine_voice_cancel(engine: *mut IorEngine) -> u32 {
    as_mut(engine).map_or(0, |e| voice_state::cancel(&mut e.voice))
}

/// 1 when a transcript is ready (then call `take_text`), else 0.
///
/// # Safety
///
/// `engine` must be null or a live pointer from `ior_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn ior_engine_voice_poll(engine: *mut IorEngine) -> u32 {
    as_mut(engine).map_or(0, |e| voice_state::poll(&mut e.voice))
}

/// Takes the finished transcript ("" when none; never null).
///
/// # Safety
///
/// `engine` must be null or a live pointer from `ior_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn ior_engine_voice_take_text(engine: *mut IorEngine) -> *const c_char {
    match as_mut(engine) {
        Some(engine) => {
            let text = voice_state::take_text(&mut engine.voice);
            text.as_ptr().cast()
        }
        None => empty_ptr(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine_state;

    fn text(ptr: *const c_char) -> String {
        unsafe { std::ffi::CStr::from_ptr(ptr) }
            .to_string_lossy()
            .into_owned()
    }

    #[test]
    fn null_engine_is_safe() {
        // SAFETY: every function documents null tolerance.
        assert_eq!(
            unsafe { ior_engine_voice_hotkey_keysym(std::ptr::null()) },
            0
        );
        assert_eq!(unsafe { ior_engine_voice_hotkey_mods(std::ptr::null()) }, 0);
        assert_eq!(unsafe { ior_engine_voice_hotkey_mode(std::ptr::null()) }, 0);
        assert_eq!(
            text(unsafe { ior_engine_voice_hotkey_name(std::ptr::null()) }),
            ""
        );
        assert_eq!(unsafe { ior_engine_voice_state(std::ptr::null()) }, 0);
        assert_eq!(unsafe { ior_engine_voice_toggle(std::ptr::null_mut()) }, 0);
        assert_eq!(unsafe { ior_engine_voice_stop(std::ptr::null_mut()) }, 0);
        assert_eq!(unsafe { ior_engine_voice_cancel(std::ptr::null_mut()) }, 0);
        assert_eq!(unsafe { ior_engine_voice_poll(std::ptr::null_mut()) }, 0);
        assert_eq!(
            text(unsafe { ior_engine_voice_take_text(std::ptr::null_mut()) }),
            ""
        );
    }

    #[test]
    fn hotkey_uses_x11_grave_and_fcitx_ctrl_bit() {
        let engine = engine_state::new_engine();
        // SAFETY: engine comes from new_engine (same thread).
        assert_eq!(
            unsafe { ior_engine_voice_hotkey_keysym(&engine) },
            voice::hotkey::KEY_GRAVE
        );
        assert_eq!(
            unsafe { ior_engine_voice_hotkey_mods(&engine) },
            voice::hotkey::MOD_CTRL
        );
        assert_eq!(unsafe { ior_engine_voice_hotkey_mode(&engine) }, 0);
        assert_eq!(
            text(unsafe { ior_engine_voice_hotkey_name(&engine) }),
            "ctrl+`"
        );
    }

    #[test]
    fn toggle_starts_and_cancel_resets() {
        let mut engine = engine_state::new_engine();
        // SAFETY: engine comes from new_engine (same thread).
        let state = unsafe { ior_engine_voice_toggle(&mut engine) };
        assert_eq!(state, voice_state::STATE_RECORDING);
        let state = unsafe { ior_engine_voice_cancel(&mut engine) };
        assert_eq!(state, voice_state::STATE_IDLE);
        assert_eq!(unsafe { ior_engine_voice_poll(&mut engine) }, 0);
    }
}
