//! Full-width (全角) input mode: maps otherwise-forwarded ASCII keys to
//! their full-width counterparts.
//!
//! Scope follows the classic desktop-IME split (Sogou/rime):
//! - Chinese mode: idle digits and space commit full-width (`１２３`,
//!   ideometric space `U+3000`) and Shift-typed uppercase maps (`Ａ`);
//!   lowercase letters still compose pinyin.
//! - English mode: alphanumerics and space commit their full-width
//!   forms (`ａｂｃ１２３`, `U+3000`).
//!
//! Punctuation is NEVER claimed here — it independently follows the
//! Chinese-punctuation switch (`chinese_punct`), like the classic IMEs
//! where 半/全角 and 中/英标点 are two separate toggles.
//!
//! Half-width (the default) is a pure passthrough: [`intercept`] returns
//! `None` and behavior is unchanged.

use crate::session::{ProcessResult, Session};

/// Full-width form of one ASCII key, if it has one: space becomes the
/// ideographic space `U+3000`, printable ASCII (`!`..=`~`) shifts into
/// the Halfwidth/Fullwidth block (`ch + 0xFEE0`).
pub fn full_width_char(ch: char) -> Option<char> {
    match ch {
        ' ' => Some('\u{3000}'),
        '!'..='~' => char::from_u32(ch as u32 + 0xFEE0),
        _ => None,
    }
}

/// Takes over the key when full-width mode should handle it, else `None`
/// (half-width mode, non-mappable keys, or keys that keep their editing
/// meaning: anything typed mid-composition, and Chinese-mode letters).
pub fn intercept(s: &Session, ch: char) -> Option<ProcessResult> {
    if !s.full_width || !s.raw.is_empty() {
        return None;
    }
    if !(ch.is_ascii_alphanumeric() || ch == ' ') {
        return None; // punctuation follows `chinese_punct`, never width
    }
    let mapped = full_width_char(ch)?;
    // Chinese mode keeps lowercase letters for pinyin; digits, space
    // and (Shift-typed) uppercase letters become full-width.
    if s.ascii_mode || ch.is_ascii_digit() || ch == ' ' || ch.is_ascii_uppercase() {
        return Some(ProcessResult::Committed(mapped.to_string()));
    }
    None
}

/// Sets the per-session full-width flag (idempotent); returns the new
/// state, mirroring the shape of the ASCII-mode pair for the FFI.
pub fn set_full_width(s: &mut Session, enable: bool) -> bool {
    s.full_width = enable;
    s.full_width
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::default_config;
    use crate::session::{new_session, process_key};

    fn session_full_width(ascii: bool) -> Session {
        let mut s = new_session(default_config());
        s.ascii_mode = ascii;
        set_full_width(&mut s, true);
        assert!(s.full_width);
        s
    }

    #[test]
    fn half_width_is_a_pure_passthrough() {
        let s = new_session(default_config());
        for ch in ['a', '5', ' ', '!', 'A'] {
            assert_eq!(intercept(&s, ch), None);
        }
    }

    #[test]
    fn english_mode_maps_alphanumerics_and_space() {
        let mut s = session_full_width(true);
        assert_eq!(
            process_key(&mut s, 'a'),
            ProcessResult::Committed("ａ".into())
        );
        assert_eq!(
            process_key(&mut s, 'Z'),
            ProcessResult::Committed("Ｚ".into())
        );
        assert_eq!(
            process_key(&mut s, '5'),
            ProcessResult::Committed("５".into())
        );
        assert_eq!(
            process_key(&mut s, ' '),
            ProcessResult::Committed("\u{3000}".into())
        );
        // Punctuation is independent of the width toggle: it follows the
        // Chinese-punctuation switch (default half-width passthrough).
        assert_eq!(process_key(&mut s, '!'), ProcessResult::Forwarded);
    }

    #[test]
    fn chinese_mode_maps_only_digits_and_space() {
        let mut s = session_full_width(false);
        assert_eq!(
            process_key(&mut s, '7'),
            ProcessResult::Committed("７".into())
        );
        assert_eq!(
            process_key(&mut s, 'A'),
            ProcessResult::Committed("Ａ".into())
        );
        assert_eq!(
            process_key(&mut s, ' '),
            ProcessResult::Committed("\u{3000}".into())
        );
        // Letters still compose pinyin.
        assert_eq!(process_key(&mut s, 'm'), ProcessResult::Editing);
        assert_eq!(s.raw, "m");
        // Punctuation follows its own switch: half-width by default...
        let mut idle = session_full_width(false);
        assert_eq!(process_key(&mut idle, ','), ProcessResult::Forwarded);
        // ...and the Chinese mapping when explicitly enabled.
        idle.config.chinese_punct = true;
        assert_eq!(
            process_key(&mut idle, ','),
            ProcessResult::Committed("，".into())
        );
    }

    #[test]
    fn composing_keys_keep_their_editing_meaning() {
        let mut s = session_full_width(false);
        for ch in "ma".chars() {
            process_key(&mut s, ch);
        }
        // Mid-composition: digit selects (Editing or Committed),
        // space commits the top candidate.
        let digit = process_key(&mut s, '2');
        assert!(matches!(
            digit,
            ProcessResult::Editing | ProcessResult::Committed(_)
        ));
        let mut s = session_full_width(false);
        for ch in "ma".chars() {
            process_key(&mut s, ch);
        }
        assert!(matches!(
            process_key(&mut s, ' '),
            ProcessResult::Committed(_)
        ));
    }

    #[test]
    fn set_full_width_round_trips() {
        let mut s = new_session(default_config());
        assert!(!s.full_width);
        set_full_width(&mut s, true);
        assert!(s.full_width);
        set_full_width(&mut s, false);
    }
}
