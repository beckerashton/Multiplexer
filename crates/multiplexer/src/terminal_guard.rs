use std::io::{self, Stdout, Write};

use crossterm::{
    cursor::{Hide, Show},
    event::{
        DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, EnableBracketedPaste,
        EnableFocusChange, EnableMouseCapture,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};

#[cfg(unix)]
use crossterm::event::{
    KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};

/// Restores the caller's terminal even if the application exits through an
/// error path. Keep it alive for the entire interactive event loop.
pub struct TerminalGuard {
    stdout: Stdout,
}

impl TerminalGuard {
    pub fn enter() -> io::Result<Self> {
        enable_raw_mode()?;
        let mut guard = Self {
            stdout: io::stdout(),
        };
        execute!(
            guard.stdout,
            EnterAlternateScreen,
            Hide,
            EnableMouseCapture,
            EnableFocusChange,
            EnableBracketedPaste
        )?;
        // Crossterm rejects kitty keyboard commands on Windows, whose
        // console input records already carry key modifiers.
        #[cfg(unix)]
        execute!(
            guard.stdout,
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        )?;
        Ok(guard)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        #[cfg(unix)]
        let _ = execute!(self.stdout, PopKeyboardEnhancementFlags);
        let _ = execute!(
            self.stdout,
            crossterm::terminal::EndSynchronizedUpdate,
            DisableBracketedPaste,
            DisableFocusChange,
            DisableMouseCapture,
            Show,
            LeaveAlternateScreen
        );
        let _ = self.stdout.flush();
        let _ = disable_raw_mode();
    }
}
