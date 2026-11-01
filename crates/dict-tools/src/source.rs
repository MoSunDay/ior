//! Source ingestion for `compile` and `validate`: auto-detects the two
//! supported input formats and applies structural validation with 1-based
//! line numbers in every error.
//!
//! - **ior TSV**: `pinyin<TAB>hanzi[<TAB>freq]`, pinyin first.
//! - **rime dict.yaml**: a line containing exactly `...` opens the table;
//!   everything before it is skipped, each later line is
//!   `hanzi<TAB>pinyin[<TAB>weight]`, hanzi first. Columns split on tabs
//!   first; lines without tabs fall back to rime's space-separated style.
//!
//! Validation rules (applied to the canonicalized pinyin, i.e. after
//! lowercasing and separator normalization): pinyin may only contain
//! `a..z` and `'`, with no leading/trailing or consecutive separators and
//! no empty syllables; hanzi is non-empty and free of tabs/newlines;
//! `freq >= 1`. Duplicate `(pinyin, hanzi)` pairs are aggregated by a later
//! `normalize` call, not rejected, but they are counted and reported.

use crate::format::{canonical_pinyin, Entry};
use std::collections::BTreeSet;

/// Marker line that opens the table section of a rime dict.yaml.
const RIME_TABLE_MARKER: &str = "...";

/// Syllables per row when printing `--dump-syllables` output.
const SYLLABLES_PER_ROW: usize = 12;

/// Which of the two source formats an input was detected as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceFormat {
    /// `pinyin<TAB>hanzi[<TAB>freq]` (pinyin first).
    IorTsv,
    /// rime dict.yaml table section (hanzi first).
    RimeDictYaml,
}

impl SourceFormat {
    /// Human-readable label used in command reports.
    pub fn name(self) -> &'static str {
        match self {
            SourceFormat::IorTsv => "ior-tsv",
            SourceFormat::RimeDictYaml => "rime-dict.yaml",
        }
    }
}

/// A parsed and validated source file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    /// Detected input format.
    pub format: SourceFormat,
    /// Validated entries (pinyin canonicalized, duplicates still present).
    pub entries: Vec<Entry>,
    /// How many `(pinyin, hanzi)` pairs appeared more than once.
    pub duplicate_pairs: usize,
    /// Unique syllables seen across all entries, sorted ascending.
    pub syllables: Vec<String>,
}

/// True when `text` contains a standalone `...` line (rime table marker).
pub fn is_rime_dict(text: &str) -> bool {
    text.lines().any(|line| line.trim() == RIME_TABLE_MARKER)
}

/// Parse and validate `text`, auto-detecting the format.
pub fn load_source(text: &str) -> Result<Source, String> {
    if is_rime_dict(text) {
        load_rime(text)
    } else {
        load_tsv(text)
    }
}

/// Parse and validate an ior TSV source (pinyin first).
fn load_tsv(text: &str) -> Result<Source, String> {
    let mut collector = Collector::new();
    for (index, raw_line) in text.lines().enumerate() {
        let lineno = index + 1;
        let trimmed = raw_line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = raw_line.split('\t').map(str::trim).collect();
        if fields.len() < 2 {
            return Err(format!(
                "line {lineno}: expected 'pinyin<TAB>hanzi[<TAB>freq]', got '{trimmed}'"
            ));
        }
        if fields.len() > 3 {
            return Err(format!(
                "line {lineno}: {} fields, expected at most 'pinyin<TAB>hanzi<TAB>freq'",
                fields.len()
            ));
        }
        let pinyin = canonical_pinyin(fields[0]);
        let freq = parse_freq(fields.get(2).copied(), lineno)?;
        validate_raw_pinyin(fields[0], lineno)?;
        validate_entry(&pinyin, fields[1], freq, lineno)?;
        collector.push(pinyin, fields[1].to_string(), freq);
    }
    Ok(collector.finish(SourceFormat::IorTsv))
}

/// Parse and validate a rime dict.yaml source (hanzi first, after `...`).
fn load_rime(text: &str) -> Result<Source, String> {
    let mut collector = Collector::new();
    let mut in_table = false;
    for (index, raw_line) in text.lines().enumerate() {
        let lineno = index + 1;
        if !in_table {
            if raw_line.trim() == RIME_TABLE_MARKER {
                in_table = true;
            }
            continue;
        }
        let trimmed = raw_line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let fields = split_columns(raw_line);
        if fields.len() < 2 {
            return Err(format!(
                "line {lineno}: expected 'hanzi<TAB>pinyin[<TAB>weight]', got '{trimmed}'"
            ));
        }
        if fields.len() > 3 {
            return Err(format!(
                "line {lineno}: {} columns, expected at most 'hanzi<TAB>pinyin<TAB>weight'",
                fields.len()
            ));
        }
        let hanzi = fields[0];
        let pinyin = canonical_pinyin(fields[1]);
        let freq = parse_freq(fields.get(2).copied(), lineno)?;
        validate_raw_pinyin(fields[1], lineno)?;
        validate_entry(&pinyin, hanzi, freq, lineno)?;
        collector.push(pinyin, hanzi.to_string(), freq);
    }
    if !in_table {
        return Err(format!("no '{RIME_TABLE_MARKER}' table marker line found"));
    }
    Ok(collector.finish(SourceFormat::RimeDictYaml))
}

/// Split one rime row into columns: tabs win, whitespace otherwise.
fn split_columns(line: &str) -> Vec<&str> {
    if line.contains('\t') {
        line.split('\t').map(str::trim).collect()
    } else {
        line.split_whitespace().collect()
    }
}

/// Parse the optional frequency/weight column.
fn parse_freq(raw: Option<&str>, lineno: usize) -> Result<u32, String> {
    match raw {
        None => Ok(1),
        Some(field) => field
            .parse::<u32>()
            .map_err(|err| format!("line {lineno}: invalid freq '{field}': {err}")),
    }
}

/// Structural validation of the raw pinyin field: separator shape must be
/// sane *before* canonicalization, which would silently repair it.
pub fn validate_raw_pinyin(raw: &str, lineno: usize) -> Result<(), String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(format!("line {lineno}: empty pinyin"));
    }
    if trimmed.starts_with('\'') || trimmed.ends_with('\'') {
        return Err(format!(
            "line {lineno}: pinyin '{trimmed}' has a leading/trailing separator"
        ));
    }
    if trimmed.contains("''") {
        return Err(format!(
            "line {lineno}: pinyin '{trimmed}' has consecutive separators"
        ));
    }
    Ok(())
}

/// Structural validation of one (canonical pinyin, hanzi, freq) triple.
pub fn validate_entry(pinyin: &str, hanzi: &str, freq: u32, lineno: usize) -> Result<(), String> {
    if pinyin.is_empty() {
        return Err(format!("line {lineno}: empty pinyin"));
    }
    if let Some(bad) = pinyin
        .chars()
        .find(|c| !c.is_ascii_lowercase() && *c != '\'')
    {
        return Err(format!(
            "line {lineno}: pinyin '{pinyin}' has invalid character '{bad}' (allowed: a-z and ')"
        ));
    }
    if pinyin.split('\'').any(str::is_empty) {
        return Err(format!(
            "line {lineno}: pinyin '{pinyin}' has an empty syllable"
        ));
    }
    if hanzi.is_empty() {
        return Err(format!("line {lineno}: empty hanzi"));
    }
    if hanzi.contains('\t') || hanzi.contains('\n') || hanzi.contains('\r') {
        return Err(format!(
            "line {lineno}: hanzi must not contain tabs or newlines"
        ));
    }
    if freq < 1 {
        return Err(format!("line {lineno}: freq {freq} must be >= 1"));
    }
    Ok(())
}

/// Accumulates entries, duplicate counts, and the syllable inventory.
struct Collector {
    entries: Vec<Entry>,
    seen: BTreeSet<(String, String)>,
    duplicate_pairs: usize,
    syllables: BTreeSet<String>,
}

impl Collector {
    fn new() -> Collector {
        Collector {
            entries: Vec::new(),
            seen: BTreeSet::new(),
            duplicate_pairs: 0,
            syllables: BTreeSet::new(),
        }
    }

    fn push(&mut self, pinyin: String, hanzi: String, freq: u32) {
        for syllable in pinyin.split('\'') {
            self.syllables.insert(syllable.to_string());
        }
        if !self.seen.insert((pinyin.clone(), hanzi.clone())) {
            self.duplicate_pairs += 1;
        }
        self.entries.push(Entry {
            pinyin,
            hanzi,
            freq,
        });
    }

    fn finish(self, format: SourceFormat) -> Source {
        Source {
            format,
            entries: self.entries,
            duplicate_pairs: self.duplicate_pairs,
            syllables: self.syllables.into_iter().collect(),
        }
    }
}

/// Render the `--dump-syllables` block (sorted list + provenance note).
pub fn render_syllables(syllables: &[String]) -> String {
    let mut out = format!("syllables ({}):\n", syllables.len());
    for chunk in syllables.chunks(SYLLABLES_PER_ROW) {
        out.push_str("  ");
        out.push_str(&chunk.join(" "));
        out.push('\n');
    }
    out.push_str(
        "note: dict-tools does not embed the engine-core syllable table;\n\
         \x20     compare this list against it manually\n",
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const KAI: &str = "\u{5f00}\u{53d1}"; // kai fa
    const CE: &str = "\u{6d4b}\u{8bd5}"; // ce shi

    #[test]
    fn detects_rime_by_marker_line() {
        assert!(is_rime_dict("# Rime dictionary\n...\nkai\tkai\n"));
        assert!(!is_rime_dict("kai'fa\tkai\n"));
    }

    #[test]
    fn loads_tsv_with_canonical_pinyin() {
        let source = load_source(&format!("KAI FA\t{KAI}\t500\nce'shi\t{CE}\n")).unwrap();
        assert_eq!(source.format, SourceFormat::IorTsv);
        assert_eq!(source.entries.len(), 2);
        assert_eq!(source.entries[0].pinyin, "kai'fa");
        assert_eq!(source.entries[0].freq, 500);
        assert_eq!(source.entries[1].freq, 1);
        assert_eq!(source.syllables, vec!["ce", "fa", "kai", "shi"]);
    }

    #[test]
    fn loads_rime_table_and_skips_header() {
        let jia = "\u{7532}"; // jia
        let text = format!(
            "# Rime dictionary\nname: luna_pinyin\nversion: \"0.9\"\n...\n\
             {KAI}\tkai fa\t500\n{CE}\tce'shi\n{jia} kai\n"
        );
        let source = load_source(&text).unwrap();
        assert_eq!(source.format, SourceFormat::RimeDictYaml);
        assert_eq!(source.entries.len(), 3);
        assert_eq!(source.entries[0].hanzi, KAI);
        assert_eq!(source.entries[0].pinyin, "kai'fa");
        assert_eq!(source.entries[0].freq, 500);
        // space-separated rime row (no tabs at all)
        assert_eq!(source.entries[2].hanzi, jia);
        assert_eq!(source.entries[2].pinyin, "kai");
        assert_eq!(source.entries[2].freq, 1);
    }

    #[test]
    fn counts_duplicates_instead_of_erroring() {
        let source = load_source(&format!("a\t{KAI}\t1\na\t{KAI}\t2\n")).unwrap();
        assert_eq!(source.duplicate_pairs, 1);
        assert_eq!(source.entries.len(), 2);
    }

    #[test]
    fn validation_rejects_bad_pinyin_shapes_with_line_numbers() {
        let cases = [
            ("\u{fc}\t{KAI}\t1", "invalid character"),
            ("ni''hao\t{KAI}\t1", "consecutive separators"),
            ("'ni\t{KAI}\t1", "leading/trailing separator"),
            ("ni'\t{KAI}\t1", "leading/trailing separator"),
            ("\t{KAI}\t1", "empty pinyin"),
        ];
        for (text, expected) in cases {
            let err = load_source(&format!("{text}\n")).unwrap_err();
            assert!(err.contains("line 1"), "case '{text}': {err}");
            assert!(err.contains(expected), "case '{text}': {err}");
        }
    }

    #[test]
    fn validation_rejects_bad_freq_and_hanzi() {
        assert!(load_source(&format!("a\t{KAI}\t0\n"))
            .unwrap_err()
            .contains("must be >= 1"));
        assert!(load_source("a\t\t5\n").unwrap_err().contains("empty hanzi"));
        assert!(load_source("a\t{KAI}\tx\n")
            .unwrap_err()
            .contains("invalid freq"));
        assert!(load_source("...\n\tkai\n").unwrap_err().contains("line 2"));
    }

    #[test]
    fn rime_without_marker_falls_back_to_tsv_parsing() {
        // No `...` line: detected as ior TSV, whose column order then fails.
        let err = load_source("name: luna_pinyin\nkai\tkai\n").unwrap_err();
        assert!(err.contains("line 1"), "{err}");
        // Direct rime parsing without a marker is explicit about it.
        let err = load_rime("kai\tkai\n").unwrap_err();
        assert!(err.contains("table marker"), "{err}");
    }

    #[test]
    fn syllable_dump_is_rowed_and_noted() {
        let rendered = render_syllables(&["a".to_string(), "ai".to_string()]);
        assert!(rendered.contains("syllables (2):"));
        assert!(rendered.contains("a ai"));
        assert!(rendered.contains("engine-core syllable table"));
    }
}
