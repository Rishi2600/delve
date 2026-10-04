# delve — development log

Running log of what was built, in order, and the decisions taken along the way.

## Stage 1 — scaffold

- `cargo init` as `delve`, **edition 2021**, stable toolchain (rustc 1.98).
- Dependencies: `ratatui`, `crossterm`, `ignore`, `rand`, `rand_chacha`, `clap` (derive). Nothing else;
  the no-write test uses a hand-rolled temp dir rather than pulling in `tempfile`.
- `rand` 0.10 renamed its traits (`Rng` is the core trait, `RngExt` carries `random_range` etc.).
- CLI skeleton: `delve [path] [--seed <u64>] [--no-color] [--dump]`.
- `/target` is gitignored; `Cargo.lock` is committed (binary crate).
