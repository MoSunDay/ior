//! Integration tests for the user-dictionary persistence ABI
//! (`ior_engine_userdb_save/load/decay`), driven exactly like a C host
//! would: raw pointers, NUL-terminated paths, no crate-internal helpers.
//!
//! engine-core owns the formats (snapshot + append-only log); these tests
//! pin the ABI contract: status codes, cold-start leniency, merge (not
//! replace) semantics and that decay never lets zero-frequency rows reach
//! the disk.

use std::ffi::{c_char, CStr, CString};
use std::path::PathBuf;

use engine_core::user_dict;
use ior_engine::engine_state::IorEngine;
use ior_engine::ffi::{
    ior_engine_ascii_mode, ior_engine_candidate_count, ior_engine_free, ior_engine_new,
    ior_engine_preedit, ior_engine_process_key, ior_engine_take_commit, ior_engine_userdb_decay,
    ior_engine_userdb_load, ior_engine_userdb_save,
};

/// `n i h a o` keysyms (the canonical "nihao" composition).
const NIHAO: [u32; 5] = [0x6E, 0x69, 0x68, 0x61, 0x6F];

/// Reads an engine-owned C string immediately, per the FFI contract.
fn text(ptr: *const c_char) -> String {
    assert!(!ptr.is_null());
    // SAFETY: the pointer comes from an ior_* call and is read before any
    // other ior_* call touches the engine.
    unsafe { CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned()
}

/// Types `keysyms` into `engine`, asserting each is consumed as editing.
fn type_keys(engine: *mut IorEngine, keysyms: &[u32]) {
    for keysym in keysyms {
        // SAFETY: engine is live for the whole test.
        assert_eq!(unsafe { ior_engine_process_key(engine, *keysym) }, 1);
    }
}

/// NUL-terminated path for an FFI call. Bind the return value to a local —
/// the pointer is only valid while the CString lives.
fn c_path(path: &std::path::Path) -> CString {
    CString::new(path.as_os_str().as_encoded_bytes()).expect("temp path has no NUL")
}

/// A unique temp file path so parallel test runs never collide.
fn temp_userdb(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock after epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "ior-userdb-{}-{}-{}.tsv",
        tag,
        std::process::id(),
        nanos
    ))
}

/// Learned frequency of `hanzi` under `key` in the engine's user dict.
fn learned_freq(engine: *mut IorEngine, key: &str, hanzi: &str) -> u32 {
    // SAFETY: engine is live and only read here (no concurrent call).
    let user = &unsafe { &*engine }.session.user_dict;
    user_dict::lookup(user, key)
        .into_iter()
        .find(|(h, _)| h == hanzi)
        .map_or(0, |(_, freq)| freq)
}

#[test]
fn save_then_load_roundtrip_keeps_frequencies() {
    let engine = ior_engine_new(); // safe: takes no pointers
    type_keys(engine, &NIHAO);
    // Space commits the top candidate and learns it under `ni'hao`.
    // SAFETY: engine is live.
    assert_eq!(unsafe { ior_engine_process_key(engine, 0x20) }, 2);
    // SAFETY: engine is live; the commit pointer is read immediately.
    assert_eq!(text(unsafe { ior_engine_take_commit(engine) }), "你好");
    let expected = learned_freq(engine, "ni'hao", "你好");
    assert_eq!(expected, 1);

    let path = temp_userdb("roundtrip");
    let path_c = c_path(&path);
    // SAFETY: engine is live; path_c is a NUL-terminated temp path.
    assert_eq!(
        unsafe { ior_engine_userdb_save(engine, path_c.as_ptr()) },
        0
    );
    // A null path is rejected without touching the engine.
    // SAFETY: null path is explicitly allowed.
    assert_eq!(
        unsafe { ior_engine_userdb_save(engine, std::ptr::null()) },
        -1
    );

    // A second engine loads the snapshot and ends up with the same data.
    let other = ior_engine_new(); // safe: takes no pointers
                                  // SAFETY: other is live; path_c is NUL-terminated.
    assert_eq!(unsafe { ior_engine_userdb_load(other, path_c.as_ptr()) }, 0);
    assert_eq!(learned_freq(other, "ni'hao", "你好"), expected);

    // SAFETY: each engine pointer is freed exactly once.
    unsafe { ior_engine_free(engine) };
    unsafe { ior_engine_free(other) };
    let _ = std::fs::remove_file(&path);
}

#[test]
fn load_missing_file_is_not_fatal() {
    let engine = ior_engine_new(); // safe: takes no pointers
    let path = temp_userdb("missing");
    let path_c = c_path(&path);
    // SAFETY: engine is live; the unique path does not exist.
    assert_eq!(
        unsafe { ior_engine_userdb_load(engine, path_c.as_ptr()) },
        1
    );
    // The engine keeps its state: idle preedit, no candidates, Chinese
    // mode, no pending commit, nothing learned.
    // SAFETY: engine is live; each pointer is read right away.
    assert_eq!(text(unsafe { ior_engine_preedit(engine) }), "");
    assert_eq!(unsafe { ior_engine_candidate_count(engine) }, 0);
    assert_eq!(unsafe { ior_engine_ascii_mode(engine) }, 0);
    assert_eq!(text(unsafe { ior_engine_take_commit(engine) }), "");
    type_keys(engine, &NIHAO);
    assert_eq!(learned_freq(engine, "ni'hao", "你好"), 0); // not learned yet
                                                           // SAFETY: engine pointer is freed exactly once.
    unsafe { ior_engine_free(engine) };
}

#[test]
fn load_replays_log_and_absorbs_into_live_dict() {
    // An append-only log file: two uses of 泥嚎 under `ni'hao`.
    let path = temp_userdb("log");
    std::fs::write(&path, "L\tni'hao\t泥嚎\nL\tni'hao\t泥嚎\n").expect("write log");

    let engine = ior_engine_new(); // safe: takes no pointers
    type_keys(engine, &NIHAO);
    // SAFETY: engine is live.
    assert_eq!(unsafe { ior_engine_process_key(engine, 0x20) }, 2); // learns 你好
    let path_c = c_path(&path);
    // SAFETY: engine is live; path_c is NUL-terminated.
    assert_eq!(
        unsafe { ior_engine_userdb_load(engine, path_c.as_ptr()) },
        0
    );
    // The log entries are merged on top of the live one, not replacing it.
    assert_eq!(learned_freq(engine, "ni'hao", "泥嚎"), 2);
    assert_eq!(learned_freq(engine, "ni'hao", "你好"), 1);
    // Loading does not disturb the (empty) editing state.
    // SAFETY: engine is live; the pointer is read immediately.
    assert_eq!(text(unsafe { ior_engine_preedit(engine) }), "");

    // SAFETY: engine pointer is freed exactly once.
    unsafe { ior_engine_free(engine) };
    let _ = std::fs::remove_file(&path);
}

#[test]
fn decay_then_save_drops_zero_frequencies() {
    let engine = ior_engine_new(); // safe: takes no pointers
    type_keys(engine, &NIHAO);
    // SAFETY: engine is live.
    assert_eq!(unsafe { ior_engine_process_key(engine, 0x20) }, 2); // freq 1
                                                                    // One half-life since the last save: 1 >> 1 == 0, so the entry (and
                                                                    // its key) disappear. A zero half-life is a legal no-op first.
    const DAY: u32 = 24 * 3600;
    // SAFETY: engine is live.
    assert_eq!(unsafe { ior_engine_userdb_decay(engine, DAY, 0) }, 0);
    assert_eq!(learned_freq(engine, "ni'hao", "你好"), 1);
    // SAFETY: engine is live.
    assert_eq!(unsafe { ior_engine_userdb_decay(engine, 30 * DAY, DAY) }, 0);
    assert_eq!(learned_freq(engine, "ni'hao", "你好"), 0);

    let path = temp_userdb("decay");
    std::fs::write(&path, b"stale contents\n").expect("seed file");
    let path_c = c_path(&path);
    // SAFETY: engine is live; path_c is NUL-terminated.
    assert_eq!(
        unsafe { ior_engine_userdb_save(engine, path_c.as_ptr()) },
        0
    );
    let saved = std::fs::read_to_string(&path).expect("read back");
    // The empty dictionary serializes to just the header: no zero rows.
    assert_eq!(saved, "# ior userdb v1\n");
    assert!(!saved.contains("\t0\n"));

    // SAFETY: engine pointer is freed exactly once.
    unsafe { ior_engine_free(engine) };
    let _ = std::fs::remove_file(&path);
}

#[test]
fn load_rejects_garbage_file_without_learning() {
    let path = temp_userdb("garbage");
    std::fs::write(&path, "not\ta\tuserdb\n").expect("write garbage");
    let engine = ior_engine_new(); // safe: takes no pointers
    let path_c = c_path(&path);
    // SAFETY: engine is live; path_c is NUL-terminated.
    assert_eq!(
        unsafe { ior_engine_userdb_load(engine, path_c.as_ptr()) },
        3
    );
    // A corrupt file must not learn anything either.
    assert_eq!(learned_freq(engine, "not", "userdb"), 0);
    // SAFETY: engine pointer is freed exactly once.
    unsafe { ior_engine_free(engine) };
    let _ = std::fs::remove_file(&path);
}
