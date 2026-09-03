use std::io::{self, Read, Write};

use anyhow::{Context, Result};
use rustix::termios::{
    OptionalActions, SpecialCodeIndex, Termios, tcgetattr, tcgetwinsize, tcsetattr,
};

use crate::app::PickerModel;
use herdr_workspacer::Candidate;

const DEFAULT_VISIBLE_ROWS: usize = 10;
const LAYOUT_ROWS: usize = 6;
const CONTENT_INDENT: &str = "  ";
const WORKSPACE_COLOR: &str = "\x1b[36m";
const WORKTREE_COLOR: &str = "\x1b[38;2;249;226;175m";
const ZOXIDE_COLOR: &str = "\x1b[35m";
const RESET_FOREGROUND: &str = "\x1b[39m";

pub(crate) enum PickerOutcome {
    Cancelled,
    Selected(usize),
}

/// `apply_updates` runs while no key is pending and returns whether the model changed.
pub(crate) fn run(
    model: &mut PickerModel,
    apply_updates: impl FnMut(&mut PickerModel) -> Result<bool>,
) -> Result<PickerOutcome> {
    let mut terminal = TerminalSession::enter()?;
    let result = terminal.run(model, apply_updates);
    let restore_result = terminal.restore();

    match result {
        Ok(outcome) => {
            restore_result?;
            Ok(outcome)
        }
        Err(error) => Err(error),
    }
}

pub(crate) fn show_error(message: &str) -> Result<()> {
    let mut terminal = TerminalSession::enter()?;
    let result = terminal.run_error(message);
    let restore_result = terminal.restore();

    result?;
    restore_result?;
    Ok(())
}

struct TerminalSession {
    stdout: io::Stdout,
    saved_state: Termios,
    visible_rows: usize,
    active: bool,
}

impl TerminalSession {
    fn enter() -> Result<Self> {
        let visible_rows = terminal_rows()
            .unwrap_or(DEFAULT_VISIBLE_ROWS)
            .saturating_sub(LAYOUT_ROWS)
            .max(1);
        let stdin = io::stdin();
        let saved_state = tcgetattr(&stdin).context("could not read terminal state")?;

        // Reads time out after 100 ms so background updates can be applied.
        let mut raw_state = saved_state.clone();
        raw_state.make_raw();
        raw_state.special_codes[SpecialCodeIndex::VMIN] = 0;
        raw_state.special_codes[SpecialCodeIndex::VTIME] = 1;
        tcsetattr(&stdin, OptionalActions::Now, &raw_state)
            .context("could not switch the terminal to raw mode")?;

        let mut stdout = io::stdout();
        if let Err(error) = write!(stdout, "\x1b[?1049h\x1b[?25l").and_then(|()| stdout.flush()) {
            let _ = write!(stdout, "\x1b[?25h\x1b[?1049l");
            let _ = tcsetattr(&stdin, OptionalActions::Now, &saved_state);
            return Err(error.into());
        }

        Ok(Self {
            stdout,
            saved_state,
            visible_rows,
            active: true,
        })
    }

    fn run(
        &mut self,
        model: &mut PickerModel,
        mut apply_updates: impl FnMut(&mut PickerModel) -> Result<bool>,
    ) -> Result<PickerOutcome> {
        let stdin = io::stdin();
        let mut input = stdin.lock();
        let page_size = isize::try_from(self.visible_rows).unwrap_or(isize::MAX);

        render_picker(&mut self.stdout, model, self.visible_rows)?;
        loop {
            let Some(key) = read_key(&mut input)? else {
                if apply_updates(model)? {
                    render_picker(&mut self.stdout, model, self.visible_rows)?;
                }
                continue;
            };
            match key {
                Key::Cancel => return Ok(PickerOutcome::Cancelled),
                Key::Select => {
                    return Ok(model
                        .selected_candidate_index()
                        .map_or(PickerOutcome::Cancelled, PickerOutcome::Selected));
                }
                Key::Up => model.move_selection(-1),
                Key::Down => model.move_selection(1),
                Key::PageUp => model.move_selection(-page_size),
                Key::PageDown => model.move_selection(page_size),
                Key::Backspace => model.backspace(),
                Key::Clear => model.clear_query(),
                Key::Character(character) => model.push_query_character(character),
                Key::Ignore => {}
            }
            render_picker(&mut self.stdout, model, self.visible_rows)?;
        }
    }

    fn run_error(&mut self, message: &str) -> Result<()> {
        let stdin = io::stdin();
        let mut input = stdin.lock();

        render_error(&mut self.stdout, message)?;
        loop {
            if matches!(read_key(&mut input)?, Some(Key::Cancel | Key::Select)) {
                return Ok(());
            }
        }
    }

    fn restore(&mut self) -> Result<()> {
        if !self.active {
            return Ok(());
        }
        self.active = false;

        let terminal_result =
            write!(self.stdout, "\x1b[?25h\x1b[?1049l").and_then(|()| self.stdout.flush());
        let state_result = tcsetattr(io::stdin(), OptionalActions::Now, &self.saved_state)
            .context("could not restore terminal state");
        terminal_result?;
        state_result
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        if self.active {
            let _ = self.restore();
        }
    }
}

fn render_picker(stdout: &mut io::Stdout, model: &PickerModel, visible_rows: usize) -> Result<()> {
    write!(
        stdout,
        "\x1b[2J\x1b[H{CONTENT_INDENT}\x1b[1mFind workspace\x1b[0m\r\n\
         {CONTENT_INDENT}\x1b[2mType to filter\x1b[0m  {}\r\n\r\n",
        model.query()
    )?;

    let visible = model.visible();
    let start = model
        .selected()
        .saturating_sub(visible_rows >> 1)
        .min(visible.len().saturating_sub(visible_rows));
    let end = start.saturating_add(visible_rows).min(visible.len());

    if visible.is_empty() {
        write!(
            stdout,
            "{CONTENT_INDENT}\x1b[2mNo matching workspaces\x1b[0m\r\n"
        )?;
    } else if let Some(rows) = visible.get(start..end) {
        for (offset, index) in rows.iter().enumerate() {
            if let Some(candidate) = model.candidate(*index) {
                write!(stdout, "{CONTENT_INDENT}")?;
                if model.selected() == start.saturating_add(offset) {
                    write!(stdout, "\x1b[7m")?;
                }
                write_candidate(stdout, candidate)?;
                if model.selected() == start.saturating_add(offset) {
                    write!(stdout, "\x1b[0m")?;
                }
                write!(stdout, "\r\n")?;
            }
        }
    }

    write!(stdout, "\r\n")?;
    if let Some(warning) = model.warning() {
        write!(stdout, "{CONTENT_INDENT}\x1b[33m{warning}\x1b[0m\r\n")?;
    }
    write!(
        stdout,
        "{CONTENT_INDENT}\x1b[2mEnter select  Esc cancel  ↑/↓ move  PgUp/PgDn jump\x1b[0m\r\n"
    )?;
    stdout.flush()?;
    Ok(())
}

fn render_error(stdout: &mut io::Stdout, message: &str) -> Result<()> {
    write!(
        stdout,
        "\x1b[2J\x1b[H\x1b[1mWorkspace error\x1b[0m\r\n\r\n{}\r\n\r\nEnter or Esc closes this popup.",
        safe_terminal_text(message)
    )?;
    stdout.flush()?;
    Ok(())
}

fn write_candidate(stdout: &mut impl Write, candidate: &Candidate) -> io::Result<()> {
    let (color, marker, source) = if candidate.is_worktree() {
        (WORKTREE_COLOR, "●", "worktree")
    } else if candidate.is_workspace() {
        (WORKSPACE_COLOR, "●", "workspace")
    } else {
        (ZOXIDE_COLOR, " ", "zoxide")
    };
    write!(
        stdout,
        "{color}{marker} [{source}]{RESET_FOREGROUND} {}  {}",
        candidate.label, candidate.display_path
    )
}

fn terminal_rows() -> Option<usize> {
    let size = tcgetwinsize(io::stdout()).ok()?;
    (size.ws_row > 0).then(|| usize::from(size.ws_row))
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Key {
    Cancel,
    Select,
    Up,
    Down,
    PageUp,
    PageDown,
    Backspace,
    Clear,
    Character(char),
    Ignore,
}

/// `None` means the terminal read timed out.
fn read_key(input: &mut impl Read) -> io::Result<Option<Key>> {
    let Some(byte) = read_optional_byte(input)? else {
        return Ok(None);
    };
    match byte {
        b'\x03' => Ok(Key::Cancel),
        b'\x1b' => read_escape(input),
        b'\r' | b'\n' => Ok(Key::Select),
        b'\x7f' | b'\x08' => Ok(Key::Backspace),
        b'\x10' => Ok(Key::Up),
        b'\x0e' => Ok(Key::Down),
        b'\x15' => Ok(Key::Clear),
        byte if byte.is_ascii_control() => Ok(Key::Ignore),
        byte if byte.is_ascii() => Ok(Key::Character(char::from(byte))),
        byte => read_utf8_character(input, byte),
    }
    .map(Some)
}

fn read_escape(input: &mut impl Read) -> io::Result<Key> {
    let Some(second) = read_optional_byte(input)? else {
        return Ok(Key::Cancel);
    };
    if second != b'[' {
        return Ok(Key::Cancel);
    }

    match read_byte(input)? {
        b'A' => Ok(Key::Up),
        b'B' => Ok(Key::Down),
        b'5' => {
            let _ = read_byte(input)?;
            Ok(Key::PageUp)
        }
        b'6' => {
            let _ = read_byte(input)?;
            Ok(Key::PageDown)
        }
        _ => Ok(Key::Ignore),
    }
}

fn read_utf8_character(input: &mut impl Read, first: u8) -> io::Result<Key> {
    let width = match first {
        0b1100_0000..=0b1101_1111 => 2,
        0b1110_0000..=0b1110_1111 => 3,
        0b1111_0000..=0b1111_0111 => 4,
        _ => return Ok(Key::Ignore),
    };
    let mut bytes = vec![first];
    while bytes.len() < width {
        bytes.push(read_byte(input)?);
    }

    Ok(std::str::from_utf8(&bytes)
        .ok()
        .and_then(|value| value.chars().next())
        .map_or(Key::Ignore, Key::Character))
}

fn read_byte(input: &mut impl Read) -> io::Result<u8> {
    loop {
        if let Some(byte) = read_optional_byte(input)? {
            return Ok(byte);
        }
    }
}

fn read_optional_byte(input: &mut impl Read) -> io::Result<Option<u8>> {
    let mut byte = [0];
    match input.read(&mut byte)? {
        0 => Ok(None),
        _ => Ok(Some(byte[0])),
    }
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

    #[test]
    fn renders_worktrees_with_a_yellow_tag() -> anyhow::Result<()> {
        let candidate = herdr_workspacer::merge_candidates(
            vec![herdr_workspacer::Workspace {
                id: "worktree-feature".to_string(),
                label: "worktree-feature".to_string(),
                is_worktree: true,
                path: std::env::temp_dir(),
                native_order: 0,
            }],
            Vec::new(),
            &herdr_workspacer::MruState::default(),
        )?
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("missing worktree candidate"))?;
        let mut output = Vec::new();

        write_candidate(&mut output, &candidate)?;
        let actual = String::from_utf8(output)?;
        anyhow::ensure!(
            actual
                == format!(
                    "\x1b[38;2;249;226;175m● [worktree]\x1b[39m feature  {}",
                    candidate.display_path
                ),
            "worktree row did not use its yellow tag and trimmed name: {actual:?}"
        );
        Ok(())
    }
    use std::io::Cursor;

    use super::*;

    #[test]
    fn parses_documented_controls() -> anyhow::Result<()> {
        let cases = [
            (&b"a"[..], Key::Character('a')),
            (&b"\x7f"[..], Key::Backspace),
            (&b"\x08"[..], Key::Backspace),
            (&b"\x1b[A"[..], Key::Up),
            (&b"\x10"[..], Key::Up),
            (&b"\x1b[B"[..], Key::Down),
            (&b"\x0e"[..], Key::Down),
            (&b"\x1b[5~"[..], Key::PageUp),
            (&b"\x1b[6~"[..], Key::PageDown),
            (&b"\x15"[..], Key::Clear),
            (&b"\r"[..], Key::Select),
            (&b"\n"[..], Key::Select),
            (&b"\x1b"[..], Key::Cancel),
            (&b"\x03"[..], Key::Cancel),
        ];

        for (input, expected) in cases {
            let actual = read_key(&mut Cursor::new(input))?;
            anyhow::ensure!(
                actual == Some(expected.clone()),
                "expected {expected:?}, got {actual:?}"
            );
        }
        let actual = read_key(&mut Cursor::new("プロ".as_bytes()))?;
        anyhow::ensure!(
            actual == Some(Key::Character('プ')),
            "expected a Unicode character, got {actual:?}"
        );
        Ok(())
    }

    #[test]
    fn reports_a_timed_out_read_without_input() -> anyhow::Result<()> {
        let actual = read_key(&mut Cursor::new(b""))?;
        anyhow::ensure!(actual.is_none(), "expected no key, got {actual:?}");
        Ok(())
    }

    #[test]
    fn ignores_malformed_or_unknown_input() -> anyhow::Result<()> {
        for input in [&b"\x1b[Z"[..], &b"\xe3\x28\x28"[..], &b"\xff"[..]] {
            let actual = read_key(&mut Cursor::new(input))?;
            anyhow::ensure!(
                actual == Some(Key::Ignore),
                "expected ignored input, got {actual:?}"
            );
        }
        Ok(())
    }
}
