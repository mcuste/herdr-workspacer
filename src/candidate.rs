use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
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
        if entry.path.file_name().is_some_and(|name| name == ".git") || !entry.path.is_dir() {
            continue;
        }

        let canonical_path = normalize_path(&entry.path)?;
        if !workspace_paths.contains(&canonical_path) {
            candidates.push(Candidate::directory(entry, canonical_path, source_order));
        }
    }

    sort_candidates(&mut candidates, mru);
    Ok(candidates)
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
    use std::{
        sync::atomic::{AtomicUsize, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;

    static NEXT_TEMPORARY_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

    struct TemporaryDirectory {
        path: std::path::PathBuf,
    }

    impl TemporaryDirectory {
        fn new() -> Result<Self> {
            let sequence = NEXT_TEMPORARY_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "herdr-workspacer-candidate-{}-{}-{sequence}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_or(0, |duration| duration.as_nanos())
            ));
            std::fs::create_dir_all(&path)?;
            Ok(Self { path })
        }
    }

    impl Drop for TemporaryDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn workspace(id: &str, path: &str, native_order: usize) -> Workspace {
        Workspace {
            id: id.to_string(),
            label: id.to_string(),
            is_worktree: false,
            path: PathBuf::from(path),
            native_order,
        }
    }

    fn entry(path: &str, score: f64) -> ZoxideEntry {
        ZoxideEntry {
            path: PathBuf::from(path),
            score,
        }
    }

    #[test]
    fn workspace_suppresses_zoxide_candidate_at_the_same_path() {
        let path = std::env::temp_dir();
        let path_text = path.display().to_string();
        let workspaces = vec![workspace("workspace", &path_text, 0)];
        let entries = vec![entry(&path_text, 10.0)];
        let candidates = merge_candidates(workspaces, entries, &MruState::default());

        assert!(candidates.is_ok());
        if let Ok(candidates) = candidates {
            assert_eq!(candidates.len(), 1);
            assert!(candidates.first().is_some_and(Candidate::is_workspace));
        }
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
            vec![entry(&directory.display().to_string(), 10.0)],
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
    fn skips_missing_zoxide_directories() -> Result<()> {
        let root = TemporaryDirectory::new()?;
        let missing = root.path.join("missing");

        let candidates = merge_candidates(
            Vec::new(),
            vec![entry(&missing.display().to_string(), 10.0)],
            &MruState::default(),
        )?;

        anyhow::ensure!(candidates.is_empty(), "expected no candidates");
        Ok(())
    }

    #[test]
    fn skips_git_metadata_directories() -> Result<()> {
        let root = TemporaryDirectory::new()?;
        let git = root.path.join(".git");
        let project = root.path.join("project");
        std::fs::create_dir(&git)?;
        std::fs::create_dir(&project)?;

        let candidates = merge_candidates(
            Vec::new(),
            vec![
                entry(&git.display().to_string(), 10.0),
                entry(&project.display().to_string(), 1.0),
            ],
            &MruState::default(),
        )?;

        anyhow::ensure!(
            candidates
                .iter()
                .map(|candidate| candidate.label.as_str())
                .collect::<Vec<_>>()
                == vec!["project"],
            "git metadata directory was included"
        );
        Ok(())
    }

    #[test]
    fn searches_the_displayed_path() {
        assert_eq!(search_text("dotfiles", "~/dotfiles"), "dotfiles ~/dotfiles");
    }

    #[test]
    fn worktree_candidates_hide_the_generated_label_prefix() -> Result<()> {
        let path = std::env::temp_dir();
        let candidates = merge_candidates(
            vec![Workspace {
                id: "worktree-feature".to_string(),
                label: "worktree-feature".to_string(),
                is_worktree: true,
                path,
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
    fn keeps_multiple_workspaces_at_the_same_path() {
        let path = std::env::temp_dir().display().to_string();
        let workspaces = vec![workspace("first", &path, 0), workspace("second", &path, 1)];
        let candidates =
            merge_candidates(workspaces, vec![entry(&path, 10.0)], &MruState::default());

        assert!(candidates.is_ok());
        if let Ok(candidates) = candidates {
            assert_eq!(candidates.len(), 2);
            assert_eq!(
                candidates
                    .iter()
                    .map(|candidate| candidate.label.as_str())
                    .collect::<Vec<_>>(),
                vec!["first", "second"]
            );
        }
    }

    #[test]
    fn ranks_mru_paths_before_zoxide_score() -> Result<()> {
        let root = TemporaryDirectory::new()?;
        let first = root.path.join("first");
        let second = root.path.join("second");
        std::fs::create_dir(&first)?;
        std::fs::create_dir(&second)?;
        let first_text = first.display().to_string();
        let second_text = second.display().to_string();
        let mru = MruState {
            paths: vec![normalize_path(&second)?],
        };

        let candidates = merge_candidates(
            vec![
                workspace("first", &first_text, 0),
                workspace("second", &second_text, 1),
            ],
            vec![entry(&first_text, 1.0), entry(&second_text, 2.0)],
            &mru,
        )?;

        let labels = candidates
            .iter()
            .map(|candidate| candidate.label.as_str())
            .collect::<Vec<_>>();
        anyhow::ensure!(
            labels == vec!["second", "first"],
            "MRU order was not preserved"
        );
        Ok(())
    }

    #[test]
    fn ranks_open_workspaces_before_zoxide_paths() -> Result<()> {
        let root = TemporaryDirectory::new()?;
        let directory = root.path.join("zoxide");
        std::fs::create_dir(&directory)?;

        let candidates = merge_candidates(
            vec![workspace("workspace", "/workspaces/open", 0)],
            vec![entry(&directory.display().to_string(), 10.0)],
            &MruState::default(),
        )?;

        let labels = candidates
            .iter()
            .map(|candidate| candidate.label.as_str())
            .collect::<Vec<_>>();
        anyhow::ensure!(
            labels == vec!["workspace", "zoxide"],
            "open workspace did not appear before zoxide"
        );
        Ok(())
    }

    #[test]
    fn orders_unknown_directories_by_descending_zoxide_score() {
        let root = std::env::temp_dir().join(format!(
            "herdr-workspacer-candidate-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |duration| duration.as_nanos())
        ));
        let first = root.join("first");
        let second = root.join("second");

        assert!(std::fs::create_dir_all(&first).is_ok());
        assert!(std::fs::create_dir_all(&second).is_ok());

        let candidates = merge_candidates(
            Vec::new(),
            vec![
                entry(&first.display().to_string(), 1.0),
                entry(&second.display().to_string(), 2.0),
            ],
            &MruState::default(),
        );

        assert!(candidates.is_ok());
        if let Ok(candidates) = candidates {
            assert_eq!(
                candidates
                    .iter()
                    .map(|candidate| &candidate.path)
                    .collect::<Vec<_>>(),
                vec![&second, &first]
            );
        }

        assert!(std::fs::remove_dir_all(root).is_ok());
    }
}
