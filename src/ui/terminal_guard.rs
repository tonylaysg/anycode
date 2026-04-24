use crossterm::cursor::{Hide, Show};
use crossterm::event::{DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::ExecutableCommand;
use parking_lot::Mutex;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::io::{self, Write, Stdout};
use std::sync::Arc;

type CleanupFn = Arc<Mutex<Option<Box<dyn FnOnce() + Send + 'static>>>>;

pub struct TerminalGuard {
    cleanup: CleanupFn,
}

impl TerminalGuard {
    fn new() -> Self {
        Self {
            cleanup: Arc::new(Mutex::new(None)),
        }
    }

    fn set_cleanup<F: FnOnce() + Send + 'static>(&self, cleanup: F) {
        *self.cleanup.lock() = Some(Box::new(cleanup));
    }

    fn install_panic_hook(&self) {
        let cleanup = Arc::clone(&self.cleanup);
        let default_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if let Some(cleanup_fn) = cleanup.lock().take() {
                cleanup_fn();
            }
            default_hook(info);
        }));
    }

    fn restore(&self) {
        if let Some(cleanup_fn) = self.cleanup.lock().take() {
            cleanup_fn();
        }
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        self.restore();
    }
}

pub fn setup_terminal() -> io::Result<(Terminal<CrosstermBackend<Stdout>>, TerminalGuard)> {
    // Install panic hook and cleanup BEFORE any TTY-altering action.
    // If Terminal::new or any subsequent setup panics (e.g. size query failure
    // in exotic terminal emulators), we still want the terminal restored.
    let guard = TerminalGuard::new();
    guard.set_cleanup(|| {
        let mut stdout = io::stdout();
        // Disable mouse/paste FIRST while still in raw mode,
        // so pending mouse events don't leak into the shell.
        let _ = stdout.execute(DisableMouseCapture);
        let _ = stdout.execute(DisableBracketedPaste);
        let _ = stdout.flush();
        // Flush pending input from kernel tty buffer (in-flight mouse events).
        unsafe { libc::tcflush(libc::STDIN_FILENO, libc::TCIFLUSH); }
        let _ = disable_raw_mode();
        let _ = stdout.execute(LeaveAlternateScreen);
        let _ = stdout.write_all(b"\x1b[?1036l\x1b[?1039l");
        let _ = stdout.flush();
        let _ = stdout.execute(Show);
    });
    guard.install_panic_hook();

    // Sanity-check the terminal size BEFORE entering alt-screen. crossterm's
    // `size()` returns (0, 0) or errors when stdout is not a real TTY or when
    // the emulator reports no dimensions. Continuing past this point with a
    // zero-sized terminal triggers a silent panic inside ratatui's Buffer::new
    // on the first draw, leaving the terminal in alt-screen with no error
    // visible to the user. Fail loudly instead.
    match crossterm::terminal::size() {
        Ok((w, h)) if w == 0 || h == 0 => {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                format!(
                    "terminal reports zero size ({}x{}); is stdout a real TTY? \
                     Try running without redirecting stdout (no pipes to tee/cat), \
                     inside a real terminal emulator with a sane TERM value.",
                    w, h
                ),
            ));
        }
        Err(e) => {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                format!(
                    "failed to query terminal size: {}. \
                     anycode requires stdout to be a real terminal; \
                     check TERM (current: {:?}) and avoid piping/redirecting stdout.",
                    e,
                    std::env::var("TERM").ok()
                ),
            ));
        }
        Ok(_) => {}
    }

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    // Enable "meta/alt sends escape" — tells terminal to send ESC prefix
    // when Option/Alt modifies a key (e.g. Option+Backspace → ESC 0x7F).
    // 1036 = metaSendsEscape, 1039 = altSendsEscape.
    // Works in most terminals; Warp ignores these in alt-screen mode,
    // which is handled separately via macOS CGEvent modifier detection.
    let _ = stdout.write_all(b"\x1b[?1036h\x1b[?1039h");
    let _ = stdout.flush();
    stdout.execute(EnterAlternateScreen)?;
    stdout.execute(EnableBracketedPaste)?;
    stdout.execute(EnableMouseCapture)?;
    stdout.execute(Hide)?;

    let backend = CrosstermBackend::new(stdout);
    let terminal = Terminal::new(backend)?;

    Ok((terminal, guard))
}
