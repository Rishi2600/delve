use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Parser;

mod entities;
mod game;
mod mapgen;
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
    match run(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("delve: {msg}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: &Cli) -> Result<(), String> {
    let requested = cli.path.clone().unwrap_or_else(|| PathBuf::from("."));
    let root = requested
        .canonicalize()
        .map_err(|e| format!("cannot open {}: {e}", requested.display()))?;
    if !root.is_dir() {
        return Err(format!("{} is not a directory", root.display()));
    }
    let seed = cli.seed.unwrap_or_else(|| mapgen::seed_for_path(&root));
    let tree = scan::scan(&root);
    let level = mapgen::build_level(&tree, seed);

    if cli.dump {
        return dump(&root, seed, &tree, &level);
    }
    Err("the interactive game is not built yet; use --dump".to_string())
}

/// Print the generated map as ASCII. A closed pipe (`| head`) is not an error.
fn dump(root: &Path, seed: u64, tree: &scan::Tree, level: &mapgen::Level) -> Result<(), String> {
    let mut text = level.to_ascii();
    text.push('\n');
    text.push_str(mapgen::Level::legend());
    text.push('\n');
    text.push_str(&format!(
        "\n{}  seed {seed}  entries {}{}  rooms {}  monsters {}  map {}x{}\n",
        root.display(),
        tree.entry_count(),
        if tree.truncated { " (capped)" } else { "" },
        level.rooms.len(),
        level.spawns.len(),
        level.width,
        level.height
    ));
    match io::stdout().lock().write_all(text.as_bytes()) {
        Err(e) if e.kind() != io::ErrorKind::BrokenPipe => Err(format!("write failed: {e}")),
        _ => Ok(()),
    }
}
