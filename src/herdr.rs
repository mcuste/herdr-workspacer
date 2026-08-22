use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use anyhow::{Context, Result};
use serde::Deserialize;

use herdr_workspacer::{Workspace, normalize_path};

#[derive(Debug)]
pub(crate) struct HerdrClient {
    binary: OsString,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Snapshot {
    workspaces: Vec<ApiWorkspace>,
    panes: Vec<ApiPane>,
}

#[derive(Debug, Deserialize)]
struct ApiResponse {
    result: ApiResult,
}

#[derive(Debug, Deserialize)]
struct ApiResult {
    snapshot: Snapshot,
}

#[derive(Debug, Deserialize)]
struct ApiWorkspace {
    workspace_id: String,
    label: String,
    active_tab_id: String,
    worktree: Option<Worktree>,
}

#[derive(Debug, Deserialize)]
struct Worktree {
    checkout_path: PathBuf,
}

#[derive(Debug, Deserialize)]
struct ApiPane {
    workspace_id: String,
    tab_id: String,
    cwd: Option<PathBuf>,
    foreground_cwd: Option<PathBuf>,
}

#[derive(Debug)]
pub(crate) struct WorkspaceSource {
    pub(crate) workspaces: Vec<Workspace>,
    pub(crate) skipped: usize,
}

#[derive(Debug, Deserialize)]
struct FocusEventEnvelope {
    data: FocusEvent,
}

#[derive(Debug, Deserialize)]
struct FocusEvent {
    workspace_id: String,
}

impl HerdrClient {
    pub(crate) fn new(binary: OsString) -> Self {
        Self { binary }
    }

    pub(crate) fn from_environment() -> Result<Self> {
        let binary =
            std::env::var_os("HERDR_BIN_PATH").context("Herdr did not provide HERDR_BIN_PATH")?;
        Ok(Self::new(binary))
    }

    pub(crate) fn snapshot(&self) -> Result<Snapshot> {
        let output = self.command(["api", "snapshot"]).output()?;
        if !output.status.success() {
            return Err(command_error("herdr api snapshot", &output));
        }

        let response = serde_json::from_slice::<ApiResponse>(&output.stdout)
            .context("Herdr returned an invalid session snapshot")?;
        Ok(response.result.snapshot)
    }

    pub(crate) fn open_picker(&self) -> Result<()> {
        self.run(
            "open workspace picker",
            [
                "plugin",
                "pane",
                "open",
                "--plugin",
                "herdr-workspacer",
                "--entrypoint",
                "picker",
            ],
        )
    }

    pub(crate) fn focus_workspace(&self, workspace_id: &str) -> Result<()> {
        self.run("focus workspace", ["workspace", "focus", workspace_id])
    }

    pub(crate) fn create_workspace(&self, path: &Path) -> Result<()> {
        let output = self
            .command(["workspace", "create", "--cwd"])
            .arg(path)
            .arg("--focus")
            .output()?;
        if output.status.success() {
            Ok(())
        } else {
            Err(command_error("create workspace", &output))
        }
    }

    fn run<const N: usize>(&self, action: &str, arguments: [&str; N]) -> Result<()> {
        let output = self.command(arguments).output()?;
        if output.status.success() {
            Ok(())
        } else {
            Err(command_error(action, &output))
        }
    }

    fn command<const N: usize>(&self, arguments: [&str; N]) -> Command {
        let mut command = Command::new(&self.binary);
        command.args(arguments).stdin(Stdio::null());
        command
    }
}

pub(crate) fn workspace_source(snapshot: &Snapshot) -> WorkspaceSource {
    let workspaces = snapshot
        .workspaces
        .iter()
        .enumerate()
        .filter_map(|(native_order, workspace)| {
            workspace_path(snapshot, workspace).map(|path| Workspace {
                id: workspace.workspace_id.clone(),
                label: workspace.label.clone(),
                path,
                native_order,
            })
        })
        .collect::<Vec<_>>();
    let skipped = snapshot.workspaces.len().saturating_sub(workspaces.len());

    WorkspaceSource {
        workspaces,
        skipped,
    }
}

pub(crate) fn workspace_at_path(snapshot: &Snapshot, path: &Path) -> Result<Option<Workspace>> {
    let target = normalize_path(path)?;
    Ok(workspace_source(snapshot)
        .workspaces
        .into_iter()
        .find(|workspace| {
            normalize_path(&workspace.path).is_ok_and(|candidate_path| candidate_path == target)
        }))
}

pub(crate) fn focused_workspace_id_from_environment() -> Result<String> {
    let event = std::env::var("HERDR_PLUGIN_EVENT_JSON")
        .context("Herdr did not provide HERDR_PLUGIN_EVENT_JSON")?;
    let event = serde_json::from_str::<FocusEventEnvelope>(&event)
        .context("Herdr provided an invalid workspace focus event")?;
    Ok(event.data.workspace_id)
}

fn workspace_path(snapshot: &Snapshot, workspace: &ApiWorkspace) -> Option<PathBuf> {
    workspace
        .worktree
        .as_ref()
        .map(|worktree| worktree.checkout_path.clone())
        .or_else(|| {
            snapshot
                .panes
                .iter()
                .filter(|pane| {
                    pane.workspace_id == workspace.workspace_id
                        && pane.tab_id == workspace.active_tab_id
                })
                .find_map(|pane| pane.cwd.clone())
        })
        .or_else(|| {
            snapshot
                .panes
                .iter()
                .filter(|pane| pane.workspace_id == workspace.workspace_id)
                .find_map(|pane| pane.cwd.clone())
        })
        .or_else(|| {
            snapshot
                .panes
                .iter()
                .filter(|pane| {
                    pane.workspace_id == workspace.workspace_id
                        && pane.tab_id == workspace.active_tab_id
                })
                .find_map(|pane| pane.foreground_cwd.clone())
        })
        .or_else(|| {
            snapshot
                .panes
                .iter()
                .filter(|pane| pane.workspace_id == workspace.workspace_id)
                .find_map(|pane| pane.foreground_cwd.clone())
        })
}

fn command_error(action: &str, output: &std::process::Output) -> anyhow::Error {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let details = stderr.trim();
    if details.is_empty() {
        anyhow::anyhow!("could not {action}: Herdr exited with {}", output.status)
    } else {
        anyhow::anyhow!("could not {action}: {details}")
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn prefers_worktree_checkout_over_pane_paths() {
        let snapshot = snapshot(
            r#"
            {
              "result": {
                "snapshot": {
                  "workspaces": [{
                    "workspace_id": "w1",
                    "label": "project",
                    "active_tab_id": "w1:t1",
                    "worktree": { "checkout_path": "/worktree" }
                  }],
                  "panes": [{
                    "workspace_id": "w1",
                    "tab_id": "w1:t1",
                    "cwd": "/shell",
                    "foreground_cwd": "/process"
                  }]
                }
              }
            }
        "#,
        );

        assert!(snapshot.is_some());
        if let Some(snapshot) = snapshot {
            assert_eq!(
                workspace_source(&snapshot)
                    .workspaces
                    .first()
                    .map(|workspace| &workspace.path),
                Some(&PathBuf::from("/worktree"))
            );
        }
    }

    #[test]
    fn uses_active_tab_shell_directory_without_a_worktree() {
        let snapshot = snapshot(
            r#"
            {
              "result": {
                "snapshot": {
                  "workspaces": [{
                    "workspace_id": "w1",
                    "label": "project",
                    "active_tab_id": "w1:t2"
                  }],
                  "panes": [
                    { "workspace_id": "w1", "tab_id": "w1:t1", "cwd": "/other", "foreground_cwd": null },
                    { "workspace_id": "w1", "tab_id": "w1:t2", "cwd": "/active", "foreground_cwd": null }
                  ]
                }
              }
            }
        "#,
        );

        assert!(snapshot.is_some());
        if let Some(snapshot) = snapshot {
            assert_eq!(
                workspace_source(&snapshot)
                    .workspaces
                    .first()
                    .map(|workspace| &workspace.path),
                Some(&PathBuf::from("/active"))
            );
        }
    }

    #[test]
    fn skips_workspaces_without_a_directory() {
        let snapshot = snapshot(
            r#"
            {
              "result": {
                "snapshot": {
                  "workspaces": [{
                    "workspace_id": "w1",
                    "label": "project",
                    "active_tab_id": "w1:t1"
                  }],
                  "panes": []
                }
              }
            }
        "#,
        );

        assert!(snapshot.is_some());
        if let Some(snapshot) = snapshot {
            let source = workspace_source(&snapshot);
            assert!(source.workspaces.is_empty());
            assert_eq!(source.skipped, 1);
        }
    }

    fn snapshot(value: &str) -> Option<Snapshot> {
        serde_json::from_str::<ApiResponse>(value)
            .ok()
            .map(|response| response.result.snapshot)
    }
}
