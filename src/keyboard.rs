use std::{
    io::{self, IsTerminal, Read},
    sync::mpsc::{self, Receiver},
    thread,
};

use anyhow::{Context, Result};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KeyCommand {
    ShowLatestTutorText,
    RepeatCurrentPhrase,
    Quit,
}

pub(crate) struct KeyboardListener {
    commands: Option<Receiver<KeyCommand>>,
    raw_mode_enabled: bool,
}

impl KeyboardListener {
    pub(crate) fn start() -> Result<Self> {
        if !io::stdin().is_terminal() {
            return Ok(Self {
                commands: None,
                raw_mode_enabled: false,
            });
        }
        enable_raw_mode().context("Could not enable single-key controls")?;
        let (sender, commands) = mpsc::channel();
        thread::spawn(move || {
            let mut input = io::stdin();
            let mut byte = [0_u8; 1];
            while input.read_exact(&mut byte).is_ok() {
                if let Some(command) = command_from_byte(byte[0]) {
                    let _ = sender.send(command);
                }
            }
        });
        Ok(Self {
            commands: Some(commands),
            raw_mode_enabled: true,
        })
    }

    pub(crate) fn try_next(&self) -> Option<KeyCommand> {
        self.commands
            .as_ref()
            .and_then(|commands| commands.try_recv().ok())
    }
}

impl Drop for KeyboardListener {
    fn drop(&mut self) {
        if self.raw_mode_enabled {
            let _ = disable_raw_mode();
        }
    }
}

fn command_from_byte(byte: u8) -> Option<KeyCommand> {
    match byte {
        b's' | b'S' => Some(KeyCommand::ShowLatestTutorText),
        b'w' | b'W' => Some(KeyCommand::RepeatCurrentPhrase),
        b'q' | b'Q' | 3 => Some(KeyCommand::Quit),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{KeyCommand, command_from_byte};

    #[test]
    fn maps_show_key_without_requiring_enter() {
        // GIVEN
        let input = b's';

        // WHEN
        let actual = command_from_byte(input);

        // THEN
        assert_eq!(actual, Some(KeyCommand::ShowLatestTutorText));
    }

    #[test]
    fn maps_repeat_key_without_requiring_enter() {
        // GIVEN
        let input = b'w';

        // WHEN
        let actual = command_from_byte(input);

        // THEN
        assert_eq!(actual, Some(KeyCommand::RepeatCurrentPhrase));
    }
}
