//! Public-core integration tests.

use std::path::PathBuf;

use herdr_workspacer::{MruState, Workspace, filter_indices, merge_candidates};

#[test]
fn mru_order_survives_fuzzy_filtering() {
    let candidates = merge_candidates(
        vec![
            Workspace {
                id: "first".to_string(),
                label: "herdr".to_string(),
                path: PathBuf::from("/workspaces/herdr"),
                native_order: 0,
            },
            Workspace {
                id: "second".to_string(),
                label: "old-herdr".to_string(),
                path: PathBuf::from("/workspaces/old-herdr"),
                native_order: 1,
            },
        ],
        Vec::new(),
        &MruState {
            paths: vec![PathBuf::from("/workspaces/old-herdr")],
        },
    );

    assert!(candidates.is_ok());
    if let Ok(candidates) = candidates {
        assert_eq!(
            candidates
                .iter()
                .map(|candidate| candidate.label.as_str())
                .collect::<Vec<_>>(),
            vec!["old-herdr", "herdr"]
        );
        assert_eq!(filter_indices(&candidates, "hdr"), vec![0, 1]);
    }
}
