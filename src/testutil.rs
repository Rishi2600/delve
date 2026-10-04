//! Test-only helpers (a tiny temp-dir type, so no extra dependency is needed).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::entities::Pos;
use crate::mapgen::{Level, Room, RoomKind, Tile};

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// A uniquely named directory under the system temp dir, removed on drop.
pub struct TempDir {
    path: PathBuf,
}

impl TempDir {
    pub fn new() -> TempDir {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let path =
            std::env::temp_dir().join(format!("delve-test-{}-{}-{}", std::process::id(), nanos, n));
        fs::create_dir_all(&path).expect("create temp dir");
        // Canonicalise so paths compare equal to what the scanner sees.
        let path = path.canonicalize().expect("canonicalize temp dir");
        TempDir { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Write a file (creating parent directories) with `bytes` of content.
    pub fn file(&self, rel: &str, content: &[u8]) -> &TempDir {
        let p = self.path.join(rel);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).expect("create parent");
        }
        fs::write(&p, content).expect("write file");
        self
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// A walled, empty, rectangular room: interior (1,1)..(w-2,h-2).
pub fn arena(w: i32, h: i32) -> Level {
    let mut tiles = vec![Tile::Floor; (w * h) as usize];
    for y in 0..h {
        for x in 0..w {
            if x == 0 || y == 0 || x == w - 1 || y == h - 1 {
                tiles[(y * w + x) as usize] = Tile::Wall;
            }
        }
    }
    let stairs = Pos::new(w - 2, h - 2);
    tiles[(stairs.y * w + stairs.x) as usize] = Tile::Stairs;
    Level {
        width: w,
        height: h,
        tiles,
        room_map: vec![0; (w * h) as usize],
        rooms: vec![Room {
            id: 0,
            kind: RoomKind::Normal,
            name: "arena".into(),
            path: PathBuf::from("/arena"),
            x: 1,
            y: 1,
            w: w - 2,
            h: h - 2,
            depth: 0,
            hidden: false,
            secret_entry: false,
            entries: 0,
        }],
        start: Pos::new(2, 2),
        stairs,
        spawns: vec![],
        items: vec![],
        portals: vec![],
        seed: 1,
    }
}
