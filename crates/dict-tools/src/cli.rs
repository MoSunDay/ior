//! Hand-rolled command-line parsing and dispatch for the `dict-tools`
//! binary (no clap). Exit codes: 0 ok, 1 usage error, 2 io error,
//! 3 format error.

use std::fs;
use std::io::Write;

use crate::format::{self, Entry};

/// Parsed subcommand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// `build <in.tsv> <out.ior>`
    Build { input: String, output: String },
    /// `inspect <in.ior>`
    Inspect { input: String },
    /// `export <in.ior> <out.tsv>`
    Export { input: String, output: String },
    /// `merge <a.ior> <b.ior> -o <out.ior>`
    Merge {
        input_a: String,
        input_b: String,
        output: String,
    },
    /// `-h` / `--help`
    Help,
}

/// Usage text printed for `--help`, `-h`, and usage errors.
pub const USAGE: &str = "\
ior dictionary tool

USAGE:
    dict-tools <SUBCOMMAND> [ARGS]

SUBCOMMANDS:
    build   <in.tsv> <out.ior>          parse TSV, normalize, write .ior binary
    inspect <in.ior>                    print entry count, top entries, samples
    export <in.ior> <out.tsv>           dump .ior binary back to TSV
    merge   <a.ior> <b.ior> -o <out.ior>
                                        merge two .ior files (freq-sum, sorted)

OPTIONS:
    -h, --help                          print this help

NOTES:
    TSV lines are 'pinyin<TAB>hanzi<TAB>freq'; freq defaults to 1.
    Pinyin is lowercased; spaces or quotes separate syllables.

EXIT CODES:
    0 ok   1 usage error   2 io error   3 format error";

/// How many top-frequency entries `inspect` prints.
const TOP_COUNT: usize = 20;

/// How many file-order sample lines `inspect` prints.
const SAMPLE_COUNT: usize = 5;

/// Exit code: successful run.
pub const EXIT_OK: i32 = 0;
/// Exit code: bad command line.
pub const EXIT_USAGE: i32 = 1;
/// Exit code: file read/write failure.
pub const EXIT_IO: i32 = 2;
/// Exit code: TSV or .ior parsing failure.
pub const EXIT_FORMAT: i32 = 3;

/// Errors surfaced by command execution.
enum Error {
    Io(String),
    Format(String),
}

/// Parse raw arguments (without the program name) into a [`Command`].
pub fn parse_args(args: &[String]) -> Result<Command, String> {
    let Some(sub) = args.first() else {
        return Err("missing subcommand (try --help)".to_string());
    };
    let rest = &args[1..];
    match sub.as_str() {
        "-h" | "--help" => Ok(Command::Help),
        "build" => {
            let [input, output] = two_positional(rest, "build <in.tsv> <out.ior>")?;
            Ok(Command::Build { input, output })
        }
        "inspect" => {
            let [input] = one_positional(rest, "inspect <in.ior>")?;
            Ok(Command::Inspect { input })
        }
        "export" => {
            let [input, output] = two_positional(rest, "export <in.ior> <out.tsv>")?;
            Ok(Command::Export { input, output })
        }
        "merge" => parse_merge(rest),
        other => Err(format!("unknown subcommand '{other}' (try --help)")),
    }
}

/// Require exactly one positional argument.
fn one_positional(args: &[String], usage_line: &str) -> Result<[String; 1], String> {
    let [only] = args else {
        return Err(format!("expected: dict-tools {usage_line}"));
    };
    Ok([only.clone()])
}

/// Require exactly two positional arguments.
fn two_positional(args: &[String], usage_line: &str) -> Result<[String; 2], String> {
    let [first, second] = args else {
        return Err(format!("expected: dict-tools {usage_line}"));
    };
    Ok([first.clone(), second.clone()])
}

/// Parse `merge <a.ior> <b.ior> -o <out.ior>` (flag order is free).
fn parse_merge(args: &[String]) -> Result<Command, String> {
    let usage_line = "merge <a.ior> <b.ior> -o <out.ior>";
    let mut positional: Vec<String> = Vec::new();
    let mut output: Option<String> = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "-o" | "--output" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(format!("-o needs a value: dict-tools {usage_line}"));
                };
                output = Some(value.clone());
                index += 2;
            }
            other => {
                positional.push(other.to_string());
                index += 1;
            }
        }
    }
    if positional.len() != 2 {
        return Err(format!("expected: dict-tools {usage_line}"));
    }
    let Some(output) = output else {
        return Err(format!(
            "missing required -o <out.ior>: dict-tools {usage_line}"
        ));
    };
    let mut inputs = positional.into_iter();
    let (Some(input_a), Some(input_b)) = (inputs.next(), inputs.next()) else {
        return Err(format!("expected: dict-tools {usage_line}"));
    };
    Ok(Command::Merge {
        input_a,
        input_b,
        output,
    })
}

/// Run the parsed command, writing reports to `out`; returns the exit code.
pub fn run<W: Write>(args: &[String], out: &mut W) -> i32 {
    let command = match parse_args(args) {
        Ok(command) => command,
        Err(message) => {
            eprintln!("dict-tools: {message}\n");
            eprintln!("{USAGE}");
            return EXIT_USAGE;
        }
    };
    match execute(&command, out) {
        Ok(()) => EXIT_OK,
        Err(Error::Io(message)) => {
            eprintln!("dict-tools: io error: {message}");
            EXIT_IO
        }
        Err(Error::Format(message)) => {
            eprintln!("dict-tools: format error: {message}");
            EXIT_FORMAT
        }
    }
}

/// Dispatch one parsed command.
fn execute<W: Write>(command: &Command, out: &mut W) -> Result<(), Error> {
    match command {
        Command::Help => {
            writeln!(out, "{USAGE}").map_err(io_error)?;
            Ok(())
        }
        Command::Build { input, output } => {
            let text = read_text(input)?;
            let entries =
                format::parse_tsv(&text).map_err(|err| Error::Format(format!("{input}: {err}")))?;
            let entries = format::normalize(entries);
            let bytes = format::serialize(&entries);
            write_bytes(output, &bytes)?;
            report(
                out,
                &format!(
                    "built {output}: {} entries, {} bytes",
                    entries.len(),
                    bytes.len()
                ),
            )
        }
        Command::Inspect { input } => {
            let entries = load_ior(input)?;
            writeln!(out, "file: {input}").map_err(io_error)?;
            writeln!(out, "entries: {}", entries.len()).map_err(io_error)?;
            write_top_entries(out, &entries)?;
            write_sample_entries(out, &entries)
        }
        Command::Export { input, output } => {
            let entries = load_ior(input)?;
            write_bytes(output, format::to_tsv(&entries).as_bytes())?;
            report(
                out,
                &format!("exported {output}: {} entries", entries.len()),
            )
        }
        Command::Merge {
            input_a,
            input_b,
            output,
        } => {
            let mut combined = load_ior(input_a)?;
            let extra = load_ior(input_b)?;
            combined.extend(extra);
            let entries = format::normalize(combined);
            let bytes = format::serialize(&entries);
            write_bytes(output, &bytes)?;
            report(
                out,
                &format!(
                    "merged into {output}: {} entries, {} bytes",
                    entries.len(),
                    bytes.len()
                ),
            )
        }
    }
}

/// Print the highest-frequency entries (stable tiebreak on pinyin, hanzi).
fn write_top_entries<W: Write>(out: &mut W, entries: &[Entry]) -> Result<(), Error> {
    writeln!(out, "top {TOP_COUNT} by freq:").map_err(io_error)?;
    if entries.is_empty() {
        return writeln!(out, "  (none)").map_err(io_error);
    }
    let mut ranked: Vec<&Entry> = entries.iter().collect();
    ranked.sort_by(|a, b| {
        b.freq
            .cmp(&a.freq)
            .then_with(|| a.pinyin.cmp(&b.pinyin))
            .then_with(|| a.hanzi.cmp(&b.hanzi))
    });
    for (rank, entry) in ranked.iter().take(TOP_COUNT).enumerate() {
        writeln!(
            out,
            "{:>4}. {:>6}  {}\t{}",
            rank + 1,
            entry.freq,
            entry.pinyin,
            entry.hanzi
        )
        .map_err(io_error)?;
    }
    Ok(())
}

/// Print a few entries in file order as sample TSV lines.
fn write_sample_entries<W: Write>(out: &mut W, entries: &[Entry]) -> Result<(), Error> {
    let shown = entries.len().min(SAMPLE_COUNT);
    writeln!(out, "first {shown} entries (file order):").map_err(io_error)?;
    for entry in entries.iter().take(SAMPLE_COUNT) {
        writeln!(out, "  {}\t{}\t{}", entry.pinyin, entry.hanzi, entry.freq).map_err(io_error)?;
    }
    Ok(())
}

/// Read and deserialize a `.ior` file.
fn load_ior(path: &str) -> Result<Vec<Entry>, Error> {
    let bytes = read_bytes(path)?;
    format::deserialize(&bytes).map_err(|err| Error::Format(format!("{path}: {err}")))
}

fn read_text(path: &str) -> Result<String, Error> {
    fs::read_to_string(path).map_err(|err| Error::Io(format!("cannot read '{path}': {err}")))
}

fn read_bytes(path: &str) -> Result<Vec<u8>, Error> {
    fs::read(path).map_err(|err| Error::Io(format!("cannot read '{path}': {err}")))
}

fn write_bytes(path: &str, bytes: &[u8]) -> Result<(), Error> {
    fs::write(path, bytes).map_err(|err| Error::Io(format!("cannot write '{path}': {err}")))
}

fn report<W: Write>(out: &mut W, line: &str) -> Result<(), Error> {
    writeln!(out, "{line}").map_err(io_error)
}

fn io_error(err: std::io::Error) -> Error {
    Error::Io(err.to_string())
}
