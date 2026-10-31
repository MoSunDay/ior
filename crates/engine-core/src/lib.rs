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
//! - [`config`]: fuzzy-pinyin flags and engine settings;
//! - [`dict`]: static pinyin -> hanzi dictionary (fuzzy rules baked into keys);
//! - [`user_dict`]: learned frequencies;
//! - [`decoder`]: candidate assembly for a raw input (M2: trigram + beam
//!   search replaces this);
//! - [`session`]: key handling, preedit, commit and learning.
//!
//! Roadmap: trigram/beam-search decoding and additional fuzzy rules in M2,
//! double-pinyin mode in M2, simplified/traditional (U) and v-mode (V) input
//! modes in M4.

pub mod config;
pub mod decoder;
pub mod dict;
pub mod segmenter;
pub mod session;
pub mod syllable;
pub mod user_dict;

pub use config::{default_config, normalize_syllable, EngineConfig, FuzzyConfig};
pub use decoder::{candidates as decode_candidates, Candidate};
pub use dict::Dictionary;
pub use segmenter::{best_segmentation, segmentations};
pub use session::{
    candidates as session_candidates, new_session, new_session_with, normalized_key, preedit,
    process_key, reset as reset_session, ProcessResult, Session,
};
pub use syllable::{is_syllable, SYLLABLES};
pub use user_dict::{
    is_empty as user_dict_is_empty, learn as learn_user, len as user_dict_len,
    lookup as lookup_user, new as new_user_dict, UserDict,
};
