//! Map generation: turns a scanned [`Tree`] into a [`Level`].
//!
//! Layout is a tidy-tree on a grid of cells: a room's column is its depth in
//! the room tree and rows are handed out by leaf count, so every room sits in
//! its own cell and rooms can never overlap. Each column is as wide, and each
//! row as tall, as the biggest room it holds. A subdirectory is joined
//! to its parent by an L-shaped corridor (down the parent's column, then
//! across); in that layout the corridor can only ever touch those two rooms,
//! which keeps secret rooms secret and the whole dungeon connected.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use rand::{Rng, RngExt, SeedableRng};
use rand_chacha::ChaCha8Rng;

use crate::entities::{vault_loot, FileInfo, Item, Pos, Spawn};
use crate::scan::{DirKind, Tree};

/// Empty tiles between layout columns / rows (room for corridors).
const GAP_X: i32 = 6;
const GAP_Y: i32 = 3;
const MAX_ROOM_W: i32 = 30;
const MAX_ROOM_H: i32 = 13;
/// Tiles of floor per monster, at most (keeps rooms from being packed solid).
const AREA_PER_MONSTER: i32 = 10;
/// Marker in `room_map` for tiles that belong to no room.
pub const NO_ROOM: u32 = u32::MAX;

// ---------------------------------------------------------------------------
// Seeding
// ---------------------------------------------------------------------------

/// 64-bit FNV-1a, hand-written (never `DefaultHasher`, whose output is not
/// stable across Rust versions).
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

/// The default seed: FNV-1a of the canonical absolute path.
pub fn seed_for_path(canonical: &Path) -> u64 {
    fnv1a64(canonical.as_os_str().as_encoded_bytes())
}

// ---------------------------------------------------------------------------
// Level data
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tile {
    /// Solid rock; renders as nothing.
    Rock,
    Wall,
    Floor,
    /// Floor of a sealed (unreadable) room.
    SealedFloor,
    Corridor,
    Door,
    /// Looks exactly like a wall until found with `s`.
    SecretDoor,
    Stairs,
}

impl Tile {
    /// Can be stepped on (a secret door cannot, until it is found).
    pub fn walkable(self) -> bool {
        !matches!(self, Tile::Rock | Tile::Wall | Tile::SecretDoor)
    }

    /// Blocks line of sight.
    pub fn opaque(self) -> bool {
        matches!(self, Tile::Rock | Tile::Wall | Tile::SecretDoor)
    }

    /// Ground-truth ASCII (secret doors show as `X`; the game draws them as walls).
    pub fn ascii(self) -> char {
        match self {
            Tile::Rock => ' ',
            Tile::Wall => '#',
            Tile::Floor => '.',
            Tile::SealedFloor => ':',
            Tile::Corridor => ',',
            Tile::Door => '+',
            Tile::SecretDoor => 'X',
            Tile::Stairs => '>',
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoomKind {
    Normal,
    /// The `.git` directory.
    Vault,
    /// An unreadable directory.
    Sealed,
    /// Holds the gitignored *files* of a directory (`.env`, `*.log`, ...).
    Cache,
}

#[derive(Debug, Clone)]
pub struct Room {
    pub id: usize,
    pub kind: RoomKind,
    /// Directory name (or `(hidden) name` for a cache).
    pub name: String,
    /// The real path of the directory this room stands for.
    pub path: PathBuf,
    /// Interior rectangle (walls are one tile outside it).
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    /// Scan depth of the directory.
    pub depth: u32,
    /// Inside an ignored directory (or an ignored-file cache).
    pub hidden: bool,
    /// Entered through a secret door.
    pub secret_entry: bool,
    /// How many entries the directory holds.
    pub entries: usize,
}

impl Room {
    pub fn center(&self) -> Pos {
        Pos::new(self.x + self.w / 2, self.y + self.h / 2)
    }

    pub fn contains(&self, p: Pos) -> bool {
        p.x >= self.x && p.x < self.x + self.w && p.y >= self.y && p.y < self.y + self.h
    }

    /// Interior tiles in row-major order.
    pub fn tiles(&self) -> impl Iterator<Item = Pos> + '_ {
        (self.y..self.y + self.h)
            .flat_map(move |y| (self.x..self.x + self.w).map(move |x| Pos::new(x, y)))
    }

    /// The outer rectangle including walls: `(x, y, w, h)`.
    pub fn outer(&self) -> (i32, i32, i32, i32) {
        (self.x - 1, self.y - 1, self.w + 2, self.h + 2)
    }
}

/// A symlink: shows only its name, never followed.
#[derive(Debug, Clone)]
pub struct Portal {
    pub pos: Pos,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct Level {
    pub width: i32,
    pub height: i32,
    pub tiles: Vec<Tile>,
    /// Room id per tile (interior *and* walls), or [`NO_ROOM`].
    pub room_map: Vec<u32>,
    pub rooms: Vec<Room>,
    pub start: Pos,
    pub stairs: Pos,
    pub spawns: Vec<Spawn>,
    /// Items lying on the floor at generation time (vault treasure).
    pub items: Vec<(Pos, Item)>,
    pub portals: Vec<Portal>,
    pub seed: u64,
}

impl Level {
    fn index(&self, p: Pos) -> Option<usize> {
        if p.x >= 0 && p.y >= 0 && p.x < self.width && p.y < self.height {
            Some((p.y * self.width + p.x) as usize)
        } else {
            None
        }
    }

    pub fn in_bounds(&self, p: Pos) -> bool {
        self.index(p).is_some()
    }

    /// Out-of-bounds reads as solid rock.
    pub fn tile(&self, p: Pos) -> Tile {
        self.index(p).map_or(Tile::Rock, |i| self.tiles[i])
    }

    pub fn set_tile(&mut self, p: Pos, t: Tile) {
        if let Some(i) = self.index(p) {
            self.tiles[i] = t;
        }
    }

    /// The room whose interior or walls contain `p`.
    pub fn room_at(&self, p: Pos) -> Option<usize> {
        let id = self.index(p).map(|i| self.room_map[i])?;
        (id != NO_ROOM).then_some(id as usize)
    }

    /// Can a creature step from `from` to the adjacent `to`? No walking
    /// through solid corners on diagonals. `secret_open` treats secret doors
    /// as open (used to check connectivity).
    pub fn can_step(&self, from: Pos, to: Pos, secret_open: bool) -> bool {
        let passable = |t: Tile| t.walkable() || (secret_open && t == Tile::SecretDoor);
        if !passable(self.tile(to)) {
            return false;
        }
        if from.x != to.x && from.y != to.y {
            let blocks = |p: Pos| self.tile(p).opaque();
            if blocks(Pos::new(from.x, to.y)) || blocks(Pos::new(to.x, from.y)) {
                return false;
            }
        }
        true
    }

    /// For each room: can the centre of its interior be reached from the
    /// start by walking? `secret_open` treats secret doors as open.
    pub fn reachable_rooms(&self, secret_open: bool) -> Vec<bool> {
        let mut seen = vec![false; self.tiles.len()];
        let mut queue = VecDeque::new();
        if let Some(i) = self.index(self.start) {
            seen[i] = true;
            queue.push_back(self.start);
        }
        while let Some(p) = queue.pop_front() {
            for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                let n = p.offset(dx, dy);
                let Some(ni) = self.index(n) else { continue };
                if !seen[ni] && self.can_step(p, n, secret_open) {
                    seen[ni] = true;
                    queue.push_back(n);
                }
            }
        }
        self.rooms
            .iter()
            .map(|r| self.index(r.center()).is_some_and(|i| seen[i]))
            .collect()
    }

    /// Ground-truth ASCII rendering: tiles plus every monster, portal,
    /// item and the start position. Used by `--dump` and the tests.
    pub fn to_ascii(&self) -> String {
        let w = self.width as usize;
        let mut grid: Vec<Vec<char>> = self
            .tiles
            .chunks(w)
            .map(|row| row.iter().map(|t| t.ascii()).collect())
            .collect();
        let mut put = |p: Pos, c: char| {
            if p.x >= 0 && p.y >= 0 && (p.y as usize) < grid.len() && (p.x as usize) < w {
                grid[p.y as usize][p.x as usize] = c;
            }
        };
        for (pos, item) in &self.items {
            put(*pos, item.kind.glyph());
        }
        for portal in &self.portals {
            put(portal.pos, 'O');
        }
        for spawn in &self.spawns {
            put(spawn.pos, spawn.class().glyph());
        }
        put(self.start, '@');

        let mut out = String::new();
        for row in grid {
            let line: String = row.into_iter().collect();
            out.push_str(line.trim_end());
            out.push('\n');
        }
        while out.ends_with("\n\n") {
            out.pop();
        }
        out
    }

    /// One-paragraph legend for `--dump`.
    pub fn legend() -> &'static str {
        "Legend: # wall  . floor  : sealed room  , corridor  + door  X secret door  > stairs  @ start\n\
         Monsters: C crab(.rs)  S snake(.py)  g goblin(.js/.ts)  G ghost(.md/.txt)  H golem(.json/.toml/.yaml)\n\
         \x20         M mimic(images)  B chest-monster(archives)  j slime(other)    O portal(symlink)\n\
         Items: ! potion  ? scroll  / blade  [ mail"
    }
}

// ---------------------------------------------------------------------------
// Generation
// ---------------------------------------------------------------------------

/// A room before it has a position.
struct Spec {
    kind: RoomKind,
    name: String,
    path: PathBuf,
    dir: usize,
    /// Indices into `tree.dirs[dir].files` that live in this room.
    files: Vec<usize>,
    depth: u32,
    hidden: bool,
    secret_entry: bool,
    parent: Option<usize>,
    children: Vec<usize>,
    /// Depth in the room tree (= layout column).
    level: i32,
    row: i32,
}

/// Turn directories into room specs (pre-order; the root is spec 0).
/// Ignored *files* of a visible directory move into a hidden cache room.
fn add_dir_specs(tree: &Tree, dir_id: usize, parent: Option<usize>, specs: &mut Vec<Spec>) {
    let d = &tree.dirs[dir_id];
    let id = specs.len();
    let level = parent.map_or(0, |p| specs[p].level + 1);
    let (visible, hidden_files): (Vec<usize>, Vec<usize>) = if d.ignored {
        ((0..d.files.len()).collect(), Vec::new())
    } else {
        (0..d.files.len()).partition(|&i| !d.files[i].ignored)
    };
    let secret_entry = d.ignored && parent.is_some_and(|p| !specs[p].hidden);
    specs.push(Spec {
        kind: match d.kind {
            DirKind::Normal => RoomKind::Normal,
            DirKind::Vault => RoomKind::Vault,
            DirKind::Sealed => RoomKind::Sealed,
        },
        name: d.name.clone(),
        path: d.path.clone(),
        dir: dir_id,
        files: visible,
        depth: d.depth,
        hidden: d.ignored,
        secret_entry,
        parent,
        children: Vec::new(),
        level,
        row: 0,
    });
    if let Some(p) = parent {
        specs[p].children.push(id);
    }
    for &child in &d.children {
        add_dir_specs(tree, child, Some(id), specs);
    }
    if !hidden_files.is_empty() && d.kind == DirKind::Normal {
        let cache = specs.len();
        specs.push(Spec {
            kind: RoomKind::Cache,
            name: format!("(hidden) {}", d.name),
            path: d.path.clone(),
            dir: dir_id,
            files: hidden_files,
            depth: d.depth,
            hidden: true,
            secret_entry: true,
            parent: Some(id),
            children: Vec::new(),
            level: level + 1,
            row: 0,
        });
        specs[id].children.push(cache);
    }
}

/// Hand out rows: each leaf gets the next row, a parent shares its first
/// child's row (so the first corridor is a short hop, the rest drop down).
fn assign_rows(specs: &mut [Spec], id: usize, next: &mut i32) {
    let children = specs[id].children.clone();
    if children.is_empty() {
        specs[id].row = *next;
        *next += 1;
    } else {
        for &c in &children {
            assign_rows(specs, c, next);
        }
        specs[id].row = specs[children[0]].row;
    }
}

/// Interior size for a room holding `n` occupants, capped to fit its cell.
fn room_dims(kind: RoomKind, n: usize) -> (i32, i32) {
    match kind {
        RoomKind::Sealed => (5, 3),
        RoomKind::Vault => (7, 5),
        _ => {
            let area = (n as i32).saturating_mul(7).clamp(30, 2000);
            let w = (f64::from(area * 2).sqrt() as i32).clamp(7, MAX_ROOM_W);
            let h = ((area + w - 1) / w).clamp(5, MAX_ROOM_H);
            (w, h)
        }
    }
}

fn shuffle<T, R: Rng + ?Sized>(items: &mut [T], rng: &mut R) {
    for i in (1..items.len()).rev() {
        let j = rng.random_range(0..=i);
        items.swap(i, j);
    }
}

/// Carve a corridor tile: rock becomes corridor, a wall becomes a doorway.
fn carve(level: &mut Level, p: Pos) {
    match level.tile(p) {
        Tile::Rock => level.set_tile(p, Tile::Corridor),
        Tile::Wall => level.set_tile(p, Tile::Door),
        _ => {}
    }
}

/// Generate the dungeon for `tree`. Same tree + same seed => identical level.
pub fn build_level(tree: &Tree, seed: u64) -> Level {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);

    let mut specs: Vec<Spec> = Vec::new();
    add_dir_specs(tree, 0, None, &mut specs);
    let mut rows = 0;
    assign_rows(&mut specs, 0, &mut rows);
    let cols = specs.iter().map(|s| s.level).max().unwrap_or(0) + 1;

    // Size every column and row to the biggest room in it.
    let dims: Vec<(i32, i32)> = specs
        .iter()
        .map(|s| room_dims(s.kind, s.files.len()))
        .collect();
    let mut col_w = vec![0; cols as usize];
    let mut row_h = vec![0; rows as usize];
    for (spec, &(w, h)) in specs.iter().zip(&dims) {
        let (c, r) = (spec.level as usize, spec.row as usize);
        col_w[c] = col_w[c].max(w + 2);
        row_h[r] = row_h[r].max(h + 2);
    }
    let origins = |sizes: &[i32], gap: i32| -> Vec<i32> {
        let mut at = 1;
        sizes
            .iter()
            .map(|size| {
                let here = at;
                at += size + gap;
                here
            })
            .collect()
    };
    let col_x = origins(&col_w, GAP_X);
    let row_y = origins(&row_h, GAP_Y);

    let width = col_x[cols as usize - 1] + col_w[cols as usize - 1] + 2;
    let height = row_y[rows as usize - 1] + row_h[rows as usize - 1] + 2;
    let mut level = Level {
        width,
        height,
        tiles: vec![Tile::Rock; (width * height) as usize],
        room_map: vec![NO_ROOM; (width * height) as usize],
        rooms: Vec::new(),
        start: Pos::default(),
        stairs: Pos::default(),
        spawns: Vec::new(),
        items: Vec::new(),
        portals: Vec::new(),
        seed,
    };

    // --- rooms -----------------------------------------------------------
    for (id, spec) in specs.iter().enumerate() {
        let occupants = spec.files.len();
        let (w, h) = dims[id];
        let (c, r) = (spec.level as usize, spec.row as usize);
        let ox = rng.random_range(0..=col_w[c] - (w + 2));
        let oy = rng.random_range(0..=row_h[r] - (h + 2));
        let outer_x = col_x[c] + ox;
        let outer_y = row_y[r] + oy;
        let room = Room {
            id,
            kind: spec.kind,
            name: spec.name.clone(),
            path: spec.path.clone(),
            x: outer_x + 1,
            y: outer_y + 1,
            w,
            h,
            depth: spec.depth,
            hidden: spec.hidden,
            secret_entry: spec.secret_entry,
            entries: occupants,
        };
        let floor = if spec.kind == RoomKind::Sealed {
            Tile::SealedFloor
        } else {
            Tile::Floor
        };
        for y in outer_y..outer_y + h + 2 {
            for x in outer_x..outer_x + w + 2 {
                let p = Pos::new(x, y);
                let edge =
                    x == outer_x || y == outer_y || x == outer_x + w + 1 || y == outer_y + h + 1;
                level.set_tile(p, if edge { Tile::Wall } else { floor });
                if let Some(i) = level.index(p) {
                    level.room_map[i] = id as u32;
                }
            }
        }
        level.rooms.push(room);
    }

    // --- corridors -------------------------------------------------------
    for (id, spec) in specs.iter().enumerate() {
        let Some(parent) = spec.parent else { continue };
        let a = level.rooms[parent].center();
        let b = level.rooms[id].center();
        // Down (or up) the parent's column, then across to the child.
        let (y0, y1) = (a.y.min(b.y), a.y.max(b.y));
        for y in y0..=y1 {
            carve(&mut level, Pos::new(a.x, y));
        }
        let (x0, x1) = (a.x.min(b.x), a.x.max(b.x));
        for x in x0..=x1 {
            carve(&mut level, Pos::new(x, b.y));
        }
        if spec.secret_entry {
            // The doorway in the child's west wall.
            let wall_x = level.rooms[id].x - 1;
            level.set_tile(Pos::new(wall_x, b.y), Tile::SecretDoor);
        }
    }

    // --- start and stairs --------------------------------------------------
    // Start just inside the root room's first doorway, so a corridor to fight
    // in is one step away when the hunters close in. A lone room has no doors.
    level.start = match first_door(&level, 0) {
        Some(door) => {
            let room = &level.rooms[0];
            Pos::new(
                door.x.clamp(room.x, room.x + room.w - 1),
                door.y.clamp(room.y, room.y + room.h - 1),
            )
        }
        None => {
            let root_tiles: Vec<Pos> = level.rooms[0].tiles().collect();
            root_tiles[rng.random_range(0..root_tiles.len())]
        }
    };

    // The exit lives in the deepest room you can reach without secrets.
    let exit_room = level
        .rooms
        .iter()
        .filter(|r| !r.hidden)
        .max_by_key(|r| (r.depth, r.id))
        .map_or(0, |r| r.id);
    let candidates: Vec<Pos> = level.rooms[exit_room]
        .tiles()
        .filter(|&p| p != level.start)
        .collect();
    level.stairs = candidates[rng.random_range(0..candidates.len())];
    let stairs = level.stairs;
    level.set_tile(stairs, Tile::Stairs);

    // --- occupants -----------------------------------------------------------
    for (id, spec) in specs.iter().enumerate() {
        let room = level.rooms[id].clone();
        let mut free: Vec<Pos> = room
            .tiles()
            .filter(|&p| p != level.start && p != level.stairs)
            .filter(|&p| id != 0 || p.dist(level.start) > 3)
            .collect();
        shuffle(&mut free, &mut rng);

        if spec.kind == RoomKind::Vault {
            for item in vault_loot(&mut rng) {
                if let Some(pos) = free.pop() {
                    level.items.push((pos, item));
                }
            }
            continue;
        }
        if matches!(spec.kind, RoomKind::Sealed) {
            continue;
        }

        let dir = &tree.dirs[spec.dir];
        let mut walkers: Vec<usize> = Vec::new();
        let mut portal_names: Vec<&str> = Vec::new();
        for &fi in &spec.files {
            if dir.files[fi].portal {
                portal_names.push(&dir.files[fi].name);
            } else {
                walkers.push(fi);
            }
        }
        // Keep the biggest files if the room cannot hold them all.
        walkers.sort_by(|&a, &b| {
            dir.files[b]
                .metadata
                .size
                .cmp(&dir.files[a].metadata.size)
                .then_with(|| dir.files[a].name.cmp(&dir.files[b].name))
        });
        let cap = ((room.w * room.h) / AREA_PER_MONSTER).max(1) as usize;
        walkers.truncate(cap);
        for fi in walkers {
            let Some(pos) = free.pop() else { break };
            let f = &dir.files[fi];
            level.spawns.push(Spawn {
                pos,
                file: FileInfo {
                    name: f.name.clone(),
                    size: f.metadata.size,
                    modified: f.metadata.modified,
                },
                hidden: spec.hidden,
            });
        }

        // Portals hug the walls, away from doorways, so they can never seal a room.
        let ring: Vec<Pos> = free
            .iter()
            .copied()
            .filter(|p| {
                p.x == room.x
                    || p.y == room.y
                    || p.x == room.x + room.w - 1
                    || p.y == room.y + room.h - 1
            })
            .filter(|&p| !near_doorway(&level, p))
            .collect();
        for (name, pos) in portal_names.iter().zip(ring.iter().take(ring.len() / 3)) {
            level.portals.push(Portal {
                pos: *pos,
                name: (*name).to_string(),
            });
        }
    }

    level
}

/// The first doorway (row-major) in a room's wall ring.
fn first_door(level: &Level, room: usize) -> Option<Pos> {
    let (ox, oy, ow, oh) = level.rooms[room].outer();
    (oy..oy + oh)
        .flat_map(|y| (ox..ox + ow).map(move |x| Pos::new(x, y)))
        .find(|&p| level.tile(p) == Tile::Door)
}

/// Is `p` within one step of a door or secret door?
fn near_doorway(level: &Level, p: Pos) -> bool {
    (-1..=1).any(|dy| {
        (-1..=1).any(|dx| matches!(level.tile(p.offset(dx, dy)), Tile::Door | Tile::SecretDoor))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::Class;
    use crate::scan::{self, DirNode, FileNode, Meta, MAX_DEPTH};
    use crate::testutil::TempDir;

    /// A random but valid tree, built without touching the filesystem.
    fn synthetic_tree(seed: u64, n_dirs: usize) -> Tree {
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let mut dirs = vec![DirNode {
            parent: None,
            name: "root".into(),
            path: PathBuf::from("/synthetic/root"),
            depth: 0,
            kind: DirKind::Normal,
            ignored: false,
            files: Vec::new(),
            children: Vec::new(),
        }];
        for id in 1..n_dirs {
            let candidates: Vec<usize> = dirs
                .iter()
                .enumerate()
                .filter(|(_, d)| d.kind == DirKind::Normal && d.depth < MAX_DEPTH)
                .map(|(i, _)| i)
                .collect();
            let parent = candidates[rng.random_range(0..candidates.len())];
            let kind = match rng.random_range(0..100) {
                0..=3 => DirKind::Vault,
                4..=7 => DirKind::Sealed,
                _ => DirKind::Normal,
            };
            let ignored = dirs[parent].ignored || rng.random_bool(0.15);
            let depth = dirs[parent].depth + 1;
            dirs.push(DirNode {
                parent: Some(parent),
                name: format!("d{id}"),
                path: PathBuf::from(format!("/synthetic/root/d{id}")),
                depth,
                kind,
                ignored,
                files: Vec::new(),
                children: Vec::new(),
            });
            dirs[parent].children.push(id);
        }
        const EXTS: [&str; 9] = ["rs", "py", "js", "md", "json", "png", "zip", "bin", ""];
        for d in dirs.iter_mut() {
            if d.kind != DirKind::Normal {
                continue;
            }
            let n = if rng.random_bool(0.1) {
                rng.random_range(40..260)
            } else {
                rng.random_range(0..25)
            };
            for i in 0..n {
                let ext = EXTS[rng.random_range(0..EXTS.len())];
                let name = if ext.is_empty() {
                    format!("file{i}")
                } else {
                    format!("file{i}.{ext}")
                };
                let size = 1u64 << rng.random_range(0..26);
                d.files.push(FileNode {
                    name,
                    metadata: Meta {
                        size,
                        modified: None,
                    },
                    ignored: d.ignored || rng.random_bool(0.15),
                    portal: rng.random_bool(0.05),
                });
            }
        }
        Tree {
            dirs,
            truncated: false,
            skipped: 0,
        }
    }

    #[test]
    fn fnv1a_matches_reference_vectors() {
        assert_eq!(fnv1a64(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a64(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv1a64(b"foobar"), 0x8594_4171_f739_67e8);
    }

    #[test]
    fn seed_depends_on_the_path_only() {
        let a = seed_for_path(Path::new("/home/me/project"));
        assert_eq!(a, seed_for_path(Path::new("/home/me/project")));
        assert_ne!(a, seed_for_path(Path::new("/home/me/project2")));
    }

    #[test]
    fn same_tree_and_seed_give_identical_levels() {
        for s in 0..10 {
            let tree = synthetic_tree(s, 40);
            let a = build_level(&tree, 99);
            let b = build_level(&tree, 99);
            assert_eq!(a.to_ascii(), b.to_ascii());
            assert_eq!(a.start, b.start);
            assert_eq!(a.stairs, b.stairs);
            assert_eq!(a.spawns.len(), b.spawns.len());
        }
    }

    #[test]
    fn different_seeds_give_different_levels() {
        let tree = synthetic_tree(5, 40);
        assert_ne!(
            build_level(&tree, 1).to_ascii(),
            build_level(&tree, 2).to_ascii()
        );
    }

    #[test]
    fn real_directory_generation_is_deterministic() {
        let tmp = TempDir::new();
        tmp.file(".gitignore", b"private/\n.env\n")
            .file("src/main.rs", b"fn main(){}")
            .file("src/lib.rs", b"")
            .file("docs/README.md", b"hi")
            .file("private/inner/app", b"bin")
            .file(".env", b"X=1");
        let seed = seed_for_path(tmp.path());
        let a = build_level(&scan::scan(tmp.path()), seed).to_ascii();
        let b = build_level(&scan::scan(tmp.path()), seed).to_ascii();
        assert_eq!(a, b);
        let c = build_level(&scan::scan(tmp.path()), seed + 1).to_ascii();
        assert_ne!(a, c);
    }

    #[test]
    fn bfs_from_start_reaches_every_room() {
        for s in 0..200u64 {
            let n = 1 + (s as usize * 7) % 120;
            let tree = synthetic_tree(s, n);
            let level = build_level(&tree, s ^ 0xabcd);
            let reached = level.reachable_rooms(true);
            assert_eq!(reached.len(), level.rooms.len());
            for (room, ok) in level.rooms.iter().zip(&reached) {
                assert!(
                    *ok,
                    "seed {s}: room {} ({}) unreachable",
                    room.id, room.name
                );
            }
        }
    }

    #[test]
    fn single_empty_directory_is_a_valid_level() {
        let tree = synthetic_tree(0, 1);
        let mut tree = tree;
        tree.dirs[0].files.clear();
        let level = build_level(&tree, 1);
        assert_eq!(level.rooms.len(), 1);
        assert!(level.reachable_rooms(true)[0]);
        assert_eq!(level.tile(level.stairs), Tile::Stairs);
        assert_ne!(level.start, level.stairs);
    }

    #[test]
    fn secret_rooms_are_only_reachable_through_secret_doors() {
        for s in 0..100u64 {
            let tree = synthetic_tree(s, 2 + (s as usize % 60));
            let level = build_level(&tree, s);
            let reached = level.reachable_rooms(false);
            for (room, ok) in level.rooms.iter().zip(&reached) {
                assert_eq!(
                    *ok, !room.hidden,
                    "seed {s}: room {} hidden={}",
                    room.name, room.hidden
                );
            }
            let secret_doors = level
                .tiles
                .iter()
                .filter(|&&t| t == Tile::SecretDoor)
                .count();
            let secret_rooms = level.rooms.iter().filter(|r| r.secret_entry).count();
            assert_eq!(secret_doors, secret_rooms, "seed {s}");
        }
    }

    #[test]
    fn rooms_never_overlap_and_fit_the_map() {
        for s in 0..60u64 {
            let tree = synthetic_tree(s, 2 + (s as usize * 3) % 150);
            let level = build_level(&tree, s);
            let outers: Vec<_> = level.rooms.iter().map(|r| r.outer()).collect();
            for (i, a) in outers.iter().enumerate() {
                assert!(
                    a.0 >= 0 && a.1 >= 0 && a.0 + a.2 <= level.width && a.1 + a.3 <= level.height
                );
                for b in outers.iter().skip(i + 1) {
                    let apart = a.0 + a.2 <= b.0
                        || b.0 + b.2 <= a.0
                        || a.1 + a.3 <= b.1
                        || b.1 + b.3 <= a.1;
                    assert!(apart, "seed {s}: rooms overlap {a:?} {b:?}");
                }
            }
        }
    }

    #[test]
    fn start_is_in_root_and_exit_is_in_the_deepest_visible_room() {
        for s in 0..80u64 {
            let tree = synthetic_tree(s, 2 + (s as usize % 50));
            let level = build_level(&tree, s);
            assert_eq!(level.room_at(level.start), Some(0));
            assert!(level.rooms[0].contains(level.start));
            let exit = level.room_at(level.stairs).expect("stairs in a room");
            assert!(!level.rooms[exit].hidden);
            let deepest = level
                .rooms
                .iter()
                .filter(|r| !r.hidden)
                .map(|r| r.depth)
                .max();
            assert_eq!(Some(level.rooms[exit].depth), deepest);
            assert_eq!(level.tile(level.stairs), Tile::Stairs);
        }
    }

    #[test]
    fn monsters_stand_on_distinct_floor_tiles_in_their_rooms() {
        for s in 0..60u64 {
            let tree = synthetic_tree(s, 2 + (s as usize % 50));
            let level = build_level(&tree, s);
            let mut seen = std::collections::HashSet::new();
            for sp in &level.spawns {
                let room = level.room_at(sp.pos).expect("monster inside a room");
                assert!(level.rooms[room].contains(sp.pos));
                assert!(matches!(level.tile(sp.pos), Tile::Floor));
                assert!(seen.insert(sp.pos), "two monsters share {:?}", sp.pos);
                assert_ne!(sp.pos, level.start);
                assert_ne!(sp.pos, level.stairs);
                assert!(room != 0 || sp.pos.dist(level.start) > 3);
                assert_eq!(sp.hidden, level.rooms[room].hidden);
            }
        }
    }

    #[test]
    fn room_size_scales_with_file_count_and_is_capped() {
        let sizes: Vec<(i32, i32)> = [0, 5, 20, 60, 150, 5000]
            .iter()
            .map(|&n| room_dims(RoomKind::Normal, n))
            .collect();
        for pair in sizes.windows(2) {
            assert!(pair[1].0 * pair[1].1 >= pair[0].0 * pair[0].1);
        }
        assert!(sizes[1].0 * sizes[1].1 < sizes[4].0 * sizes[4].1);
        for (w, h) in sizes {
            assert!((7..=MAX_ROOM_W).contains(&w) && (5..=MAX_ROOM_H).contains(&h));
        }
    }

    #[test]
    fn git_is_a_vault_ignored_entries_are_secret() {
        let tmp = TempDir::new();
        tmp.file(".gitignore", b"private/\n.env\n*.log\n")
            .file(".git/HEAD", b"ref")
            .file("src/main.rs", b"fn main(){}")
            .file("private/inner/app", b"bin")
            .file(".env", b"X=1")
            .file("run.log", b"x");
        let level = build_level(&scan::scan(tmp.path()), 1);

        let vault = level
            .rooms
            .iter()
            .find(|r| r.kind == RoomKind::Vault)
            .expect("vault");
        assert_eq!(vault.name, ".git");
        assert!(!vault.hidden && !level.items.is_empty());

        let private = level
            .rooms
            .iter()
            .find(|r| r.name == "private")
            .expect("private room");
        assert!(private.hidden && private.secret_entry);
        let inner = level
            .rooms
            .iter()
            .find(|r| r.name == "inner")
            .expect("inner room");
        assert!(
            inner.hidden && !inner.secret_entry,
            "only the outermost door is secret"
        );

        let cache = level
            .rooms
            .iter()
            .find(|r| r.kind == RoomKind::Cache)
            .expect("cache");
        assert!(cache.secret_entry);
        let names: Vec<&str> = level
            .spawns
            .iter()
            .filter(|s| level.room_at(s.pos) == Some(cache.id))
            .map(|s| s.file.name.as_str())
            .collect();
        assert!(names.contains(&".env") && names.contains(&"run.log"));
        assert!(level
            .spawns
            .iter()
            .filter(|s| level.room_at(s.pos) == Some(cache.id))
            .all(|s| s.hidden));

        // The visible monsters never include the ignored files.
        let visible: Vec<&str> = level
            .spawns
            .iter()
            .filter(|s| !s.hidden)
            .map(|s| s.file.name.as_str())
            .collect();
        assert!(visible.contains(&"main.rs") && !visible.contains(&".env"));
        assert_eq!(Class::for_file_name(".env"), Class::Slime);
    }

    #[test]
    fn portals_hug_walls_and_never_block_doorways() {
        for s in 0..60u64 {
            let tree = synthetic_tree(s, 2 + (s as usize % 40));
            let level = build_level(&tree, s);
            for portal in &level.portals {
                assert_eq!(level.tile(portal.pos), Tile::Floor);
                assert!(!near_doorway(&level, portal.pos));
                let room = level.room_at(portal.pos).expect("portal inside a room");
                assert!(level.rooms[room].contains(portal.pos));
            }
            assert!(level.reachable_rooms(true).iter().all(|&r| r));
        }
    }

    #[test]
    fn crowded_rooms_keep_only_the_biggest_files() {
        let mut tree = synthetic_tree(1, 1);
        tree.dirs[0].files.clear();
        for i in 0..400u64 {
            tree.dirs[0].files.push(FileNode {
                name: format!("f{i}.txt"),
                metadata: Meta {
                    size: i * 10,
                    modified: None,
                },
                ignored: false,
                portal: false,
            });
        }
        let level = build_level(&tree, 3);
        let room = &level.rooms[0];
        let cap = ((room.w * room.h) / AREA_PER_MONSTER) as usize;
        assert_eq!(level.spawns.len(), cap);
        let smallest_kept = level.spawns.iter().map(|s| s.file.size).min().unwrap_or(0);
        assert!(smallest_kept >= (400 - cap as u64) * 10);
    }

    #[test]
    fn ascii_dump_shows_ground_truth() {
        let tmp = TempDir::new();
        tmp.file(".gitignore", b"secret/\n")
            .file("a.rs", b"")
            .file("secret/x.py", b"");
        let ascii = build_level(&scan::scan(tmp.path()), 4).to_ascii();
        assert!(ascii.contains('@') && ascii.contains('>') && ascii.contains('X'));
        assert!(ascii.contains('C'), "a.rs is a crab");
        assert!(ascii.lines().all(|l| l == l.trim_end()));
    }
}
