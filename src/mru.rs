use std::{
    fs::{self, OpenOptions},
    path::{Path, PathBuf},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

const MRU_LIMIT: usize = 200;
const STATE_FILE: &str = "mru.json";
const LOCK_FILE: &str = "mru.lock";

/// Ordered paths with the most recently focused path first.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MruState {
    /// Canonical workspace paths in descending recency.
    pub paths: Vec<PathBuf>,
}

/// Durable MRU state stored in Herdr's plugin state directory.
#[derive(Debug)]
pub struct MruStore {
    state_dir: PathBuf,
}

#[derive(Debug)]
struct StateLock {
    path: PathBuf,
}

impl Drop for StateLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct MruFile {
    version: u8,
    paths: Vec<PathBuf>,
}

impl From<MruFile> for MruState {
    fn from(MruFile { version: _, paths }: MruFile) -> Self {
        Self { paths }
    }
}

impl From<&MruState> for MruFile {
    fn from(state: &MruState) -> Self {
        Self {
            version: 1,
            paths: state.paths.clone(),
        }
    }
}

impl MruStore {
    /// Opens the MRU store designated by Herdr's runtime environment.
    pub fn from_environment() -> Result<Self> {
        let state_dir = std::env::var_os("HERDR_PLUGIN_STATE_DIR")
            .map(PathBuf::from)
            .context("Herdr did not provide HERDR_PLUGIN_STATE_DIR")?;

        Ok(Self { state_dir })
    }

    #[cfg(test)]
    fn new(state_dir: PathBuf) -> Self {
        Self { state_dir }
    }

    /// Loads MRU state, treating a missing or malformed file as empty.
    pub fn load(&self) -> Result<MruState> {
        let _lock = self.lock()?;
        self.load_unlocked()
    }

    /// Moves a path to the front of MRU state and writes it atomically.
    pub fn record(&self, path: PathBuf) -> Result<()> {
        let _lock = self.lock()?;
        let mut state = self.load_unlocked()?;
        state.paths.retain(|known_path| known_path != &path);
        state.paths.insert(0, path);
        state.paths.truncate(MRU_LIMIT);
        self.write_unlocked(&state)
    }

    fn lock(&self) -> Result<StateLock> {
        fs::create_dir_all(&self.state_dir).with_context(|| {
            format!(
                "could not create Herdr plugin state directory {}",
                self.state_dir.display()
            )
        })?;

        let lock_path = self.state_dir.join(LOCK_FILE);
        for _ in 0..500 {
            match OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&lock_path)
            {
                Ok(_) => return Ok(StateLock { path: lock_path }),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    remove_stale_lock(&lock_path);
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!("could not create MRU lock {}", lock_path.display())
                    });
                }
            }
        }

        bail!("timed out waiting for MRU lock {}", lock_path.display())
    }

    fn load_unlocked(&self) -> Result<MruState> {
        let state_path = self.state_path();
        let bytes = match fs::read(&state_path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(MruState::default());
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("could not read MRU state {}", state_path.display()));
            }
        };

        if let Ok(file) = serde_json::from_slice::<MruFile>(&bytes) {
            Ok(file.into())
        } else {
            let _ = self.back_up_invalid_state(&state_path);
            Ok(MruState::default())
        }
    }

    fn write_unlocked(&self, state: &MruState) -> Result<()> {
        let state_path = self.state_path();
        let temporary_path = self.temporary_path();
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary_path)
            .with_context(|| {
                format!(
                    "could not create temporary MRU state {}",
                    temporary_path.display()
                )
            })?;

        serde_json::to_writer(&mut file, &MruFile::from(state))?;
        file.sync_all().with_context(|| {
            format!(
                "could not sync temporary MRU state {}",
                temporary_path.display()
            )
        })?;
        fs::rename(&temporary_path, &state_path).with_context(|| {
            format!(
                "could not replace MRU state {} with {}",
                state_path.display(),
                temporary_path.display()
            )
        })
    }

    fn back_up_invalid_state(&self, state_path: &Path) -> Result<()> {
        let backup_path = self
            .state_dir
            .join(format!("{STATE_FILE}.invalid-{}", unique_suffix()));
        fs::rename(state_path, &backup_path).with_context(|| {
            format!(
                "could not back up invalid MRU state {} to {}",
                state_path.display(),
                backup_path.display()
            )
        })
    }

    fn state_path(&self) -> PathBuf {
        self.state_dir.join(STATE_FILE)
    }

    fn temporary_path(&self) -> PathBuf {
        self.state_dir
            .join(format!(".{STATE_FILE}.{}.tmp", unique_suffix()))
    }
}

fn remove_stale_lock(lock_path: &Path) {
    let stale = fs::metadata(lock_path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| modified.elapsed().ok())
        .is_some_and(|age| age > Duration::from_secs(5));
    if stale {
        let _ = fs::remove_file(lock_path);
    }
}

fn unique_suffix() -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    format!("{}-{timestamp}", std::process::id())
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Barrier};

    use super::*;

    struct TemporaryDirectory {
        path: PathBuf,
    }

    impl TemporaryDirectory {
        fn new() -> Option<Self> {
            let path =
                std::env::temp_dir().join(format!("herdr-workspacer-test-{}", unique_suffix()));
            std::fs::create_dir_all(&path).ok().map(|()| Self { path })
        }
    }

    impl Drop for TemporaryDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn records_most_recent_path_first() {
        let directory = TemporaryDirectory::new();
        assert!(directory.is_some());
        if let Some(directory) = directory {
            let store = MruStore::new(directory.path.clone());
            let first = PathBuf::from("/first");
            let second = PathBuf::from("/second");

            assert!(store.record(first.clone()).is_ok());
            assert!(store.record(second.clone()).is_ok());
            assert!(store.record(first.clone()).is_ok());

            let state = store.load();
            assert!(state.is_ok());
            if let Ok(state) = state {
                assert_eq!(state.paths, vec![first, second]);
            }
        }
    }

    #[test]
    fn truncates_history_to_the_configured_limit() {
        let directory = TemporaryDirectory::new();
        assert!(directory.is_some());
        if let Some(directory) = directory {
            let store = MruStore::new(directory.path.clone());

            for index in 0..=MRU_LIMIT {
                assert!(store.record(PathBuf::from(format!("/{index}"))).is_ok());
            }

            let state = store.load();
            assert!(state.is_ok());
            if let Ok(state) = state {
                assert_eq!(state.paths.len(), MRU_LIMIT);
                assert_eq!(
                    state.paths.first(),
                    Some(&PathBuf::from(format!("/{MRU_LIMIT}")))
                );
                assert_eq!(state.paths.last(), Some(&PathBuf::from("/1")));
            }
        }
    }

    #[test]
    fn recovers_from_corrupt_state() {
        let directory = TemporaryDirectory::new();
        assert!(directory.is_some());
        if let Some(directory) = directory {
            assert!(fs::write(directory.path.join(STATE_FILE), b"not json").is_ok());
            let store = MruStore::new(directory.path.clone());

            let state = store.load();
            assert!(state.is_ok());
            if let Ok(state) = state {
                assert_eq!(state, MruState::default());
            }

            let entries = fs::read_dir(&directory.path);
            assert!(entries.is_ok());
            if let Ok(entries) = entries {
                let backup_count = entries
                    .filter_map(std::result::Result::ok)
                    .filter(|entry| {
                        entry
                            .file_name()
                            .to_string_lossy()
                            .starts_with("mru.json.invalid-")
                    })
                    .count();
                assert_eq!(backup_count, 1);
            }
        }
    }
    #[test]
    fn serializes_concurrent_record_updates() {
        let directory = TemporaryDirectory::new();
        assert!(directory.is_some());
        if let Some(directory) = directory {
            let barrier = Arc::new(Barrier::new(2));
            let first_barrier = Arc::clone(&barrier);
            let first_path = directory.path.clone();
            let first = std::thread::spawn(move || {
                first_barrier.wait();
                MruStore::new(first_path)
                    .record(PathBuf::from("/first"))
                    .is_ok()
            });

            let second_path = directory.path.clone();
            let second = std::thread::spawn(move || {
                barrier.wait();
                MruStore::new(second_path)
                    .record(PathBuf::from("/second"))
                    .is_ok()
            });

            let first_result = first.join();
            let second_result = second.join();
            assert!(first_result.is_ok());
            assert!(second_result.is_ok());
            if let Ok(recorded) = first_result {
                assert!(recorded);
            }
            if let Ok(recorded) = second_result {
                assert!(recorded);
            }

            let state = MruStore::new(directory.path.clone()).load();
            assert!(state.is_ok());
            if let Ok(state) = state {
                assert_eq!(state.paths.len(), 2);
                assert!(state.paths.contains(&PathBuf::from("/first")));
                assert!(state.paths.contains(&PathBuf::from("/second")));
            }
        }
    }
}
