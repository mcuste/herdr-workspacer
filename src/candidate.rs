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
    /// The workspace's stable directory.
    pub path: PathBuf,
    /// Its order in Herdr's workspace snapshot.
    pub native_order: usize,
}

impl Candidate {
    fn workspace(workspace: Workspace, canonical_path: PathBuf) -> Self {
        let display_path = display_path(&workspace.path);
        let search_text = search_text(&workspace.label, &workspace.path);
        let label = safe_terminal_text(&workspace.label);

        Self {
            kind: CandidateKind::Workspace {
                workspace_id: workspace.id,
            },
            path: workspace.path,
            canonical_path,
            display_path,
            label,
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
        let search_text = search_text(&raw_label, &entry.path);
        let label = safe_terminal_text(&raw_label);

        Self {
            kind: CandidateKind::Directory,
            path: entry.path,
            canonical_path,
            display_path,
            label,
            search_text,
            zoxide_score: Some(entry.score),
            source_order,
        }
    }

    /// Returns whether selecting this candidate focuses an open workspace.
    pub fn is_workspace(&self) -> bool {
        matches!(self.kind, CandidateKind::Workspace { .. })
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
        if !entry.path.is_dir() {
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

fn sort_candidates(candidates: &mut [Candidate], mru: &MruState) {
    let ranks: HashMap<_, _> = mru
        .paths
        .iter()
        .enumerate()
        .map(|(rank, path)| (path.as_path(), rank))
        .collect();

    candidates.sort_by(|left, right| {
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
    });
}

fn fallback_order(left: &Candidate, right: &Candidate) -> Ordering {
    left.is_workspace()
        .cmp(&right.is_workspace())
        .reverse()
        .then_with(|| match (&left.kind, &right.kind) {
            (CandidateKind::Workspace { .. }, CandidateKind::Workspace { .. }) => {
                left.source_order.cmp(&right.source_order)
            }
            (CandidateKind::Directory, CandidateKind::Directory) => right
                .zoxide_score
                .unwrap_or_default()
                .total_cmp(&left.zoxide_score.unwrap_or_default()),
            _ => Ordering::Equal,
        })
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

fn search_text(label: &str, path: &Path) -> String {
    let basename = path
        .file_name()
        .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
    format!("{label} {} {basename}", path.display())
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
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    fn workspace(id: &str, path: &str, native_order: usize) -> Workspace {
        Workspace {
            id: id.to_string(),
            label: id.to_string(),
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
    fn ranks_mru_paths_before_native_workspace_order() {
        let workspaces = vec![
            workspace("first", "/workspaces/first", 0),
            workspace("second", "/workspaces/second", 1),
        ];
        let mru = MruState {
            paths: vec![PathBuf::from("/workspaces/second")],
        };
        let candidates = merge_candidates(workspaces, Vec::new(), &mru);

        assert!(candidates.is_ok());
        if let Ok(candidates) = candidates {
            assert_eq!(
                candidates
                    .iter()
                    .map(|candidate| candidate.label.as_str())
                    .collect::<Vec<_>>(),
                vec!["second", "first"]
            );
        }
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
