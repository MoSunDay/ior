//! Headless M0 demo: drives the ior engine through its C ABI exactly the way
//! the fcitx5 shell would. Run with:
//!
//! ```sh
//! cargo run -p fcitx5-ior --example m0_demo
//! ```

use std::ffi::{c_char, CStr};

use ior_engine::engine_state::IorEngine;
use ior_engine::ffi::{
    ior_engine_candidate, ior_engine_candidate_count, ior_engine_free, ior_engine_new,
    ior_engine_preedit, ior_engine_process_key, ior_engine_take_commit,
};

/// Reads an engine-owned string. Per the FFI contract the pointer is only
/// valid until the next ior_* call, so callers copy the bytes immediately.
fn text(ptr: *const c_char) -> String {
    // SAFETY: pointers come straight from ior_* calls above and are read
    // before any other ior_* call touches the engine.
    unsafe { CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned()
}

/// Feeds one keysym; returns the raw FFI status (0=forward, 1=editing,
/// 2=committed).
fn key(engine: *mut IorEngine, keysym: u32) -> i32 {
    // SAFETY: engine comes from ior_engine_new and outlives this demo.
    unsafe { ior_engine_process_key(engine, keysym) }
}

/// Current preedit of `engine`.
fn preedit(engine: *mut IorEngine) -> String {
    // SAFETY: engine is live; the pointer is read before the next ior_* call.
    text(unsafe { ior_engine_preedit(engine) })
}

/// Current candidate list of `engine` as owned strings.
fn candidates(engine: *mut IorEngine) -> Vec<String> {
    // SAFETY: engine is live; each candidate pointer is read right away.
    let count = unsafe { ior_engine_candidate_count(engine) };
    (0..count)
        .map(|i| text(unsafe { ior_engine_candidate(engine, i) }))
        .collect()
}

/// Takes and returns the pending commit, if any.
fn take_commit(engine: *mut IorEngine) -> String {
    // SAFETY: engine is live; the pointer is read before the next ior_* call.
    text(unsafe { ior_engine_take_commit(engine) })
}

fn main() {
    let engine = ior_engine_new();
    assert!(!engine.is_null(), "engine allocation failed");

    // Type "nihao" and show preedit plus the candidate list.
    for keysym in [0x6E, 0x69, 0x68, 0x61, 0x6F] {
        assert_eq!(key(engine, keysym), 1, "letter should be consumed");
    }
    println!("preedit: {}", preedit(engine));
    println!("candidates: {}", candidates(engine).join(" "));

    // Space commits the top candidate.
    assert_eq!(key(engine, 0x20), 2, "space should commit");
    println!("commit: {}", take_commit(engine));
    println!("preedit: {}", preedit(engine));

    // Ambiguous input: "xian" splits as one syllable or as xi + an, so the
    // candidate list mixes both readings.
    for keysym in [0x78, 0x69, 0x61, 0x6E] {
        assert_eq!(key(engine, keysym), 1);
    }
    println!("preedit: {}", preedit(engine));
    println!("candidates: {}", candidates(engine).join(" "));

    // SAFETY: engine is released exactly once at the end of the demo.
    unsafe { ior_engine_free(engine) };
}
