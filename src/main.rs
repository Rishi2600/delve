use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;

mod scan;
#[cfg(test)]
mod testutil;

/// delve — turn a directory into a terminal roguelike dungeon.
///
/// Strictly read-only: the filesystem is only ever read, never written.
#[derive(Parser, Debug)]
#[command(name = "delve", version, about)]
struct Cli {
    /// Directory to turn into a dungeon (default: current directory).
    path: Option<PathBuf>,

    /// Override the seed derived from the directory path.
    #[arg(long)]
    seed: Option<u64>,

    /// Render without colors.
    #[arg(long)]
    no_color: bool,

    /// Print the generated map as ASCII and exit (no TUI).
    #[arg(long)]
    dump: bool,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    eprintln!("delve: not built yet ({cli:?})");
    ExitCode::FAILURE
}
