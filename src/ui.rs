use std::{
    io::{self, Read, Write},
    process::{Command, Stdio},
};

use anyhow::{Context, Result, bail};

use crate::app::PickerModel;
use herdr_workspacer::Candidate;

const VISIBLE_ROWS: usize = 10;

pub(crate) enum PickerOutcome {
    Cancelled,
    Selected(usize),
}

pub(crate) fn run(model: &mut PickerModel) -> Result<PickerOutcome> {
    let mut terminal = TerminalSession::enter()?;
    let result = terminal.run(model);
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
    stty_state: String,
    active: bool,
}

impl TerminalSession {
    fn enter() -> Result<Self> {
        let stty_state = stty_output(["-g"])?;
        let stty_state = String::from_utf8_lossy(&stty_state.stdout)
            .trim()
            .to_string();
        if stty_state.is_empty() {
            bail!("could not read terminal state");
        }

        run_stty(["raw", "-echo", "min", "0", "time", "1"])?;
        let mut stdout = io::stdout();
        if let Err(error) = write!(stdout, "\x1b[?1049h\x1b[?25l").and_then(|()| stdout.flush()) {
            let _ = write!(stdout, "\x1b[?25h\x1b[?1049l");
            let _ = run_stty([stty_state.as_str()]);
            return Err(error.into());
        }

        Ok(Self {
            stdout,
            stty_state,
            active: true,
        })
    }

    fn run(&mut self, model: &mut PickerModel) -> Result<PickerOutcome> {
        let stdin = io::stdin();
        let mut input = stdin.lock();

        loop {
            render_picker(&mut self.stdout, model)?;
            match read_key(&mut input)? {
                Key::Cancel => return Ok(PickerOutcome::Cancelled),
                Key::Select => {
                    return Ok(model
                        .selected_candidate_index()
                        .map_or(PickerOutcome::Cancelled, PickerOutcome::Selected));
                }
                Key::Up => model.move_selection(-1),
                Key::Down => model.move_selection(1),
                Key::PageUp => model.move_selection(-10),
                Key::PageDown => model.move_selection(10),
                Key::Backspace => model.backspace(),
                Key::Clear => model.clear_query(),
                Key::Character(character) => model.push_query_character(character),
                Key::Ignore => {}
            }
        }
    }

    fn run_error(&mut self, message: &str) -> Result<()> {
        let stdin = io::stdin();
        let mut input = stdin.lock();

        loop {
            render_error(&mut self.stdout, message)?;
            if matches!(read_key(&mut input)?, Key::Cancel | Key::Select) {
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
        let stty_result = run_stty([self.stty_state.as_str()]);
        terminal_result?;
        stty_result
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        if self.active {
            let _ = self.restore();
        }
    }
}

fn render_picker(stdout: &mut io::Stdout, model: &PickerModel) -> Result<()> {
    write!(
        stdout,
        "\x1b[2J\x1b[H\x1b[1mWorkspace\x1b[0m\nSearch: {}\n\n",
        model.query()
    )?;

    let visible = model.visible();
    let start = model
        .selected()
        .saturating_sub(VISIBLE_ROWS >> 1)
        .min(visible.len().saturating_sub(VISIBLE_ROWS));
    let end = start.saturating_add(VISIBLE_ROWS).min(visible.len());

    if visible.is_empty() {
        writeln!(stdout, "No matching workspaces.")?;
    } else if let Some(rows) = visible.get(start..end) {
        for (offset, index) in rows.iter().enumerate() {
            if let Some(candidate) = model.candidate(*index) {
                if model.selected() == start.saturating_add(offset) {
                    write!(stdout, "\x1b[7m")?;
                }
                write_candidate(stdout, candidate)?;
                if model.selected() == start.saturating_add(offset) {
                    write!(stdout, "\x1b[0m")?;
                }
                writeln!(stdout)?;
            }
        }
    }

    writeln!(stdout)?;
    writeln!(
        stdout,
        "{}",
        model
            .warning()
            .unwrap_or("Enter select  Esc cancel  Up/Down move")
    )?;
    stdout.flush()?;
    Ok(())
}

fn render_error(stdout: &mut io::Stdout, message: &str) -> Result<()> {
    write!(
        stdout,
        "\x1b[2J\x1b[H\x1b[1mWorkspace error\x1b[0m\n\n{}\n\nEnter or Esc closes this popup.",
        safe_terminal_text(message)
    )?;
    stdout.flush()?;
    Ok(())
}

fn write_candidate(stdout: &mut io::Stdout, candidate: &Candidate) -> io::Result<()> {
    let marker = if candidate.is_workspace() { "●" } else { " " };
    write!(
        stdout,
        "{} {}  {}",
        marker, candidate.label, candidate.display_path
    )
}

fn stty_output<const N: usize>(arguments: [&str; N]) -> Result<std::process::Output> {
    let output = Command::new("stty")
        .args(arguments)
        .stdin(Stdio::inherit())
        .output()
        .context("could not run stty")?;
    if output.status.success() {
        Ok(output)
    } else {
        bail!("stty exited with {}", output.status)
    }
}

fn run_stty<const N: usize>(arguments: [&str; N]) -> Result<()> {
    stty_output(arguments).map(|_| ())
}

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

fn read_key(input: &mut impl Read) -> io::Result<Key> {
    let byte = read_byte(input)?;
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
