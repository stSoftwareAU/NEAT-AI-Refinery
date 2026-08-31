//! Shared helpers for the integration tests.
//!
//! Only a throwaway directory guard lives here — the tests themselves exercise
//! the public library API rather than any private helper.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::{env, fs};

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// A uniquely named temporary directory removed when the guard is dropped.
pub struct TempDir {
    path: PathBuf,
}

impl TempDir {
    /// Creates a fresh temporary directory tagged with `label`.
    pub fn new(label: &str) -> Self {
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = env::temp_dir().join(format!(
            "neat-ai-refinery-{label}-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create temporary directory");
        Self { path }
    }

    /// The directory itself.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Writes `bytes` to `name` inside the directory and returns the path.
    pub fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.path.join(name);
        fs::write(&path, bytes).expect("write temporary file");
        path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// Encodes `values` the way the corpus stores them: native-endian `f32`.
pub fn encode(values: &[f32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_ne_bytes()).collect()
}
