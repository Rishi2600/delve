//! Play a [`Game`] in the terminal, the way the `delve` command does.
//!
//! Only available with the `tui` feature (on by default). Library users who
//! bring their own frontend can turn default features off and drive
//! [`Game::act`] themselves.
//!
//! ```no_run
//! use std::time::SystemTime;
//! use dungeon_delve::game::Game;
//! use dungeon_delve::{mapgen, scan, tui};
//!
//! let root = std::env::current_dir()?.canonicalize()?;
//! let level = mapgen::build_level(&scan::scan(&root), mapgen::seed_for_path(&root));
//! let game = tui::play(Game::new(level, SystemTime::now()), true)?;
//! print!("{}", tui::report(&game));
//! # Ok::<(), std::io::Error>(())
//! ```

use std::io;
use std::sync::Once;

use crossterm::cursor::Show;
use crossterm::event::{self, Event, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use crate::game::{Game, Outcome};
use crate::ui::{self, View};

/// Run an interactive session on stdin/stdout until the player dies,
/// escapes or quits, then hand back the finished game.
///
/// Takes over the terminal (raw mode, alternate screen) and always restores
/// it, on errors and on panic too. `color: false` draws without colors.
/// Both stdin and stdout must be a terminal.
pub fn play(mut game: Game, color: bool) -> io::Result<Game> {
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

/// The end-of-run report (title plus one line per stat) as plain text, for
/// printing to the normal screen after [`play`] returns.
pub fn report(game: &Game) -> String {
    let mut out = format!("\n{}\n", ui::end_title(game.outcome));
    for line in ui::summary_lines(game.outcome, &game.summary()) {
        out.push_str(&format!("  {line}\n"));
    }
    out
}

/// Put the terminal back the way we found it. Safe to call more than once
/// and when nothing was set up.
fn restore_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(io::stdout(), LeaveAlternateScreen, Show);
}

/// Make sure a panic anywhere leaves a usable terminal behind (and the panic
/// message readable, because it is printed after the screen is restored).
/// Installed once, however many sessions are played.
fn install_panic_hook() {
    static HOOK: Once = Once::new();
    HOOK.call_once(|| {
        let original = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            restore_terminal();
            original(info);
        }));
    });
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
