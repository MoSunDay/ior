//! ior configuration (P4 step 1, see `docs/ROADMAP-RIME-PARITY.md`): a
//! TOML-subset parser, the ior config schema, and the user-patch overlay.
//!
//! Zero third-party dependencies (no toml/serde), no `unsafe`; plain data
//! structs with free functions. Nothing panics on bad input: parse
//! failures are [`ParseError`]s carrying 1-based line/column (dict-tools
//! message style), schema/merge failures are strings prefixed with the
//! dotted key path.
//!
//! # Supported TOML subset
//! - comments: `#` on its own line or trailing after a value/header
//! - headers: `[section]` and dotted `[a.b]`; segments bare or quoted
//! - keys: `key = value` and dotted keys `a.b = value`; bare keys match
//!   `[A-Za-z0-9_-]+` — digits-only keys are rejected (use named or
//!   quoted keys), anything else must be quoted (`"," = "，"`)
//! - values: basic strings `"..."` with escapes `\" \\ \n \t \u{XXXX}`;
//!   decimal integers (i64, optional sign); booleans `true`/`false`;
//!   single-line arrays `["a", "b"]` with at most one nesting level and
//!   an optional trailing comma
//!
//! # Not supported (clear error, never a guess)
//! multi-line strings, floats, date/time values, hex/octal integers,
//! inline tables, multi-line arrays, arrays of tables (`[[x]]`),
//! re-opening a table header, extending a scalar with a header,
//! duplicate keys, digits-only keys.
//!
//! # Pipeline (for the addon wiring that follows)
//! 1. read the user's `ior.toml` (file I/O stays in the addon) and
//!    [`parse`] it into a [`Table`];
//! 2. [`apply_with_warnings`]`(&default_table(), &user)` -> [`Loaded`];
//! 3. copy `Loaded.config` fields into `engine_core::config::EngineConfig`
//!    (field names match 1:1) and push `[switches]` values onto the
//!    session; unknown keys never fail the load — surface the warnings;
//! 4. [`serialize_config`] renders the canonical TOML back out for
//!    deployment (roundtrip-stable).
//!
//! # Schema keys (defaults mirror `engine_core::config`)
//! - `engine.shift_action` — `"commit"` | `"temp"` | `"none"`
//!   (↔ engine-core `ShiftAction::CommitRaw` / `CommitBest` / `Clear`)
//! - `engine.enter_action` — `"raw"` | `"candidate"`
//!   (↔ engine-core `EnterAction::CommitRaw` / `CommitBest`)
//! - `engine.double_pinyin` — `"off"` | `"natural"` | `"xiaohe"`
//!   (alias `"flypy"`) | `"mspy"` | `"custom"`
//! - `engine.chinese_punct`, `engine.user_learning` — booleans
//! - `engine.page_size`, `engine.max_candidates` — integers >= 1
//! - `engine.fuzzy.{z_zh,c_ch,s_sh,n_l,f_h,an_ang,en_eng,in_ing}` —
//!   booleans, same names as engine-core `FuzzyConfig`
//! - `switches.<name>` — dynamic boolean switch table (any name)
//! - `punct.<name>` — string or array-of-strings mapping placeholder
//!
//! ```
//! use ior_config::{apply, default_table, parse};
//!
//! let user = parse("[engine]\npage_size = 7\n").expect("valid toml");
//! let config = apply(&default_table(), &user).expect("valid patch");
//! assert_eq!(config.engine.page_size, 7);
//! ```

pub mod error;
pub mod model;
pub mod parser;
pub mod patch;
pub mod schema;
pub mod value;

mod lex;

pub use error::ParseError;
pub use model::{
    default_switches, default_table, to_table, Config, DoublePinyin, Engine, EnterAction, Fuzzy,
    PunctValue, ShiftAction,
};
pub use parser::{is_bare_key, parse};
pub use patch::{apply, apply_with_warnings, merge_tables, serialize_config, serialize_table};
pub use schema::{from_table, Loaded};
pub use value::{equivalent, escape_string, kind, tables_equivalent, Table, Value};
