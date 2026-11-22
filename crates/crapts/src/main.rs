use std::{io, path::Path, process::ExitCode};

fn main() -> ExitCode {
    crapts::cli::run(
        Path::new("."),
        &std::env::args_os().skip(1).collect::<Vec<_>>(),
        &mut io::stdout().lock(),
        &mut io::stderr().lock(),
    )
}
