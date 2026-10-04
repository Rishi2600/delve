# delve

A terminal roguelike that turns a directory into a dungeon.

```
delve [path]        # default: the current directory
```

Every directory becomes a room, every file becomes a monster, and everything your `.gitignore`
hides becomes a secret room. The same folder always produces the same dungeon.

**Strictly read-only.** `delve` only ever reads your filesystem. Killing and looting happen in game
state only; nothing is created, modified, touched or deleted (a test checks this, see
[Testing](#testing)).

## Build and run

Requires stable Rust (edition 2021).

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

Layout is a tidy tree on a grid: a room's column is its depth and rows are handed out by leaf count,
so rooms never overlap and every room is connected. You start just inside the root room's first
doorway; the stairs are in the deepest room you can reach without finding any secret.

Scan limits: 2000 entries, depth 6. Gitignored entries are budgeted separately (at most 48 entries per ignored
directory, 300 entries and 24 directories in total), so a huge `node_modules/` or `target/` becomes a compact secret area
instead of a maze, and cannot crowd out the rest of the tree. A room holds at most one monster per ten
floor tiles; if a directory has more files than that, its biggest files are kept.

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
  per attempt. Ignored *directories* (`target/`, `node_modules/`) become secret rooms; ignored *files*
  (`.env`, `*.log`) of a directory share one hidden cache room attached to it. Their monsters are
  tougher (+50% HP, +2 attack, double XP) and their loot is better.
- **Combat:** you deal `attack + weapon + 0..2 - armor` (at least 1); monsters deal
  `attack + 0..1 - your defense` (at least 1).
- **You:** 50 HP, 5 attack. Level `L` ends at `10 x L x (L+1)` total XP; each level gives +10 max HP,
  +1 attack (+1 defense every third level) and a **full heal**. You regenerate 1 HP every 4 turns.

## `--dump`

`--dump` prints the generated map as ASCII with no TUI. It is the ground-truth view (secret doors
show as `X`, mimics as `M`), and it is what the tests use. Example, for a small project with a
`.git` vault, an ignored `target/` and `.env`, symlinks, one unreadable directory and every file type:

```
$ delve --dump --seed 7 demo/
 ###########      #########
 #..O......#      #..!../.#
 #..j......#      #.......#
 #Gj......@+,,,,,,+.......#
 #j........#      #....../#
 #.........#      #.......#
 #.HO......#      #########
 #####+#####
      ,
      ,
      ,
      ,           #########
      ,           #.......#
      ,           #...M..B#
      ,,,,,,,,,,,,+.M.....#
      ,           #.......#
      ,           #.......#
      ,           #########
      ,
      ,
      ,
      ,           #########
      ,           #.......#
      ,           #.G.....#
      ,,,,,,,,,,,,+.....G.#
      ,           #.......#
      ,           #.......#
      ,           #########
      ,
      ,
      ,
      ,           #######
      ,           #:::::#
      ,,,,,,,,,,,,+:::::#
      ,           #:::::#
      ,           #######
      ,
      ,
      ,
      ,           #########
      ,           #..S....#
      ,           #.......#
      ,,,,,,,,,,,,+.......#
      ,           #.......#
      ,           #...g.H.#
      ,           #########
      ,
      ,
      ,
      ,           #########      #########
      ,           #.......#      #......C#
      ,           #..C....#      #.......#
      ,,,,,,,,,,,,+....C..+,,,,,,+.C...>.#
      ,           #.C.....#      #.......#
      ,           #.......#      #.......#
      ,           #########      #########
      ,
      ,
      ,
      ,           #########      #########      #########
      ,           #.......#      #.......#      #.......#
      ,           #.......#      #.......#      #.......#
      ,,,,,,,,,,,,X.......+,,,,,,+.......+,,,,,,+.......#
      ,           #.......#      #.......#      #.......#
      ,           #.......#      #.....j.#      #....j..#
      ,           #########      #########      #########
      ,
      ,
      ,
      ,           #########
      ,           #.......#
      ,           #.......#
      ,,,,,,,,,,,,X.......#
                  #..jj...#
                  #.......#
                  #########

Legend: # wall  . floor  : sealed room  , corridor  + door  X secret door  > stairs  @ start
Monsters: C crab(.rs)  S snake(.py)  g goblin(.js/.ts)  G ghost(.md/.txt)  H golem(.json/.toml/.yaml)
          M mimic(images)  B chest-monster(archives)  j slime(other)    O portal(symlink)
Items: ! potion  ? scroll  / blade  [ mail

/path/to/demo  seed 7  entries 34  map 59x79
rooms 12 (8 open, 4 behind 2 secret doors)  monsters 22
```

You can read it: the root room (top left) holds three slimes, a ghost, a golem and two portals, and
you start (`@`) at its east door, which leads to the `.git` vault (`!` potion, `/` blades). The sealed
`:` room is the unreadable directory. The two `X` doors guard `target/` (with its nested `debug/` and
`deps/` rooms) and the hidden cache holding `.env` and `build.log`. The mimics `M` and the
chest-monster `B` are in the assets room; the stairs `>` are in the deepest visible room.

## Testing

```sh
cargo test                       # 81 tests, about two seconds
cargo clippy --all-targets -- -D warnings
```

What the tests pin down:

- **Determinism:** same path and seed give an identical map; different seeds differ; FNV-1a matches
  the reference vectors.
- **Connectivity:** a BFS from the start reaches every room, over 200 random trees; secret rooms are
  reachable *only* through secret doors; rooms never overlap.
- **gitignore classification:** negation, nested `.gitignore` files, ignored entries kept (not
  skipped), everything inside an ignored directory is ignored.
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
- Developed and tested on Linux. The tests use Unix permissions and symlinks, so they are Unix-only.

## Layout

```
src/main.rs      CLI, terminal setup and teardown (panic-safe), event loop
src/scan.rs      read-only walker: Tree, gitignore classification, caps
src/mapgen.rs    FNV-1a seeding, layout, corridors, secret doors, --dump rendering
src/entities.rs  classes, size tiers, loot, player
src/game.rs      turns, combat, XP, monster AI, search, fog of war
src/ui.rs        ratatui rendering and key handling
DEVLOG.md        what was built, in order, and why
```
