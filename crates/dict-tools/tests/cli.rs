//! Integration tests for the dict-tools library: they exercise the same
//! library functions the binary uses (no subprocesses), covering the
//! build/export/merge/inspect flows with real files under temp_dir.

use dict_tools::cli;
use dict_tools::format;
use std::fs;
use std::path::PathBuf;

// Hanzi via escapes to keep this source file ASCII.
const KAI: &str = "\u{5f00}\u{53d1}"; // kai fa
const CHENG: &str = "\u{7a0b}\u{5e8f}"; // cheng xu
const CE: &str = "\u{6d4b}\u{8bd5}"; // ce shi

/// Unique per-test scratch file inside a per-process temp directory.
fn temp_path(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("dict-tools-int-{}", std::process::id()));
    fs::create_dir_all(&dir).expect("create temp dir");
    dir.join(name)
}

fn arg_list(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|part| part.to_string()).collect()
}

/// The `build` flow as the binary runs it: tsv -> parse -> normalize ->
/// serialize -> file, using only library functions.
fn build(tsv_path: &std::path::Path, ior_path: &std::path::Path) -> usize {
    let text = fs::read_to_string(tsv_path).expect("read input tsv");
    let entries = format::normalize(format::parse_tsv(&text).expect("parse tsv"));
    let count = entries.len();
    fs::write(ior_path, format::serialize(&entries)).expect("write .ior");
    count
}

/// The `export` flow as the binary runs it: .ior -> deserialize -> to_tsv.
fn export(ior_path: &std::path::Path, tsv_path: &std::path::Path) -> usize {
    let bytes = fs::read(ior_path).expect("read .ior");
    let entries = format::deserialize(&bytes).expect("deserialize .ior");
    let count = entries.len();
    fs::write(tsv_path, format::to_tsv(&entries)).expect("write tsv");
    count
}

#[test]
fn build_then_export_roundtrips_through_files() {
    let input = temp_path("build-in.tsv");
    let binary = temp_path("build-out.ior");
    let output = temp_path("build-out.tsv");
    fs::write(
        &input,
        format!("# sample\nkai'fa\t{KAI}\t500\ncheng'xu\t{CHENG}\n"),
    )
    .expect("write input tsv");

    assert_eq!(build(&input, &binary), 2);
    assert_eq!(export(&binary, &output), 2);

    let expected = format!("cheng'xu\t{CHENG}\t1\nkai'fa\t{KAI}\t500\n");
    assert_eq!(
        fs::read_to_string(&output).expect("read exported tsv"),
        expected
    );
    // Rebuilding the exported TSV is a fixed point.
    assert_eq!(build(&output, &binary), 2);
    assert_eq!(export(&binary, &output), 2);
    assert_eq!(fs::read_to_string(&output).expect("re-read"), expected);

    fs::remove_file(&input).ok();
    fs::remove_file(&binary).ok();
    fs::remove_file(&output).ok();
}

#[test]
fn merge_flow_sums_frequencies_and_dedups() {
    let base_tsv = temp_path("merge-a.tsv");
    let base = temp_path("merge-a.ior");
    let extra_tsv = temp_path("merge-b.tsv");
    let extra = temp_path("merge-b.ior");
    let out = temp_path("merge-out.ior");

    fs::write(&base_tsv, format!("kai'fa\t{KAI}\t500\n")).expect("write a tsv");
    fs::write(
        &extra_tsv,
        format!("KAI FA\t{KAI}\t100\ncheng'xu\t{CHENG}\t400\n"),
    )
    .expect("write b tsv");
    build(&base_tsv, &base);
    build(&extra_tsv, &extra);

    // The merge flow as the binary runs it: concat + normalize + serialize.
    let mut combined = format::deserialize(&fs::read(&base).expect("read a")).expect("a");
    combined.extend(format::deserialize(&fs::read(&extra).expect("read b")).expect("b"));
    let merged = format::normalize(combined);
    fs::write(&out, format::serialize(&merged)).expect("write merged");

    let loaded = format::deserialize(&fs::read(&out).expect("read merged")).expect("merged");
    assert_eq!(loaded.len(), 2);
    assert_eq!(
        loaded
            .iter()
            .find(|e| e.pinyin == "kai'fa")
            .expect("kai'fa")
            .freq,
        600
    );

    fs::remove_file(&base_tsv).ok();
    fs::remove_file(&base).ok();
    fs::remove_file(&extra_tsv).ok();
    fs::remove_file(&extra).ok();
    fs::remove_file(&out).ok();
}

#[test]
fn cli_run_covers_help_usage_and_exit_codes() {
    let mut out = Vec::new();

    assert_eq!(cli::run(&arg_list(&["--help"]), &mut out), 0);
    assert!(String::from_utf8(out).unwrap().contains("USAGE"));

    let mut out = Vec::new();
    assert_eq!(cli::run(&[], &mut out), 1); // no args: usage error
    assert_eq!(cli::run(&arg_list(&["frobnicate"]), &mut out), 1);

    let missing = temp_path("missing.ior");
    let args = arg_list(&["inspect", missing.to_str().unwrap()]);
    assert_eq!(cli::run(&args, &mut out), 2); // io error

    let bad = temp_path("bad.tsv");
    fs::write(&bad, "not-a-valid-line\n").expect("write bad tsv");
    let target = temp_path("bad-out.ior");
    let args = arg_list(&["build", bad.to_str().unwrap(), target.to_str().unwrap()]);
    assert_eq!(cli::run(&args, &mut out), 3); // format error
    assert!(!target.exists());

    fs::remove_file(&bad).ok();
}

#[test]
fn cli_run_build_inspect_export_end_to_end() {
    let tsv = temp_path("e2e-in.tsv");
    let ior = temp_path("e2e-out.ior");
    let back = temp_path("e2e-back.tsv");
    fs::write(
        &tsv,
        format!("kai'fa\t{KAI}\t500\ncheng'xu\t{CHENG}\t400\n"),
    )
    .expect("write tsv");

    let mut out = Vec::new();
    let args = arg_list(&["build", tsv.to_str().unwrap(), ior.to_str().unwrap()]);
    assert_eq!(cli::run(&args, &mut out), 0);

    let mut out = Vec::new();
    let args = arg_list(&["inspect", ior.to_str().unwrap()]);
    assert_eq!(cli::run(&args, &mut out), 0);
    let report = String::from_utf8(out).expect("inspect output");
    assert!(report.contains("entries: 2"), "report: {report}");
    assert!(report.contains("kai'fa"), "report: {report}");

    let mut out = Vec::new();
    let args = arg_list(&["export", ior.to_str().unwrap(), back.to_str().unwrap()]);
    assert_eq!(cli::run(&args, &mut out), 0);
    assert_eq!(
        fs::read_to_string(&back).expect("read back"),
        format!("cheng'xu\t{CHENG}\t400\nkai'fa\t{KAI}\t500\n")
    );

    fs::remove_file(&tsv).ok();
    fs::remove_file(&ior).ok();
    fs::remove_file(&back).ok();
}

#[test]
fn parse_args_accepts_merge_flag_after_positionals() {
    let args = arg_list(&["merge", "a.ior", "b.ior", "--output", "c.ior"]);
    assert_eq!(
        cli::parse_args(&args),
        Ok(cli::Command::Merge {
            input_a: "a.ior".into(),
            input_b: "b.ior".into(),
            output: "c.ior".into()
        })
    );
}

// ---------------------------------------------------------------------------
// compile
// ---------------------------------------------------------------------------

/// A minimal rime dict.yaml: everything before `...` is skipped, table rows
/// are `hanzi<TAB>pinyin<TAB>weight` (space-separated pinyin tolerated).
const RIME_SAMPLE: &str = "\
# Rime dictionary
name: luna_pinyin.my
version: \"0.9\"
sort: original
...
\u{5f00}\u{53d1}\tkai fa\t500
\u{7a0b}\u{5e8f}\tcheng'xu\t400
\u{6d4b}\u{8bd5}\tce shi
";

#[test]
fn compile_rime_yaml_to_v2_and_roundtrip() {
    let input = temp_path("rime-in.yaml");
    let output = temp_path("rime-out.ior");
    fs::write(&input, RIME_SAMPLE).expect("write rime sample");

    let mut out = Vec::new();
    let args = arg_list(&["compile", input.to_str().unwrap(), output.to_str().unwrap()]);
    assert_eq!(cli::run(&args, &mut out), 0);
    let report = String::from_utf8(out).expect("compile report");
    assert!(report.contains("rime-dict.yaml"), "{report}");
    assert!(report.contains("3 keys"), "{report}");
    assert!(report.contains("3 entries"), "{report}");

    let bytes = fs::read(&output).expect("read compiled .ior");
    assert_eq!(bytes[4], 2, "compile must emit v2");
    let entries = format::deserialize(&bytes).expect("deserialize v2");
    assert_eq!(entries.len(), 3);
    // canonical ordering: key asc, then freq desc
    assert_eq!(entries[0].pinyin, "ce'shi");
    assert_eq!(entries[0].freq, 1);
    assert_eq!(entries[1].pinyin, "cheng'xu");
    assert_eq!(entries[1].hanzi, CHENG);
    assert_eq!(entries[2].pinyin, "kai'fa");
    assert_eq!(entries[2].hanzi, KAI);
    assert_eq!(entries[2].freq, 500);

    fs::remove_file(&input).ok();
    fs::remove_file(&output).ok();
}

/// v1 legacy file -> deserialize -> export -> compile back to v2.
#[test]
fn compile_roundtrips_a_legacy_v1_file() {
    fn v1_bytes(entries: &[format::Entry]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"IORD");
        out.push(1);
        out.extend_from_slice(&u32::try_from(entries.len()).unwrap().to_le_bytes());
        for entry in entries {
            for field in [&entry.pinyin, &entry.hanzi] {
                out.extend_from_slice(&u32::try_from(field.len()).unwrap().to_le_bytes());
                out.extend_from_slice(field.as_bytes());
            }
            out.extend_from_slice(&entry.freq.to_le_bytes());
        }
        out
    }

    let legacy = vec![
        format::Entry {
            pinyin: "kai'fa".into(),
            hanzi: KAI.into(),
            freq: 500,
        },
        format::Entry {
            pinyin: "cheng'xu".into(),
            hanzi: CHENG.into(),
            freq: 400,
        },
    ];
    let v1_path = temp_path("legacy-v1.ior");
    let tsv_path = temp_path("legacy-back.tsv");
    let v2_path = temp_path("legacy-v2.ior");
    fs::write(&v1_path, v1_bytes(&legacy)).expect("write v1 file");
    assert_eq!(fs::read(&v1_path).unwrap()[4], 1);

    // legacy file still loads through the shared flow
    let count = export(&v1_path, &tsv_path);
    assert_eq!(count, 2);

    let mut out = Vec::new();
    let args = arg_list(&[
        "compile",
        tsv_path.to_str().unwrap(),
        v2_path.to_str().unwrap(),
    ]);
    assert_eq!(cli::run(&args, &mut out), 0);
    let bytes = fs::read(&v2_path).expect("read v2 file");
    assert_eq!(bytes[4], 2, "recompiled file must be v2");
    // compile normalizes: same set, key-ascending order
    let mut expected = legacy;
    expected.sort_by(|a, b| a.pinyin.cmp(&b.pinyin));
    assert_eq!(format::deserialize(&bytes).unwrap(), expected);

    fs::remove_file(&v1_path).ok();
    fs::remove_file(&tsv_path).ok();
    fs::remove_file(&v2_path).ok();
}

#[test]
fn compile_rejects_structural_errors_with_line_numbers() {
    let bad = temp_path("compile-bad.tsv");
    let target = temp_path("compile-bad.ior");

    // non-pinyin alphabet on line 2 (after a comment)
    fs::write(&bad, "# header\nl\u{fc}\t\u{5415}\t3\n").expect("write bad tsv");
    let mut out = Vec::new();
    let args = arg_list(&["compile", bad.to_str().unwrap(), target.to_str().unwrap()]);
    assert_eq!(cli::run(&args, &mut out), 3);
    assert!(!target.exists(), "failed compile must not write output");

    // freq 0 on line 1
    fs::write(&bad, format!("a\t{KAI}\t0\n")).expect("write zero freq");
    let mut out = Vec::new();
    let args = arg_list(&["compile", bad.to_str().unwrap(), target.to_str().unwrap()]);
    assert_eq!(cli::run(&args, &mut out), 3);

    // rime table row missing its pinyin column
    fs::write(&bad, "...\n\u{5f00}\u{53d1}\n").expect("write broken rime row");
    let mut out = Vec::new();
    let args = arg_list(&["compile", bad.to_str().unwrap(), target.to_str().unwrap()]);
    assert_eq!(cli::run(&args, &mut out), 3);

    fs::remove_file(&bad).ok();
}

// ---------------------------------------------------------------------------
// validate
// ---------------------------------------------------------------------------

#[test]
fn validate_reports_stats_and_duplicate_note() {
    let input = temp_path("validate-ok.tsv");
    fs::write(
        &input,
        format!("kai'fa\t{KAI}\t100\nkai'fa\t{KAI}\t50\ncheng'xu\t{CHENG}\n"),
    )
    .expect("write input");

    let mut out = Vec::new();
    let args = arg_list(&["validate", input.to_str().unwrap()]);
    assert_eq!(cli::run(&args, &mut out), 0);
    let report = String::from_utf8(out).expect("validate report");
    assert!(report.contains("ok:"), "{report}");
    assert!(report.contains("2 keys"), "{report}");
    assert!(report.contains("3 entries"), "{report}");
    assert!(
        report.contains("1 duplicate (pinyin,hanzi) pair"),
        "{report}"
    );

    fs::remove_file(&input).ok();
}

#[test]
fn validate_dump_syllables_lists_sorted_inventory_with_note() {
    let input = temp_path("validate-syl.tsv");
    fs::write(&input, format!("shi\t{CE}\nkai'fa\t{KAI}\n")).expect("write input");

    let mut out = Vec::new();
    let args = arg_list(&["validate", input.to_str().unwrap(), "--dump-syllables"]);
    assert_eq!(cli::run(&args, &mut out), 0);
    let report = String::from_utf8(out).expect("validate report");
    assert!(report.contains("syllables (3):"), "{report}");
    let list = report
        .lines()
        .find(|line| line.starts_with("  "))
        .expect("syllable list row");
    assert_eq!(list, "  fa kai shi");
    assert!(
        report.contains("does not embed the engine-core syllable table"),
        "{report}"
    );

    fs::remove_file(&input).ok();
}

#[test]
fn validate_exits_3_on_rule_violations_with_line_numbers() {
    let input = temp_path("validate-bad.tsv");

    // consecutive separators, then a non-pinyin character, then freq 0
    fs::write(
        &input,
        format!("ni''hao\t{KAI}\nl\u{fc}\t{CE}\na\t{CHENG}\t0\n"),
    )
    .expect("write bad input");
    let mut out = Vec::new();
    let args = arg_list(&["validate", input.to_str().unwrap()]);
    assert_eq!(cli::run(&args, &mut out), 3);

    fs::write(&input, format!("ni'\t{KAI}\n")).expect("write trailing quote");
    let mut out = Vec::new();
    let args = arg_list(&["validate", input.to_str().unwrap()]);
    assert_eq!(cli::run(&args, &mut out), 3);

    // usage error: unknown flag
    let mut out = Vec::new();
    let args = arg_list(&["validate", "--bogus", input.to_str().unwrap()]);
    assert_eq!(cli::run(&args, &mut out), 1);

    fs::remove_file(&input).ok();
}

#[test]
fn validate_accepts_rime_sources_and_compile_agrees() {
    let input = temp_path("validate-rime.yaml");
    let output = temp_path("validate-rime.ior");
    fs::write(&input, RIME_SAMPLE).expect("write rime sample");

    let mut out = Vec::new();
    let args = arg_list(&["validate", input.to_str().unwrap()]);
    assert_eq!(cli::run(&args, &mut out), 0);
    assert!(String::from_utf8(out).unwrap().contains("rime-dict.yaml"));

    let mut out = Vec::new();
    let args = arg_list(&["compile", input.to_str().unwrap(), output.to_str().unwrap()]);
    assert_eq!(cli::run(&args, &mut out), 0);

    fs::remove_file(&input).ok();
    fs::remove_file(&output).ok();
}

#[test]
fn compile_parses_vocabulary_flag_in_any_position() {
    let args = arg_list(&[
        "compile",
        "--vocabulary",
        "data/dicts/essay.txt",
        "in.yaml",
        "out.ior",
    ]);
    assert_eq!(
        cli::parse_args(&args).unwrap(),
        cli::Command::Compile {
            input: "in.yaml".into(),
            output: "out.ior".into(),
            vocabulary: Some("data/dicts/essay.txt".into()),
            simplify: false,
        }
    );
    let args = arg_list(&["compile", "in.yaml", "out.ior"]);
    assert_eq!(
        cli::parse_args(&args).unwrap(),
        cli::Command::Compile {
            input: "in.yaml".into(),
            output: "out.ior".into(),
            vocabulary: None,
            simplify: false,
        }
    );
    let args = arg_list(&["compile", "in.yaml", "out.ior", "--vocabulary"]);
    assert!(cli::parse_args(&args).unwrap_err().contains("needs a path"));
    let args = arg_list(&["compile", "in.tsv", "out.ior", "--simplify"]);
    assert_eq!(
        cli::parse_args(&args).unwrap(),
        cli::Command::Compile {
            input: "in.tsv".into(),
            output: "out.ior".into(),
            vocabulary: None,
            simplify: true,
        }
    );
}

#[test]
fn compile_simplify_rewrites_traditional_entries() {
    let input = temp_path("simplify.tsv");
    let output = temp_path("simplify.ior");
    fs::write(&input, "tian'qi\t天氣\t100\nni\t你\t50\n").unwrap();
    let args = arg_list(&[
        "compile",
        input.to_str().unwrap(),
        output.to_str().unwrap(),
        "--simplify",
    ]);
    let mut out = Vec::new();
    assert_eq!(cli::run(&args, &mut out), 0);
    let report = String::from_utf8(out).expect("utf8 report");
    assert!(report.contains("simplified 1 entrie(s)"), "{report}");
    let bytes = fs::read(&output).expect("read simplified .ior");
    let entries = dict_tools::format::deserialize(&bytes).expect("valid .ior");
    assert!(entries.iter().any(|entry| entry.hanzi == "天气"));
    assert!(!entries.iter().any(|entry| entry.hanzi == "天氣"));
    fs::remove_file(&input).ok();
    fs::remove_file(&output).ok();
}

#[test]
fn compile_loads_sibling_essay_for_preset_vocabulary() {
    let input = temp_path("preset-dict.yaml");
    let essay = temp_path("essay.txt");
    let output = temp_path("preset-out.ior");
    fs::write(
        &input,
        "# Rime dictionary\nuse_preset_vocabulary: true\n...\n\
         \u{4eca}\tjin\t10\n\u{5929}\ttian\t10\n",
    )
    .expect("write preset dict");
    fs::write(&essay, "# preset\n\u{4eca}\u{5929}\t500\n").expect("write essay");

    let mut out = Vec::new();
    let args = arg_list(&["compile", input.to_str().unwrap(), output.to_str().unwrap()]);
    assert_eq!(cli::run(&args, &mut out), 0);
    let report = String::from_utf8(out).unwrap();
    assert!(report.contains("preset vocabulary"), "{report}");
    assert!(report.contains("1 encoded entrie(s)"), "{report}");

    let bytes = fs::read(&output).expect("read .ior");
    let entries = format::deserialize(&bytes).expect("deserialize");
    let phrase = entries
        .iter()
        .find(|entry| entry.hanzi == "\u{4eca}\u{5929}")
        .expect("encoded 今天");
    assert_eq!(phrase.pinyin, "jin'tian");
    assert_eq!(phrase.freq, 50_000);
    // Absolute weights are fixed-point scaled when a preset is active.
    let jin = entries
        .iter()
        .find(|entry| entry.hanzi == "\u{4eca}")
        .unwrap();
    assert_eq!(jin.freq, 1_000);

    fs::remove_file(&input).ok();
    fs::remove_file(&essay).ok();
    fs::remove_file(&output).ok();
}
