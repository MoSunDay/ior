//! Double-pinyin framework (P1/M2a): a loadable key table plus the
//! typed-code -> full-pinyin transform.
//!
//! The real scheme tables (自然碼/小鶴/微軟) live in
//! `data/double-pinyin/{natural,flypy,mspy}.tsv`, converted from the
//! rime schema algebra by `scripts/convert-rime-double-pinyin.py`; the
//! roundtrip tests below load them verbatim. A session can also load a
//! custom table from host-supplied text via [`parse_tsv`].
//!
//! Table format (TSV, one rule per line, `#` comments and blanks skipped):
//!
//! ```text
//! iang<TAB>d      # final "iang" is typed with key d in the final slot
//! zh<TAB>v        # the zh initial is typed with key v in the initial slot
//! 0a<TAB>o        # zero-initial syllable "a" uses key o in the initial slot
//! ```
//!
//! Keys are one ASCII lowercase letter or `;` (微软雙拼 types `ing` as
//! `;`). A final may appear on several keys (scheme aliases) and one
//! zero key may carry several syllables (e.g. 自然碼 types `e`, `ei`,
//! `en`, `eng`, `er` all with an `e` initial slot), disambiguated by the
//! final key.
//!
//! Decode contract of [`to_full`]: the typed string is even-length and
//! consumed as (initial-code, final-code) pairs per syllable. For each
//! pair the first match wins in this order: (1) a zero-initial entry
//! whose syllable is keyed by the final code, (2) a multi-letter initial
//! row (`zh`/`ch`/`sh`), (3) the initial letter itself. Candidates are
//! filtered through [`crate::syllable::is_syllable`]; the
//! lexicographically smallest survivor is used. Any unresolvable pair,
//! odd length, or invalid produced syllable makes the whole transform
//! return `None`.

use crate::config::DoublePinyin;
use crate::syllable::is_syllable;

/// A loaded double-pinyin table: pure data, no scheme logic.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DpTable {
    /// Final-slot key character -> finals typed with it (several finals
    /// may share one key; disambiguated by syllable validity).
    finals: Vec<(char, Vec<String>)>,
    /// Initial-slot key character -> the zero-initial syllable it stands
    /// for (rows written `0a<TAB>o` in the table text). Several syllables
    /// may share one key.
    zero_initial: Vec<(char, String)>,
    /// Multi-letter initial (zh/ch/sh) -> its initial-slot key character
    /// (rows written `zh<TAB>v` in the table text).
    initials: Vec<(String, char)>,
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
                "line {lineno}: expected 'final<TAB>key', 'zh|ch|sh<TAB>key' or \
                 '0syllable<TAB>key', got '{trimmed}'"
            ));
        }
        let key_char = key.chars().next().expect("validated above");
        if let Some(syllable) = left.strip_prefix('0') {
            if !is_syllable(syllable) {
                return Err(format!(
                    "line {lineno}: zero-initial '{syllable}' is not a syllable"
                ));
            }
            let pair = (key_char, syllable.to_string());
            if !table.zero_initial.contains(&pair) {
                table.zero_initial.push(pair);
            }
        } else if is_multi_letter_initial(left) {
            match table.initials.iter().find(|(name, _)| name == left) {
                Some((_, existing)) if *existing != key_char => {
                    return Err(format!(
                        "line {lineno}: initial '{left}' already keyed as '{existing}'"
                    ));
                }
                Some(_) => {}
                None => table.initials.push((left.to_string(), key_char)),
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

/// The real scheme tables converted from the rime schema algebra by
/// `scripts/convert-rime-double-pinyin.py`. Bundled via `include_str!`
/// so the engine stays I/O-free; parse failures are impossible for the
/// checked-in files (the roundtrip tests below load them verbatim).
pub fn builtin(scheme: DoublePinyin) -> Option<DpTable> {
    let text = match scheme {
        DoublePinyin::Natural => Some(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../data/double-pinyin/natural.tsv"
        ))),
        DoublePinyin::Xiaohe => Some(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../data/double-pinyin/flypy.tsv"
        ))),
        DoublePinyin::Mspy => Some(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../data/double-pinyin/mspy.tsv"
        ))),
        DoublePinyin::Off | DoublePinyin::Custom => None,
    }?;
    Some(parse_tsv(text).expect("bundled double-pinyin table must parse"))
}

/// Builtin tables are the same three checked-in files the tests verify.
#[cfg(test)]
mod builtin_tests {
    use super::*;

    #[test]
    fn builtin_schemes_parse_and_decode() {
        // ni hao: 自然碼 "nihk", 小鶴 "nihc", 微軟 "nihk"
        for (scheme, hao) in [
            (DoublePinyin::Natural, "nihk"),
            (DoublePinyin::Xiaohe, "nihc"),
            (DoublePinyin::Mspy, "nihk"),
        ] {
            let table = builtin(scheme).unwrap_or_else(|| panic!("{scheme:?}"));
            assert_eq!(to_full(&table, hao).as_deref(), Some("ni'hao"));
        }
        assert!(builtin(DoublePinyin::Off).is_none());
        assert!(builtin(DoublePinyin::Custom).is_none());
    }
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
    // zero-initial first: initial key is a zero key and one of its
    // syllables is keyed by the final code.
    let mut zero: Vec<&String> = table
        .zero_initial
        .iter()
        .filter(|(k, syllable)| *k == initial_key && finals.iter().any(|f| f == syllable))
        .map(|(_, syllable)| syllable)
        .collect();
    zero.sort();
    if let Some(syllable) = zero.into_iter().next() {
        return Some(syllable.clone());
    }
    // multi-letter initials (zh/ch/sh) are typed with their own key.
    if let Some((initial, _)) = table.initials.iter().find(|(_, k)| *k == initial_key) {
        let mut candidates: Vec<String> = finals
            .iter()
            .map(|f| format!("{initial}{f}"))
            .filter(|s| is_syllable(s))
            .collect();
        candidates.sort();
        return candidates.into_iter().next();
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

/// A table key must be exactly one ASCII lowercase letter or `;` (the
/// key 微软雙拼 uses for the `ing` final).
fn single_ascii_key(key: &str) -> bool {
    let mut chars = key.chars();
    matches!(chars.next(), Some('a'..='z' | ';')) && chars.next().is_none()
}

/// The initials that need an explicit row because they are typed as one
/// key of their own (zh/ch/sh); single-letter initials are the key itself.
fn is_multi_letter_initial(name: &str) -> bool {
    matches!(name, "zh" | "ch" | "sh")
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
        // one zero key may carry several syllables (real schemes do this)
        let t = parse_tsv("0a\to\n0e\to\n").expect("shared zero key");
        assert_eq!(t.zero_initial.len(), 2);
        let err = parse_tsv("zh\tv\nzh\tw\n").unwrap_err();
        assert!(
            err.contains("line 2") && err.contains("initial 'zh'"),
            "err: {err}"
        );
        // exact duplicates are fine
        assert!(parse_tsv("in\tl\nin\tl\n0a\to\n0a\to\n").is_ok());
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
    fn initial_rows_decode_zh_ch_sh_and_semicolon_keys() {
        let t = parse_tsv("an\tj\nzh\tv\ning\t;\nb\tb\n").expect("initial table");
        // v + j = zhan; b + ; = bing
        assert_eq!(to_full(&t, "vj").as_deref(), Some("zhan"));
        assert_eq!(to_full(&t, "b;").as_deref(), Some("bing"));
        // unknown initial key that is not a letter-initial
        assert_eq!(to_full(&t, "uj"), None);
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
    // ---- real scheme tables (converted from rime schema algebra) ----

    const NATURAL: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../data/double-pinyin/natural.tsv"
    ));
    const FLYPY: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../data/double-pinyin/flypy.tsv"
    ));
    const MSPY: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../data/double-pinyin/mspy.tsv"
    ));

    /// Every typed code for `syll` under `t`: initial-slot key (initial
    /// row, the letter itself, or any zero key) crossed with every
    /// final-slot key carrying the syllable's final.
    fn codes_for(t: &DpTable, syll: &str) -> Vec<String> {
        let initial = ["zh", "ch", "sh"]
            .iter()
            .find(|i| syll.starts_with(*i))
            .map_or_else(
                || {
                    let first = syll.chars().next().expect("non-empty");
                    if "bpmfdtnlgkhjqxrzcsyw".contains(first) {
                        first.to_string()
                    } else {
                        String::new()
                    }
                },
                |i| (*i).to_string(),
            );
        let fin = &syll[initial.len()..];
        let mut final_keys: Vec<char> = t
            .finals
            .iter()
            .filter(|(_, finals)| finals.iter().any(|f| f == fin))
            .map(|(k, _)| *k)
            .collect();
        final_keys.sort_unstable();
        let initial_keys: Vec<char> = if initial.is_empty() {
            let mut keys: Vec<char> = t
                .zero_initial
                .iter()
                .filter(|(_, s)| s == syll)
                .map(|(k, _)| *k)
                .collect();
            keys.sort_unstable();
            keys
        } else {
            match t.initials.iter().find(|(name, _)| name == &initial) {
                Some((_, k)) => vec![*k],
                None => vec![initial.chars().next().expect("single-letter initial")],
            }
        };
        initial_keys
            .into_iter()
            .flat_map(|i| final_keys.iter().map(move |f| format!("{i}{f}")))
            .collect()
    }

    /// Roundtrips every syllable through its scheme code(s). Codes that
    /// are genuinely shared by two syllables (scheme-inherent collisions
    /// such as 自然碼's `lo`/`luo` both being typed `lo`) must decode to
    /// one of the colliding syllables and stay a fixed point.
    fn roundtrip_scheme(label: &str, text: &str, expected_collisions: &[(&str, &str)]) {
        let t = parse_tsv(text).unwrap_or_else(|err| panic!("{label}: {err}"));
        let mut collisions = Vec::new();
        for syll in crate::syllable::SYLLABLES {
            let codes = codes_for(&t, syll);
            assert!(!codes.is_empty(), "{label}: no code for {syll}");
            for code in codes {
                let decoded = to_full(&t, &code)
                    .unwrap_or_else(|| panic!("{label}: '{code}' for {syll} does not decode"));
                if decoded == *syll {
                    continue;
                }
                // collision: decoded must be another syllable sharing the code
                assert!(
                    codes_for(&t, &decoded).contains(&code),
                    "{label}: '{code}' ({syll}) decoded to unrelated '{decoded}'"
                );
                collisions.push(((*syll).to_string(), decoded));
            }
        }
        collisions.sort();
        collisions.dedup();
        let expected: Vec<(String, String)> = expected_collisions
            .iter()
            .map(|(a, b)| ((*a).to_string(), (*b).to_string()))
            .collect();
        assert_eq!(collisions, expected, "{label}: collision set drifted");
    }

    #[test]
    fn natural_table_roundtrips_every_syllable() {
        roundtrip_scheme("natural", NATURAL, &[("luo", "lo")]);
    }

    #[test]
    fn flypy_table_roundtrips_every_syllable() {
        roundtrip_scheme("flypy", FLYPY, &[("luo", "lo")]);
    }

    #[test]
    fn mspy_table_roundtrips_every_syllable() {
        roundtrip_scheme("mspy", MSPY, &[("luo", "lo")]);
    }
}
