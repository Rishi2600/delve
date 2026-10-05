//! The engine behind `delve`, a roguelike that turns a directory into a dungeon.
//!
//! Every directory becomes a room, every file a monster, and everything a
//! `.gitignore` hides a secret room. The same folder and seed always give
//! the same dungeon.
//!
//! **Strictly read-only.** Scanning only ever reads the filesystem, and
//! killing and looting change in-game state only.
//!
//! The pipeline has three steps:
//!
//! 1. [`scan::scan`] walks a directory into a [`scan::Tree`].
//! 2. [`mapgen::build_level`] lays that tree out as a [`mapgen::Level`].
//! 3. [`game::Game`] plays the level: feed it [`game::Action`]s, read its
//!    state, and ask for a [`game::Summary`] at the end.
//!
//! ```
//! use std::time::SystemTime;
//! use dungeon_delve::game::{Action, Game};
//! use dungeon_delve::{mapgen, scan};
//!
//! let root = std::env::current_dir()?.canonicalize()?;
//! let tree = scan::scan(&root);
//! let level = mapgen::build_level(&tree, mapgen::seed_for_path(&root));
//! println!("{}", level.to_ascii());
//!
//! let mut game = Game::new(level, SystemTime::now());
//! game.act(Action::Search);
//! game.act(Action::Move(1, 0));
//! let summary = game.summary();
//! println!("{} turns, {} of {} rooms explored",
//!     summary.turns, summary.rooms_explored, summary.rooms_total);
//! # Ok::<(), std::io::Error>(())
//! ```
//!
//! # Features
//!
//! - `tui` (default): the [`tui`] module, which plays a game in the terminal
//!   with ratatui and crossterm.
//! - `cli` (default): everything the `delve` command needs on top of `tui`.
//!
//! For the engine alone, turn default features off:
//! `dungeon-delve = { version = "0.2", default-features = false }`.

#![warn(missing_docs)]

pub mod entities;
pub mod game;
pub mod mapgen;
pub mod scan;
#[cfg(feature = "tui")]
pub mod tui;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod testutil;
#[cfg(feature = "tui")]
mod ui;
