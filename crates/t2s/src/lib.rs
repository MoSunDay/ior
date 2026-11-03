//! Traditional -> simplified hanzi conversion.
//!
//! The tables are vendored OpenCC data (Apache-2.0, pinned to
//! `ver.1.4.2`; see `data/opencc/README.md`). Conversion mirrors OpenCC's
//! `t2s.json`: try the longest phrase at each position first, then fall
//! back to the single-character table. Only the first value on a line is
//! used, which is OpenCC's default output.
//!
//! Used at dictionary compile time (`dict-tools compile --simplify`) and
//! at voice runtime: ASR models such as whisper-small often emit
//! traditional script for `language=zh`, so the addon normalizes
//! transcripts to simplified before committing.

use std::collections::HashMap;

const TS_CHARACTERS: &str = include_str!("../../../data/opencc/TSCharacters.txt");
const TS_PHRASES: &str = include_str!("../../../data/opencc/TSPhrases.txt");

/// Immutable conversion tables: one character map plus phrase lists
/// bucketed by their first character and longest-first.
#[derive(Debug, Clone, Default)]
pub struct Simplifier {
    characters: HashMap<char, char>,
    phrases: HashMap<char, Vec<(String, String)>>,
}

/// Loads the bundled OpenCC tables, failing only if the vendored data is
/// malformed.
pub fn bundled() -> Result<Simplifier, String> {
    from_sources(TS_CHARACTERS, TS_PHRASES)
}

/// Parses OpenCC `key<TAB>value(s)` tables. Kept separate so tests can
/// exercise validation without touching the vendored files.
pub fn from_sources(characters: &str, phrases: &str) -> Result<Simplifier, String> {
    let mut char_map = HashMap::new();
    for (line_no, key, value) in records(characters, "TSCharacters")? {
        let Some(from) = single_char(key) else {
            return Err(format!(
                "TSCharacters line {line_no}: key '{key}' is not one character"
            ));
        };
        let Some(to) = single_char(value) else {
            return Err(format!(
                "TSCharacters line {line_no}: value '{value}' is not one character"
            ));
        };
        char_map.insert(from, to);
    }
    let mut phrase_map: HashMap<char, Vec<(String, String)>> = HashMap::new();
    for (line_no, key, value) in records(phrases, "TSPhrases")? {
        let Some(first) = key.chars().next() else {
            return Err(format!("TSPhrases line {line_no}: empty key"));
        };
        if key.chars().count() < 2 {
            return Err(format!(
                "TSPhrases line {line_no}: key '{key}' is shorter than two characters"
            ));
        }
        phrase_map
            .entry(first)
            .or_default()
            .push((key.to_string(), value.to_string()));
    }
    for list in phrase_map.values_mut() {
        list.sort_by(|a, b| {
            b.0.chars()
                .count()
                .cmp(&a.0.chars().count())
                .then_with(|| a.0.cmp(&b.0))
        });
    }
    Ok(Simplifier {
        characters: char_map,
        phrases: phrase_map,
    })
}

/// Iterates over usable `(line number, key, first value)` records.
fn records<'a>(
    source: &'a str,
    table: &str,
) -> Result<impl Iterator<Item = (usize, &'a str, &'a str)> + 'a, String> {
    let mut parsed = Vec::new();
    for (index, raw_line) in source.lines().enumerate() {
        let line = raw_line.trim_end();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line_no = index + 1;
        let Some((key, values)) = line.split_once('\t') else {
            return Err(format!("{table} line {line_no}: missing tab separator"));
        };
        if key.is_empty() {
            return Err(format!("{table} line {line_no}: empty key"));
        }
        let Some(value) = values.split_whitespace().next() else {
            return Err(format!("{table} line {line_no}: empty value list"));
        };
        parsed.push((line_no, key, value));
    }
    Ok(parsed.into_iter())
}

fn single_char(text: &str) -> Option<char> {
    let mut chars = text.chars();
    match (chars.next(), chars.next()) {
        (Some(ch), None) => Some(ch),
        _ => None,
    }
}

/// Converts one hanzi string. Ordinary ASCII and non-hanzi characters are
/// copied unchanged.
pub fn simplify_text(simplifier: &Simplifier, text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(first) = rest.chars().next() {
        let phrase = simplifier
            .phrases
            .get(&first)
            .and_then(|list| list.iter().find(|(key, _)| rest.starts_with(key.as_str())));
        if let Some((key, value)) = phrase {
            out.push_str(value);
            rest = &rest[key.len()..];
        } else {
            out.push(simplifier.characters.get(&first).copied().unwrap_or(first));
            rest = &rest[first.len_utf8()..];
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_tables_convert_common_words() {
        let s = bundled().expect("vendored tables parse");
        for (traditional, simplified) in [
            ("今天天氣不錯", "今天天气不错"),
            ("頭髮", "头发"),
            ("後臺", "后台"),
            ("發展", "发展"),
            ("乾隆", "乾隆"),
            ("乾杯", "干杯"),
            ("你", "你"),
            ("abc", "abc"),
        ] {
            assert_eq!(simplify_text(&s, traditional), simplified, "{traditional}");
        }
    }

    #[test]
    fn longest_phrase_wins_over_character_map() {
        let s = from_sources("乾\t干\n", "乾隆\t乾隆\n乾杯\t干杯\n").expect("valid custom tables");
        assert_eq!(simplify_text(&s, "乾隆乾杯"), "乾隆干杯");
    }

    #[test]
    fn asr_style_transcripts_normalize() {
        let s = bundled().unwrap();
        assert_eq!(
            simplify_text(&s, "今天天氣很好,我們一起去公園三步吧!"),
            "今天天气很好,我们一起去公园三步吧!"
        );
    }

    #[test]
    fn malformed_tables_are_rejected_with_line_numbers() {
        let err = from_sources("繁 体\n", "").unwrap_err();
        assert!(err.contains("TSCharacters line 1"), "{err}");
        let err = from_sources("", "繁\t体\n").unwrap_err();
        assert!(err.contains("TSPhrases line 1"), "{err}");
        let err = from_sources("繁\t繁体\n", "").unwrap_err();
        assert!(err.contains("value '繁体' is not one character"), "{err}");
    }
}
