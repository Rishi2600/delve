# Using delve

`delve` is a terminal roguelike that turns a directory into a dungeon. Every folder becomes a room, every file
becomes a monster, and everything your `.gitignore` hides becomes a secret room. The same folder always produces
the same dungeon.

It ships in two forms from one package, [`dungeon-delve`](https://crates.io/crates/dungeon-delve):

- **The `delve` command**, which you play in a terminal. See [Playing the game](#playing-the-game).
- **The `dungeon_delve` Rust library**, which runs the same engine from your own code. See
  [Using the library](#using-the-library).

**Strictly read-only.** `delve` only reads your filesystem. Killing monsters and looting files happen in game
state only. Nothing on disk is created, changed or deleted, and a test checks this on every release.

---

## Playing the game

### Requirements

- **Linux.** This is the only platform `delve` has been tested on. macOS will probably work, and Windows is
  untested.
- **A terminal at least 40 columns by 14 rows.**
- **Rust 1.89 or newer**, only if you install with `cargo`. The prebuilt download needs no Rust.

### Install

Pick one of these three ways.

**1. From crates.io (needs Rust).** This is the usual way.

```sh
cargo install dungeon-delve
```

Cargo downloads the source, compiles it and puts the `delve` command in `~/.cargo/bin`. That folder is already on
your `PATH` if you installed Rust with rustup. The package is called `dungeon-delve` because the name `delve` was
taken on crates.io, but the command it installs is `delve`.

**2. Prebuilt Linux binary (no Rust needed).**

1. Open the [latest release](https://github.com/Rishi2600/delve/releases/latest) and download
   `delve-vX.Y.Z-x86_64-unknown-linux-gnu.tar.gz`. The `.sha256` file next to it is optional and lets you check
   the download.
2. Unpack it and put `delve` somewhere on your `PATH`:

   ```sh
   sha256sum -c delve-v*-x86_64-unknown-linux-gnu.tar.gz.sha256      # optional: prints "OK"
   tar xzf delve-v*-x86_64-unknown-linux-gnu.tar.gz
   sudo mv delve-v*-x86_64-unknown-linux-gnu/delve /usr/local/bin/   # or ~/.local/bin, no sudo needed
   ```

The binary is built for 64-bit Linux on Ubuntu 22.04, and should run on that or any newer distribution.

**3. Straight from GitHub (needs Rust).** This gets the latest unreleased code.

```sh
cargo install --git https://github.com/Rishi2600/delve
```

### Update and uninstall

An installed `delve` never updates itself.

| Installed with | Update | Uninstall |
|---|---|---|
| `cargo install dungeon-delve` | Run the same command again; it upgrades when a newer version exists | `cargo uninstall dungeon-delve` |
| Release download | Download the new release and replace the file | Delete the `delve` file |
| `cargo install --git ...` | Run the same command again; it rebuilds when there are new commits | `cargo uninstall dungeon-delve` |

Check what you have with `delve --version`.

### Run it

```sh
delve                      # the current folder becomes the dungeon
delve ~/code/some-project  # any folder you can read
delve --seed 42 .          # same folder, different layout
delve --dump .             # print the map as text and exit, no game
delve --no-color .         # play without colors
```

| Option | Meaning |
|---|---|
| `[path]` | Folder to turn into a dungeon (default: the current folder) |
| `--seed <number>` | Use a different layout. By default the seed comes from the folder's full path, so a folder always gets the same dungeon. |
| `--dump` | Print the generated map as ASCII with a legend and exit. This works in pipes and scripts. |
| `--no-color` | Draw without colors. Setting the `NO_COLOR` environment variable does the same. |
| `--help`, `--version` | What you would expect |

### Controls

| Key | Action |
|---|---|
| Arrow keys, `h` `j` `k` `l` | Move. Walk into a monster to attack it. |
| `y` `u` `b` `n` | Move diagonally (up-left, up-right, down-left, down-right) |
| `s` | Search the eight surrounding tiles for secret doors |
| `i` | Open the inventory, then press an item's letter to use or equip it |
| `.` | Wait a turn |
| `q`, `Ctrl-C` | Quit (`q` closes the inventory instead while it is open) |

### How to win

Find the stairs (`>`) in the deepest room and step on them. You lose if your HP reaches 0. Either way you get a
report of the rooms you explored, the files you slew, the biggest foe you defeated, and the real path where your
run ended.

A few things worth knowing:

- **Monsters come from your files.** The file's extension decides what it is: `.rs` is an armored crab, `.py` a
  fast snake, `.md` a ghost that walks through walls, and an image is a mimic that looks like treasure. The
  file's size decides how strong it is.
- **Fresh files hunt you.** Files changed in the last week are awake and hunting. Files untouched for a year are
  asleep.
- **Gitignored things are secret rooms.** They sit behind secret doors (`X` in `--dump`), which look like plain
  walls in the game. Press `s` next to a suspicious wall to search for one.
- **The status bar shows a real path**, the folder of the room you are standing in.

The [README](../README.md) has the full monster, item and dungeon-building tables.

### Troubleshooting

| Message or problem | Fix |
|---|---|
| `an interactive terminal is required` | `delve` was run with its input or output redirected (a pipe, a file, some IDE consoles). Run it in a real terminal, or use `--dump`. |
| `Terminal too small: need 40x14` instead of the map | Make the window bigger; the game redraws as soon as it fits. |
| `cannot open ...` or `... is not a directory` | Pass a folder that exists. Files are not dungeons. |
| `binary 'delve' already exists ... as part of ...` during `cargo install` | Another package installed a `delve` command. Add `--force` to replace it. |
| A huge folder makes a very tall map | This is known: every leaf folder gets its own row. Point `delve` at a smaller subfolder. |

---

## Using the library

The engine is a normal Rust library. You can scan a folder, generate its dungeon, and play it from your own code,
with your own frontend (a web page, a bot, a different TUI). The full API reference is on
[docs.rs/dungeon-delve](https://docs.rs/dungeon-delve).

### Add it

The library is a dependency of your own Rust project, like an npm package. Inside your project, run:

```sh
cargo add dungeon-delve --no-default-features    # engine only: no terminal libraries are pulled in
```

That is the Rust version of `npm install <package>`. It adds this line to your `Cargo.toml`, which you can also
write by hand:

```toml
[dependencies]
dungeon-delve = { version = "0.2.0", default-features = false }
```

The next `cargo build` downloads and compiles it. In code the library is called `dungeon_delve`, with an
underscore.

Do not use `cargo install` for this. `cargo install` only installs commands (like `npm install -g`). It installs
the `delve` game, not the library.

| Feature | On by default | What it adds |
|---|---|---|
| *(none)* | | The engine: `scan`, `mapgen`, `entities`, `game`. Depends only on `ignore`, `rand` and `rand_chacha`. |
| `tui` | yes | The `tui` module: the terminal game as a function (pulls in ratatui and crossterm) |
| `cli` | yes | Everything the `delve` command needs (`tui` plus clap). Library users do not need it. |

To embed the terminal game without the command-line parts, run
`cargo add dungeon-delve --no-default-features --features tui`.

### How it fits together

Everything goes through three steps:

```text
folder ──scan::scan──▶ Tree ──mapgen::build_level──▶ Level ──Game::new──▶ Game ◀──game.act(Action)
```

1. **`scan::scan(&path)`** walks the folder (read-only) into a `Tree` of directories and files. Gitignored
   entries are marked, not skipped.
2. **`mapgen::build_level(&tree, seed)`** lays the tree out as a `Level`: tiles, rooms, monster spawns, items,
   the start and the stairs. Use `mapgen::seed_for_path(&path)` for the same seed the `delve` command would use.
3. **`Game::new(level, now)`** starts a game, and **`game.act(action)`** plays one move at a time. You read the
   game's public fields and methods to draw it.

### Example: print a folder's map

This is what `delve --dump` does.

```rust
use dungeon_delve::{mapgen, scan};

fn main() -> std::io::Result<()> {
    let root = std::path::Path::new(".").canonicalize()?;
    let tree = scan::scan(&root);
    let seed = mapgen::seed_for_path(&root); // or any u64 you like
    let level = mapgen::build_level(&tree, seed);

    print!("{}", level.to_ascii());
    println!("{}", mapgen::Level::legend());
    println!("{} rooms, {} monsters, map {}x{}",
        level.rooms.len(), level.spawns.len(), level.width, level.height);
    Ok(())
}
```

### Example: look at what the scan found

```rust
use std::collections::BTreeMap;
use dungeon_delve::entities::{size_tier, Class};
use dungeon_delve::scan;

fn main() -> std::io::Result<()> {
    let tree = scan::scan(&std::path::Path::new(".").canonicalize()?);
    println!("{} entries{}", tree.entry_count(), if tree.truncated { " (capped)" } else { "" });

    let mut bestiary: BTreeMap<&str, usize> = BTreeMap::new();
    for dir in &tree.dirs {
        for file in dir.files.iter().filter(|f| !f.portal) {
            let class = Class::for_file_name(&file.name);
            *bestiary.entry(class.name()).or_default() += 1;
            if file.ignored {
                println!("secret: {}/{} ({} {})", dir.path.display(), file.name,
                    size_tier(file.metadata.size).adjective(), class.name());
            }
        }
    }
    println!("{bestiary:?}"); // e.g. {"crab": 10, "ghost": 2, "golem": 2, "slime": 3}
    Ok(())
}
```

### Example: draw the game yourself

A frontend reads the game's state and draws it. This one draws a text frame with fog of war: tiles the player has
seen stay on the map, but monsters show only while in view.

```rust
use std::time::SystemTime;
use dungeon_delve::entities::Pos;
use dungeon_delve::game::Game;
use dungeon_delve::mapgen::{self, Tile};
use dungeon_delve::scan;

fn frame(game: &Game) -> String {
    let mut out = String::new();
    for y in 0..game.level.height {
        for x in 0..game.level.width {
            let p = Pos::new(x, y);
            let ch = if p == game.player.pos {
                '@'
            } else if !game.is_seen(p) {
                ' '
            } else if let Some(i) = game.monster_at(p).filter(|_| game.is_visible(p)) {
                let m = &game.monsters[i];
                if m.disguised { '$' } else { m.class.glyph() }
            } else {
                match game.level.tile(p) {
                    Tile::SecretDoor => '#', // looks like a wall until found
                    tile => tile.ascii(),
                }
            };
            out.push(ch);
        }
        out.push('\n');
    }
    out
}

fn main() -> std::io::Result<()> {
    let root = std::path::Path::new(".").canonicalize()?;
    let level = mapgen::build_level(&scan::scan(&root), mapgen::seed_for_path(&root));
    let game = Game::new(level, SystemTime::now());
    print!("{}", frame(&game));
    println!("HP {}/{}  in {}", game.player.hp, game.player.max_hp, game.current_path().display());
    Ok(())
}
```

### Example: play a game from code

`Action::Move(dx, dy)` steps one tile (and attacks whatever is there). Moves that go nowhere, like bumping a
wall, cost no turn.

```rust
use std::time::SystemTime;
use dungeon_delve::game::{Action, Game, Outcome};
use dungeon_delve::{mapgen, scan};

fn main() -> std::io::Result<()> {
    let root = std::path::Path::new(".").canonicalize()?;
    let level = mapgen::build_level(&scan::scan(&root), 7);
    let mut game = Game::new(level, SystemTime::now());

    // A very silly bot: walk east, search now and then, give up after 500 turns.
    while game.outcome == Outcome::Playing && game.turn < 500 {
        let action = if game.turn % 10 == 0 { Action::Search } else { Action::Move(1, 0) };
        let before = game.turn;
        game.act(action);
        if game.turn == before {
            game.act(Action::Wait); // bumped a wall: that costs no turn
        }
    }
    if game.outcome == Outcome::Playing {
        game.act(Action::Quit);
    }

    for line in game.messages.iter().rev().take(5) {
        println!("log: {line}");
    }
    let s = game.summary();
    println!("{:?} after {} turns: {}/{} rooms, {} files slain, ended in {}",
        game.outcome, s.turns, s.rooms_explored, s.rooms_total, s.files_slain, s.final_path.display());
    Ok(())
}
```

### Example: embed the terminal game

Needs the `tui` feature.

```rust
use std::time::SystemTime;
use dungeon_delve::game::Game;
use dungeon_delve::{mapgen, scan, tui};

fn main() -> std::io::Result<()> {
    let root = std::path::Path::new(".").canonicalize()?;
    let level = mapgen::build_level(&scan::scan(&root), mapgen::seed_for_path(&root));
    let game = tui::play(Game::new(level, SystemTime::now()), true)?; // blocks until the run ends
    print!("{}", tui::report(&game));
    Ok(())
}
```

`tui::play` takes over the terminal and always restores it, including on errors and panics. Both standard input
and output must be a terminal.

### What the API offers

| Module | Main items |
|---|---|
| `scan` | `scan(&Path) -> Tree`; `Tree` (`dirs`, `truncated`, `skipped`, `entry_count()`); `DirNode`, `FileNode`, `Meta`, `DirKind`; caps `MAX_ENTRIES` (2000) and `MAX_DEPTH` (6) |
| `mapgen` | `seed_for_path`, `build_level`; `Level` (`width`, `height`, `rooms`, `start`, `stairs`, `spawns`, `items`, `portals`, `tile()`, `room_at()`, `can_step()`, `reachable_rooms()`, `to_ascii()`, `legend()`); `Tile`, `Room`, `RoomKind`, `Portal` |
| `entities` | `Pos`; `Class` (from a file name, with glyph, speed and armor); `SizeTier` and `size_tier()`; `MonsterState`; `Monster`; `Item`, `ItemKind`; `Player`; `FileInfo`, `Spawn` |
| `game` | `Game` (`new`, `act`, `summary`, `is_visible`, `is_seen`, `monster_at`, `portal_at`, `current_path`, plus the `level`, `player`, `monsters`, `floor_items`, `messages`, `turn` and `outcome` fields); `Action`; `Outcome`; `Summary` |
| `tui` (feature) | `play(game, color) -> io::Result<Game>`, `report(&game) -> String` |

### Good to know

- **Determinism.** The same folder and seed always give the same `Level`. A game's dice are seeded from the
  level, so the same level, the same `now` and the same actions replay the same game. `now` only decides which
  monsters start awake.
- **Read-only.** The library never writes to disk, and it never follows symlinks (they become portals).
- **Scan limits.** At most 2000 entries and 6 levels deep. Dependency and cache folders (`node_modules`, `.venv`,
  `__pycache__` and similar) and lock files are left out entirely.
- **Matching on enums needs a `_` arm.** The public enums (`Tile`, `Class`, `Outcome` and so on) and data structs
  are `#[non_exhaustive]`, so new variants and fields can arrive in a minor release. For the same reason you
  cannot build those structs yourself; the library hands them to you.
- **Versioning.** The library is pre-1.0. Patch releases (0.2.x) never break your code. A minor release (0.3)
  may, and its notes will say what changed. Depending on `"0.2"` gets you all 0.2.x fixes automatically.

---

## Help and source

- Source, issues and releases: [github.com/Rishi2600/delve](https://github.com/Rishi2600/delve)
- API reference: [docs.rs/dungeon-delve](https://docs.rs/dungeon-delve)
- License: MIT or Apache-2.0, at your option
