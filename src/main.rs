//! Herdr Workspacer command-line entrypoint.

mod app;
mod herdr;
mod ui;

use std::{sync::mpsc, thread, time::Duration};

use anyhow::{Context, Result};
use herdr_workspacer::{
    Candidate, CandidateKind, MruState, MruStore, Workspace, ZoxideSource, load_zoxide_directories,
    merge_candidates, normalize_path,
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

/// Directory checks slower than this join the list in a later update.
const ZOXIDE_DIRECTORY_WAIT: Duration = Duration::from_millis(500);

fn run_picker_inner() -> Result<()> {
    let client = HerdrClient::from_environment()?;
    let mru_store = MruStore::from_environment()?;
    let zoxide_updates = load_zoxide_in_background();
    let snapshot = client.snapshot()?;
    let workspace_source = workspace_source(&snapshot);
    let mru = mru_store.load()?;
    let mut warnings = Vec::new();
    if workspace_source.skipped > 0 {
        warnings.push(format!(
            "{} open workspace(s) have no usable directory.",
            workspace_source.skipped
        ));
    }

    let workspaces = workspace_source.workspaces;
    let candidates = merge_candidates(workspaces.clone(), Vec::new(), &mru)?;
    let mut model = PickerModel::new(candidates, &warnings);
    let outcome = ui::run(&mut model, |model| {
        apply_zoxide_updates(model, &zoxide_updates, &workspaces, &mru)
    })?;

    if let PickerOutcome::Selected(index) = outcome {
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

/// Runs on a thread so the picker opens before zoxide answers.
fn load_zoxide_in_background() -> mpsc::Receiver<ZoxideSource> {
    let (sender, receiver) = mpsc::channel();
    let loader_sender = sender.clone();
    let loader = thread::Builder::new().spawn(move || {
        load_zoxide_directories(ZOXIDE_DIRECTORY_WAIT, |source| {
            let _ = loader_sender.send(source);
        });
    });
    if loader.is_err() {
        let _ = sender.send(ZoxideSource {
            entries: Vec::new(),
            warning: Some("zoxide could not load. Showing open workspaces only.".to_string()),
        });
    }
    receiver
}

fn apply_zoxide_updates(
    model: &mut PickerModel,
    updates: &mpsc::Receiver<ZoxideSource>,
    workspaces: &[Workspace],
    mru: &MruState,
) -> Result<bool> {
    let mut changed = false;
    while let Ok(source) = updates.try_recv() {
        if let Some(warning) = &source.warning {
            model.push_warning(warning);
        }
        model.replace_candidates(merge_candidates(workspaces.to_vec(), source.entries, mru)?);
        changed = true;
    }
    Ok(changed)
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

#[cfg(all(test, unix))]
mod tests {
    use std::{
        fs,
        os::unix::fs::PermissionsExt,
        path::PathBuf,
        sync::atomic::{AtomicUsize, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };

    use herdr_workspacer::{MruState, ZoxideEntry};

    use super::*;

    static NEXT_TEMPORARY_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

    struct TemporaryDirectory {
        path: PathBuf,
    }

    impl TemporaryDirectory {
        fn new() -> Result<Self> {
            let sequence = NEXT_TEMPORARY_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |duration| duration.as_nanos());
            let path = std::env::temp_dir().join(format!(
                "herdr-workspacer-main-{}-{timestamp}-{sequence}",
                std::process::id()
            ));
            fs::create_dir(&path)?;
            Ok(Self { path })
        }
    }

    impl Drop for TemporaryDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn selecting_directory_focuses_workspace_that_appeared_since_picker_opened() -> Result<()> {
        let temporary = TemporaryDirectory::new()?;
        let directory = temporary.path.join("project");
        fs::create_dir(&directory)?;

        let binary = temporary.path.join("herdr");
        fs::write(
            &binary,
            r#"#!/bin/sh
base=${0%/*}
printf '%s\n' "$*" >> "$base/commands.log"
case "$1:$2" in
api:snapshot)
    cat "$base/snapshot.json"
    ;;
workspace:focus)
    exit 0
    ;;
*)
    printf '%s\n' 'unexpected command' >&2
    exit 1
    ;;
esac
"#,
        )?;
        let mut permissions = fs::metadata(&binary)?.permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&binary, permissions)?;

        let directory_text = directory.display().to_string();
        let snapshot = serde_json::json!({
            "result": {
                "snapshot": {
                    "workspaces": [{
                        "workspace_id": "fresh-workspace",
                        "label": "project",
                        "active_tab_id": "tab",
                        "worktree": { "checkout_path": directory_text }
                    }],
                    "panes": []
                }
            }
        });
        fs::write(
            temporary.path.join("snapshot.json"),
            serde_json::to_vec(&snapshot)?,
        )?;

        let candidates = merge_candidates(
            Vec::new(),
            vec![ZoxideEntry {
                path: directory.clone(),
                score: 10.0,
            }],
            &MruState::default(),
        )?;
        let Some(candidate) = candidates.into_iter().next() else {
            anyhow::bail!("expected a directory candidate");
        };
        let expected_path = candidate.canonical_path.clone();
        let client = HerdrClient::new(binary.into_os_string());
        let mru_store = MruStore::new(temporary.path.join("state"));

        select_candidate(&client, &mru_store, &candidate)?;

        let commands = fs::read_to_string(temporary.path.join("commands.log"))?;
        anyhow::ensure!(
            commands == "api snapshot\nworkspace focus fresh-workspace\n",
            "unexpected Herdr commands: {commands:?}"
        );
        let state = mru_store.load()?;
        anyhow::ensure!(
            state.paths == vec![expected_path],
            "expected the selected directory in MRU state"
        );
        Ok(())
    }
}
