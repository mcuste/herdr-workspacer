//! Public-core integration tests.

use herdr_workspacer::{
    MruState, Workspace, ZoxideEntry, filter_indices, merge_candidates, normalize_path,
};

#[test]
fn fuzzy_relevance_overrides_mru_order() -> anyhow::Result<()> {
    let first = std::path::PathBuf::from("/projects/herdr");
    let second = std::path::PathBuf::from("/projects/old-herdr");
    let candidates = merge_candidates(
        vec![
            Workspace {
                id: "first".to_string(),
                label: "herdr".to_string(),
                is_worktree: false,
                path: first.clone(),
                native_order: 0,
            },
            Workspace {
                id: "second".to_string(),
                label: "old-herdr".to_string(),
                is_worktree: false,
                path: second.clone(),
                native_order: 1,
            },
        ],
        vec![
            ZoxideEntry {
                path: first,
                score: 1.0,
            },
            ZoxideEntry {
                path: second.clone(),
                score: 1.0,
            },
        ],
        &MruState {
            paths: vec![normalize_path(&second)?],
        },
    )?;

    let labels = candidates
        .iter()
        .map(|candidate| candidate.label.as_str())
        .collect::<Vec<_>>();
    anyhow::ensure!(
        labels == vec!["old-herdr", "herdr"],
        "MRU order was not preserved"
    );
    anyhow::ensure!(
        filter_indices(&candidates, "hdr") == vec![1, 0],
        "fuzzy filtering did not rank the stronger match first"
    );
    Ok(())
}

#[test]
fn worktree_labels_match_generated_and_displayed_forms() -> anyhow::Result<()> {
    let candidates = merge_candidates(
        vec![Workspace {
            id: "worktree-feature".to_string(),
            label: "worktree-feature".to_string(),
            is_worktree: true,
            path: std::path::PathBuf::from("/projects/feature"),
            native_order: 0,
        }],
        Vec::new(),
        &MruState::default(),
    )?;

    for query in ["worktree-feature", "feature"] {
        anyhow::ensure!(
            filter_indices(&candidates, query) == vec![0],
            "worktree candidate did not match {query:?}"
        );
    }
    Ok(())
}
