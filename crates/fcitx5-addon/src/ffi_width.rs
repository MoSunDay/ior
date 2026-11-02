//! Full-width (全角) mode C ABI (additive, still ABI v2).
//!
//! - `ior_engine_full_width` reports 0 = half-width (default), 1 =
//!   full-width.
//! - `ior_engine_set_full_width` sets the flag and returns the new
//!   state; unlike the Shift transition it never touches a pending
//!   preedit (width only affects future keys).
//! - In full-width mode idle ASCII keys commit full-width forms:
//!   Chinese mode maps digits and space (１２３, U+3000) and keeps
//!   pinyin/Chinese punctuation untouched; English mode maps every
//!   printable ASCII key. See `engine_core::width`.

use crate::engine_state::IorEngine;
use engine_core::width;

/// Current full-width state (0 = half, 1 = full; 0 for a null engine).
///
/// # Safety
///
/// `engine` must be null or a live pointer from `ior_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn ior_engine_full_width(engine: *const IorEngine) -> u32 {
    let Some(engine) = engine.as_ref() else {
        return 0;
    };
    u32::from(engine.session.full_width)
}

/// Sets full-width mode (any non-zero `enable` turns it on); returns the
/// new state. A null engine returns 0.
///
/// # Safety
///
/// `engine` must be null or a live pointer from `ior_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn ior_engine_set_full_width(engine: *mut IorEngine, enable: u32) -> u32 {
    let Some(engine) = engine.as_mut() else {
        return 0;
    };
    u32::from(width::set_full_width(&mut engine.session, enable != 0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine_state;
    use std::os::raw::c_char;

    fn take(engine: *mut IorEngine) -> String {
        unsafe {
            let p = crate::ffi::ior_engine_take_commit(engine) as *const c_char;
            std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned()
        }
    }

    #[test]
    fn set_full_width_round_trips_and_drives_key_mapping() {
        let mut engine = engine_state::new_engine();
        let raw = &mut engine as *mut IorEngine;
        unsafe {
            assert_eq!(ior_engine_full_width(raw), 0); // default half-width
            assert_eq!(ior_engine_set_full_width(raw, 1), 1);
            assert_eq!(ior_engine_full_width(raw), 1);
            // English mode + full width: letters commit full-width.
            assert_eq!(crate::ffi::ior_engine_set_ascii_mode(raw, 1), 1);
            assert_eq!(crate::ffi::ior_engine_process_key(raw, 0x61), 2); // 'a'
            assert_eq!(take(raw), "\u{ff41}"); // ａ
                                               // Chinese mode + full width: idle digits commit full-width.
            assert_eq!(crate::ffi::ior_engine_set_ascii_mode(raw, 0), 0);
            assert_eq!(crate::ffi::ior_engine_process_key(raw, 0x39), 2); // '9'
            assert_eq!(take(raw), "\u{ff19}"); // ９
            assert_eq!(ior_engine_set_full_width(raw, 0), 0);
            assert_eq!(crate::ffi::ior_engine_process_key(raw, 0x39), 0); // forwarded
        }
    }
}
