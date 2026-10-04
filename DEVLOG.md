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
