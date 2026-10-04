//! Test-only helpers (a tiny temp-dir type, so no extra dependency is needed).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

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
        let path = std::env::temp_dir().join(format!(
            "delve-test-{}-{}-{}",
            std::process::id(),
            nanos,
            n
        ));
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

    /// Write a file of exactly `size` bytes.
    pub fn sized(&self, rel: &str, size: usize) -> &TempDir {
        self.file(rel, &vec![b'x'; size])
    }

    pub fn dir(&self, rel: &str) -> &TempDir {
        fs::create_dir_all(self.path.join(rel)).expect("create dir");
        self
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
