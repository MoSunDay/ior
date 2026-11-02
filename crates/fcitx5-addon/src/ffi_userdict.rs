//! User-dictionary deletion C ABI (additive, still ABI v2).
//!
//! - `ior_engine_candidate_delete(engine, page_local_index)` removes the
//!   learned entry behind the page-local candidate — the delete-on-click
//!   counterpart of `ior_engine_select`: 0 = deleted, 1 = the candidate
//!   backs no learned word (static dictionary hits are read-only),
//!   2 = index out of range, -1 = null engine.
//! - The composition is kept intact: the host refreshes the panel and
//!   the word simply loses its learned boost (a dictionary word with the
//!   same text stays available). The host persists the change with
//!   `ior_engine_userdb_save` right after a successful deletion.
//! - Deleting is dictionary management, not learning: it is not gated by
//!   `[engine] user_learning` (which in practice only hides learned
//!   candidates from the list in the first place).

use crate::engine_state::{self, IorEngine};

/// Deletes the learned entry behind the page-local candidate at
/// `page_local_index` of the current page. The pending commit and the
/// composition are untouched.
///
/// # Safety
///
/// `engine` must be null or a live pointer from `ior_engine_new`.
#[no_mangle]
pub unsafe extern "C" fn ior_engine_candidate_delete(
    engine: *mut IorEngine,
    page_local_index: u32,
) -> i32 {
    let Some(engine) = engine.as_mut() else {
        return -1;
    };
    engine_state::delete_page_local(engine, page_local_index)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine_state;
    use engine_core::config::EngineConfig;
    use engine_core::dict::Dictionary;
    use std::os::raw::c_char;

    fn text(ptr: *const c_char) -> String {
        assert!(!ptr.is_null());
        // SAFETY: pointers come from ffi's CString buffers (or the static
        // EMPTY_NUL) and are read immediately after being returned.
        unsafe { std::ffi::CStr::from_ptr(ptr) }
            .to_string_lossy()
            .into_owned()
    }

    fn new_raw() -> *mut IorEngine {
        let engine = engine_state::new_engine_with(EngineConfig::default(), Dictionary::builtin());
        Box::into_raw(Box::new(engine))
    }

    fn type_keys(engine: *mut IorEngine, keysyms: &[u32]) {
        for ks in keysyms {
            // SAFETY: engine is live for the whole test.
            assert_eq!(
                unsafe { crate::ffi::ior_engine_process_key(engine, *ks) },
                1
            );
        }
    }

    #[test]
    fn delete_round_trip_and_status_codes() {
        // SAFETY: engine stays live until ior_engine_free below.
        let engine = new_raw();
        // Learn 你 under `ni` with a digit commit.
        type_keys(engine, &[0x6E, 0x69]); // n i
                                          // SAFETY: live engine; page-local index 0 of the first page.
        let word = unsafe { crate::ffi::ior_engine_select(engine, 0) };
        assert_eq!(text(word), "你");
        assert_eq!(
            text(unsafe { crate::ffi::ior_engine_take_commit(engine) }),
            "你"
        );

        type_keys(engine, &[0x6E, 0x69]); // n i
                                          // SAFETY: live engine.
        assert_eq!(
            text(unsafe { crate::ffi::ior_engine_candidate(engine, 0) }),
            "你"
        );
        // SAFETY: live engine; delete the learned top hit.
        assert_eq!(unsafe { ior_engine_candidate_delete(engine, 0) }, 0);
        // Not learned anymore (the dictionary hit behind it is read-only).
        // SAFETY: live engine.
        assert_eq!(unsafe { ior_engine_candidate_delete(engine, 0) }, 1);
        // Out of the page-local window.
        // SAFETY: live engine.
        assert_eq!(unsafe { ior_engine_candidate_delete(engine, 99) }, 2);
        // Composition survived the deletions.
        // SAFETY: live engine; the pointer is read before the next call.
        let preedit = unsafe { crate::ffi::ior_engine_preedit(engine) as *const c_char };
        assert_eq!(text(preedit), "ni");
        unsafe { crate::ffi::ior_engine_free(engine) };
    }

    #[test]
    fn null_engine_is_safe() {
        // SAFETY: null is documented as tolerated.
        assert_eq!(
            unsafe { ior_engine_candidate_delete(std::ptr::null_mut(), 0) },
            -1
        );
    }
}
