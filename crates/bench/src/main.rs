//! ior-bench: hand-timed performance scenarios for the dictionary stack.
//!
//!  1. dict-tools `parse_tsv -> normalize -> serialize` on a synthetic
//!     100k-entry TSV (target: < 1 s total).
//!  2. engine-core `Dictionary::from_ior_bytes` load time plus
//!     `Dictionary::lookup` P50/P99 (target: single lookup < 10 µs),
//!     cross-checked against bytes written by dict-tools itself.
//!  3. engine-core `decoder::candidates` latency per raw input.
//!  4. P2/M2b sentence decode: dict-tools trains a `.gram` model,
//!     engine-core beam-searches ≥ 8-syllable inputs (live cross-crate
//!     format interlock).
//!
//! Timing uses `std::time::Instant` with median-of-runs / percentile
//! statistics; there are no external dependencies (no criterion).

mod sentence;
mod synth;

use dict_tools::format as dt_format;
use engine_core::decoder;
use engine_core::dict::Dictionary;
use std::time::Instant;

/// Usage text printed for `--help` / `-h`.
const USAGE: &str = "\
ior-bench: hand-timed performance scenarios for the dictionary stack

USAGE:
    ior-bench [--help]

OPTIONS:
    -h, --help    print this help and exit

SCENARIOS (all run, in order):
    [1] dict-tools compile of a synthetic 100k-entry TSV
    [2] engine-core .ior load + key lookup latency
    [3] engine-core legacy decoder candidates per input
    [4] P2/M2b sentence decode: dict-tools trains a .gram model,
        engine-core beam-searches 8-syllable inputs (also a live
        cross-crate .gram format interlock)";

/// Synthetic workload size for the compile scenario.
const ENTRY_COUNT: usize = 100_000;

/// PRNG seed: fixed so numbers are comparable across runs.
const SEED: u64 = 0x10C_0FFEE;

/// Repeated load runs for a stable load-time estimate.
const LOAD_RUNS: usize = 5;

/// Timed lookup samples (after warm-up).
const LOOKUP_SAMPLES: usize = 20_000;

/// Lookups executed (and discarded) before timing starts.
const LOOKUP_WARMUP: usize = 2_000;

/// Timed `candidates` calls per input.
const DECODE_RUNS: usize = 2_000;

/// `candidates` warm-up calls per input.
const DECODE_WARMUP: usize = 100;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None => {}
        Some("-h") | Some("--help") => {
            println!("{USAGE}");
            return;
        }
        Some(other) => {
            eprintln!("ior-bench: unknown argument '{other}' (try --help)");
            std::process::exit(2);
        }
    }
    let compile = scenario_compile();
    let (load, dict, bytes) = scenario_load(&compile.output_bytes);
    let decode = scenario_decode(&dict);
    let sentence = sentence::scenario();
    print_report(&compile.rows, &load, &decode, &sentence, &bytes);
}

/// One scenario's report rows plus artifacts later scenarios consume.
struct CompileOutcome {
    rows: Vec<(String, String)>,
    output_bytes: Vec<u8>,
}

/// One scenario's report rows plus artifacts later scenarios consume.
struct LoadOutcome {
    rows: Vec<(String, String)>,
}

/// Seconds elapsed while running `body` (wall clock).
fn timed<T>(body: impl FnOnce() -> T) -> (f64, T) {
    let start = Instant::now();
    let value = body();
    (start.elapsed().as_secs_f64(), value)
}

/// Scenario 1: full compile pipeline over the synthetic TSV.
fn scenario_compile() -> CompileOutcome {
    let (gen_secs, tsv) = timed(|| synth::synth_tsv(ENTRY_COUNT, SEED));
    let (parse_secs, raw) = timed(|| dt_format::parse_tsv(&tsv).expect("synthetic tsv parses"));
    let (norm_secs, entries) = timed(|| dt_format::normalize(raw));
    let (ser_secs, bytes) = timed(|| dt_format::serialize(&entries));
    let total = gen_secs + parse_secs + norm_secs + ser_secs;
    let keys = entries
        .windows(2)
        .filter(|pair| pair[0].pinyin != pair[1].pinyin)
        .count()
        + usize::from(!entries.is_empty());
    let rows = vec![
        (
            "generate synthetic tsv".to_string(),
            format!("{} ms ({} entries)", synth::millis(gen_secs), ENTRY_COUNT),
        ),
        ("parse_tsv".to_string(), synth::millis(parse_secs) + " ms"),
        ("normalize".to_string(), synth::millis(norm_secs) + " ms"),
        (
            "serialize (v2)".to_string(),
            synth::millis(ser_secs) + " ms",
        ),
        (
            "total".to_string(),
            format!(
                "{} ms  (target < 1000 ms: {})",
                synth::millis(total),
                verdict(total < 1.0)
            ),
        ),
        (
            "output size".to_string(),
            format!(
                "{} bytes ({:.2} MiB), {} keys, {} entries",
                bytes.len(),
                bytes.len() as f64 / (1024.0 * 1024.0),
                keys,
                entries.len()
            ),
        ),
    ];
    CompileOutcome {
        rows,
        output_bytes: bytes,
    }
}

/// Scenario 2: load the dict-tools-written bytes with engine-core and
/// measure lookup latency. Loading data produced by dict-tools doubles as
/// a cross-crate format compatibility check.
fn scenario_load(bytes: &[u8]) -> (LoadOutcome, Dictionary, Vec<u8>) {
    let mut load_secs = Vec::with_capacity(LOAD_RUNS);
    let mut dict = Dictionary::from_ior_bytes(bytes).expect("dict-tools v2 bytes load");
    for _ in 0..LOAD_RUNS {
        let (secs, reloaded) = timed(|| Dictionary::from_ior_bytes(bytes));
        load_secs.push(secs);
        dict = reloaded.expect("reload");
    }
    let keys: Vec<String> = dict.iter().map(|(key, _)| key.to_string()).collect();

    // format cross-check: every sampled dict-tools key must resolve
    let mut state = SEED | 1;
    let mut misses = 0usize;
    for _ in 0..LOOKUP_WARMUP + LOOKUP_SAMPLES {
        let key = &keys[synth::next_u64(&mut state) as usize % keys.len()];
        if dict.lookup(key).is_none() {
            misses += 1;
        }
    }
    assert_eq!(misses, 0, "engine-core failed to resolve a dict-tools key");

    let mut samples = Vec::with_capacity(LOOKUP_SAMPLES);
    let mut state = SEED | 1;
    for _ in 0..LOOKUP_WARMUP {
        let key = &keys[synth::next_u64(&mut state) as usize % keys.len()];
        std::hint::black_box(dict.lookup(key));
    }
    let mut state = SEED | 1;
    for _ in 0..LOOKUP_SAMPLES {
        let key = &keys[synth::next_u64(&mut state) as usize % keys.len()];
        let (secs, hit) = timed(|| dict.lookup(key));
        std::hint::black_box(hit);
        samples.push(secs);
    }
    let rows = vec![
        (
            format!("load (median of {LOAD_RUNS})"),
            synth::millis(synth::median(&load_secs)) + " ms",
        ),
        (
            "keys / entries".to_string(),
            format!("{} keys loaded", dict.len()),
        ),
        (
            format!("lookup P50 / mean ({LOOKUP_SAMPLES} samples)"),
            format!(
                "{} / {} us",
                synth::micros(synth::median(&samples)),
                synth::micros(synth::mean(&samples))
            ),
        ),
        (
            "lookup P99".to_string(),
            format!(
                "{} us  (target < 10 us: {})",
                synth::micros(synth::percentile(&samples, 99.0)),
                verdict(synth::percentile(&samples, 99.0) < 10e-6)
            ),
        ),
        (
            "format cross-check".to_string(),
            format!(
                "{} lookups all hit (dict-tools -> engine-core)",
                LOOKUP_SAMPLES
            ),
        ),
    ];
    (LoadOutcome { rows }, dict, bytes.to_vec())
}

/// Scenario 3: decoder candidate latency on a few realistic raw inputs.
fn scenario_decode(dict: &Dictionary) -> Vec<Vec<(String, String)>> {
    let inputs = [
        "ni'hao",
        "kai'fa",
        "shu'ju'ku",
        "nihaoshijie",
        "xian",
        "zhong'guo'ren",
    ];
    inputs
        .iter()
        .map(|raw| {
            let mut samples = Vec::with_capacity(DECODE_RUNS);
            let mut count = 0;
            for round in 0..DECODE_WARMUP + DECODE_RUNS {
                let (secs, candidates) = timed(|| decoder::candidates(dict, None, raw, 10));
                count = candidates.len();
                if round >= DECODE_WARMUP {
                    samples.push(secs);
                }
            }
            vec![
                (format!("input \"{raw}\""), format!("{} candidates", count)),
                (
                    "  P50 / P99".to_string(),
                    format!(
                        "{} / {} us",
                        synth::micros(synth::median(&samples)),
                        synth::micros(synth::percentile(&samples, 99.0))
                    ),
                ),
            ]
        })
        .collect()
}

/// PASS/FAIL marker for target lines.
fn verdict(ok: bool) -> &'static str {
    if ok {
        "PASS"
    } else {
        "FAIL"
    }
}

/// Print all scenarios as aligned tables.
fn print_report(
    compile: &[(String, String)],
    load: &LoadOutcome,
    decode: &[Vec<(String, String)>],
    sentence: &[(String, String)],
    bytes: &[u8],
) {
    println!("ior-bench  (std-only timing, medians/percentiles over Instant)");
    println!();
    println!("[1] dict-tools compile - synthetic {ENTRY_COUNT}-entry TSV");
    print_rows(compile);
    println!();
    println!(
        "[2] engine-core from_ior_bytes - {} bytes written by dict-tools",
        bytes.len()
    );
    print_rows(&load.rows);
    println!();
    println!("[3] engine-core decoder candidates (limit 10)");
    for input_rows in decode {
        print_rows(input_rows);
    }
    println!();
    println!("[4] P2/M2b sentence decode - beam search with a dict-tools .gram model");
    print_rows(sentence);
}

/// Print one `label | value` table with aligned columns.
fn print_rows(rows: &[(String, String)]) {
    let width = rows.iter().map(|(label, _)| label.len()).max().unwrap_or(0);
    for (label, value) in rows {
        println!("  {label:<width$}  {value}");
    }
}
