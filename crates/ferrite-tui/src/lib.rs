//! `ferrite-tui` — ratatui 0.29 + crossterm 0.28 terminal UI for Ferrite.

pub mod app;
pub mod carving_session;
pub mod screens;
pub mod session;

pub use app::App;

use crossterm::{
    event::{DisableBracketedPaste, EnableBracketedPaste},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io;
use thiserror::Error;

/// Built-in file-carving signatures (workspace `config/signatures.toml`).
pub(crate) const SIGNATURES_TOML: &str = include_str!("../../../config/signatures.toml");

#[derive(Debug, Error)]
pub enum TuiError {
    #[error("terminal I/O: {0}")]
    Io(#[from] io::Error),
}

pub type Result<T> = std::result::Result<T, TuiError>;

/// Set up the terminal, run the event loop, and restore on exit (including panic).
pub fn run() -> Result<()> {
    // Restore terminal on panic so the shell is not left in a broken state.
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, DisableBracketedPaste);
        prev_hook(info);
    }));

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableBracketedPaste)?;

    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;

    let res = App::new().run_loop(&mut terminal);

    // Always restore terminal.
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableBracketedPaste
    )?;
    terminal.show_cursor()?;

    res
}
