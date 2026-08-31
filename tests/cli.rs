//! Process-level tests for the plugin command contract.
#![cfg(unix)]

use std::{
    env,
    fmt::Write as _,
    fs,
    io::Write,
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::{
        LazyLock, Mutex, MutexGuard,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use serde_json::{Value, json};

static TEST_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));
static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

const HERDR_SCRIPT: &str = r#"#!/bin/sh
base=${0%/*}
printf '%s\n' "$*" >> "$base/herdr.log"

if [ -f "$base/herdr-error" ]; then
    cat "$base/herdr-error" >&2
    exit 42
fi

case "$1:$2" in
api:snapshot)
    cat "$base/snapshot.json"
    ;;
plugin:pane|workspace:focus|workspace:create)
    exit 0
    ;;
*)
    printf '%s\n' 'unexpected Herdr command' >&2
    exit 17
    ;;
esac
"#;

const STTY_SCRIPT: &str = r#"#!/bin/sh
base=${0%/*}
state="$base/terminal-state"
printf '%s\n' "$*" >> "$base/stty.log"

case "$1" in
size)
    printf '%s\n' '24 80'
    ;;
-g)
    printf '%s\n' saved-state
    ;;
raw)
    printf '%s\n' raw > "$state"
    ;;
saved-state)
    if [ ! -f "$state" ] || [ "$(cat "$state")" != raw ]; then
        printf '%s\n' 'terminal was not in raw mode' >&2
        exit 91
    fi
    printf '%s\n' saved-state > "$state"
    ;;
*)
    printf '%s\n' 'unexpected stty command' >&2
    exit 92
    ;;
esac
"#;

const ZOXIDE_FAILURE_SCRIPT: &str = r#"#!/bin/sh
base=${0%/*}
printf '%s\n' "$*" >> "$base/zoxide.log"
exit 1
"#;

const ZOXIDE_EMPTY_SCRIPT: &str = r#"#!/bin/sh
base=${0%/*}
printf '%s\n' "$*" >> "$base/zoxide.log"

case "$1:$2" in
query:-ls)
    exit 0
    ;;
*)
    exit 17
    ;;
esac
"#;

const ZOXIDE_DIRECTORY_SCRIPT: &str = r#"#!/bin/sh
base=${0%/*}
printf '%s\n' "$*" >> "$base/zoxide.log"

case "$1:$2" in
query:-ls)
    printf '10\t%s\n' "$base/zoxide-directory"
    ;;
*)
    exit 17
    ;;
esac
"#;

struct TestDirectory {
    path: PathBuf,
}

impl TestDirectory {
    fn new() -> Result<Self> {
        let sequence = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos());
        let path = env::temp_dir().join(format!(
            "herdr-workspacer-cli-{}-{timestamp}-{sequence}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir(&path)
            .with_context(|| format!("could not create test directory {}", path.display()))?;
        Ok(Self { path })
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

struct Fixture {
    directory: TestDirectory,
}

impl Fixture {
    fn new() -> Result<Self> {
        let fixture = Self {
            directory: TestDirectory::new()?,
        };
        fs::create_dir(fixture.bin_dir())?;
        write_executable(&fixture.bin_dir(), "herdr", HERDR_SCRIPT)?;
        write_executable(&fixture.bin_dir(), "stty", STTY_SCRIPT)?;
        Ok(fixture)
    }

    fn bin_dir(&self) -> PathBuf {
        self.directory.path.join("bin")
    }

    fn file(&self, name: &str) -> PathBuf {
        self.bin_dir().join(name)
    }

    fn herdr_path(&self) -> PathBuf {
        self.file("herdr")
    }

    fn state_dir(&self) -> PathBuf {
        self.directory.path.join("state")
    }

    fn state_file(&self) -> PathBuf {
        self.state_dir().join("mru.json")
    }

    fn command(&self, action: &str) -> Result<Command> {
        let inherited_path = env::var_os("PATH").context("PATH is not set")?;
        let path = env::join_paths(
            std::iter::once(self.bin_dir()).chain(env::split_paths(&inherited_path)),
        )
        .context("could not construct fixture PATH")?;
        let mut command = Command::new(env!("CARGO_BIN_EXE_herdr-workspacer"));
        command
            .arg(action)
            .env("HERDR_BIN_PATH", self.herdr_path())
            .env("HERDR_PLUGIN_STATE_DIR", self.state_dir())
            .env("PATH", path);
        Ok(command)
    }

    fn write_snapshot(&self, snapshot: &Value) -> Result<()> {
        fs::write(self.file("snapshot.json"), serde_json::to_vec(snapshot)?)?;
        Ok(())
    }

    fn write_herdr_error(&self, message: &str) -> Result<()> {
        fs::write(self.file("herdr-error"), format!("{message}\n"))?;
        Ok(())
    }

    fn install_zoxide_failure(&self) -> Result<()> {
        write_executable(&self.bin_dir(), "zoxide", ZOXIDE_FAILURE_SCRIPT)
    }

    fn install_empty_zoxide(&self) -> Result<()> {
        write_executable(&self.bin_dir(), "zoxide", ZOXIDE_EMPTY_SCRIPT)
    }

    fn install_zoxide_directory(&self) -> Result<PathBuf> {
        let directory = self.file("zoxide-directory");
        fs::create_dir(&directory)?;
        write_executable(&self.bin_dir(), "zoxide", ZOXIDE_DIRECTORY_SCRIPT)?;
        Ok(directory)
    }

    fn install_zoxide_directories(&self, count: usize) -> Result<()> {
        let mut script = String::from(
            "#!/bin/sh\nbase=${0%/*}\nprintf '%s\\n' \"$*\" >> \"$base/zoxide.log\"\n\n\
             case \"$1:$2\" in\nquery:-ls)\n",
        );
        for index in 0..count {
            fs::create_dir(self.file(&format!("zoxide-{index}")))?;
            writeln!(
                &mut script,
                "    printf '{}\\t%s/zoxide-{index}\\n' \"$base\"",
                count.saturating_sub(index)
            )?;
        }
        script.push_str("    ;;\n*)\n    exit 17\n    ;;\nesac\n");
        write_executable(&self.bin_dir(), "zoxide", &script)
    }

    fn log(&self, name: &str) -> Result<String> {
        Ok(fs::read_to_string(self.file(name))?)
    }
    fn mru_paths(&self) -> Result<Value> {
        let state = serde_json::from_slice::<Value>(&fs::read(self.state_file())?)?;
        state
            .get("paths")
            .cloned()
            .context("MRU state did not contain paths")
    }

    fn terminal_state(&self) -> Result<String> {
        Ok(fs::read_to_string(self.file("terminal-state"))?)
    }
}

fn write_executable(directory: &Path, name: &str, contents: &str) -> Result<()> {
    let path = directory.join(name);
    fs::write(&path, contents)?;
    let mut permissions = fs::metadata(&path)?.permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(path, permissions)?;
    Ok(())
}

fn serial_guard() -> MutexGuard<'static, ()> {
    match TEST_LOCK.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn worktree_snapshot(workspace_id: &str, path: &Path) -> Value {
    json!({
        "result": {
            "snapshot": {
                "workspaces": [{
                    "workspace_id": workspace_id,
                    "label": "project",
                    "active_tab_id": "tab",
                    "worktree": {
                        "checkout_path": path_to_string(path),
                        "is_linked_worktree": true
                    }
                }],
                "panes": []
            }
        }
    })
}

fn workspace_snapshot(workspace_id: &str, label: &str, path: &Path) -> Value {
    json!({
        "result": {
            "snapshot": {
                "workspaces": [{
                    "workspace_id": workspace_id,
                    "label": label,
                    "active_tab_id": "tab",
                    "worktree": {
                        "checkout_path": path_to_string(path),
                        "is_linked_worktree": false
                    }
                }],
                "panes": []
            }
        }
    })
}

fn empty_snapshot() -> Value {
    json!({
        "result": {
            "snapshot": {
                "workspaces": [],
                "panes": []
            }
        }
    })
}

fn run_picker(fixture: &Fixture, input: &[u8]) -> Result<Output> {
    let mut child = fixture
        .command("picker")?
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut stdin = child.stdin.take().context("picker stdin is unavailable")?;
    stdin.write_all(input)?;
    drop(stdin);
    Ok(child.wait_with_output()?)
}

fn ensure_success(output: &Output) -> Result<()> {
    anyhow::ensure!(
        output.status.success(),
        "command failed with {}\nstdout: {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}

fn ensure_terminal_restored(fixture: &Fixture) -> Result<()> {
    anyhow::ensure!(
        fixture.terminal_state()? == "saved-state\n",
        "terminal state was not restored"
    );
    anyhow::ensure!(
        fixture.log("stty.log")? == "size\n-g\nraw -echo min 0 time 1\nsaved-state\n",
        "picker did not use the terminal size or restore its state"
    );
    Ok(())
}

fn ensure_terminal_line_endings(output: &Output) -> Result<()> {
    let mut previous = None;
    let mut saw_line_ending = false;
    for byte in &output.stdout {
        if *byte == b'\n' {
            anyhow::ensure!(
                previous == Some(b'\r'),
                "picker emitted a line feed without a carriage return"
            );
            saw_line_ending = true;
        }
        previous = Some(*byte);
    }
    anyhow::ensure!(saw_line_ending, "picker did not render a line ending");
    Ok(())
}

fn ensure_picker_layout(output: &Output) -> Result<()> {
    let stdout = String::from_utf8_lossy(&output.stdout);
    anyhow::ensure!(
        stdout.contains(
            "\x1b[2J\x1b[H  \x1b[1mFind workspace\x1b[0m\r\n  \
             \x1b[2mType to filter\x1b[0m  \r\n\r\n"
        ),
        "picker did not render its heading and search prompt"
    );
    anyhow::ensure!(
        stdout
            .contains("\r\n  \x1b[2mEnter select  Esc cancel  ↑/↓ move  PgUp/PgDn jump\x1b[0m\r\n"),
        "picker did not render its controls"
    );
    Ok(())
}

fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[test]
fn open_requests_the_picker_from_the_herdr_cli() -> Result<()> {
    let _guard = serial_guard();
    let fixture = Fixture::new()?;

    let output = fixture.command("open")?.output()?;

    ensure_success(&output)?;
    anyhow::ensure!(
        fixture.log("herdr.log")?
            == "plugin pane open --plugin herdr-workspacer --entrypoint picker\n",
        "open sent an unexpected Herdr command"
    );
    Ok(())
}

#[test]
fn focus_event_records_the_canonical_workspace_path() -> Result<()> {
    let _guard = serial_guard();
    let fixture = Fixture::new()?;
    let workspace = fixture.file("workspace");
    let alias = fixture.file("workspace-alias");
    fs::create_dir(&workspace)?;
    symlink(&workspace, &alias)?;
    fixture.write_snapshot(&worktree_snapshot("focused", &alias))?;
    let event = json!({ "data": { "workspace_id": "focused" } });

    let output = fixture
        .command("record-focus")?
        .env("HERDR_PLUGIN_EVENT_JSON", event.to_string())
        .output()?;

    ensure_success(&output)?;
    let expected_paths = json!([path_to_string(&fs::canonicalize(&workspace)?)]);
    anyhow::ensure!(
        fixture.mru_paths()? == expected_paths,
        "focus event did not persist the canonical workspace path"
    );
    anyhow::ensure!(
        fixture.log("herdr.log")? == "api snapshot\n",
        "focus event sent an unexpected Herdr command"
    );
    Ok(())
}

#[test]
fn picker_keeps_open_workspaces_available_when_zoxide_fails() -> Result<()> {
    let _guard = serial_guard();
    let fixture = Fixture::new()?;
    let workspace = fixture.file("workspace");
    fs::create_dir(&workspace)?;
    fixture.write_snapshot(&worktree_snapshot("workspace", &workspace))?;
    fixture.install_zoxide_failure()?;

    let output = run_picker(&fixture, b"\n")?;

    ensure_success(&output)?;
    ensure_terminal_line_endings(&output)?;
    ensure_picker_layout(&output)?;
    anyhow::ensure!(
        String::from_utf8_lossy(&output.stdout)
            .contains("zoxide query failed. Showing open workspaces only."),
        "picker did not render the zoxide warning"
    );
    anyhow::ensure!(
        fixture.log("zoxide.log")? == "query -ls\n",
        "picker sent an unexpected zoxide command"
    );
    anyhow::ensure!(
        fixture.log("herdr.log")? == "api snapshot\nworkspace focus workspace\n",
        "picker did not focus the open workspace"
    );
    let expected_paths = json!([path_to_string(&fs::canonicalize(&workspace)?)]);
    anyhow::ensure!(
        fixture.mru_paths()? == expected_paths,
        "picker did not persist the selected workspace"
    );
    ensure_terminal_restored(&fixture)
}

#[test]
fn picker_marks_matching_zoxide_paths_as_active_workspaces() -> Result<()> {
    let _guard = serial_guard();
    let fixture = Fixture::new()?;
    let directory = fixture.install_zoxide_directory()?;
    fixture.write_snapshot(&worktree_snapshot("workspace", &directory))?;

    let output = run_picker(&fixture, b"\n")?;

    ensure_success(&output)?;
    anyhow::ensure!(
        String::from_utf8_lossy(&output.stdout)
            .contains("\x1b[7m\x1b[38;2;249;226;175m● [worktree]\x1b[39m project"),
        "picker did not mark the active worktree path"
    );
    anyhow::ensure!(
        fixture.log("herdr.log")? == "api snapshot\nworkspace focus workspace\n",
        "picker did not focus the active zoxide workspace"
    );
    let expected_paths = json!([path_to_string(&fs::canonicalize(&directory)?)]);
    anyhow::ensure!(
        fixture.mru_paths()? == expected_paths,
        "picker did not persist the active zoxide path"
    );
    ensure_terminal_restored(&fixture)
}

#[test]
fn picker_renders_regular_workspaces_with_the_workspace_tag() -> Result<()> {
    let _guard = serial_guard();
    let fixture = Fixture::new()?;
    let workspace = fixture.file("workspace");
    fs::create_dir(&workspace)?;
    fixture.write_snapshot(&workspace_snapshot(
        "workspace",
        "worktree-feature",
        &workspace,
    ))?;
    fixture.install_empty_zoxide()?;

    let output = run_picker(&fixture, b"\n")?;

    ensure_success(&output)?;
    anyhow::ensure!(
        String::from_utf8_lossy(&output.stdout)
            .contains("\x1b[7m\x1b[36m● [workspace]\x1b[39m worktree-feature"),
        "picker did not render the ordinary workspace tag and label"
    );
    anyhow::ensure!(
        fixture.log("herdr.log")? == "api snapshot\nworkspace focus workspace\n",
        "picker did not focus the ordinary workspace"
    );
    ensure_terminal_restored(&fixture)
}

#[test]
fn picker_uses_the_available_terminal_height() -> Result<()> {
    let _guard = serial_guard();
    let fixture = Fixture::new()?;
    fixture.write_snapshot(&empty_snapshot())?;
    fixture.install_zoxide_directories(20)?;

    let output = run_picker(&fixture, b"\x1b")?;

    ensure_success(&output)?;
    anyhow::ensure!(
        String::from_utf8_lossy(&output.stdout)
            .matches("[zoxide]")
            .count()
            == 18,
        "picker did not fill the available candidate rows"
    );
    ensure_terminal_restored(&fixture)
}

#[test]
fn picker_creates_a_workspace_for_a_zoxide_directory() -> Result<()> {
    let _guard = serial_guard();
    let fixture = Fixture::new()?;
    fixture.write_snapshot(&empty_snapshot())?;
    let directory = fixture.install_zoxide_directory()?;

    let output = run_picker(&fixture, b"\n")?;

    ensure_success(&output)?;
    let expected_command = format!(
        "api snapshot\napi snapshot\nworkspace create --cwd {} --focus\n",
        path_to_string(&directory)
    );
    anyhow::ensure!(
        fixture.log("herdr.log")? == expected_command,
        "picker did not create a workspace for the zoxide directory"
    );
    anyhow::ensure!(
        fixture.log("zoxide.log")? == "query -ls\n",
        "picker sent an unexpected zoxide command"
    );
    anyhow::ensure!(
        String::from_utf8_lossy(&output.stdout)
            .contains("\x1b[7m\x1b[35m  [zoxide]\x1b[39m zoxide-directory"),
        "picker did not render the selected zoxide badge"
    );
    let expected_paths = json!([path_to_string(&fs::canonicalize(&directory)?)]);
    anyhow::ensure!(
        fixture.mru_paths()? == expected_paths,
        "picker did not persist the selected zoxide directory"
    );
    ensure_terminal_restored(&fixture)
}

#[test]
fn picker_cancellation_has_no_workspace_side_effect() -> Result<()> {
    let _guard = serial_guard();
    let fixture = Fixture::new()?;
    let workspace = fixture.file("workspace");
    fs::create_dir(&workspace)?;
    fixture.write_snapshot(&worktree_snapshot("workspace", &workspace))?;
    fixture.install_empty_zoxide()?;

    let output = run_picker(&fixture, b"\x1b")?;

    ensure_success(&output)?;
    anyhow::ensure!(
        fixture.log("herdr.log")? == "api snapshot\n",
        "picker acted after cancellation"
    );
    anyhow::ensure!(
        !fixture.state_file().exists(),
        "picker persisted MRU state after cancellation"
    );
    ensure_terminal_restored(&fixture)
}

#[test]
fn picker_displays_herdr_failures_and_restores_the_terminal() -> Result<()> {
    let _guard = serial_guard();
    let fixture = Fixture::new()?;
    fixture.write_herdr_error("snapshot unavailable")?;

    let output = run_picker(&fixture, b"\x1b")?;

    anyhow::ensure!(!output.status.success(), "picker unexpectedly succeeded");
    ensure_terminal_line_endings(&output)?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    anyhow::ensure!(
        stdout.contains("Workspace error") && stdout.contains("snapshot unavailable"),
        "picker did not render the Herdr failure"
    );
    anyhow::ensure!(
        fixture.log("herdr.log")? == "api snapshot\n",
        "picker sent an unexpected Herdr command after failure"
    );
    ensure_terminal_restored(&fixture)
}
