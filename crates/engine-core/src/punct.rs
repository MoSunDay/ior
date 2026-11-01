//! Chinese punctuation mapping, driven purely by the tables below.
//!
//! [`map`] is a total function over `char`: with `chinese == false` every
//! key passes through untouched; with `chinese == true` the printable ASCII
//! punctuation set is translated via [`SIMPLE_MAP`] while the paired quotes
//! `"` and `'` alternate open/close through the parity bits in
//! [`PunctState`]. Anything not listed passes through so the frontend can
//! forward it to the application. Keeping the mapping in `const` tables
//! makes a future config-driven variant a data change, not a code change.

/// Session-level punctuation state. Only the paired quotes carry state
/// (which of “ ” / ‘ ’ comes next); everything else is stateless lookup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PunctState {
    /// Next `"` yields the opening double quote “ (vs. the closing ”).
    double_open_next: bool,
    /// Next `'` yields the opening single quote ‘ (vs. the closing ’).
    single_open_next: bool,
}

/// Outcome of mapping one punctuation key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PunctOut {
    /// Commit this text (e.g. `，` for `,`).
    Text(String),
    /// Not ours: forward the key to the application.
    Passthrough,
}

/// State-less punctuation mappings: ASCII key -> Chinese text.
const SIMPLE_MAP: &[(char, &str)] = &[
    (',', "，"),
    ('.', "。"),
    (';', "；"),
    (':', "："),
    ('?', "？"),
    ('!', "！"),
    ('(', "（"),
    (')', "）"),
    ('[', "【"),
    (']', "】"),
    ('{', "｛"),
    ('}', "｝"),
    ('<', "《"),
    ('>', "》"),
    ('\\', "、"),
    ('/', "／"),
    ('~', "～"),
    ('@', "＠"),
    ('#', "＃"),
    ('$', "￥"),
    ('%', "％"),
    ('^', "……"),
    ('&', "＆"),
    ('*', "＊"),
    ('_', "——"),
    ('-', "－"),
    ('+', "＋"),
    ('=', "＝"),
    ('|', "｜"),
];

/// Fresh state: both quote pairs start with their opening mark.
pub fn new() -> PunctState {
    PunctState {
        double_open_next: true,
        single_open_next: true,
    }
}

/// Maps one key under the given mode. `chinese == false` always passes
/// through (English mode or the mapping switched off). Unlisted printable
/// ASCII also passes through.
pub fn map(state: &mut PunctState, ch: char, chinese: bool) -> PunctOut {
    if !chinese {
        return PunctOut::Passthrough;
    }
    match ch {
        '"' => {
            let out = if state.double_open_next { "“" } else { "”" };
            state.double_open_next = !state.double_open_next;
            PunctOut::Text(out.to_string())
        }
        '\'' => {
            let out = if state.single_open_next { "‘" } else { "’" };
            state.single_open_next = !state.single_open_next;
            PunctOut::Text(out.to_string())
        }
        _ => match SIMPLE_MAP.iter().find(|(key, _)| *key == ch) {
            Some((_, text)) => PunctOut::Text((*text).to_string()),
            None => PunctOut::Passthrough,
        },
    }
}

/// Returns the state to its initial parity (both quotes open next).
pub fn reset(state: &mut PunctState) {
    *state = new();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn txt(out: PunctOut) -> String {
        match out {
            PunctOut::Text(t) => t,
            PunctOut::Passthrough => panic!("expected Text, got Passthrough"),
        }
    }

    #[test]
    fn simple_mappings_hit_the_table() {
        let mut st = new();
        assert_eq!(txt(map(&mut st, ',', true)), "，");
        assert_eq!(txt(map(&mut st, '.', true)), "。");
        assert_eq!(txt(map(&mut st, '^', true)), "……");
        assert_eq!(txt(map(&mut st, '_', true)), "——");
        assert_eq!(txt(map(&mut st, '\\', true)), "、");
    }

    #[test]
    fn english_mode_always_passes_through() {
        let mut st = new();
        assert_eq!(map(&mut st, ',', false), PunctOut::Passthrough);
        assert_eq!(map(&mut st, '"', false), PunctOut::Passthrough);
        // English keys leave the quote parity untouched.
        assert_eq!(txt(map(&mut st, '"', true)), "“");
    }

    #[test]
    fn unlisted_ascii_passes_through() {
        let mut st = new();
        assert_eq!(map(&mut st, 'a', true), PunctOut::Passthrough);
        assert_eq!(map(&mut st, '5', true), PunctOut::Passthrough);
        assert_eq!(map(&mut st, ' ', true), PunctOut::Passthrough);
        assert_eq!(map(&mut st, '\t', true), PunctOut::Passthrough);
        assert_eq!(map(&mut st, '中', true), PunctOut::Passthrough);
    }

    #[test]
    fn double_quote_alternates_twice() {
        let mut st = new();
        assert_eq!(txt(map(&mut st, '"', true)), "“");
        assert_eq!(txt(map(&mut st, '"', true)), "”");
        assert_eq!(txt(map(&mut st, '"', true)), "“");
        assert_eq!(txt(map(&mut st, '"', true)), "”");
    }

    #[test]
    fn single_quote_alternates_independently() {
        let mut st = new();
        assert_eq!(txt(map(&mut st, '\'', true)), "‘");
        assert_eq!(txt(map(&mut st, '"', true)), "“"); // other pair untouched
        assert_eq!(txt(map(&mut st, '\'', true)), "’");
        assert_eq!(txt(map(&mut st, '\'', true)), "‘");
    }

    #[test]
    fn reset_restores_opening_quotes() {
        let mut st = new();
        assert_eq!(txt(map(&mut st, '"', true)), "“");
        assert_eq!(txt(map(&mut st, '\'', true)), "‘");
        reset(&mut st);
        assert_eq!(txt(map(&mut st, '"', true)), "“");
        assert_eq!(txt(map(&mut st, '\'', true)), "‘");
    }

    #[test]
    fn table_covers_distinct_keys_only() {
        let mut seen = std::collections::HashSet::new();
        for (key, _) in SIMPLE_MAP.iter() {
            assert!(seen.insert(*key), "duplicate key {key:?} in SIMPLE_MAP");
        }
    }
}
