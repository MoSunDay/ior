//! # engine-core
//!
//! Headless core of the `ior` input method engine (M0 milestone).
//!
//! Scope of M0: full-pinyin typing -> segmentation -> dictionary candidates
//! -> selection/commit, plus user-dictionary learning. A [`Session`] ties the
//! pieces together; everything else is pure functions over plain data.
//!
//! Module map:
//! - [`syllable`]: the toneless pinyin syllable table;
//! - [`segmenter`]: all/best splits of a letter run into syllables;
//! - [`config`]: fuzzy-pinyin flags, double-pinyin mode and engine settings;
//! - [`spelling`]: query-side spelling pipeline (fuzzy variants, 简拼);
//! - [`double_pinyin`]: loadable double-pinyin key tables + transform;
//! - [`dict`]: static pinyin -> hanzi dictionary (canonical keys, initials
//!   index for 简拼);
//! - [`user_dict`]: learned frequencies;
//! - [`lm`]: character n-gram language model parsed from `.gram` bytes
//!   (P2/M2b; the format spec lives in its module docs);
//! - [`beam`]/[`beam_search`]: whole-sentence beam search over the word
//!   lattice, scored by the LM (P2/M2b);
//! - [`punct`]: Chinese punctuation mapping (data-driven, paired quotes);
//! - [`decoder`]: candidate assembly for a raw input — legacy three-step
//!   path, plus the [`decoder::candidates_with_lm`] beam routing;
//! - [`session`]: key handling, preedit, commit and learning.
//!
//! Roadmap: additional fuzzy rules with the same query-side pipeline,
//! host-installed custom double-pinyin tables (builtin natural/flypy/mspy
//! tables are already bundled, see [`double_pinyin`]), simplified/
//! traditional (U) and v-mode (V) input modes in M4; corpus-level LM data
//! to replace the word-list-derived counts (P2 follow-up).

pub mod beam;
pub mod beam_search;
pub mod config;
pub mod decoder;
pub mod dict;
pub mod double_pinyin;
pub mod lm;
mod lm_format;
mod prefix_index;
pub mod punct;
pub mod segmenter;
pub mod session;
pub mod spelling;
pub mod syllable;
pub mod user_dict;
pub mod user_dict_store;
pub mod width;

pub use beam::{candidates as beam_candidates, BeamParams, MIN_BEAM_SYLLABLES};
pub use config::{default_config, normalize_syllable, DoublePinyin, EngineConfig, FuzzyConfig};
pub use decoder::{
    candidates as decode_candidates, candidates_with as decode_candidates_with,
    candidates_with_lm as decode_candidates_with_lm, Candidate,
};
pub use dict::Dictionary;
pub use double_pinyin::{parse_tsv as dp_parse_table, to_full as dp_to_full, DpTable};
pub use lm::{from_gram_bytes, GramModel, BACKOFF_PENALTY, GRAM_MAGIC, GRAM_ORDER, GRAM_VERSION};
pub use punct::{map as punct_map, new as new_punct, reset as punct_reset, PunctOut, PunctState};
pub use segmenter::{best_segmentation, segmentations};
pub use session::{
    candidates as session_candidates, current_page, delete_flat, new_session, new_session_with,
    next_page, normalized_key, page_count, preedit, prev_page, process_key, process_shift,
    reset as reset_session, select_flat, set_double_pinyin, toggle_ascii_punct, DeleteOutcome,
    ProcessResult, Session,
};
pub use spelling::{initials_prefix, key_variants, syllable_variants};
pub use syllable::{is_syllable, SYLLABLES};
pub use user_dict::{
    absorb as absorb_user, decay as decay_user, forget as forget_user,
    is_empty as user_dict_is_empty, learn as learn_user, len as user_dict_len,
    lookup as lookup_user, new as new_user_dict, UserDict,
    SNAPSHOT_HEADER as USERDB_SNAPSHOT_HEADER,
};
pub use user_dict_store::{
    compact as compact_userdb, export_tsv as export_user_tsv,
    from_snapshot as userdb_from_snapshot, import_tsv as import_user_tsv,
    log_line as userdb_log_line, replay as replay_userdb, to_snapshot as userdb_to_snapshot,
};
