//! Public-core integration tests.

use herdr_workspacer::{
    MruState, Workspace, ZoxideEntry, filter_indices, merge_candidates, normalize_path,
};

#[test]
fn fuzzy_relevance_overrides_mru_order() -> anyhow::Result<()> {
    let first = std::env::temp_dir();
    let Some(second) = first.parent().map(std::path::Path::to_path_buf) else {
        anyhow::bail!("temporary directory has no parent");
    };
    let candidates = merge_candidates(
        vec![
            Workspace {
                id: "first".to_string(),
                label: "herdr".to_string(),
                path: first.clone(),
                native_order: 0,
            },
            Workspace {
                id: "second".to_string(),
                label: "old-herdr".to_string(),
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
