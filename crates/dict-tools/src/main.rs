//! `dict-tools` binary entry point: a thin wrapper that hands
//! `std::env::args()` to [`dict_tools::cli::run`] and exits with its code.

use dict_tools::cli;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = cli::run(&args, &mut std::io::stdout().lock());
    std::process::exit(code);
}
