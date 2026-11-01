//! # fcitx5-ior
//!
//! fcitx5 host layer of `ior`: a C ABI bridge ([`ffi`]) from the Rust engine
//! (engine-core) to the fcitx5 C++ shell, plus the C++ shell sources and
//! build files under `cpp/` (see `cpp/README.md`; the shell targets fcitx5
//! 5.1.x and contains no engine logic).
//!
//! # FFI boundary contract
//!
//! - All exported symbols are prefixed `ior_` and form the stable surface
//!   consumed by the C++ shell; see [`ffi`] for the full contract.
//! - Engine state is opaque (`IorEngineC` in C). Obtain it from
//!   `ior_engine_new` and release it with `ior_engine_free`; never free
//!   anything else, in particular never free returned `const char*` values.
//! - Returned string pointers are only valid until the next `ior_*` call on
//!   the same engine. Copy the bytes (e.g. into `std::string`) if they must
//!   survive longer.
//! - Engined are **not** thread-safe: fcitx5 runs its input pipeline on one
//!   thread, and the bridge does no locking. Concurrent calls on one engine
//!   are undefined behavior.
//! - Every entry point tolerates a null engine (process_key returns -1,
//!   preedit/take_commit/select return "", candidate_count/page_info/
//!   ascii_mode return 0, page_next/page_prev return 0, candidate returns
//!   null, userdb_save/load/decay return -1).
//! - Candidate symbols expose the **current page** only; paging is cyclic
//!   (`page_next`/`page_prev`, snapshot via `page_info`), clicks commit via
//!   `select(page_local_index)`, and the Chinese/English mode pair is
//!   `ascii_mode`/`set_ascii_mode` (ABI v2).
//! - The userdb pair `userdb_save`/`userdb_load` (plus `userdb_decay`)
//!   persists the learned dictionary: saving writes a compacted snapshot,
//!   loading sniffs snapshot-vs-log format and only merges into the user
//!   dictionary (preedit/commit/mode untouched; a missing file is status 1,
//!   not an error). File I/O lives in [`engine_state`] only — engine-core
//!   just produces and consumes bytes.
//!
//! Stability rules: the `ior_*` C ABI is versioned with the addon version;
//! additive changes only. Anything not `#[no_mangle] extern "C"` here is
//! internal and may change between releases.

pub mod engine_state;
pub mod ffi;
