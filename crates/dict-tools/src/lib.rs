//! dict-tools: standalone dictionary toolchain for the ior input method.
//!
//! This crate intentionally does NOT depend on engine-core; it owns its own
//! entry type and file formats. Later milestone glue (M1) will convert the
//! built dictionaries into the engine-core `Dictionary` representation.
//!
//! Layout:
//! - [`format`]: `Entry`, TSV parsing/rendering, `.ior` binary
//!   (de)serialization (v2 writer, v1+v2 reader), and deterministic
//!   normalization.
//! - [`varint`]: LEB128 u32 varints used by the `.ior` v2 layout.
//! - [`source`]: input auto-detection (ior TSV vs rime dict.yaml tables)
//!   and structural validation for `compile`/`validate`.
//! - [`cli`]: hand-rolled argument parsing (no clap) and command dispatch
//!   with stable exit codes (0 ok, 2 io, 3 format).

pub mod cli;
pub mod format;
pub mod gram;
pub mod preset;
pub mod source;
pub mod varint;

pub use cli::{parse_args, run, Command, USAGE};
pub use format::{deserialize, normalize, parse_tsv, serialize, to_tsv, Entry, MAGIC, VERSION};
pub use gram::{bigrams, parse_gram, total_weight, train_counts, trigrams, unigrams, GramCounts};
pub use preset::{PresetVocab, WEIGHT_SCALE};
pub use source::{load_source, load_source_with_preset, rime_settings, RimeSettings};
