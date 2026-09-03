use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering as AtomicOrdering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::Result;

use crate::{mru::MruState, zoxide::ZoxideEntry};

/// The target selected from the workspace picker.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CandidateKind {
    /// An already-open Herdr workspace.
    Workspace {
        /// Herdr's opaque workspace identifier.
        workspace_id: String,
    },
    /// A directory known only by zoxide.
    Directory,
}

/// A workspace-picker row after source merging and ordering.
#[derive(Clone, Debug)]
pub struct Candidate {
    /// The action that selecting this row performs.
    pub kind: CandidateKind,
    /// The source path shown to the user.
    pub path: PathBuf,
    /// The normalized path used for deduplication and MRU state.
    pub canonical_path: PathBuf,
    /// A home-relative form of `path` when possible.
    pub display_path: String,
    /// The workspace or directory label.
    pub label: String,
    pub(crate) is_worktree: bool,
    pub(crate) search_text: String,
    pub(crate) zoxide_score: Option<f64>,
    pub(crate) source_order: usize,
}

/// An open Herdr workspace with its stable directory.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Workspace {
    /// Herdr's opaque workspace identifier.
    pub id: String,
    /// The label shown by Herdr.
    pub label: String,
    /// Whether Herdr created this workspace for a Git worktree.
    pub is_worktree: bool,
    /// The workspace's stable directory.
    pub path: PathBuf,
    /// Its order in Herdr's workspace snapshot.
    pub native_order: usize,
}

impl Candidate {
    fn workspace(workspace: Workspace, canonical_path: PathBuf) -> Self {
        let display_path = display_path(&workspace.path);
        let raw_label = worktree_label(&workspace.label, workspace.is_worktree);
        let search_text = format!("{} {raw_label} {display_path}", workspace.label);
        let label = safe_terminal_text(raw_label);
        Self {
            kind: CandidateKind::Workspace {
                workspace_id: workspace.id,
            },
            path: workspace.path,
            canonical_path,
            display_path,
            label,
            is_worktree: workspace.is_worktree,
            search_text,
            zoxide_score: None,
            source_order: workspace.native_order,
        }
    }

    fn directory(entry: ZoxideEntry, canonical_path: PathBuf, source_order: usize) -> Self {
        let display_path = display_path(&entry.path);
        let raw_label = entry
            .path
            .file_name()
            .filter(|name| !name.is_empty())
            .map_or_else(
                || display_path.clone(),
                |name| name.to_string_lossy().into_owned(),
            );
        let search_text = search_text(&raw_label, &display_path);
        let label = safe_terminal_text(&raw_label);

        Self {
            kind: CandidateKind::Directory,
            path: entry.path,
            canonical_path,
            display_path,
            label,
            is_worktree: false,
            search_text,
            zoxide_score: Some(entry.score),
            source_order,
        }
    }

    /// Returns whether selecting this candidate focuses an open workspace.
    pub fn is_workspace(&self) -> bool {
        matches!(self.kind, CandidateKind::Workspace { .. })
    }

    /// Returns whether this workspace represents a Git worktree.
    pub fn is_worktree(&self) -> bool {
        self.is_workspace() && self.is_worktree
    }
}

/// Merges workspace and zoxide sources into the picker order.
///
/// Callers pass zoxide entries that [`existing_directories`] already confirmed. This function does
/// not check the filesystem again.
pub fn merge_candidates(
    workspaces: Vec<Workspace>,
    zoxide_entries: Vec<ZoxideEntry>,
    mru: &MruState,
) -> Result<Vec<Candidate>> {
    let mut workspace_paths = HashSet::with_capacity(workspaces.len());
    let mut candidates = Vec::with_capacity(workspaces.len() + zoxide_entries.len());

    for workspace in workspaces {
        let canonical_path = normalize_path(&workspace.path)?;
        workspace_paths.insert(canonical_path.clone());
        candidates.push(Candidate::workspace(workspace, canonical_path));
    }

    for (source_order, entry) in zoxide_entries.into_iter().enumerate() {
        let canonical_path = normalize_path(&entry.path)?;
        if !workspace_paths.contains(&canonical_path) {
            candidates.push(Candidate::directory(entry, canonical_path, source_order));
        }
    }

    sort_candidates(&mut candidates, mru);
    Ok(candidates)
}

const DIRECTORY_CHECK_THREADS: usize = 8;

/// Keeps the zoxide entries whose paths are directories, checking them on worker threads.
///
/// `on_update` receives the entries confirmed within `wait`, then the complete list once more if
/// some checks finish later, so an unresponsive filesystem delays only its own entry.
pub fn existing_directories(
    entries: Vec<ZoxideEntry>,
    wait: Duration,
    mut on_update: impl FnMut(Vec<ZoxideEntry>),
) {
    let entries = Arc::new(entries);
    let next_index = Arc::new(AtomicUsize::new(0));
    let (sender, receiver) = mpsc::channel();
    let mut workers = 0;
    for _ in 0..entries.len().min(DIRECTORY_CHECK_THREADS) {
        let entries = Arc::clone(&entries);
        let next_index = Arc::clone(&next_index);
        let sender = sender.clone();
        let spawned = thread::Builder::new()
            .spawn(move || check_directories(&entries, &next_index, &sender))
            .is_ok();
        if spawned {
            workers += 1;
        }
    }
    if workers == 0 {
        check_directories(&entries, &next_index, &sender);
    }
    drop(sender);

    let deadline = Instant::now() + wait;
    let mut is_directory = vec![None; entries.len()];
    let mut pending = entries.len();
    while pending > 0 {
        let timeout = deadline.saturating_duration_since(Instant::now());
        match receiver.recv_timeout(timeout) {
            Ok((index, result)) => {
                if let Some(slot) = is_directory.get_mut(index) {
                    *slot = Some(result);
                }
                pending -= 1;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => break,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                pending = 0;
            }
        }
    }
    on_update(confirmed_entries(&entries, &is_directory));

    let mut late_results = false;
    while pending > 0 {
        let Ok((index, result)) = receiver.recv() else {
            break;
        };
        if let Some(slot) = is_directory.get_mut(index) {
            *slot = Some(result);
        }
        pending -= 1;
        late_results |= result;
    }
    if late_results {
        on_update(confirmed_entries(&entries, &is_directory));
    }
}

fn check_directories(
    entries: &[ZoxideEntry],
    next_index: &AtomicUsize,
    sender: &mpsc::Sender<(usize, bool)>,
) {
    loop {
        let index = next_index.fetch_add(1, AtomicOrdering::Relaxed);
        let Some(entry) = entries.get(index) else {
            return;
        };
        let result = !is_git_metadata(&entry.path) && entry.path.is_dir();
        if sender.send((index, result)).is_err() {
            return;
        }
    }
}

fn confirmed_entries(entries: &[ZoxideEntry], is_directory: &[Option<bool>]) -> Vec<ZoxideEntry> {
    entries
        .iter()
        .zip(is_directory)
        .filter(|(_, result)| **result == Some(true))
        .map(|(entry, _)| entry.clone())
        .collect()
}

fn is_git_metadata(path: &Path) -> bool {
    path.file_name().is_some_and(|name| name == ".git")
}

/// Returns a canonical path, retaining an absolute path when canonicalization fails.
pub fn normalize_path(path: &Path) -> Result<PathBuf> {
    let absolute_path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };

    Ok(std::fs::canonicalize(&absolute_path).unwrap_or(absolute_path))
}

fn worktree_label(label: &str, is_worktree: bool) -> &str {
    if is_worktree {
        label.strip_prefix("worktree-").unwrap_or(label)
    } else {
        label
    }
}
fn sort_candidates(candidates: &mut [Candidate], mru: &MruState) {
    let ranks: HashMap<_, _> = mru
        .paths
        .iter()
        .enumerate()
        .map(|(rank, path)| (path.as_path(), rank))
        .collect();

    candidates.sort_by(|left, right| {
        right
            .is_workspace()
            .cmp(&left.is_workspace())
            .then_with(|| {
                let left_rank = ranks.get(left.canonical_path.as_path());
                let right_rank = ranks.get(right.canonical_path.as_path());

                match (left_rank, right_rank) {
                    (Some(left_rank), Some(right_rank)) => left_rank
                        .cmp(right_rank)
                        .then_with(|| fallback_order(left, right)),
                    (Some(_), None) => Ordering::Less,
                    (None, Some(_)) => Ordering::Greater,
                    (None, None) => fallback_order(left, right),
                }
            })
    });
}

fn fallback_order(left: &Candidate, right: &Candidate) -> Ordering {
    match (&left.kind, &right.kind) {
        (CandidateKind::Workspace { .. }, CandidateKind::Workspace { .. }) => {
            left.source_order.cmp(&right.source_order)
        }
        (CandidateKind::Directory, CandidateKind::Directory) => right
            .zoxide_score
            .unwrap_or_default()
            .total_cmp(&left.zoxide_score.unwrap_or_default()),
        (CandidateKind::Directory, CandidateKind::Workspace { .. }) => Ordering::Less,
        (CandidateKind::Workspace { .. }, CandidateKind::Directory) => Ordering::Greater,
    }
    .then_with(|| left.label.cmp(&right.label))
    .then_with(|| left.canonical_path.cmp(&right.canonical_path))
}

fn display_path(path: &Path) -> String {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let display = if let Some(relative_path) = home
        .as_deref()
        .and_then(|home| path.strip_prefix(home).ok())
    {
        if relative_path.as_os_str().is_empty() {
            "~".to_string()
        } else {
            format!("~/{}", relative_path.display())
        }
    } else {
        path.display().to_string()
    };

    safe_terminal_text(&display)
}

fn search_text(label: &str, display_path: &str) -> String {
    format!("{label} {display_path}")
}

fn safe_terminal_text(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_control() {
                '�'
            } else {
                character
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::os::unix::fs::symlink;

    use super::*;
    use crate::{fuzzy::filter_indices, test_support::TemporaryDirectory};

    fn workspace(id: &str, path: &str, native_order: usize) -> Workspace {
        Workspace {
            id: id.to_string(),
            label: id.to_string(),
            is_worktree: false,
            path: PathBuf::from(path),
            native_order,
        }
    }

    fn entry(path: &Path, score: f64) -> ZoxideEntry {
        ZoxideEntry {
            path: path.to_path_buf(),
            score,
        }
    }

    fn labels(candidates: &[Candidate]) -> Vec<&str> {
        candidates
            .iter()
            .map(|candidate| candidate.label.as_str())
            .collect()
    }

    fn is_subsequence(part: &[PathBuf], whole: &[PathBuf]) -> bool {
        let mut remaining = whole.iter();
        part.iter()
            .all(|path| remaining.by_ref().any(|candidate| candidate == path))
    }

    #[cfg(unix)]
    #[test]
    fn canonical_paths_deduplicate_workspace_and_zoxide_entries() -> Result<()> {
        let root = TemporaryDirectory::new()?;
        let directory = root.path.join("project");
        let alias = root.path.join("project-alias");
        std::fs::create_dir(&directory)?;
        symlink(&directory, &alias)?;

        let candidates = merge_candidates(
            vec![workspace("workspace", &alias.display().to_string(), 0)],
            vec![entry(&directory, 10.0)],
            &MruState::default(),
        )?;

        anyhow::ensure!(candidates.len() == 1, "expected one candidate");
        anyhow::ensure!(
            candidates.first().is_some_and(Candidate::is_workspace),
            "expected the workspace candidate"
        );
        Ok(())
    }

    #[test]
    fn confirms_directories_in_zoxide_order() -> Result<()> {
        let root = TemporaryDirectory::new()?;
        let first = root.path.join("first");
        let second = root.path.join("second");
        let git = root.path.join(".git");
        std::fs::create_dir(&first)?;
        std::fs::create_dir(&second)?;
        std::fs::create_dir(&git)?;
        let mut updates = Vec::new();

        existing_directories(
            vec![
                entry(&second, 3.0),
                entry(&root.path.join("missing"), 2.5),
                entry(&git, 2.0),
                entry(&first, 1.0),
            ],
            Duration::from_secs(5),
            |entries| updates.push(entries),
        );

        anyhow::ensure!(updates.len() == 1, "expected one update");
        let paths = updates
            .iter()
            .flatten()
            .map(|entry| entry.path.clone())
            .collect::<Vec<_>>();
        anyhow::ensure!(
            paths == vec![second, first],
            "directory check changed the entry order or kept a non-directory"
        );
        Ok(())
    }

    #[test]
    fn late_directory_checks_end_in_one_complete_update() -> Result<()> {
        let root = TemporaryDirectory::new()?;
        let directories = (0..16)
            .map(|index| root.path.join(format!("directory-{index}")))
            .collect::<Vec<_>>();
        for directory in &directories {
            std::fs::create_dir(directory)?;
        }
        let mut entries = directories
            .iter()
            .map(|directory| entry(directory, 1.0))
            .collect::<Vec<_>>();
        entries.insert(4, entry(&root.path.join("missing"), 1.0));
        let mut updates = Vec::new();

        existing_directories(entries, Duration::ZERO, |entries| {
            updates.push(
                entries
                    .into_iter()
                    .map(|entry| entry.path)
                    .collect::<Vec<_>>(),
            );
        });

        anyhow::ensure!(
            (1..=2).contains(&updates.len()),
            "expected one or two updates, got {}",
            updates.len()
        );
        anyhow::ensure!(
            updates.last() == Some(&directories),
            "final update did not list every directory in zoxide order: {updates:?}"
        );
        anyhow::ensure!(
            updates
                .iter()
                .all(|update| is_subsequence(update, &directories)),
            "an update reordered or added entries: {updates:?}"
        );
        Ok(())
    }

    #[test]
    fn reports_no_directories_without_entries() {
        let mut updates = 0;
        existing_directories(Vec::new(), Duration::from_secs(1), |entries| {
            assert!(entries.is_empty());
            updates += 1;
        });
        assert_eq!(updates, 1);
    }

    #[test]
    fn replaces_control_characters_in_labels_and_paths() -> Result<()> {
        let raw_label = "label\x1b[31mred";
        let candidates = merge_candidates(
            vec![Workspace {
                id: "workspace".to_string(),
                label: raw_label.to_string(),
                is_worktree: false,
                path: PathBuf::from("/projects/workspace"),
                native_order: 0,
            }],
            vec![entry(Path::new("/projects/dir\x1b[31mred"), 1.0)],
            &MruState::default(),
        )?;

        anyhow::ensure!(
            labels(&candidates) == vec!["label\u{fffd}[31mred", "dir\u{fffd}[31mred"],
            "labels kept control characters: {:?}",
            labels(&candidates)
        );
        anyhow::ensure!(
            candidates
                .iter()
                .all(|candidate| !candidate.display_path.contains('\x1b')),
            "a display path kept a control character"
        );
        anyhow::ensure!(
            filter_indices(&candidates, raw_label) == vec![0],
            "the original label is no longer searchable"
        );
        Ok(())
    }

    #[test]
    fn worktree_candidates_hide_the_generated_label_prefix() -> Result<()> {
        let candidates = merge_candidates(
            vec![Workspace {
                id: "worktree-feature".to_string(),
                label: "worktree-feature".to_string(),
                is_worktree: true,
                path: PathBuf::from("/projects/feature"),
                native_order: 0,
            }],
            Vec::new(),
            &MruState::default(),
        )?;

        let candidate = candidates
            .first()
            .ok_or_else(|| anyhow::anyhow!("missing worktree candidate"))?;
        anyhow::ensure!(
            candidate.label == "feature",
            "worktree name kept its prefix"
        );
        anyhow::ensure!(candidate.is_worktree(), "worktree metadata was lost");
        Ok(())
    }

    #[test]
    fn keeps_multiple_workspaces_at_the_same_path() -> Result<()> {
        let path = "/projects/shared";
        let candidates = merge_candidates(
            vec![workspace("first", path, 0), workspace("second", path, 1)],
            vec![entry(Path::new(path), 10.0)],
            &MruState::default(),
        )?;

        anyhow::ensure!(
            labels(&candidates) == vec!["first", "second"],
            "expected both workspaces and no zoxide row: {:?}",
            labels(&candidates)
        );
        Ok(())
    }

    #[test]
    fn ranks_mru_paths_before_native_order() -> Result<()> {
        let mru = MruState {
            paths: vec![PathBuf::from("/projects/second")],
        };

        let candidates = merge_candidates(
            vec![
                workspace("first", "/projects/first", 0),
                workspace("second", "/projects/second", 1),
            ],
            Vec::new(),
            &mru,
        )?;

        anyhow::ensure!(
            labels(&candidates) == vec!["second", "first"],
            "MRU order was not preserved"
        );
        Ok(())
    }

    #[test]
    fn ranks_open_workspaces_before_zoxide_paths() -> Result<()> {
        let candidates = merge_candidates(
            vec![workspace("workspace", "/workspaces/open", 0)],
            vec![entry(Path::new("/projects/zoxide"), 10.0)],
            &MruState {
                paths: vec![PathBuf::from("/projects/zoxide")],
            },
        )?;

        anyhow::ensure!(
            labels(&candidates) == vec!["workspace", "zoxide"],
            "open workspace did not appear before zoxide"
        );
        Ok(())
    }

    #[test]
    fn orders_unknown_directories_by_descending_zoxide_score() -> Result<()> {
        let candidates = merge_candidates(
            Vec::new(),
            vec![
                entry(Path::new("/projects/first"), 1.0),
                entry(Path::new("/projects/second"), 2.0),
            ],
            &MruState::default(),
        )?;

        anyhow::ensure!(
            labels(&candidates) == vec!["second", "first"],
            "zoxide score order was not preserved"
        );
        Ok(())
    }
}
