# delve — development log

Running log of what was built, in order, and the decisions taken along the way.

## Stage 1 — scaffold

- `cargo init` as `delve`, **edition 2021**, stable toolchain (rustc 1.98).
- Dependencies: `ratatui`, `crossterm`, `ignore`, `rand`, `rand_chacha`, `clap` (derive). Nothing else;
  the no-write test uses a hand-rolled temp dir rather than pulling in `tempfile`.
- `rand` 0.10 renamed its traits (`Rng` is the core trait, `RngExt` carries `random_range` etc.).
- CLI skeleton: `delve [path] [--seed <u64>] [--no-color] [--dump]`.
- `/target` is gitignored; `Cargo.lock` is committed (binary crate).

## Stage 2 — `scan.rs` (read-only walker)

- BFS walk, entries sorted by raw name bytes, so output is deterministic and the 2000-entry cap trims the
  *deepest* entries first. Depth cap 6.
- `Tree { dirs, .. }` / `DirNode` / `FileNode { metadata: Meta, ignored, portal }`. Gitignore matching uses
  `ignore::gitignore::GitignoreBuilder` and classifies, never skips. Matcher chain per entry: the directory's own
  `.gitignore`, then each ancestor's, then `.gitignore` files above the root up to the enclosing repo, then
  `.git/info/exclude`. First matcher with an opinion wins (git semantics, so `!keep.log` works). Everything
  inside an ignored directory inherits `ignored = true`.
- `.git` directory → `DirKind::Vault`, never descended. Symlinks (to files *or* dirs) → `portal` file nodes, only
  `symlink_metadata` is ever used. Unreadable directory → `DirKind::Sealed`. No `unwrap`/`expect` in the module.
- **Decision:** ignored directories record at most 48 entries each, so a huge `node_modules/` cannot eat the
  whole 2000-entry budget and starve the real tree.
- 9 unit tests (classification, nested ignore, negation, vault, portals, entry/depth caps, sealed dirs,
  determinism). A hand-rolled `TempDir` lives in `src/testutil.rs` (test-only).

## Stage 3 — `entities.rs`

- `Pos` (Chebyshev `dist`), `Class` + the extension table (`.rs` crab, `.py` snake, `.js/.ts` goblin, `.md/.txt` ghost,
  `.json/.toml/.yaml` golem, images mimic, archives chest-monster, anything else slime; case-insensitive; dotfiles like
  `.env` have no extension → slime). Behaviour flags per class: speed (fast/normal/slow), armor, passes walls, erratic,
  stationary.
- `size_tier(size)`: `<1KiB` Rat, `<16KiB` Small, `<256KiB` Medium, `<1MiB` Large, `<10MiB` Huge, else Boss; each tier has
  base (hp, atk, xp). `monster_stats` mixes class modifiers in (golem +60% HP, chest +30%, ghost −30%, mimic +20%) and the
  secret-room bonus (+50% HP, +2 atk, 2× XP).
- `initial_state(mtime, now)`: modified < 7 days ago → Hunting; ≥ 365 days → Asleep (wakes within 3 tiles); in between →
  Idle (wanders until you are close). Future mtimes count as fresh; unknown mtime is Idle.
- Loot is always named after the file ("Scroll of README.md"): potion (heal), scroll (magic mapping), blade (+atk),
  mail (+def). Chests drop 3–5 items; secret-room monsters always drop and roll higher quality; bosses drop one extra.
- `Player`: 40 HP, 4 atk; level-up at cumulative `10·L·(L+1)` XP gives +8 max HP, +1 atk (+1 def every third level).
- 11 unit tests (table, tier boundaries, monotonic tiers, state by age, loot naming/determinism, XP).

## Stage 4 — `mapgen.rs` + `--dump`

- **Seeding:** hand-written 64-bit FNV-1a (checked against the reference vectors for `""`, `"a"`, `"foobar"`) over the
  canonical absolute path → `ChaCha8Rng::seed_from_u64`. `--seed` overrides the hash.
- **Layout:** a tidy-tree on a grid of cells. Column = depth in the room tree, rows are handed out by leaf count, and a
  parent shares its first child's row. Each column is as wide / each row as tall as the biggest room in it (an early
  fixed-size-cell version produced a 146×121 map for a 33-entry folder; sizing to content gave 59×70).
  Every room owns a cell, so rooms cannot overlap — by construction, and asserted by a test.
- **Corridors:** L-shaped: down (or up) the parent's centre column, then across to the child. In this layout the path can
  only touch the parent and the child, so connectivity is guaranteed and secret rooms cannot be entered by accident.
  Rock becomes corridor, a crossed wall becomes a door.
- **Room size** scales with occupant count (`area = max(30, 7·n)`, 2:1 aspect) and is capped at 30×13 interior. A room only
  holds `area/6` monsters; if more files exist, the *largest* files are kept.
- **Secret doors:** an ignored directory whose parent is visible gets a `SecretDoor` in its west wall; everything nested
  inside is hidden but normally connected. Ignored *files* of a visible directory go into a hidden **cache room** attached
  to that directory behind its own secret door (this is how `.env` / `*.log` become secret rooms).
- **`.git`** → vault room (7×5) with three pieces of treasure on the floor. **Unreadable dir** → sealed room (5×3, empty,
  `:` floor) that is still connected, so the BFS-connectivity guarantee holds.
- **Portals** (symlinks) are placed on wall-hugging tiles that are not next to a doorway, at most a third of the free ring,
  so they can never seal a room.
- **Start** in the root room; **stairs** in the deepest room that is *not* hidden, so the game is winnable without finding a
  secret. Monsters keep 3+ tiles away from the start.
- `Level::can_step` forbids squeezing diagonally between solid corners, so 4-connected BFS matches real movement.
- `--dump` prints the ground-truth map (secret doors as `X`, mimics as `M`) plus a legend and a summary line; a closed pipe is
  not an error.
- 16 tests: FNV vectors, determinism (synthetic + real dir), BFS reachability over 200 random trees, secret rooms
  reachable *only* through secret doors, no overlap, start/stairs placement, monster placement invariants, size scaling,
  vault/cache/secret classification from a real directory, portals, crowded-room cap.

## Stage 5 — `game.rs`

- `Game::act(Action)`; only time-consuming actions run the monsters' phase (bumping a wall or a portal is free). Actions:
  move/attack (bump), wait, search, use/equip inventory item, quit. Nothing in this module touches the filesystem.
- **Combat:** player damage = `atk + weapon + 0..2 − armor` (min 1); monster damage = `atk + 0..1 − defense` (min 1). Kills give
  XP, a `files_slain` count, a "biggest foe" record (by file size), and drop loot named after the file onto the nearest free
  floor tile. Diagonal strikes cannot cut solid corners.
- **Monster AI by class:** snake acts twice per turn, golem every other turn, goblin moves randomly half the time, ghost drifts
  straight at you through walls, chest-monster and mimic never move (a mimic stays an "item" until you are adjacent, then
  ambushes). Others follow a bounded BFS distance map (30 steps) around walls; if you are out of reach they mill about.
  Awake-and-hunting / idle / asleep come from the file's mtime (`entities::initial_state`); sleepers wake within 3 tiles,
  idle monsters notice you within 7. Monsters more than 40 tiles away are frozen to keep turns cheap.
- **Search (`s`):** each adjacent secret door is found with chance `min(90, 35 + 10·level)%`.
- **Items:** potion heals, scroll = magic mapping (reveals everything reachable *without* secrets, so hidden rooms stay hidden),
  blade/mail equip and swap. Pack holds 26 items.
- **Fog of war / LOS:** rooms are lit — standing inside one shows the whole room — and everything else uses Bresenham line of
  sight out to radius 9. `seen` persists, `visible` is per turn. Secret doors are opaque, like walls.
- **Summary** for the end screen: rooms explored, files slain, biggest foe, real path of the final room, killer, secrets, turns.
  Reaching the stairs → `Escaped`. Light HP regeneration (1 per 8 turns).
- 24 tests on a hand-built arena level: movement/corners, combat/loot/stats, chests, pickup, search (adjacent-only, level scaling),
  waking, hunting, pathing around walls, ghosts through walls, exact speed classes (snake 16 / slime 8 / golem 4 tiles in 8 turns),
  mimics, death + real path, potions/gear, fog memory, LOS, scroll mapping, stairs, portals, quit, determinism.
- Test-writing lesson: a single huge lit room marks everything seen, which made three fog/scroll tests vacuous at first;
  they now use a deliberately tiny "lit room" so only line of sight applies.
