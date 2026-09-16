//! Shared by the library, the binary and the integration tests through `#[path]` includes,
//! because those targets cannot reach the library's `cfg(test)` modules.

use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

pub(crate) struct TemporaryDirectory {
    pub(crate) path: PathBuf,
}

impl TemporaryDirectory {
    pub(crate) fn new() -> Result<Self> {
        let sequence = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos());
        let path = std::env::temp_dir().join(format!(
            "herdr-workspacer-{}-{timestamp}-{sequence}",
            std::process::id()
        ));
        std::fs::create_dir(&path)
            .with_context(|| format!("could not create temporary directory {}", path.display()))?;
        Ok(Self { path })
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}
