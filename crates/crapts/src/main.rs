use std::{
    io::{self, IsTerminal},
    path::Path,
    process::ExitCode,
};

fn main() -> ExitCode {
    let stdout = io::stdout();
    let terminal = stdout.is_terminal();
    crapts::cli::run_with_stdout(
        Path::new("."),
        &std::env::args_os().skip(1).collect::<Vec<_>>(),
        &mut stdout.lock(),
        &mut io::stderr().lock(),
        terminal,
        cargo_crap_core::report::output_width(terminal),
    )
}
