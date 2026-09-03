//! Process-level tests for the plugin command contract.
#![cfg(unix)]

use std::{
    env,
    fmt::Write as _,
    fs::{self, File},
    io::{self, Read, Write},
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::{
        LazyLock, Mutex, MutexGuard,
        atomic::{AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use rustix::{
    fs::{Mode, OFlags, fcntl_setfl},
    io::Errno,
    pty::{OpenptFlags, grantpt, openpt, ptsname, unlockpt},
    termios::{
        ControlModes, InputModes, LocalModes, OutputModes, SpecialCodeIndex, Winsize, tcgetattr,
        tcsetwinsize,
    },
};
use serde_json::{Value, json};

use StepAction::{Do, Key};

const PICKER_TIMEOUT: Duration = Duration::from_secs(10);
const CLEAR_SCREEN: &str = "\x1b[2J\x1b[H";

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

/// Blocks the zoxide answer until the test creates the release file.
const ZOXIDE_BLOCKED_DIRECTORY_SCRIPT: &str = r#"#!/bin/sh
base=${0%/*}
printf '%s\n' "$*" >> "$base/zoxide.log"

case "$1:$2" in
query:-ls)
    while [ ! -f "$base/release" ]; do
        sleep 0.02
    done
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

/// The fixture directory is `HOME`, so displayed paths are `~/bin/...` on every machine. An empty
/// zoxide fake is installed by default so no test reaches the real zoxide.
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
        write_executable(&fixture.bin_dir(), "zoxide", ZOXIDE_EMPTY_SCRIPT)?;
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

    fn command(&self, action: &str) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_herdr-workspacer"));
        command
            .arg(action)
            .env("HOME", &self.directory.path)
            .env("HERDR_BIN_PATH", self.herdr_path())
            .env("HERDR_PLUGIN_STATE_DIR", self.state_dir())
            .env("HERDR_WORKSPACER_ZOXIDE_PATH", self.file("zoxide"));
        command
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

    fn install_zoxide_directory(&self) -> Result<PathBuf> {
        let directory = self.file("zoxide-directory");
        fs::create_dir(&directory)?;
        write_executable(&self.bin_dir(), "zoxide", ZOXIDE_DIRECTORY_SCRIPT)?;
        Ok(directory)
    }

    fn install_blocked_zoxide_directory(&self) -> Result<PathBuf> {
        let directory = self.file("zoxide-directory");
        fs::create_dir(&directory)?;
        write_executable(&self.bin_dir(), "zoxide", ZOXIDE_BLOCKED_DIRECTORY_SCRIPT)?;
        Ok(directory)
    }

    fn release_zoxide(&self) -> Result<()> {
        fs::write(self.file("release"), b"")?;
        Ok(())
    }

    /// Installs directories with descending zoxide scores in the given order.
    fn install_zoxide_directories(&self, names: &[&str]) -> Result<()> {
        let mut script = String::from(
            "#!/bin/sh\nbase=${0%/*}\nprintf '%s\\n' \"$*\" >> \"$base/zoxide.log\"\n\n\
             case \"$1:$2\" in\nquery:-ls)\n",
        );
        for (index, name) in names.iter().enumerate() {
            fs::create_dir(self.file(name))?;
            writeln!(
                &mut script,
                "    printf '{}\\t%s/{name}\\n' \"$base\"",
                names.len().saturating_sub(index)
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

struct PickerRun {
    output: Output,
    /// Raw mode was active when the first scripted step ran.
    raw_mode: bool,
    initial_modes: TerminalModes,
    final_modes: TerminalModes,
}

impl PickerRun {
    fn stdout(&self) -> String {
        String::from_utf8_lossy(&self.output.stdout).into_owned()
    }

    /// The output after the last screen clear.
    fn last_frame(&self) -> String {
        let stdout = self.stdout();
        stdout
            .rsplit(CLEAR_SCREEN)
            .next()
            .unwrap_or_default()
            .to_string()
    }
}

enum StepAction<'a> {
    Key(&'a [u8]),
    /// Test-side code, for example to unblock a fake binary.
    Do(&'a dyn Fn() -> Result<()>),
}

/// Wait until the output satisfies the predicate, then run the action.
type Step<'a> = (&'a dyn Fn(&str) -> bool, StepAction<'a>);

#[derive(Debug, Eq, PartialEq)]
struct TerminalModes {
    input: InputModes,
    output: OutputModes,
    control: ControlModes,
    local: LocalModes,
    min: u8,
    time: u8,
}

impl TerminalModes {
    fn read(terminal: &File) -> Result<Self> {
        let state = tcgetattr(terminal)?;
        Ok(Self {
            input: state.input_modes,
            output: state.output_modes,
            control: state.control_modes,
            // The kernel sets PENDIN by itself when canonical mode returns with unread input.
            local: state.local_modes.difference(LocalModes::PENDIN),
            min: state.special_codes[SpecialCodeIndex::VMIN],
            time: state.special_codes[SpecialCodeIndex::VTIME],
        })
    }

    fn is_raw(&self) -> bool {
        !self.local.intersects(LocalModes::ICANON | LocalModes::ECHO)
            && !self.output.contains(OutputModes::OPOST)
            && self.min == 0
            && self.time == 1
    }
}

struct Terminal {
    master: File,
    slave: File,
}

impl Terminal {
    fn open() -> Result<Self> {
        let master = openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY)?;
        grantpt(&master)?;
        unlockpt(&master)?;
        let name = ptsname(&master, Vec::new())?;
        let slave = rustix::fs::open(
            name.as_c_str(),
            OFlags::RDWR | OFlags::NOCTTY,
            Mode::empty(),
        )?;
        tcsetwinsize(
            &slave,
            Winsize {
                ws_row: 24,
                ws_col: 80,
                ws_xpixel: 0,
                ws_ypixel: 0,
            },
        )?;
        fcntl_setfl(&master, OFlags::NONBLOCK)?;
        Ok(Self {
            master: File::from(master),
            slave: File::from(slave),
        })
    }

    fn read_available(&mut self, output: &mut Vec<u8>) -> Result<()> {
        let mut buffer = [0; 4096];
        loop {
            match self.master.read(&mut buffer) {
                Ok(0) => return Ok(()),
                Ok(count) => {
                    output.extend_from_slice(buffer.get(..count).context("short read")?);
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(error) if error.raw_os_error() == Some(Errno::IO.raw_os_error()) => {
                    return Ok(());
                }
                Err(error) => return Err(error.into()),
            }
        }
    }
}

fn run_picker(fixture: &Fixture, steps: &[Step<'_>]) -> Result<PickerRun> {
    let mut terminal = Terminal::open()?;
    let initial_modes = TerminalModes::read(&terminal.slave)?;
    let mut child = fixture
        .command("picker")
        .stdin(Stdio::from(terminal.slave.try_clone()?))
        .stdout(Stdio::from(terminal.slave.try_clone()?))
        .stderr(Stdio::piped())
        .spawn()?;

    let started = Instant::now();
    let mut stdout = Vec::new();
    let mut raw_mode = None;
    let mut steps = steps.iter();
    let mut step = steps.next();
    let status = loop {
        terminal.read_available(&mut stdout)?;
        if let Some((ready, action)) = step
            && ready(&String::from_utf8_lossy(&stdout))
        {
            if raw_mode.is_none() {
                raw_mode = Some(TerminalModes::read(&terminal.slave)?.is_raw());
            }
            match action {
                Key(input) => terminal.master.write_all(input)?,
                Do(action) => action()?,
            }
            step = steps.next();
        }
        if let Some(status) = child.try_wait()? {
            terminal.read_available(&mut stdout)?;
            break status;
        }
        if started.elapsed() > PICKER_TIMEOUT {
            let _ = child.kill();
            anyhow::bail!(
                "picker did not finish in time\nstdout: {}",
                String::from_utf8_lossy(&stdout)
            );
        }
        thread::sleep(Duration::from_millis(5));
    };

    let mut stderr = Vec::new();
    if let Some(mut pipe) = child.stderr.take() {
        pipe.read_to_end(&mut stderr)?;
    }
    let final_modes = TerminalModes::read(&terminal.slave)?;
    Ok(PickerRun {
        output: Output {
            status,
            stdout,
            stderr,
        },
        raw_mode: raw_mode.unwrap_or(false),
        initial_modes,
        final_modes,
    })
}

fn shows(text: &'static str) -> impl Fn(&str) -> bool {
    move |output| output.contains(text)
}

fn selected_worktree_row(label: &str) -> String {
    format!("\x1b[7m\x1b[38;2;249;226;175m● [worktree]\x1b[39m {label}")
}

fn selected_workspace_row(label: &str) -> String {
    format!("\x1b[7m\x1b[36m● [workspace]\x1b[39m {label}")
}

fn selected_zoxide_row(label: &str) -> String {
    format!("\x1b[7m\x1b[35m  [zoxide]\x1b[39m {label}")
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

fn ensure_terminal_restored(run: &PickerRun) -> Result<()> {
    anyhow::ensure!(
        run.raw_mode,
        "picker did not switch the terminal to raw mode before reading keys"
    );
    anyhow::ensure!(
        run.final_modes == run.initial_modes,
        "terminal state was not restored\nbefore: {:?}\nafter: {:?}",
        run.initial_modes,
        run.final_modes
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

    let output = fixture.command("open").output()?;

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
        .command("record-focus")
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

    let run = run_picker(
        &fixture,
        &[(
            &shows("zoxide query failed. Showing open workspaces only."),
            Key(b"\n"),
        )],
    )?;

    ensure_success(&run.output)?;
    ensure_terminal_line_endings(&run.output)?;
    ensure_picker_layout(&run.output)?;
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
    ensure_terminal_restored(&run)
}

#[test]
fn picker_marks_matching_zoxide_paths_as_active_workspaces() -> Result<()> {
    let _guard = serial_guard();
    let fixture = Fixture::new()?;
    let directory = fixture.install_zoxide_directory()?;
    fixture.write_snapshot(&worktree_snapshot("workspace", &directory))?;

    let run = run_picker(&fixture, &[(&shows("[worktree]"), Key(b"\n"))])?;

    ensure_success(&run.output)?;
    let frame = run.last_frame();
    anyhow::ensure!(
        frame.contains(&selected_worktree_row("project")) && !frame.contains("[zoxide]"),
        "picker did not replace the zoxide row with the active worktree: {frame:?}"
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
    ensure_terminal_restored(&run)
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

    let run = run_picker(&fixture, &[(&shows("[workspace]"), Key(b"\n"))])?;

    ensure_success(&run.output)?;
    anyhow::ensure!(
        run.last_frame()
            .contains(&selected_workspace_row("worktree-feature")),
        "picker did not render the ordinary workspace tag and label"
    );
    anyhow::ensure!(
        fixture.log("herdr.log")? == "api snapshot\nworkspace focus workspace\n",
        "picker did not focus the ordinary workspace"
    );
    ensure_terminal_restored(&run)
}

#[test]
fn picker_fills_the_terminal_height_and_pages_to_the_last_row() -> Result<()> {
    let _guard = serial_guard();
    let fixture = Fixture::new()?;
    fixture.write_snapshot(&empty_snapshot())?;
    let names = (0..20)
        .map(|index| format!("zoxide-{index}"))
        .collect::<Vec<_>>();
    fixture.install_zoxide_directories(&names.iter().map(String::as_str).collect::<Vec<_>>())?;

    let run = run_picker(
        &fixture,
        &[
            (
                &|output| output.matches("[zoxide]").count() >= 18,
                Key(b"\x1b[6~\x1b[6~"),
            ),
            (
                &|output| output.contains(&selected_zoxide_row("zoxide-19")),
                Key(b"\n"),
            ),
        ],
    )?;

    ensure_success(&run.output)?;
    let frame = run.last_frame();
    anyhow::ensure!(
        frame.matches("[zoxide]").count() == 18,
        "picker did not fill the available candidate rows"
    );
    anyhow::ensure!(
        frame.contains(&selected_zoxide_row("zoxide-19")) && !frame.contains("zoxide-0 "),
        "two page downs did not stop at the last row: {frame:?}"
    );
    let expected_command = format!(
        "api snapshot\napi snapshot\nworkspace create --cwd {} --focus\n",
        path_to_string(&fixture.file("zoxide-19"))
    );
    anyhow::ensure!(
        fixture.log("herdr.log")? == expected_command,
        "picker did not create a workspace for the last row"
    );
    ensure_terminal_restored(&run)
}

#[test]
fn picker_filters_by_typed_query_and_shows_home_relative_paths() -> Result<()> {
    let _guard = serial_guard();
    let fixture = Fixture::new()?;
    fixture.write_snapshot(&empty_snapshot())?;
    fixture.install_zoxide_directories(&["alpha", "bravo", "charlie"])?;

    let run = run_picker(
        &fixture,
        &[
            (&shows("charlie"), Key(b"brav")),
            (&shows("Type to filter\x1b[0m  brav\r\n"), Key(b"\n")),
        ],
    )?;

    ensure_success(&run.output)?;
    let frame = run.last_frame();
    anyhow::ensure!(
        frame.matches("[zoxide]").count() == 1
            && frame.contains(&format!("{}  ~/bin/bravo", selected_zoxide_row("bravo"))),
        "picker did not show only the matching row with a home-relative path: {frame:?}"
    );
    let expected_command = format!(
        "api snapshot\napi snapshot\nworkspace create --cwd {} --focus\n",
        path_to_string(&fixture.file("bravo"))
    );
    anyhow::ensure!(
        fixture.log("herdr.log")? == expected_command,
        "picker did not create a workspace for the filtered row"
    );
    ensure_terminal_restored(&run)
}

#[test]
fn picker_select_without_matches_has_no_side_effect() -> Result<()> {
    let _guard = serial_guard();
    let fixture = Fixture::new()?;
    fixture.write_snapshot(&empty_snapshot())?;
    fixture.install_zoxide_directories(&["alpha", "bravo"])?;

    let run = run_picker(
        &fixture,
        &[
            (&shows("bravo"), Key(b"zzzz")),
            (&shows("No matching workspaces"), Key(b"\n")),
        ],
    )?;

    ensure_success(&run.output)?;
    anyhow::ensure!(
        fixture.log("herdr.log")? == "api snapshot\n",
        "picker acted on an empty result list"
    );
    anyhow::ensure!(
        !fixture.state_file().exists(),
        "picker persisted MRU state without a selection"
    );
    ensure_terminal_restored(&run)
}

#[test]
fn picker_creates_a_workspace_for_a_zoxide_directory() -> Result<()> {
    let _guard = serial_guard();
    let fixture = Fixture::new()?;
    fixture.write_snapshot(&empty_snapshot())?;
    let directory = fixture.install_zoxide_directory()?;

    let run = run_picker(&fixture, &[(&shows("[zoxide]"), Key(b"\n"))])?;

    ensure_success(&run.output)?;
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
        run.last_frame()
            .contains(&selected_zoxide_row("zoxide-directory")),
        "picker did not render the selected zoxide badge"
    );
    let expected_paths = json!([path_to_string(&fs::canonicalize(&directory)?)]);
    anyhow::ensure!(
        fixture.mru_paths()? == expected_paths,
        "picker did not persist the selected zoxide directory"
    );
    ensure_terminal_restored(&run)
}

#[test]
fn picker_shows_open_workspaces_before_zoxide_finishes() -> Result<()> {
    let _guard = serial_guard();
    let fixture = Fixture::new()?;
    let workspace = fixture.file("workspace");
    fs::create_dir(&workspace)?;
    fixture.write_snapshot(&worktree_snapshot("workspace", &workspace))?;
    fixture.install_blocked_zoxide_directory()?;

    let run = run_picker(
        &fixture,
        &[
            (&shows("[worktree]"), Do(&|| fixture.release_zoxide())),
            (&shows("[zoxide]"), Key(b"\n")),
        ],
    )?;

    ensure_success(&run.output)?;
    let stdout = run.stdout();
    anyhow::ensure!(
        stdout.find("[worktree]") < stdout.find("[zoxide]"),
        "picker did not render the open workspace before the zoxide directory"
    );
    let frame = run.last_frame();
    anyhow::ensure!(
        frame.contains(&selected_worktree_row("project"))
            && frame.contains("\x1b[35m  [zoxide]\x1b[39m zoxide-directory"),
        "picker did not keep the workspace selected after adding the zoxide directory: {frame:?}"
    );
    anyhow::ensure!(
        fixture.log("herdr.log")? == "api snapshot\nworkspace focus workspace\n",
        "picker did not focus the workspace that stayed selected"
    );
    ensure_terminal_restored(&run)
}

#[test]
fn picker_cancellation_has_no_workspace_side_effect() -> Result<()> {
    let _guard = serial_guard();
    let fixture = Fixture::new()?;
    let workspace = fixture.file("workspace");
    fs::create_dir(&workspace)?;
    fixture.write_snapshot(&worktree_snapshot("workspace", &workspace))?;

    let run = run_picker(&fixture, &[(&shows("Find workspace"), Key(b"\x1b"))])?;

    ensure_success(&run.output)?;
    anyhow::ensure!(
        fixture.log("herdr.log")? == "api snapshot\n",
        "picker acted after cancellation"
    );
    anyhow::ensure!(
        !fixture.state_file().exists(),
        "picker persisted MRU state after cancellation"
    );
    ensure_terminal_restored(&run)
}

#[test]
fn picker_displays_herdr_failures_and_restores_the_terminal() -> Result<()> {
    let _guard = serial_guard();
    let fixture = Fixture::new()?;
    fixture.write_herdr_error("snapshot unavailable")?;

    let run = run_picker(&fixture, &[(&shows("Workspace error"), Key(b"\x1b"))])?;

    anyhow::ensure!(
        !run.output.status.success(),
        "picker unexpectedly succeeded"
    );
    ensure_terminal_line_endings(&run.output)?;
    let stdout = run.stdout();
    anyhow::ensure!(
        stdout.contains("Workspace error") && stdout.contains("snapshot unavailable"),
        "picker did not render the Herdr failure"
    );
    anyhow::ensure!(
        fixture.log("herdr.log")? == "api snapshot\n",
        "picker sent an unexpected Herdr command after failure"
    );
    ensure_terminal_restored(&run)
}
