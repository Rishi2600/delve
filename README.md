# delve

A terminal roguelike that turns a directory into a dungeon.

```
delve [path]        # default: the current directory
```

Every directory becomes a room, every file becomes a monster, and everything your `.gitignore`
hides becomes a secret room. The same folder always produces the same dungeon.

**New here?** The [usage guide](doc/USAGE.md) covers installing, playing and using the library step by step.

**Strictly read-only.** `delve` only ever reads your filesystem. Killing and looting happen in game
state only; nothing is created, modified, touched or deleted (a test checks this, see
[Testing](#testing)).

## Install

`delve` is developed and tested on **Linux** only. macOS will probably work (it uses only `crossterm` and
the standard library), but it has not been tried, and Windows is untested as well. It needs Rust 1.89 or newer.
The package is called `dungeon-delve` (the name `delve` was already taken on crates.io), and the command it
installs is `delve`.

```sh
cargo install dungeon-delve                                    # from crates.io
cargo install --git https://github.com/Rishi2600/delve         # straight from GitHub
```

Prebuilt Linux x86_64 binaries are attached to each
[GitHub release](https://github.com/Rishi2600/delve/releases): download, unpack and put `delve` on your `PATH`.
No Rust is needed for that route.

Then point it at any folder; it only reads:

```sh
delve                  # the current folder becomes the dungeon
delve ~/code/some-project
delve --seed 42 .      # a different layout
delve --dump .         # print the map without the game
```

## Build and run

To work on `delve` itself, clone the repository and build with stable Rust 1.89 or newer (edition 2021).

```sh
cargo build --release
./target/release/delve ~/some/project      # play
./target/release/delve --dump              # print the map as ASCII and exit
cargo run -- path/to/dir                   # or straight through cargo
```

| Option          | Meaning                                                                   |
|-----------------|---------------------------------------------------------------------------|
| `[path]`        | Directory to delve into (default: the current directory)                  |
| `--seed <u64>`  | Override the seed (default: FNV-1a hash of the canonical absolute path)   |
| `--no-color`    | Render without colors (the `NO_COLOR` environment variable works as well) |
| `--dump`        | Print the generated map as ASCII and exit, with no TUI                    |

The terminal is restored on normal exit, on errors **and on panic**.

## Use as a library

The same package is also a Rust library, imported as `dungeon_delve`: scan a folder, generate its dungeon and play
it from your own code, with your own frontend. For the engine alone, without the terminal UI and its dependencies,
turn default features off:

```toml
[dependencies]
dungeon-delve = { version = "0.2", default-features = false }
```

```rust
use std::time::SystemTime;
use dungeon_delve::game::{Action, Game};
use dungeon_delve::{mapgen, scan};

let root = std::path::Path::new("some/project").canonicalize()?;
let tree = scan::scan(&root);                                     // read-only walk
let level = mapgen::build_level(&tree, mapgen::seed_for_path(&root));
print!("{}", level.to_ascii());                                   // what --dump prints

let mut game = Game::new(level, SystemTime::now());
game.act(Action::Move(1, 0));                                     // step east (or attack)
println!("{} turns, {} files slain", game.summary().turns, game.summary().files_slain);
```

| Feature         | Default | Adds                                                                       |
|-----------------|---------|----------------------------------------------------------------------------|
| `tui`           | yes     | `dungeon_delve::tui::play`, the terminal game as a function (ratatui, crossterm) |
| `cli`           | yes     | the `delve` command itself (`tui` plus clap)                               |

The full API is documented on [docs.rs](https://docs.rs/dungeon-delve). The library is pre-1.0, so a minor
release (0.2 to 0.3) may still change the API.

## Controls

| Key                        | Action                                                          |
|----------------------------|-----------------------------------------------------------------|
| Arrow keys, `h` `j` `k` `l`| Move; walk into a monster to attack it                          |
| `y` `u` `b` `n`            | Diagonal moves (up-left, up-right, down-left, down-right)       |
| `s`                        | Search the eight surrounding tiles for secret doors             |
| `i`                        | Inventory: press an item's letter to use or equip it            |
| `.`                        | Wait a turn                                                     |
| `q` / `Ctrl-C`             | Quit (`q` closes the inventory instead while it is open)        |

The status bar shows HP, level, XP, attack, defense, the turn, and the **real filesystem path of
the room you are standing in**. Rooms are lit (you see all of one while inside it); elsewhere you see
by line of sight out to nine tiles, and explored areas stay on the map, dimmed.

You win by reaching the stairs (`>`) in the deepest room. You lose when your HP reaches 0. Either
way you get a report: rooms explored, files slain, the biggest foe defeated, and the real path where
the run ended.

## How the dungeon is built

| Filesystem                  | Dungeon                                                                 |
|-----------------------------|-------------------------------------------------------------------------|
| Directory                   | A room; its size scales with its file count (7x5 up to 30x13 interior)  |
| Subdirectory                | A room joined to its parent by an L-shaped corridor                     |
| File                        | A monster (class from extension, strength from size, mood from mtime)   |
| `.git`                      | A vault: a small room with treasure, never descended into               |
| Symlink                     | A portal `O` showing only its name; never followed                      |
| Unreadable directory        | A sealed room (`:` floor), empty but reachable                          |
| Gitignored dir or file      | A **secret room** behind a secret door (see below)                      |
| `node_modules/`, caches, lock files | Left out of the dungeon entirely (see below)                    |

Layout is a tidy tree on a grid: a room's column is its depth and rows are handed out by leaf count,
so rooms never overlap and every room is connected. You start just inside the root room's first
doorway; the stairs are in the deepest room you can reach without finding any secret.

Scan limits: 2000 entries, depth 6. Gitignored entries are budgeted separately (at most 48 entries per ignored
directory, 300 entries and 24 directories in total), so a project's own ignored folders become a compact secret
area instead of a maze, and cannot crowd out the rest of the tree. A room holds at most one monster per ten
floor tiles; if a directory has more files than that, its biggest files are kept.

**Left out entirely:** bulk that is no fun to play in is never read, never counts against the caps and
never becomes a room or a monster. That is `node_modules/` (always, gitignored or not) and the other
dependency and cache folders (`bower_components`, `.venv`, `venv`, `__pycache__`, `.next`, `.nuxt`,
`.svelte-kit`, `.turbo`, `.gradle`, `.tox`, and the mypy/pytest/ruff/parcel caches), the build-output folders
`target`, `dist`, `build`, `out`, `coverage`, `vendor` and `.cache` (only when gitignore says they are
generated, since a tracked `build/` is real source), and generated lock files (`package-lock.json`,
`yarn.lock`, `pnpm-lock.yaml`, `Cargo.lock`, `poetry.lock`, `go.sum`, ...). `--dump` reports how many entries
were left out. The lists are the constants at the top of `src/scan.rs`.

The seed is a hand-written 64-bit FNV-1a hash of the canonical absolute path, fed to ChaCha8, so the
same folder gives the same dungeon.

## Monsters

Class comes from the file extension (case-insensitive):

| Glyph | Class         | Extensions                          | Behavior                                                    |
|:-----:|---------------|-------------------------------------|-------------------------------------------------------------|
| `C`   | Crab          | `.rs`                               | Armored: takes 2 less damage                                |
| `S`   | Snake         | `.py` `.pyw`                        | Fast: moves two tiles per turn (but strikes once)           |
| `g`   | Goblin        | `.js` `.jsx` `.mjs` `.cjs` `.ts` `.tsx` | Erratic: moves randomly half the time                   |
| `G`   | Ghost         | `.md` `.markdown` `.txt` `.rst`     | Passes through walls; frail (-30% HP)                       |
| `H`   | Golem         | `.json` `.toml` `.yaml` `.yml`      | Slow (acts every other turn) and tanky (+60% HP, 1 armor)   |
| `M`   | Mimic         | `.png` `.jpg` `.jpeg` `.gif` `.bmp` `.webp` `.svg` `.ico` `.tif` `.tiff` | Looks like a potion `!` until you are adjacent, then ambushes you; never moves |
| `B`   | Chest-monster | `.zip` `.tar` `.gz` `.tgz` `.bz2` `.xz` `.7z` `.rar` `.zst` | Never moves, +30% HP, drops 3-5 items |
| `j`   | Slime         | anything else (including `.env`, `Makefile`) | Plain                                              |

Size sets the tier, on a log scale:

| File size       | Tier   | HP | Attack | XP  |
|-----------------|--------|---:|-------:|----:|
| < 1 KiB         | Rat    |  3 |      1 |   4 |
| < 16 KiB        | Small  |  6 |      1 |   8 |
| < 256 KiB       | Medium | 11 |      2 |  14 |
| < 1 MiB         | Large  | 18 |      3 |  26 |
| < 10 MiB        | Huge   | 30 |      5 |  50 |
| >= 10 MiB       | Boss   | 70 |      9 | 140 |

The class modifiers above apply on top. Monsters are named by both, e.g. *sturdy crab 'main.rs'*.

Modification time sets the mood:

| Last modified       | State                                                        |
|---------------------|--------------------------------------------------------------|
| within 7 days       | **Hunting**: awake and coming for you (within 16 steps)      |
| 1 year or more ago  | **Asleep** until you come within 3 tiles (drawn dim)         |
| anything in between | **Idle**: wanders, starts hunting when you get within 7 tiles |

A freshly cloned repository is therefore the hardest case: every file is hunting. Fight in
doorways and corridors.

## Loot, secrets and progression

- **Loot is named after the file**: killing `README.md` can drop *Scroll of README.md*. Each kill
  drops an item with probability 50% + 10% per tier (chest-monsters always drop 3-5, bosses one
  extra). Items: **Potion** (50%, heals `8 + 4q`), **Scroll** (15%, magic mapping: reveals
  everything reachable without a secret), **Blade** (18%, `+1 + q/2` attack), **Mail** (17%,
  `+1 + q/3` defense), where quality `q` is the foe's tier (0-5), plus 2 in secret rooms. Walk over
  items to pick them up (26 slots).
- **Secret doors:** ignored entries are invisible and drawn as plain wall. A corridor that dead-ends
  at a wall is the hint; `s` finds an adjacent secret door with probability `min(90, 35 + 10 x level)`%
  per attempt. Ignored *directories* (say a private `notes/` folder) become secret rooms; ignored *files*
  (`.env`, `*.log`) of a directory share one hidden cache room attached to it. Their monsters are
  tougher (+50% HP, +2 attack, double XP) and their loot is better.
- **Combat:** you deal `attack + weapon + 0..2 - armor` (at least 1); monsters deal
  `attack + 0..1 - your defense` (at least 1).
- **You:** 50 HP, 5 attack. Level `L` ends at `10 x L x (L+1)` total XP; each level gives +10 max HP,
  +1 attack (+1 defense every third level) and a **full heal**. You regenerate 1 HP every 4 turns.

## `--dump`

`--dump` prints the generated map as ASCII with no TUI. It is the ground-truth view (secret doors
show as `X`, mimics as `M`), and it is what the tests use. Example, for a small project with a
`.git` vault, an ignored `private/` folder and `.env`, symlinks, one unreadable directory, a `node_modules/`
and a lock file (both left out) and every file type:

```
$ delve --dump --seed 7 demo/
 ############      #########
 #....O.....#      #.......#
 #..jj......#      #.......#
 #OGG.H....@+,,,,,,+?/.....#
 #..........#      #.......#
 #.H........#      #....[..#
 #..........#      #########
 ######+#####
       ,
       ,
       ,
       ,           #########
       ,           #M......#
       ,           #.......#
       ,,,,,,,,,,,,+.......#
       ,           #..M....#
       ,           #B......#
       ,           #########
       ,
       ,
       ,
       ,           #########
       ,           #......H#
       ,           #.......#
       ,,,,,,,,,,,,+.......#
       ,           #......H#
       ,           #.......#
       ,           #########
       ,
       ,
       ,
       ,           #########
       ,           #.......#
       ,           #.......#
       ,,,,,,,,,,,,+...G...#
       ,           #.......#
       ,           #.....G.#
       ,           #########
       ,
       ,
       ,
       ,             #######
       ,             #:::::#
       ,,,,,,,,,,,,,,+:::::#
       ,             #:::::#
       ,             #######
       ,
       ,
       ,
       ,           #########      #########
       ,           #.......#      #.......#
       ,           #.......#      #.......#
       ,,,,,,,,,,,,X.......+,,,,,,+.......#
       ,           #.......#      #.......#
       ,           #.......#      #j......#
       ,           ####+####      #########
       ,               ,
       ,               ,
       ,               ,
       ,               ,          #########
       ,               ,          #.......#
       ,               ,          #.......#
       ,               ,,,,,,,,,,,+.......#
       ,                          #.......#
       ,                          #...j...#
       ,                          #########
       ,
       ,
       ,
       ,           #########
       ,           #...S...#
       ,           #.......#
       ,,,,,,,,,,,,+.......#
       ,           #.......#
       ,           #.g..H..#
       ,           #########
       ,
       ,
       ,
       ,           #########      #########
       ,           #.......#      #.......#
       ,           #.......#      #.......#
       ,,,,,,,,,,,,+...C...+,,,,,,+.S.....#
       ,           #....CC.#      #.......#
       ,           #.......#      #...>...#
       ,           #########      #########
       ,
       ,
       ,
       ,           #########
       ,           #.......#
       ,           #.....j.#
       ,,,,,,,,,,,,X.j.....#
                   #.......#
                   #.......#
                   #########

Legend: # wall  . floor  : sealed room  , corridor  + door  X secret door  > stairs  @ start
Monsters: C crab(.rs)  S snake(.py)  g goblin(.js/.ts)  G ghost(.md/.txt)  H golem(.json/.toml/.yaml)
          M mimic(images)  B chest-monster(archives)  j slime(other)    O portal(symlink)
Items: ! potion  ? scroll  / blade  [ mail

/path/to/demo  seed 7  entries 38, 2 left out (dependency dirs / lock files)  map 45x99
rooms 13 (9 open, 4 behind 2 secret doors)  monsters 24
```

You can read it: the root room (top left) holds two slimes, two ghosts, two golems and two portals, and
you start (`@`) at its east door, which leads to the `.git` vault (`?` scroll, `/` blade, `[` mail). The
sealed `:` room is the unreadable directory. The two `X` doors guard `private/` (with its nested `inner/`
and `deps/` rooms) and the hidden cache holding `.env` and `build.log`. The mimics `M` and the
chest-monster `B` are in the assets room; the stairs `>` are in the deepest visible room. `node_modules/`
and `package-lock.json` are in the demo project too, but they are left out, as the summary line says.

## Testing

```sh
cargo test                       # 85 tests, about two seconds
cargo clippy --all-targets -- -D warnings
```

What the tests pin down:

- **Determinism:** same path and seed give an identical map; different seeds differ; FNV-1a matches
  the reference vectors.
- **Connectivity:** a BFS from the start reaches every room, over 200 random trees; secret rooms are
  reachable *only* through secret doors; rooms never overlap.
- **gitignore classification:** negation, nested `.gitignore` files, ignored entries kept (not
  skipped), everything inside an ignored directory is ignored.
- **Left-out bulk:** `node_modules/` and lock files vanish with or without a `.gitignore`, look-alike names
  stay, `build/`-style folders go only when ignored, and left-out entries never eat the entry cap.
- **Size to tier mapping**, the extension table, mtime to state, loot naming, XP curve.
- **Read-only guarantee:** a snapshot of a temp directory (kinds, sizes, permissions, mtimes, content
  hashes, symlink targets) is identical before and after both a scan and a whole 400-turn session of
  fighting and looting. The test was mutation-checked: making the scanner create a file, or rewrite
  one with identical bytes, makes it fail.
- **Game rules:** combat, loot, searching, waking, pathing, per-class speed, mimics, ghosts, fog of
  war, line of sight, death and escape.
- **UI:** rendered frames through ratatui's `TestBackend`, key bindings, no-color mode.
- **Balance:** a stair-diving bot must be able to win *and* lose on a just-cloned project.

## Limitations

- A directory with hundreds of sibling subdirectories makes a very tall map (one row per leaf room).
- The scan is a snapshot at start-up; changes to the directory while you play are not seen.
- Developed and tested on Linux only. The tests use Unix permissions and symlinks, so they are Unix-only; macOS and Windows are untried.

## Layout

```
src/lib.rs       library root: crate docs, modules, features
src/main.rs      the `delve` command: argument parsing, --dump
src/tui.rs       terminal session: setup and teardown (panic-safe), event loop, end report
src/scan.rs      read-only walker: Tree, gitignore classification, caps
src/mapgen.rs    FNV-1a seeding, layout, corridors, secret doors, --dump rendering
src/entities.rs  classes, size tiers, loot, player
src/game.rs      turns, combat, XP, monster AI, search, fog of war
src/ui.rs        ratatui rendering and key handling
src/tests.rs     whole-session tests (read-only guarantee, winnability bot)
DEVLOG.md        what was built, in order, and why
```

## License

Licensed under either of

- [Apache License, Version 2.0](LICENSE-APACHE)
- [MIT license](LICENSE-MIT)

at your option. Unless you explicitly state otherwise, any contribution you intentionally submit for inclusion
in this project, as defined in the Apache-2.0 license, is dual licensed as above, without any additional terms
or conditions.
