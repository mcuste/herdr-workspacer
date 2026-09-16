use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
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
    #[serde(default)]
    is_linked_worktree: bool,
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
        let output = checked_output("herdr api snapshot", self.command(["api", "snapshot"]))?;
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
        let mut command = self.command(["workspace", "create", "--cwd"]);
        command.arg(path).arg("--focus");
        checked_output("create workspace", command)?;
        Ok(())
    }

    fn run<const N: usize>(&self, action: &str, arguments: [&str; N]) -> Result<()> {
        checked_output(action, self.command(arguments))?;
        Ok(())
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
                is_worktree: workspace
                    .worktree
                    .as_ref()
                    .is_some_and(|worktree| worktree.is_linked_worktree),
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
    let workspace_panes = || {
        snapshot
            .panes
            .iter()
            .filter(|pane| pane.workspace_id == workspace.workspace_id)
    };
    let active_tab_panes =
        || workspace_panes().filter(|pane| pane.tab_id == workspace.active_tab_id);

    workspace
        .worktree
        .as_ref()
        .map(|worktree| worktree.checkout_path.clone())
        .or_else(|| active_tab_panes().find_map(|pane| pane.cwd.clone()))
        .or_else(|| workspace_panes().find_map(|pane| pane.cwd.clone()))
        .or_else(|| active_tab_panes().find_map(|pane| pane.foreground_cwd.clone()))
        .or_else(|| workspace_panes().find_map(|pane| pane.foreground_cwd.clone()))
}

/// `action` names the operation in the error when the command exits non-zero.
fn checked_output(action: &str, mut command: Command) -> Result<Output> {
    let output = command.output()?;
    if output.status.success() {
        Ok(output)
    } else {
        Err(command_error(action, &output))
    }
}

fn command_error(action: &str, output: &Output) -> anyhow::Error {
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

    fn snapshot(value: &str) -> Result<Snapshot> {
        Ok(serde_json::from_str::<ApiResponse>(value)
            .context("test snapshot is not valid JSON")?
            .result
            .snapshot)
    }

    fn first_workspace(value: &str) -> Result<Workspace> {
        workspace_source(&snapshot(value)?)
            .workspaces
            .into_iter()
            .next()
            .context("snapshot produced no workspace")
    }

    #[test]
    fn prefers_worktree_checkout_over_pane_paths() -> Result<()> {
        let workspace = first_workspace(
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
        )?;

        anyhow::ensure!(
            workspace.path.as_path() == Path::new("/worktree"),
            "pane paths won over the worktree checkout: {:?}",
            workspace.path
        );
        Ok(())
    }

    #[test]
    fn treats_primary_git_workspaces_as_regular_workspaces() -> Result<()> {
        let workspace = first_workspace(
            r#"
            {
              "result": {
                "snapshot": {
                  "workspaces": [{
                    "workspace_id": "w1",
                    "label": "loom",
                    "active_tab_id": "w1:t1",
                    "worktree": {
                      "checkout_path": "/loom",
                      "is_linked_worktree": false
                    }
                  }],
                  "panes": []
                }
              }
            }
        "#,
        )?;

        anyhow::ensure!(
            !workspace.is_worktree,
            "a primary checkout was marked as a worktree"
        );
        Ok(())
    }

    #[test]
    fn uses_active_tab_shell_directory_without_a_worktree() -> Result<()> {
        let workspace = first_workspace(
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
        )?;

        anyhow::ensure!(
            workspace.path.as_path() == Path::new("/active"),
            "another tab won over the active tab: {:?}",
            workspace.path
        );
        Ok(())
    }

    #[test]
    fn skips_workspaces_without_a_directory() -> Result<()> {
        let source = workspace_source(&snapshot(
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
        )?);

        anyhow::ensure!(
            source.workspaces.is_empty() && source.skipped == 1,
            "a workspace without a directory was not skipped"
        );
        Ok(())
    }

    #[test]
    fn falls_back_to_any_pane_shell_directory_before_foreground_directories() -> Result<()> {
        let workspace = first_workspace(
            r#"
            {
              "result": {
                "snapshot": {
                  "workspaces": [{
                    "workspace_id": "w1",
                    "label": "project",
                    "active_tab_id": "w1:t1"
                  }],
                  "panes": [
                    { "workspace_id": "w1", "tab_id": "w1:t1", "cwd": null, "foreground_cwd": "/active-process" },
                    { "workspace_id": "w2", "tab_id": "w2:t1", "cwd": "/other-workspace", "foreground_cwd": null },
                    { "workspace_id": "w1", "tab_id": "w1:t2", "cwd": "/other-tab", "foreground_cwd": null }
                  ]
                }
              }
            }
        "#,
        )?;

        anyhow::ensure!(
            workspace.path.as_path() == Path::new("/other-tab"),
            "a foreground directory won over another tab shell directory: {:?}",
            workspace.path
        );
        Ok(())
    }

    #[test]
    fn falls_back_to_the_active_tab_foreground_directory() -> Result<()> {
        let workspace = first_workspace(
            r#"
            {
              "result": {
                "snapshot": {
                  "workspaces": [{
                    "workspace_id": "w1",
                    "label": "project",
                    "active_tab_id": "w1:t1"
                  }],
                  "panes": [
                    { "workspace_id": "w1", "tab_id": "w1:t2", "cwd": null, "foreground_cwd": "/other-tab" },
                    { "workspace_id": "w1", "tab_id": "w1:t1", "cwd": null, "foreground_cwd": "/active-process" }
                  ]
                }
              }
            }
        "#,
        )?;

        anyhow::ensure!(
            workspace.path.as_path() == Path::new("/active-process"),
            "another tab won over the active tab foreground directory: {:?}",
            workspace.path
        );
        Ok(())
    }
}
