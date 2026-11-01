//! Text persistence for [`crate::user_dict::UserDict`]: deterministic
//! snapshots, an append-only learn log, compaction and cloud-backup TSV.
//!
//! engine-core stays I/O-free — these are pure translations between a
//! `UserDict` and bytes/strings; the host (fcitx5 addon) owns the files.
//!
//! # Formats
//!
//! Snapshot (what a save writes):
//!
//! ```text
//! # ior userdb v1
//! ni'hao<TAB>泥嚎<TAB>2
//! ni'hao<TAB>你好<TAB>2
//! ```
//!
//! Rows are `pinyin\thanzi\tfreq`, keys in byte order ascending and, within
//! a key, frequency descending (hanzi ascending on ties). Two equal
//! dictionaries therefore serialize to equal bytes.
//!
//! Append-only log line (one learned use per line):
//!
//! ```text
//! L<TAB>ni'hao<TAB>你好
//! ```
//!
//! `L` records replay exactly like [`crate::user_dict::learn`]; `#` comment
//! lines and blank lines are ignored everywhere. [`compact`] folds a log (or
//! snapshot, or a mix — a snapshot is already compact) into snapshot bytes.

use crate::user_dict::{add, learn, new, normalize_pinyin, UserDict};

/// First line of every snapshot; hosts sniff it to tell snapshots from logs.
pub const SNAPSHOT_HEADER: &str = "# ior userdb v1";

/// Serializes `u` as a deterministic snapshot: the [`SNAPSHOT_HEADER`] line,
/// then one `pinyin\thanzi\tfreq` row per entry — keys in byte order
/// ascending, and within a key frequency descending (hanzi ascending on
/// ties, the in-memory invariant). Equal dictionaries produce equal bytes,
/// so snapshots double as canonical form for compaction and diffing.
pub fn to_snapshot(u: &UserDict) -> Vec<u8> {
    let mut keys: Vec<&String> = u.entries.keys().collect();
    keys.sort();
    let mut out = String::from(SNAPSHOT_HEADER);
    out.push('\n');
    for key in keys {
        for (hanzi, freq) in &u.entries[key] {
            out.push_str(key);
            out.push('\t');
            out.push_str(hanzi);
            out.push('\t');
            out.push_str(&freq.to_string());
            out.push('\n');
        }
    }
    out.into_bytes()
}

/// Parses snapshot bytes produced by [`to_snapshot`] (see the module docs
/// for the format). `#` comment lines and blank lines are skipped; the file
/// frequencies are adopted verbatim — nothing is incremented. Malformed
/// rows (wrong column count, non-numeric or overflowing frequency, unknown
/// future header version, invalid UTF-8) are reported as `Err` carrying the
/// 1-based line number and what is wrong with it.
pub fn from_snapshot(bytes: &[u8]) -> Result<UserDict, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "userdb: invalid UTF-8".to_string())?;
    let mut u = new();
    for (lineno, line) in numbered_lines(text) {
        if is_skipped(line, lineno)? {
            continue;
        }
        let (pinyin, hanzi, freq) = parse_row(line, lineno)?;
        add(&mut u, &normalize_pinyin(pinyin), hanzi, freq);
    }
    Ok(u)
}

/// Builds one append-only log record `L\tpinyin\thanzi` for a learned use.
/// `pinyin` is normalized exactly like [`crate::user_dict::learn`], so
/// writing the line and replaying it later are inverse operations.
pub fn log_line(pinyin: &str, hanzi: &str) -> String {
    format!("L\t{}\t{}", normalize_pinyin(pinyin), hanzi)
}

/// Replays an append-only log: each well-formed `L\tpinyin\thanzi` line is
/// one [`crate::user_dict::learn`] call, so replaying a log is equivalent
/// to learning its records one by one. Blank and `#` comment lines are
/// skipped; anything else (including malformed `L` records) is `Err` with
/// the 1-based line number.
pub fn replay(log: &str) -> Result<UserDict, String> {
    let mut u = new();
    for (lineno, line) in numbered_lines(log) {
        if is_skipped(line, lineno)? {
            continue;
        }
        match parse_log_record(line) {
            Some((pinyin, hanzi)) => learn(&mut u, pinyin, hanzi),
            None => {
                return Err(format!(
                    "line {lineno}: expected `L\\tpinyin\\thanzi`, got `{line}`"
                ))
            }
        }
    }
    Ok(u)
}

/// Folds userdb text into deterministic snapshot bytes — the compaction
/// step of the append-only scheme. For a pure log this is exactly
/// `to_snapshot(replay(log)?)`; snapshot rows (already-compacted files) are
/// accepted as well, so compacting twice yields byte-identical output.
/// Malformed lines are `Err` with the 1-based line number.
pub fn compact(text: &str) -> Result<Vec<u8>, String> {
    let mut u = new();
    for (lineno, line) in numbered_lines(text) {
        if is_skipped(line, lineno)? {
            continue;
        }
        if let Some((pinyin, hanzi)) = parse_log_record(line) {
            learn(&mut u, pinyin, hanzi);
            continue;
        }
        let (pinyin, hanzi, freq) = parse_row(line, lineno)?;
        add(&mut u, &normalize_pinyin(pinyin), hanzi, freq);
    }
    Ok(to_snapshot(&u))
}

/// Full-dictionary dump for cloud backup (roadmap P3): byte-identical to
/// [`to_snapshot`] — TSV is the interchange format both directions.
pub fn export_tsv(u: &UserDict) -> Vec<u8> {
    to_snapshot(u)
}

/// Restores a cloud backup written by [`export_tsv`]: the same parser (and
/// therefore the same errors) as [`from_snapshot`].
pub fn import_tsv(bytes: &[u8]) -> Result<UserDict, String> {
    from_snapshot(bytes)
}

/// Splits `text` into lines, stripping a trailing `\r` (CRLF tolerance) and
/// numbering them 1-based for error messages.
fn numbered_lines(text: &str) -> impl Iterator<Item = (usize, &str)> {
    text.split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .enumerate()
        .map(|(idx, line)| (idx + 1, line))
}

/// True for blank and `#` comment lines. A line that looks like a userdb
/// header but is not this version is rejected instead of skipped, so a
/// future format fails loudly rather than loading as an empty dictionary.
fn is_skipped(line: &str, lineno: usize) -> Result<bool, String> {
    if let Some(rest) = line.strip_prefix("# ior userdb") {
        if rest != " v1" {
            return Err(format!("line {lineno}: unsupported userdb header `{line}`"));
        }
        return Ok(true);
    }
    Ok(line.is_empty() || line.starts_with('#'))
}

/// Parses a snapshot row `pinyin\thanzi\tfreq`; errors carry the 1-based
/// `lineno` plus the reason (column count or frequency).
fn parse_row(line: &str, lineno: usize) -> Result<(&str, &str, u32), String> {
    let cols: Vec<&str> = line.split('\t').collect();
    if cols.len() != 3 {
        return Err(format!(
            "line {lineno}: expected `pinyin\\thanzi\\tfreq`, got {} column(s)",
            cols.len()
        ));
    }
    let freq = cols[2]
        .parse::<u32>()
        .map_err(|_| format!("line {lineno}: invalid frequency `{}`", cols[2]))?;
    Ok((cols[0], cols[1], freq))
}

/// Parses an append-only record `L\tpinyin\thanzi`; `None` when the line is
/// not a well-formed record.
fn parse_log_record(line: &str) -> Option<(&str, &str)> {
    let cols: Vec<&str> = line.split('\t').collect();
    if cols.len() == 3 && cols[0] == "L" {
        Some((cols[1], cols[2]))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::user_dict::{lookup, new as new_dict};

    /// Snapshot of a learned dictionary, as an exact byte string.
    fn sample_snapshot() -> Vec<u8> {
        let mut u = new_dict();
        learn(&mut u, "zz", "丁");
        learn(&mut u, "ma", "丙");
        learn(&mut u, "MA", "丙"); // 丙 -> 2
        learn(&mut u, "aa", "甲");
        learn(&mut u, "aa", "乙");
        learn(&mut u, "aa", "乙"); // 乙 -> 2 sorts ahead of 甲
        to_snapshot(&u)
    }

    #[test]
    fn snapshot_is_ordered_and_roundtrips() {
        let bytes = sample_snapshot();
        assert_eq!(
            String::from_utf8(bytes.clone()).expect("utf-8"),
            "# ior userdb v1\n\
             aa\t乙\t2\n\
             aa\t甲\t1\n\
             ma\t丙\t2\n\
             zz\t丁\t1\n"
        );
        // Round trip: the parsed dictionary re-serializes to equal bytes
        // and carries the frequencies verbatim (nothing gets +1'd).
        let u = from_snapshot(&bytes).expect("snapshot parses");
        assert_eq!(to_snapshot(&u), bytes);
        assert_eq!(
            lookup(&u, "aa"),
            vec![("乙".to_string(), 2), ("甲".to_string(), 1)]
        );
    }

    #[test]
    fn from_snapshot_rejects_bad_lines_with_numbers() {
        // Bad frequency on line 3 (1-based, header included).
        let bad_freq = from_snapshot("# ior userdb v1\nma\t丙\t2\nma\t丁\tx\n".as_bytes());
        assert_eq!(
            bad_freq.expect_err("bad freq"),
            "line 3: invalid frequency `x`"
        );
        // Wrong column count on line 2.
        let bad_cols = from_snapshot("# ior userdb v1\nma\t丙\n".as_bytes());
        assert_eq!(
            bad_cols.expect_err("bad columns"),
            "line 2: expected `pinyin\\thanzi\\tfreq`, got 2 column(s)"
        );
        // Unknown future header version fails loudly instead of skipping.
        let bad_hdr = from_snapshot("# ior userdb v9\nma\t丙\t2\n".as_bytes());
        assert!(bad_hdr
            .expect_err("future header")
            .starts_with("line 1: unsupported userdb header"));
        assert!(from_snapshot(b"\xff\xfe").is_err()); // not UTF-8
                                                      // Comments, blank lines and a missing trailing newline are fine.
        let ok = from_snapshot("# note\n\nma\t丙\t2".as_bytes());
        assert_eq!(
            lookup(&ok.expect("lenient"), "ma"),
            vec![("丙".to_string(), 2)]
        );
    }

    #[test]
    fn log_lines_replay_like_learn() {
        let log = [
            log_line("NI HAO", "你好"), // normalized on the way out
            log_line("ni'hao", "你好"),
            log_line("ni'hao", "泥嚎"),
        ]
        .join("\n");
        assert_eq!(log, "L\tni'hao\t你好\nL\tni'hao\t你好\nL\tni'hao\t泥嚎");
        let replayed = replay(&log).expect("log replays");
        let mut learned = new_dict();
        learn(&mut learned, "ni'hao", "你好");
        learn(&mut learned, "ni'hao", "你好");
        learn(&mut learned, "ni'hao", "泥嚎");
        assert_eq!(lookup(&replayed, "ni'hao"), lookup(&learned, "ni'hao"));
        // Garbage records are rejected with their line number.
        assert_eq!(
            replay("L\tni\t你\nX\tno\n").expect_err("bad record"),
            "line 2: expected `L\\tpinyin\\thanzi`, got `X\tno`"
        );
        assert!(replay("# comment\n\n")
            .expect("only comments")
            .entries
            .is_empty());
    }

    #[test]
    fn compact_is_idempotent_and_matches_replay() {
        let log = "L\tma\t丙\nL\tma\t丙\nL\tma\t丙\nL\tzz\t丁\n";
        let once = compact(log).expect("compacts");
        assert_eq!(once, "# ior userdb v1\nma\t丙\t3\nzz\t丁\t1\n".as_bytes());
        // Compacting the (snapshot-shaped) result again changes nothing.
        let twice = compact(std::str::from_utf8(&once).expect("utf-8")).expect("recompacts");
        assert_eq!(once, twice);
        // For a pure log, compact == to_snapshot . replay.
        assert_eq!(once, to_snapshot(&replay(log).expect("replays")));
        // Mixed files (snapshot rows plus fresh L records) fold together.
        let mixed = String::from_utf8(once).expect("utf-8") + "L\tma\t丙\n";
        assert_eq!(
            compact(&mixed).expect("folds"),
            "# ior userdb v1\nma\t丙\t4\nzz\t丁\t1\n".as_bytes()
        );
    }

    #[test]
    fn export_tsv_imports_back_verbatim() {
        let bytes = sample_snapshot();
        assert_eq!(export_tsv(&from_snapshot(&bytes).expect("parses")), bytes);
        let restored =
            import_tsv(&export_tsv(&from_snapshot(&bytes).expect("parses"))).expect("imports");
        assert_eq!(lookup(&restored, "ma"), vec![("丙".to_string(), 2)]);
        assert!(import_tsv("ma\t丙\tz".as_bytes()).is_err());
    }
}
