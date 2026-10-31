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
