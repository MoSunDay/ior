//! dict-tools: standalone dictionary toolchain for the ior input method.
//!
//! This crate intentionally does NOT depend on engine-core; it owns its own
//! entry type and file formats. Later milestone glue (M1) will convert the
//! built dictionaries into the engine-core `Dictionary` representation.
//!
//! Layout:
//! - [`format`]: `Entry`, TSV parsing/rendering, `.ior` binary
//!   (de)serialization, and deterministic normalization.
//! - [`cli`]: hand-rolled argument parsing (no clap) and command dispatch
//!   with stable exit codes (0 ok, 1 usage, 2 io, 3 format).

pub mod cli;
pub mod format;

pub use cli::{parse_args, run, Command, USAGE};
pub use format::{deserialize, normalize, parse_tsv, serialize, to_tsv, Entry, MAGIC, VERSION};
