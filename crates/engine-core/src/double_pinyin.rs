//! Double-pinyin framework (P1/M2a): a loadable key table plus the
//! typed-code -> full-pinyin transform.
//!
//! **Data gap, on purpose:** the real scheme tables (自然码/小鹤/微软) are
//! factual key-layout data that must not be reproduced from memory; the
//! network being down, they are *not* bundled. A double-pinyin session
//! therefore loads its table from host-supplied text via [`parse_tsv`]
//! (roadmap: import the real tables into `data/` once network access is
//! restored and feed them through this same loader).
//!
//! Table format (TSV, one rule per line, `#` comments and blanks skipped):
//!
//! ```text
//! iang<TAB>d      # final "iang" is typed with key d in the final slot
//! 0a<TAB>o        # zero-initial syllable "a" uses key o in the initial slot
//! ```
//!
//! Decode contract of [`to_full`]: the typed string is even-length and
//! consumed as (initial-code, final-code) pairs per syllable. For each
//! pair, a zero-initial match wins first (the initial code is a zero key
//! and the final code resolves to the same syllable); otherwise the
//! initial code is read as the initial letter itself and joined with every
//! final keyed by the second code — the lexicographically smallest result
//! that passes [`crate::syllable::is_syllable`] is used. Any unresolvable
//! pair, odd length, or invalid produced syllable makes the whole
//! transform return `None`.

use crate::syllable::is_syllable;

/// A loaded double-pinyin table: pure data, no scheme logic.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DpTable {
    /// Final-slot key character -> finals typed with it (several finals
    /// may share one key; disambiguated by syllable validity).
    finals: Vec<(char, Vec<String>)>,
    /// Initial-slot key character -> the zero-initial syllable it stands
    /// for (rows written `0a<TAB>o` in the table text).
    zero_initial: Vec<(char, String)>,
}

/// Parses table text into a [`DpTable`]. Rows are `final<TAB>key` (one
/// final per row, key = a single ASCII character); zero-initial rows are
/// `0syllable<TAB>key` where `syllable` must itself be a valid syllable.
/// Malformed rows fail with their 1-based line number; re-declaring a
/// zero key for a different syllable is an error, exact duplicates are
/// accepted.
pub fn parse_tsv(text: &str) -> Result<DpTable, String> {
    let mut table = DpTable::default();
    for (idx, line) in text.lines().enumerate() {
        let lineno = idx + 1;
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let mut fields = trimmed.split('\t');
        let left = fields.next().unwrap_or("");
        let key = fields.next().unwrap_or("");
        if left.is_empty() || fields.next().is_some() || !single_ascii_key(key) {
            return Err(format!(
                "line {lineno}: expected 'final<TAB>key' or '0syllable<TAB>key', got '{trimmed}'"
            ));
        }
        let key_char = key.chars().next().expect("validated above");
        if let Some(syllable) = left.strip_prefix('0') {
            if !is_syllable(syllable) {
                return Err(format!(
                    "line {lineno}: zero-initial '{syllable}' is not a syllable"
                ));
            }
            match table.zero_initial.iter_mut().find(|(k, _)| *k == key_char) {
                Some((_, existing)) if existing != syllable => {
                    return Err(format!(
                        "line {lineno}: zero key '{key_char}' already maps to '{existing}'"
                    ));
                }
                Some(_) => {}
                None => table.zero_initial.push((key_char, syllable.to_string())),
            }
        } else {
            let finals = table
                .finals
                .iter_mut()
                .find(|(k, _)| *k == key_char)
                .map(|(_, finals)| finals);
            match finals {
                Some(finals) if !finals.iter().any(|f| f == left) => finals.push(left.to_string()),
                Some(_) => {}
                None => table.finals.push((key_char, vec![left.to_string()])),
            }
        }
    }
    Ok(table)
}

/// Decodes a double-pinyin string into a `'`-joined full-pinyin key
/// (see the module docs for the pair contract and tie-breaking). Returns
/// `None` unless every produced syllable is a valid pinyin syllable.
pub fn to_full(table: &DpTable, typed: &str) -> Option<String> {
    let chars: Vec<char> = typed.chars().collect();
    if chars.is_empty() || !chars.len().is_multiple_of(2) {
        return None;
    }
    let mut syllables: Vec<String> = Vec::with_capacity(chars.len() / 2);
    for pair in chars.as_chunks::<2>().0 {
        syllables.push(decode_pair(table, pair[0], pair[1])?);
    }
    Some(syllables.join("'"))
}

/// Decodes one (initial-code, final-code) pair; `None` when no zero-initial
/// or initial-letter interpretation yields a valid syllable.
fn decode_pair(table: &DpTable, initial_key: char, final_key: char) -> Option<String> {
    let finals = finals_of(table, final_key)?;
    // zero-initial first: initial key is a zero key and the final key
    // resolves to the very syllable that key stands for.
    if let Some((_, syllable)) = table.zero_initial.iter().find(|(k, _)| *k == initial_key) {
        if finals.iter().any(|f| f == syllable) {
            return Some(syllable.clone());
        }
    }
    // otherwise the initial code is the initial letter itself; pick the
    // smallest valid syllable deterministically.
    let mut candidates: Vec<String> = finals
        .iter()
        .map(|f| format!("{initial_key}{f}"))
        .filter(|s| is_syllable(s))
        .collect();
    candidates.sort();
    candidates.into_iter().next()
}

/// Finals keyed by `key`, in table order; `None` when the key is unknown.
fn finals_of(table: &DpTable, key: char) -> Option<&[String]> {
    table
        .finals
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, finals)| finals.as_slice())
}

/// A table key must be exactly one ASCII lowercase letter (real schemes
/// only ever key finals onto letters; `;` style keys stay out of scope
/// until the real tables are imported).
fn single_ascii_key(key: &str) -> bool {
    let mut chars = key.chars();
    matches!(chars.next(), Some('a'..='z')) && chars.next().is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Self-made 8-row fixture. Deliberately NOT the key layout of any
    /// real double-pinyin scheme — it only exercises the framework.
    const FIXTURE: &str = "# self-made fixture table (not a real scheme)\n\
                           in\tl\n\
                           ian\tj\n\
                           i\ti\n\
                           a\ta\n\
                           ao\tq\n\
                           uang\tx\n\
                           0a\to\n\
                           0e\tw\n";

    fn table() -> DpTable {
        parse_tsv(FIXTURE).expect("fixture table must parse")
    }

    #[test]
    fn parse_builds_final_and_zero_maps() {
        let t = table();
        assert_eq!(finals_of(&t, 'l'), Some(&["in".to_string()][..]));
        assert_eq!(finals_of(&t, 'q'), Some(&["ao".to_string()][..]));
        assert_eq!(t.zero_initial.len(), 2);
        assert!(t.zero_initial.iter().any(|(k, s)| *k == 'o' && s == "a"));
        assert_eq!(finals_of(&t, 'z'), None);
    }

    #[test]
    fn parse_rejects_bad_rows_with_line_numbers() {
        let err = parse_tsv("in\n").unwrap_err();
        assert!(err.contains("line 1"), "err: {err}");
        let err = parse_tsv("in\tll\n").unwrap_err();
        assert!(err.contains("line 1"), "err: {err}");
        let err = parse_tsv("# c\n\nin\tl\n0xx\to\n").unwrap_err();
        assert!(
            err.contains("line 4") && err.contains("syllable"),
            "err: {err}"
        );
        let err = parse_tsv("0a\to\n0e\to\n").unwrap_err();
        assert!(
            err.contains("line 2") && err.contains("zero key"),
            "err: {err}"
        );
        // exact duplicates are fine
        assert!(parse_tsv("in\tl\nin\tl\n").is_ok());
    }

    #[test]
    fn to_full_decodes_fixture_pairs() {
        let t = table();
        // j + l = "jin", t + j = "tian"
        assert_eq!(to_full(&t, "jltj").as_deref(), Some("jin'tian"));
        // n + i = "ni", h + a = "ha"
        assert_eq!(to_full(&t, "niha").as_deref(), Some("ni'ha"));
        // zero-initial: o marks "a", second code must resolve to "a" too
        assert_eq!(to_full(&t, "oa").as_deref(), Some("a"));
        assert_eq!(to_full(&t, "oaoa").as_deref(), Some("a'a"));
    }

    #[test]
    fn to_full_returns_none_on_invalid_syllables() {
        let t = table();
        // b + uang = "buang" is not a syllable
        assert_eq!(to_full(&t, "bx"), None);
        // zero key whose final code disagrees
        assert_eq!(to_full(&t, "oq"), None);
        // unknown final key
        assert_eq!(to_full(&t, "jz"), None);
        // odd length / empty
        assert_eq!(to_full(&t, "jlt"), None);
        assert_eq!(to_full(&t, ""), None);
    }

    #[test]
    fn to_full_picks_the_smallest_valid_candidate_deterministically() {
        // one key holding two finals: "i" and "in"-style ambiguity is
        // resolved by syllable validity, then lexicographic order.
        let t = parse_tsv("i\ti\nin\ti\n0a\to\na\ta\n").expect("ambiguous table");
        // n + i-key -> {"ni", "nin"}: both valid, "ni" < "nin"
        assert_eq!(to_full(&t, "ni").as_deref(), Some("ni"));
    }

    #[test]
    fn empty_table_rejects_everything() {
        let t = DpTable::default();
        assert_eq!(to_full(&t, "ni"), None);
        assert_eq!(to_full(&t, ""), None);
    }
}
