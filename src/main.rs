use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::SystemTime;

use clap::Parser;

use dungeon_delve::game::Game;
use dungeon_delve::{mapgen, scan, tui};

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

    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err(
            "an interactive terminal is required (use --dump for a plain-text map)".to_string(),
        );
    }
    let color = !cli.no_color && std::env::var_os("NO_COLOR").is_none();
    let game = tui::play(Game::new(level, SystemTime::now()), color)
        .map_err(|e| format!("terminal error: {e}"))?;
    // Echo the report to the normal screen, so it stays in scrollback.
    let _ = io::stdout().lock().write_all(tui::report(&game).as_bytes());
    Ok(())
}

/// Print the generated map as ASCII. A closed pipe (`| head`) is not an error.
fn dump(root: &Path, seed: u64, tree: &scan::Tree, level: &mapgen::Level) -> Result<(), String> {
    let mut text = level.to_ascii();
    text.push('\n');
    text.push_str(mapgen::Level::legend());
    text.push('\n');
    let open = level.reachable_rooms(false).iter().filter(|&&r| r).count();
    let secret = level.rooms.iter().filter(|r| r.secret_entry).count();
    text.push_str(&format!(
        "\n{}  seed {seed}  entries {}{}{}  map {}x{}\nrooms {} ({open} open, {} behind {secret} secret door{})  monsters {}\n",
        root.display(),
        tree.entry_count(),
        if tree.truncated { " (capped)" } else { "" },
        if tree.skipped > 0 {
            format!(", {} left out (dependency dirs / lock files)", tree.skipped)
        } else {
            String::new()
        },
        level.width,
        level.height,
        level.rooms.len(),
        level.rooms.len() - open,
        if secret == 1 { "" } else { "s" },
        level.spawns.len(),
    ));
    match io::stdout().lock().write_all(text.as_bytes()) {
        Err(e) if e.kind() != io::ErrorKind::BrokenPipe => Err(format!("write failed: {e}")),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_paths_and_files_are_clean_errors_not_panics() {
        let cli = |path: &str| Cli {
            path: Some(PathBuf::from(path)),
            seed: None,
            no_color: true,
            dump: true,
        };
        let err = run(&cli("/definitely/not/a/real/path")).expect_err("must fail");
        assert!(err.contains("cannot open"), "{err}");
        let file = concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml");
        let err = run(&cli(file)).expect_err("must fail");
        assert!(err.contains("not a directory"), "{err}");
    }
}
