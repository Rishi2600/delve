use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::SystemTime;

use clap::Parser;
use crossterm::cursor::Show;
use crossterm::event::{self, Event, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use game::{Game, Outcome};
use ui::View;

mod entities;
mod game;
mod mapgen;
mod scan;
#[cfg(test)]
mod testutil;
mod ui;

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
            "delve needs an interactive terminal (use --dump for a plain-text map)".to_string(),
        );
    }
    let color = !cli.no_color && std::env::var_os("NO_COLOR").is_none();
    let game = play(Game::new(level, SystemTime::now()), color)
        .map_err(|e| format!("terminal error: {e}"))?;
    print_report(&game);
    Ok(())
}

/// Put the terminal back the way we found it. Safe to call more than once
/// and when nothing was set up.
fn restore_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(io::stdout(), LeaveAlternateScreen, Show);
}

/// Make sure a panic anywhere leaves a usable terminal behind (and the panic
/// message readable, because it is printed after the screen is restored).
fn install_panic_hook() {
    let original = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        original(info);
    }));
}

fn play(mut game: Game, color: bool) -> io::Result<Game> {
    install_panic_hook();
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    if let Err(e) = execute!(stdout, EnterAlternateScreen) {
        restore_terminal();
        return Err(e);
    }
    let result = Terminal::new(CrosstermBackend::new(stdout))
        .and_then(|mut terminal| event_loop(&mut terminal, &mut game, color));
    restore_terminal();
    result.map(|()| game)
}

fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    game: &mut Game,
    color: bool,
) -> io::Result<()> {
    let mut view = View {
        color,
        inventory: false,
    };
    loop {
        terminal.draw(|frame| ui::draw(frame, game, &view))?;
        let Event::Key(key) = event::read()? else {
            continue; // resize etc: just redraw
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        if game.outcome != Outcome::Playing {
            return Ok(()); // any key leaves the end screen
        }
        ui::handle_key(game, &mut view, key);
        if game.outcome == Outcome::Quit {
            return Ok(());
        }
    }
}

/// Echo the end-of-run report to the normal screen, so it stays in scrollback.
fn print_report(game: &Game) {
    let mut out = io::stdout().lock();
    let _ = writeln!(out, "\n{}", ui::end_title(game.outcome));
    for line in ui::summary_lines(game.outcome, &game.summary()) {
        let _ = writeln!(out, "  {line}");
    }
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
        "\n{}  seed {seed}  entries {}{}  map {}x{}\nrooms {} ({open} open, {} behind {secret} secret door{})  monsters {}\n",
        root.display(),
        tree.entry_count(),
        if tree.truncated { " (capped)" } else { "" },
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
    use crate::entities::Pos;
    use crate::game::Action;
    use crate::testutil::{sample_project, snapshot};
    use rand::{RngExt, SeedableRng};
    use rand_chacha::ChaCha8Rng;

    /// Scan, generate, then play hundreds of turns of fighting and looting:
    /// the directory must come out byte-for-byte, mtime-for-mtime unchanged.
    #[test]
    fn a_whole_session_never_writes_to_the_filesystem() {
        let tmp = sample_project();
        let before = snapshot(tmp.path());

        let root = tmp.path().canonicalize().expect("canonical");
        let tree = scan::scan(&root);
        let level = mapgen::build_level(&tree, mapgen::seed_for_path(&root));
        let mut game = Game::new(level, SystemTime::now());
        game.player.atk = 500; // kill everything we bump into: maximum "destruction"
        game.player.max_hp = 100_000;
        game.player.hp = 100_000;

        let mut rng = ChaCha8Rng::seed_from_u64(5);
        for _ in 0..400 {
            let action = match rng.random_range(0..10) {
                0 => Action::Search,
                1 => Action::Wait,
                2 if !game.player.inventory.is_empty() => Action::Use(0),
                _ => Action::Move(rng.random_range(-1..=1), rng.random_range(-1..=1)),
            };
            game.act(action);
            // Teleport next to a random monster now and then so fights really happen.
            if rng.random_bool(0.1) {
                if let Some(m) = game.monsters.first() {
                    game.player.pos = Pos::new(m.pos.x + 1, m.pos.y);
                }
            }
        }
        assert!(game.summary().turns > 100, "a real session happened");
        assert_eq!(snapshot(tmp.path()), before);
    }

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
        let tmp = sample_project();
        let file = tmp.path().join("src/main.rs");
        let err = run(&cli(&file.display().to_string())).expect_err("must fail");
        assert!(err.contains("not a directory"), "{err}");
    }
}
