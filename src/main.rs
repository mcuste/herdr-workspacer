//! Herdr Workspacer command-line entrypoint.

mod app;
mod herdr;
mod ui;

use anyhow::{Context, Result};
use herdr_workspacer::{
    Candidate, CandidateKind, MruStore, load_zoxide, merge_candidates, normalize_path,
};

use crate::{
    app::PickerModel,
    herdr::{
        HerdrClient, focused_workspace_id_from_environment, workspace_at_path, workspace_source,
    },
    ui::PickerOutcome,
};

fn main() {
    if let Err(error) = run() {
        eprintln!("{error:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let command = std::env::args()
        .nth(1)
        .context("usage: herdr-workspacer <open|picker|record-focus>")?;

    match command.as_str() {
        "open" => HerdrClient::from_environment()?.open_picker(),
        "picker" => run_picker(),
        "record-focus" => record_focused_workspace(),
        _ => anyhow::bail!("usage: herdr-workspacer <open|picker|record-focus>"),
    }
}

fn run_picker() -> Result<()> {
    let result = run_picker_inner();
    if let Err(error) = &result {
        ui::show_error(&format!("{error:#}"))?;
    }
    result
}

fn run_picker_inner() -> Result<()> {
    let client = HerdrClient::from_environment()?;
    let mru_store = MruStore::from_environment()?;
    let snapshot = client.snapshot()?;
    let workspace_source = workspace_source(&snapshot);
    let mru = mru_store.load()?;
    let zoxide_source = load_zoxide();
    let mut warnings = zoxide_source.warning.into_iter().collect::<Vec<_>>();
    if workspace_source.skipped > 0 {
        warnings.push(format!(
            "{} open workspace(s) have no usable directory.",
            workspace_source.skipped
        ));
    }

    let candidates = merge_candidates(workspace_source.workspaces, zoxide_source.entries, &mru)?;
    let mut model = PickerModel::new(candidates, &warnings);

    if let PickerOutcome::Selected(index) = ui::run(&mut model)? {
        let candidate = model
            .candidate(index)
            .context("picker selected a candidate that no longer exists")?;
        select_candidate(&client, &mru_store, candidate)?;
    }

    Ok(())
}

fn select_candidate(
    client: &HerdrClient,
    mru_store: &MruStore,
    candidate: &Candidate,
) -> Result<()> {
    match &candidate.kind {
        CandidateKind::Workspace { workspace_id } => client.focus_workspace(workspace_id)?,
        CandidateKind::Directory => {
            let snapshot = client.snapshot()?;
            if let Some(workspace) = workspace_at_path(&snapshot, &candidate.canonical_path)? {
                client.focus_workspace(&workspace.id)?;
            } else {
                client.create_workspace(&candidate.path)?;
            }
        }
    }

    mru_store.record(candidate.canonical_path.clone())
}

fn record_focused_workspace() -> Result<()> {
    let workspace_id = focused_workspace_id_from_environment()?;
    let client = HerdrClient::from_environment()?;
    let snapshot = client.snapshot()?;
    let workspace = workspace_source(&snapshot)
        .workspaces
        .into_iter()
        .find(|workspace| workspace.id == workspace_id);

    if let Some(workspace) = workspace {
        let mru_store = MruStore::from_environment()?;
        mru_store.record(normalize_path(&workspace.path)?)?;
    }

    Ok(())
}
