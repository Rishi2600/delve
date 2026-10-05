//! Test-only helpers (a tiny temp-dir type, so no extra dependency is needed).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::entities::Pos;
use crate::mapgen::{fnv1a64, Level, Room, RoomKind, Tile};

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

/// A complete, sorted description of everything under `root` (the root
/// included): kind, size, permissions, mtime, content hash, symlink target.
/// Two snapshots are equal only if nothing was created, removed, renamed,
/// rewritten, `touch`ed or re-permissioned in between.
pub fn snapshot(root: &Path) -> Vec<String> {
    fn describe(root: &Path, path: &Path, out: &mut Vec<String>) {
        use std::os::unix::fs::PermissionsExt;
        let md = fs::symlink_metadata(path).expect("stat");
        let rel = path
            .strip_prefix(root)
            .unwrap_or(path)
            .display()
            .to_string();
        let mtime = md
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_nanos());
        let ft = md.file_type();
        let detail = if ft.is_symlink() {
            format!("link->{}", fs::read_link(path).expect("readlink").display())
        } else if ft.is_dir() {
            "dir".to_string()
        } else {
            format!("file:{:016x}", fnv1a64(&fs::read(path).expect("read")))
        };
        out.push(format!(
            "{rel}|{detail}|size={}|mode={:o}|mtime={mtime}",
            md.len(),
            md.permissions().mode()
        ));
        if ft.is_dir() && !ft.is_symlink() {
            let mut children: Vec<_> = fs::read_dir(path)
                .expect("read_dir")
                .map(|e| e.expect("entry").path())
                .collect();
            children.sort();
            for child in children {
                describe(root, &child, out);
            }
        }
    }
    let mut out = Vec::new();
    describe(root, root, &mut out);
    out
}

/// A directory shaped like a small real project: nested dirs, a gitignore,
/// a `.git`, ignored dirs/files, a symlink and a few differently sized files.
pub fn sample_project() -> TempDir {
    let tmp = TempDir::new();
    tmp.file(".gitignore", b"private/\n*.log\n.env\n")
        .file(".git/HEAD", b"ref: refs/heads/main\n")
        .file(".git/objects/aa/bb", b"object")
        .file("src/main.rs", &vec![b'x'; 5000])
        .file("src/lib.rs", b"pub fn f() {}")
        .file("src/util/mod.py", b"print(1)")
        .file("docs/README.md", b"# hi")
        .file("docs/notes.txt", &vec![b'n'; 3000])
        .file("assets/logo.png", &vec![0u8; 20_000])
        .file("assets/bundle.tar.gz", &vec![1u8; 300_000])
        .file("config/app.toml", b"a = 1")
        .file("config/app.json", b"{}")
        .file("private/inner/app", &vec![7u8; 2_000_000])
        .file(".env", b"SECRET=1")
        .file("run.log", b"log")
        .file("Makefile", b"all:");
    std::os::unix::fs::symlink(tmp.path().join("docs"), tmp.path().join("docs_link"))
        .expect("symlink");
    tmp
}
