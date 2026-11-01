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
use crate::preset::{PresetVocab, WEIGHT_SCALE};
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

/// Rime dict.yaml header settings that affect compilation.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RimeSettings {
    /// `use_preset_vocabulary: true` in the dict header.
    pub use_preset_vocabulary: bool,
    /// `vocabulary: essay` (defaults to `essay` when enabled).
    pub vocabulary: Option<String>,
    /// `max_phrase_length` (0 = Rime's hard cap of 32 characters).
    pub max_phrase_length: usize,
    /// `min_phrase_weight` (0 = keep every positive preset weight).
    pub min_phrase_weight: f64,
}

impl RimeSettings {
    /// Rime enables the preset vocabulary when the flag is set *or* an
    /// explicit `vocabulary:` name is present.
    pub fn preset_enabled(&self) -> bool {
        self.use_preset_vocabulary || self.vocabulary.is_some()
    }

    /// Vocabulary file stem, `essay` unless overridden.
    pub fn vocabulary_name(&self) -> &str {
        self.vocabulary.as_deref().unwrap_or("essay")
    }
}

/// True when `text` contains a standalone `...` line (rime table marker).
pub fn is_rime_dict(text: &str) -> bool {
    text.lines().any(|line| line.trim() == RIME_TABLE_MARKER)
}

/// Reads the simple `key: value` settings before the `...` marker.
/// Unknown keys are ignored, matching Rime's tolerant header handling.
pub fn rime_settings(text: &str) -> RimeSettings {
    let mut settings = RimeSettings::default();
    for raw_line in text.lines() {
        let trimmed = raw_line.trim();
        if trimmed == RIME_TABLE_MARKER {
            break;
        }
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed == "---" {
            continue;
        }
        let Some((key, value)) = trimmed.split_once(':') else {
            continue;
        };
        let value = value.trim().trim_matches(['"', '\'']);
        match key.trim() {
            "use_preset_vocabulary" => {
                settings.use_preset_vocabulary = matches!(
                    value.to_ascii_lowercase().as_str(),
                    "true" | "yes" | "on" | "1"
                );
            }
            "vocabulary" if !value.is_empty() => {
                settings.vocabulary = Some(value.to_string());
            }
            "max_phrase_length" => {
                settings.max_phrase_length = value.parse().unwrap_or(0);
            }
            "min_phrase_weight" => {
                settings.min_phrase_weight = value.parse().unwrap_or(0.0);
            }
            _ => {}
        }
    }
    settings
}

/// Parse and validate `text`, auto-detecting the format.
pub fn load_source(text: &str) -> Result<Source, String> {
    load_source_with_preset(text, None)
}

/// Like [`load_source`], but resolves rime weights against `preset`:
/// missing weights take the essay weight, percentages scale it, and all
/// weights are multiplied by [`WEIGHT_SCALE`] (Rime keeps doubles).
pub fn load_source_with_preset(text: &str, preset: Option<&PresetVocab>) -> Result<Source, String> {
    if is_rime_dict(text) {
        load_rime(text, preset)
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
        validate_raw_pinyin(fields[0], lineno)?;
        let Some(freq) = parse_freq(fields.get(2).copied(), lineno)? else {
            // the tombstone convention applies to rime dicts only
            return Err(format!("line {lineno}: freq 0 must be >= 1"));
        };
        validate_entry(&pinyin, fields[1], freq, lineno)?;
        collector.push(pinyin, fields[1].to_string(), freq);
    }
    Ok(collector.finish(SourceFormat::IorTsv))
}

/// Parse and validate a rime dict.yaml source (hanzi first, after `...`).
fn load_rime(text: &str, preset: Option<&PresetVocab>) -> Result<Source, String> {
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
        validate_raw_pinyin(fields[1], lineno)?;
        let Some(freq) = parse_rime_weight(fields.get(2).copied(), hanzi, preset, lineno)? else {
            continue; // zero weight = removed entry (rime tombstone)
        };
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

/// Rime weight column. Without a preset vocabulary this keeps the old
/// per-10000 percentage scaling; with one, missing weights take the essay
/// weight and percentages scale it (Rime's `essay_weight * pct / 100`),
/// fixed-point multiplied by [`WEIGHT_SCALE`]. Zero stays a tombstone.
fn parse_rime_weight(
    raw: Option<&str>,
    hanzi: &str,
    preset: Option<&PresetVocab>,
    lineno: usize,
) -> Result<Option<u32>, String> {
    let Some(preset) = preset else {
        return parse_freq(raw, lineno);
    };
    let essay = preset.weight(hanzi);
    let Some(field) = raw else {
        return Ok(match essay {
            Some(0) => None,
            Some(weight) => Some(weight.saturating_mul(WEIGHT_SCALE)),
            None => Some(1),
        });
    };
    if let Some(pct) = field.strip_suffix('%') {
        let value = pct
            .trim()
            .parse::<f64>()
            .map_err(|err| format!("line {lineno}: invalid percentage weight '{field}': {err}"))?;
        if !(0.0..=100.0).contains(&value) {
            return Err(format!(
                "line {lineno}: percentage weight '{field}' outside 0..=100"
            ));
        }
        if value == 0.0 {
            return Ok(None);
        }
        let base = f64::from(essay.unwrap_or(1));
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let scaled = (base * value).round().max(1.0) as u32;
        return Ok(Some(scaled));
    }
    let weight = field
        .parse::<f64>()
        .map_err(|err| format!("line {lineno}: invalid weight '{field}': {err}"))?;
    if !weight.is_finite() || weight < 0.0 {
        return Err(format!(
            "line {lineno}: weight '{field}' must be finite and >= 0"
        ));
    }
    if weight == 0.0 {
        return Ok(None);
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let scaled = (weight * f64::from(WEIGHT_SCALE)).round().max(1.0) as u32;
    Ok(Some(scaled))
}

/// Parse the optional frequency/weight column.
///
/// Accepts a plain integer (`42`) or a rime percentage weight
/// (`99.93%`), which real dicts use for multi-pronunciation
/// disambiguation. Percentages scale to per-10000 integers. A zero
/// weight (plain `0` or rounding from e.g. `0%`) is rime's tombstone
/// convention: the entry is skipped rather than kept at weight 1.
fn parse_freq(raw: Option<&str>, lineno: usize) -> Result<Option<u32>, String> {
    let Some(field) = raw else { return Ok(Some(1)) };
    if let Some(pct) = field.strip_suffix('%') {
        let value = pct
            .trim()
            .parse::<f64>()
            .map_err(|err| format!("line {lineno}: invalid percentage weight '{field}': {err}"))?;
        if !(0.0..=100.0).contains(&value) {
            return Err(format!(
                "line {lineno}: percentage weight '{field}' outside 0..=100"
            ));
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let scaled = (value * 100.0).round() as u32;
        return Ok((scaled > 0).then_some(scaled));
    }
    let freq = field
        .parse::<u32>()
        .map_err(|err| format!("line {lineno}: invalid freq '{field}': {err}"))?;
    Ok((freq > 0).then_some(freq))
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
    fn parses_percentage_weights_from_real_rime_dicts() {
        // rime's multi-pronunciation rows carry probability weights.
        let text = "\u{4e01}\tding\t99.93%\n\u{4e01}\tzheng\t0.07%\n\u{4e07}\twan\t100%\n\u{4e09}\tsa\t0%\n";
        let source = load_source(&format!("# c\n...\n{text}")).unwrap();
        let freqs: Vec<u32> = source.entries.iter().map(|e| e.freq).collect();
        // the 0% row is a tombstone and must be skipped
        assert_eq!(freqs, vec![9993, 7, 10000]);
        let err = parse_freq(Some("120%"), 3).unwrap_err();
        assert!(err.contains("line 3") && err.contains("100"));
        assert!(parse_freq(Some("99.9%%"), 4).is_err());
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
        // ior TSV keeps the freq >= 1 rule...
        assert!(load_source(&format!("a\t{KAI}\t0\n"))
            .unwrap_err()
            .contains("must be >= 1"));
        // ...while rime zero weight drops the entry (its tombstone convention)
        let source = load_source(&format!("...\na\t{KAI}\t0\n")).unwrap();
        assert!(source.entries.is_empty());
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
        let err = load_rime("kai\tkai\n", None).unwrap_err();
        assert!(err.contains("table marker"), "{err}");
    }

    #[test]
    fn syllable_dump_is_rowed_and_noted() {
        let rendered = render_syllables(&["a".to_string(), "ai".to_string()]);
        assert!(rendered.contains("syllables (2):"));
        assert!(rendered.contains("a ai"));
        assert!(rendered.contains("engine-core syllable table"));
    }

    #[test]
    fn parses_preset_vocabulary_header() {
        let text = "# Rime dictionary\nuse_preset_vocabulary: true\n\
                    vocabulary: \"essay\"\nmax_phrase_length: 6\n\
                    min_phrase_weight: 12.5\n...\nkai\tkai\n";
        let settings = rime_settings(text);
        assert!(settings.use_preset_vocabulary);
        assert!(settings.preset_enabled());
        assert_eq!(settings.vocabulary_name(), "essay");
        assert_eq!(settings.max_phrase_length, 6);
        assert!((settings.min_phrase_weight - 12.5).abs() < f64::EPSILON);
        let plain = rime_settings("...\nkai\tkai\n");
        assert!(!plain.preset_enabled());
        assert_eq!(plain.vocabulary_name(), "essay");
    }

    #[test]
    fn preset_weights_follow_rime_semantics() {
        let preset = PresetVocab::parse("\u{5929}\t44972\n\u{8695}\t1\n").unwrap();
        let text = "use_preset_vocabulary: true\n...\n\
                    \u{5929}\ttian\n\u{8695}\tcan\t1%\n\u{8695}\ttian\t99%\n\
                    \u{5929}\ttian\t0\n";
        let source = load_source_with_preset(text, Some(&preset)).unwrap();
        let tian = source
            .entries
            .iter()
            .find(|e| e.hanzi == "\u{5929}")
            .unwrap();
        assert_eq!(tian.freq, 4_497_200);
        let can = source
            .entries
            .iter()
            .find(|e| e.hanzi == "\u{8695}" && e.pinyin == "can")
            .unwrap();
        assert_eq!(can.freq, 1);
        let tian_silkworm = source
            .entries
            .iter()
            .find(|e| e.hanzi == "\u{8695}" && e.pinyin == "tian")
            .unwrap();
        assert_eq!(tian_silkworm.freq, 99);
    }

    #[test]
    fn preset_percentage_scales_the_essay_weight() {
        // 1000 * 50% = 500 raw weight; fixed point x100 = 50_000.
        let preset = PresetVocab::parse("\u{597d}\t1000\n").unwrap();
        let text = "use_preset_vocabulary: true\n...\n\u{597d}\thao\t50%\n";
        let source = load_source_with_preset(text, Some(&preset)).unwrap();
        assert_eq!(source.entries[0].freq, 50_000);
    }
}
