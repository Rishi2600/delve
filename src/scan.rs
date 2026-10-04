//! Read-only directory scanner.
//!
//! Walks a directory tree breadth-first (so the entry cap trims the deepest
//! entries first) and produces a [`Tree`]. Entries matched by gitignore rules
//! are *classified* (`ignored = true`), never skipped: the map generator turns
//! them into secret rooms.
//!
//! This module only ever calls `read_dir`, `symlink_metadata` and reads
//! `.gitignore` files. It never writes, and never follows symlinks.

use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use ignore::gitignore::{Gitignore, GitignoreBuilder};

/// Maximum number of entries (files, dirs, portals) recorded in a [`Tree`].
pub const MAX_ENTRIES: usize = 2000;
/// Maximum depth below the root at which entries are recorded.
pub const MAX_DEPTH: u32 = 6;
/// Entries recorded per ignored directory, so `node_modules/` cannot eat the
/// whole budget and starve the rest of the dungeon.
pub const IGNORED_DIR_ENTRY_CAP: usize = 48;

/// What kind of room a directory becomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirKind {
    /// An ordinary, readable directory.
    Normal,
    /// A `.git` directory: shown as a single vault room, never descended into.
    Vault,
    /// A directory that could not be read: a sealed room.
    Sealed,
}

/// The metadata of a file that the game cares about.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Meta {
    pub size: u64,
    pub modified: Option<SystemTime>,
}

/// A file (or symlink "portal") inside a directory.
#[derive(Debug, Clone)]
pub struct FileNode {
    pub name: String,
    pub metadata: Meta,
    /// Matched by gitignore rules (or lives inside an ignored directory).
    pub ignored: bool,
    /// A symlink. Never followed; only its name is ever shown.
    pub portal: bool,
}

/// A directory in the scanned tree.
#[derive(Debug, Clone)]
pub struct DirNode {
    pub parent: Option<usize>,
    pub name: String,
    pub path: PathBuf,
    /// Depth below the root (the root is 0).
    pub depth: u32,
    pub kind: DirKind,
    /// Matched by gitignore rules (or lives inside an ignored directory).
    pub ignored: bool,
    pub files: Vec<FileNode>,
    /// Child directory ids, sorted by name.
    pub children: Vec<usize>,
}

/// The scanned directory tree. `dirs[0]` is always the root.
#[derive(Debug, Clone)]
pub struct Tree {
    pub dirs: Vec<DirNode>,
    /// True if a cap (entries or per-ignored-dir) cut the scan short.
    pub truncated: bool,
}

impl Tree {
    /// Total number of recorded entries (everything except the root itself).
    pub fn entry_count(&self) -> usize {
        self.dirs.len() - 1 + self.dirs.iter().map(|d| d.files.len()).sum::<usize>()
    }
}

/// Scan `root`. Never fails: an unreadable root becomes a sealed root room.
pub fn scan(root: &Path) -> Tree {
    let mut tree = Tree {
        dirs: Vec::new(),
        truncated: false,
    };

    let root_name = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "/".to_string());
    tree.dirs.push(DirNode {
        parent: None,
        name: root_name,
        path: root.to_path_buf(),
        depth: 0,
        kind: DirKind::Normal,
        ignored: false,
        files: Vec::new(),
        children: Vec::new(),
    });

    // Per-directory matcher (None if the dir has no readable .gitignore),
    // parallel to `tree.dirs`. `base` holds matchers of ancestors above the
    // root plus `.git/info/exclude`, deepest first.
    let base = base_matchers(root);
    let mut matchers: Vec<Option<Gitignore>> = vec![load_gitignore(root)];

    let mut count = 0usize;
    let mut queue: VecDeque<usize> = VecDeque::from([0]);

    'walk: while let Some(id) = queue.pop_front() {
        if tree.dirs[id].kind == DirKind::Vault {
            continue;
        }
        let dir_path = tree.dirs[id].path.clone();
        let dir_ignored = tree.dirs[id].ignored;
        let depth = tree.dirs[id].depth;

        let mut names: Vec<std::ffi::OsString> = match fs::read_dir(&dir_path) {
            Ok(rd) => rd.filter_map(|e| e.ok()).map(|e| e.file_name()).collect(),
            Err(_) => {
                tree.dirs[id].kind = DirKind::Sealed;
                continue;
            }
        };
        names.sort_by(|a, b| a.as_encoded_bytes().cmp(b.as_encoded_bytes()));

        let mut recorded_here = 0usize;
        for name in names {
            if count >= MAX_ENTRIES {
                tree.truncated = true;
                break 'walk;
            }
            if dir_ignored && recorded_here >= IGNORED_DIR_ENTRY_CAP {
                tree.truncated = true;
                break;
            }

            let path = dir_path.join(&name);
            let Ok(md) = fs::symlink_metadata(&path) else {
                continue;
            };
            let ft = md.file_type();
            let display = name.to_string_lossy().into_owned();
            let meta = Meta {
                size: md.len(),
                modified: md.modified().ok(),
            };

            if ft.is_dir() && !ft.is_symlink() {
                let is_vault = display == ".git";
                let ignored = dir_ignored
                    || (!is_vault && is_ignored(&matchers, &base, &tree, id, &path, true));
                let new_id = tree.dirs.len();
                tree.dirs.push(DirNode {
                    parent: Some(id),
                    name: display,
                    path: path.clone(),
                    depth: depth + 1,
                    kind: if is_vault {
                        DirKind::Vault
                    } else {
                        DirKind::Normal
                    },
                    ignored,
                    files: Vec::new(),
                    children: Vec::new(),
                });
                matchers.push(if is_vault || depth + 1 >= MAX_DEPTH {
                    None
                } else {
                    load_gitignore(&path)
                });
                tree.dirs[id].children.push(new_id);
                if depth + 1 < MAX_DEPTH {
                    queue.push_back(new_id);
                }
            } else {
                let portal = ft.is_symlink();
                let ignored = dir_ignored || is_ignored(&matchers, &base, &tree, id, &path, false);
                tree.dirs[id].files.push(FileNode {
                    name: display,
                    metadata: if portal { Meta::default() } else { meta },
                    ignored,
                    portal,
                });
            }
            count += 1;
            recorded_here += 1;
        }
    }

    tree
}

/// Classify `path` (an entry of directory `dir_id`) against the gitignore
/// chain: the directory's own matcher first, then each ancestor's, then the
/// bases. The first matcher with an opinion wins, as in git.
fn is_ignored(
    matchers: &[Option<Gitignore>],
    base: &[Gitignore],
    tree: &Tree,
    dir_id: usize,
    path: &Path,
    is_dir: bool,
) -> bool {
    let mut cur = Some(dir_id);
    while let Some(id) = cur {
        if let Some(gi) = &matchers[id] {
            let m = gi.matched(path, is_dir);
            if m.is_ignore() {
                return true;
            }
            if m.is_whitelist() {
                return false;
            }
        }
        cur = tree.dirs[id].parent;
    }
    for gi in base {
        let m = gi.matched(path, is_dir);
        if m.is_ignore() {
            return true;
        }
        if m.is_whitelist() {
            return false;
        }
    }
    false
}

/// Load `<dir>/.gitignore` if it exists and is readable. Parse errors in
/// individual lines are tolerated (those lines are skipped).
fn load_gitignore(dir: &Path) -> Option<Gitignore> {
    let file = dir.join(".gitignore");
    if !file.is_file() {
        return None;
    }
    let mut builder = GitignoreBuilder::new(dir);
    let _ = builder.add(&file);
    builder.build().ok()
}

/// Matchers that apply to the whole tree but live outside it: `.gitignore`
/// files between the root and the enclosing repository root (deepest first),
/// then that repository's `.git/info/exclude`. Empty if the root is not
/// inside a git repository.
fn base_matchers(root: &Path) -> Vec<Gitignore> {
    let Some(repo_root) = root
        .ancestors()
        .find(|a| fs::symlink_metadata(a.join(".git")).is_ok())
    else {
        return Vec::new();
    };

    let mut out = Vec::new();
    if repo_root != root {
        for dir in root.ancestors().skip(1) {
            if let Some(gi) = load_gitignore(dir) {
                out.push(gi);
            }
            if dir == repo_root {
                break;
            }
        }
    }
    let exclude = repo_root.join(".git").join("info").join("exclude");
    if exclude.is_file() {
        let mut builder = GitignoreBuilder::new(repo_root);
        let _ = builder.add(&exclude);
        if let Ok(gi) = builder.build() {
            out.push(gi);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;

    fn find_dir<'a>(t: &'a Tree, name: &str) -> &'a DirNode {
        t.dirs
            .iter()
            .find(|d| d.name == name)
            .unwrap_or_else(|| panic!("no dir {name}"))
    }

    fn find_file<'a>(d: &'a DirNode, name: &str) -> &'a FileNode {
        d.files
            .iter()
            .find(|f| f.name == name)
            .unwrap_or_else(|| panic!("no file {name}"))
    }

    #[test]
    fn classifies_gitignored_entries_without_skipping_them() {
        let tmp = TempDir::new();
        tmp.file(".gitignore", b"target/\n*.log\n!keep.log\n.env\n")
            .file("main.rs", b"fn main() {}")
            .file("debug.log", b"x")
            .file("keep.log", b"x")
            .file(".env", b"SECRET=1")
            .file("target/debug/app", b"bin")
            .file("src/lib.rs", b"");
        let t = scan(tmp.path());

        let root = &t.dirs[0];
        assert!(!find_file(root, "main.rs").ignored);
        assert!(find_file(root, "debug.log").ignored);
        assert!(!find_file(root, "keep.log").ignored, "negation re-includes");
        assert!(find_file(root, ".env").ignored);
        assert!(!find_file(root, ".gitignore").ignored);

        // Ignored entries are kept, and everything inside an ignored dir is ignored.
        let target = find_dir(&t, "target");
        assert!(target.ignored);
        assert!(find_dir(&t, "debug").ignored);
        assert!(find_file(find_dir(&t, "debug"), "app").ignored);
        assert!(!find_dir(&t, "src").ignored);
        assert!(!find_file(find_dir(&t, "src"), "lib.rs").ignored);
    }

    #[test]
    fn nested_gitignore_applies_below_its_directory_only() {
        let tmp = TempDir::new();
        tmp.file("sub/.gitignore", b"secret.txt\n")
            .file("sub/secret.txt", b"x")
            .file("sub/public.txt", b"x")
            .file("secret.txt", b"x");
        let t = scan(tmp.path());
        assert!(find_file(find_dir(&t, "sub"), "secret.txt").ignored);
        assert!(!find_file(find_dir(&t, "sub"), "public.txt").ignored);
        assert!(!find_file(&t.dirs[0], "secret.txt").ignored);
    }

    #[test]
    fn git_dir_is_a_vault_and_is_not_descended() {
        let tmp = TempDir::new();
        tmp.file(".git/HEAD", b"ref: refs/heads/main")
            .file(".git/objects/ab/cdef", b"obj")
            .file("a.txt", b"");
        let t = scan(tmp.path());
        let git = find_dir(&t, ".git");
        assert_eq!(git.kind, DirKind::Vault);
        assert!(git.files.is_empty() && git.children.is_empty());
        assert!(t.dirs.iter().all(|d| d.name != "objects"));
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_are_portals_and_never_followed() {
        use std::os::unix::fs::symlink;
        let tmp = TempDir::new();
        tmp.file("real/inner.txt", b"x").file("file.txt", b"x");
        symlink(tmp.path().join("real"), tmp.path().join("link_dir")).expect("symlink");
        symlink(tmp.path().join("file.txt"), tmp.path().join("link_file")).expect("symlink");
        let t = scan(tmp.path());

        let root = &t.dirs[0];
        assert!(find_file(root, "link_dir").portal);
        assert!(find_file(root, "link_file").portal);
        assert!(!find_file(root, "file.txt").portal);
        // Only one directory named "real" was walked: the link was not followed.
        assert_eq!(t.dirs.iter().filter(|d| d.name == "inner.txt").count(), 0);
        assert_eq!(
            t.dirs.iter().filter(|d| d.name == "link_dir").count(),
            0,
            "a symlinked dir must not become a room"
        );
    }

    #[test]
    fn entry_cap_is_enforced() {
        let tmp = TempDir::new();
        for i in 0..(MAX_ENTRIES + 150) {
            tmp.file(&format!("f{i:05}.txt"), b"");
        }
        let t = scan(tmp.path());
        assert_eq!(t.entry_count(), MAX_ENTRIES);
        assert!(t.truncated);
    }

    #[test]
    fn depth_cap_is_enforced() {
        let tmp = TempDir::new();
        tmp.file("a/b/c/d/e/f/g/h/deep.txt", b"x");
        let t = scan(tmp.path());
        let max_depth = t.dirs.iter().map(|d| d.depth).max().unwrap_or(0);
        assert_eq!(max_depth, MAX_DEPTH);
        assert!(t.dirs.iter().all(|d| d.name != "g"));
    }

    #[test]
    fn ignored_dirs_are_capped_so_they_cannot_starve_the_tree() {
        let tmp = TempDir::new();
        tmp.file(".gitignore", b"node_modules/\n");
        for i in 0..200 {
            tmp.file(&format!("node_modules/p{i:03}.js"), b"");
        }
        let t = scan(tmp.path());
        assert_eq!(
            find_dir(&t, "node_modules").files.len(),
            IGNORED_DIR_ENTRY_CAP
        );
        assert!(t.truncated);
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_directory_becomes_a_sealed_room() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = TempDir::new();
        tmp.file("locked/x.txt", b"x");
        let locked = tmp.path().join("locked");
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).expect("chmod");
        let readable = fs::read_dir(&locked).is_ok(); // true when running as root
        let t = scan(tmp.path());
        // Restore so TempDir can clean up.
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).expect("chmod");
        if !readable {
            assert_eq!(find_dir(&t, "locked").kind, DirKind::Sealed);
        }
    }

    #[test]
    fn scan_is_deterministic_and_sorted() {
        let tmp = TempDir::new();
        for n in ["zeta", "alpha", "mid", "Beta"] {
            tmp.file(&format!("{n}/x.txt"), b"");
            tmp.file(&format!("{n}.rs"), b"");
        }
        let names = |t: &Tree| -> Vec<String> { t.dirs.iter().map(|d| d.name.clone()).collect() };
        let a = scan(tmp.path());
        let b = scan(tmp.path());
        assert_eq!(names(&a), names(&b));
        let root_files: Vec<_> = a.dirs[0].files.iter().map(|f| f.name.as_str()).collect();
        let mut sorted = root_files.clone();
        sorted.sort();
        assert_eq!(root_files, sorted);
    }

    #[test]
    fn scanning_never_writes() {
        let tmp = crate::testutil::sample_project();
        let before = crate::testutil::snapshot(tmp.path());
        assert!(before.len() > 20, "the sample project has real content");
        let tree = scan(tmp.path());
        assert!(tree.entry_count() > 20, "and it was really scanned");
        let after = crate::testutil::snapshot(tmp.path());
        assert_eq!(before, after);
    }
}
