mod app;
mod buffer;
mod config;
mod keymap;
mod save;
mod ui;
mod view;

use std::io::{self, Stdout};
use std::panic;
use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use crossterm::event::{
    DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use crate::app::App;
use crate::config::EditorConfig;
use crate::keymap::Keymap;

pub type Tui = Terminal<CrosstermBackend<Stdout>>;

/// Griffin, a non-modal terminal text editor.
#[derive(Debug, Parser)]
#[command(name = "griffin", version, about)]
struct Cli {
    /// File or folder to open.
    path: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let path = Cli::parse().path;

    // Loaded before the terminal switches screens; a bad config never stops startup.
    let (keymap, editor, config_error) = load_config();

    install_panic_hook(|| {
        // Best effort: the process is already panicking, so a failed restore can
        // only be ignored.
        let _ = restore_terminal();
    });

    let mut terminal = match setup_terminal() {
        Ok(terminal) => terminal,
        Err(err) => {
            let _ = restore_terminal();
            return Err(err);
        }
    };

    let result = App::new(keymap, editor, path, config_error)
        .run(&mut terminal)
        .await;
    let restored = restore_terminal();
    result?;
    restored
}

/// Reads `config.toml` and builds the keymap from it, falling back to the defaults
/// (and saying why in the status line) when the file is malformed or names a bad key.
fn load_config() -> (Keymap, EditorConfig, Option<String>) {
    let loaded = config::load();
    if let Some(err) = loaded.error {
        return (
            Keymap::default(),
            EditorConfig::default(),
            Some(format!("config error: {err}")),
        );
    }
    let editor = loaded.config.editor;
    match Keymap::new(&loaded.config.keys) {
        Ok(keymap) => (keymap, editor, None),
        Err(err) => (
            Keymap::default(),
            editor,
            Some(format!("config error: {err}")),
        ),
    }
}

fn setup_terminal() -> Result<Tui> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(
        stdout,
        EnterAlternateScreen,
        EnableMouseCapture,
        EnableBracketedPaste
    )?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout))?;
    terminal.clear()?;
    Ok(terminal)
}

/// Puts the terminal back the way the shell expects it. Safe to call more than once.
fn restore_terminal() -> Result<()> {
    // Undo every step even if one fails, then report the first failure.
    let raw = disable_raw_mode();
    let screen = execute!(
        io::stdout(),
        DisableBracketedPaste,
        DisableMouseCapture,
        LeaveAlternateScreen,
        crossterm::cursor::Show
    );
    raw?;
    screen?;
    Ok(())
}

/// Chains a hook that runs `restore` before the previous hook prints the panic, so
/// the message lands on the normal screen instead of the alternate one.
fn install_panic_hook<F>(restore: F)
where
    F: Fn() + Send + Sync + 'static,
{
    let previous = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        restore();
        previous(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn panic_hook_restores_terminal_before_panic_message() {
        let restored = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&restored);
        let original = panic::take_hook();
        // Silence the default message so the test output stays readable.
        panic::set_hook(Box::new(|_| {}));

        install_panic_hook(move || flag.store(true, Ordering::SeqCst));
        let outcome = panic::catch_unwind(|| panic!("boom inside the event loop"));

        panic::set_hook(original);
        assert!(outcome.is_err());
        assert!(restored.load(Ordering::SeqCst));
    }

    #[test]
    fn cli_accepts_an_optional_path() {
        assert_eq!(Cli::parse_from(["griffin"]).path, None);
        assert_eq!(
            Cli::parse_from(["griffin", "notes.txt"]).path,
            Some(PathBuf::from("notes.txt"))
        );
    }
}
