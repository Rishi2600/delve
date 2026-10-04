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
