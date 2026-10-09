mod app;
mod backup;
mod buffer;
mod clipboard;
mod config;
mod dap;
mod framing;
mod highlight;
mod keymap;
mod lsp;
mod run;
mod save;
mod search;
mod theme;
mod ui;
mod view;
mod workspace;

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

use crate::app::App;
use crate::backup::Backups;
use crate::lsp::Lsp;
use crate::view::UndercurlBackend;

pub type Tui = Terminal<UndercurlBackend<Stdout>>;

/// Glyph, a non-modal terminal text editor.
#[derive(Debug, Parser)]
#[command(name = "glyph", version, about)]
struct Cli {
    /// File or folder to open.
    path: Option<PathBuf>,
    /// List each language's server command and whether it is installed, then exit.
    #[arg(long)]
    health: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    if cli.health {
        print!("{}", health());
        return Ok(());
    }
    let path = cli.path;

    // Loaded before the terminal switches screens; a bad config never stops startup.
    let (settings, config_error) = config::Settings::startup(config::load());

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

    let backups = Backups::new(backup::data_dir(std::env::var_os("GLYPH_DATA_DIR")));
    let result = App::new(settings.keymap, settings.editor, path, config_error)
        .with_theme(settings.theme)
        .with_lsp(Lsp::new(lsp::servers::with_defaults(settings.lsp)))
        .with_debug_adapters(settings.debug)
        .with_config_path(config::config_path(std::env::var_os("GLYPH_CONFIG")))
        .with_backups(backups)
        .with_clipboard(clipboard::from_env(std::env::var_os(
            "GLYPH_CLIPBOARD_FILE",
        )))
        .run(&mut terminal)
        .await;
    let restored = restore_terminal();
    result?;
    restored
}

/// The `--health` report. A config that fails to load is reported by the editor
/// itself; here the defaults stand in for it, as they would at startup.
fn health() -> String {
    let loaded = config::load();
    let tables = if loaded.error.is_some() {
        Default::default()
    } else {
        loaded.config.lsp
    };
    lsp::servers::health(
        &lsp::servers::with_defaults(tables),
        std::env::var_os("PATH").as_deref(),
        std::env::var_os("PATHEXT").as_deref(),
    )
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
    let mut terminal = Terminal::new(UndercurlBackend::new(stdout))?;
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
        assert_eq!(Cli::parse_from(["glyph"]).path, None);
        assert!(!Cli::parse_from(["glyph"]).health);
        assert!(Cli::parse_from(["glyph", "--health"]).health);
        assert_eq!(
            Cli::parse_from(["glyph", "notes.txt"]).path,
            Some(PathBuf::from("notes.txt"))
        );
    }
}
