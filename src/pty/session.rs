use crate::pty::emulator;
use crate::pty::handle::PtyHandle;
use crate::ui::events::{AppEvent, PtyError};
use parking_lot::Mutex;
use portable_pty::{native_pty_system, Child, CommandBuilder, PtySize};
use std::error::Error;
use std::io::Read;
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::thread;

/// Strip ANSI CSI/OSC escapes so the PTY tail dumped to startup.log is
/// human-readable. Not comprehensive — covers the common sequences a
/// Copilot error message would emit (SGR, cursor, clear).
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c == 0x1b && i + 1 < bytes.len() {
            let next = bytes[i + 1];
            if next == b'[' {
                // CSI: ESC [ ... final-byte-in-0x40-0x7e
                i += 2;
                while i < bytes.len() && !(0x40..=0x7e).contains(&bytes[i]) {
                    i += 1;
                }
                i += 1;
                continue;
            } else if next == b']' {
                // OSC: ESC ] ... BEL or ESC \
                i += 2;
                while i < bytes.len() {
                    if bytes[i] == 0x07 {
                        i += 1;
                        break;
                    }
                    if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b'\\' {
                        i += 2;
                        break;
                    }
                    i += 1;
                }
                continue;
            } else {
                i += 2;
                continue;
            }
        }
        out.push(c as char);
        i += 1;
    }
    out
}

pub struct PtySession {
    handle: PtyHandle,
    child: Box<dyn Child + Send + Sync>,
    reader_handle: Option<thread::JoinHandle<()>>,
}

impl PtySession {
    pub fn spawn(
        command: String,
        args: Vec<String>,
        env: Vec<(String, String)>,
        scrollback_len: usize,
        notifier: Sender<AppEvent>,
        generation: u64,
    ) -> Result<Self, Box<dyn Error>> {
        let pty_system = native_pty_system();
        let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));
        // Defensive: some terminal envs (fresh pty.fork children, certain SSH
        // clients) report (0, 0) which Ok-wraps through unwrap_or and later
        // panics `alacritty_terminal::Term::new`. Enforce a 1×1 floor —
        // real size arrives via ResizeWatcher moments later.
        let (cols, rows) = (cols.max(1), rows.max(1));
        let pair = pty_system.openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;

        let emu = Arc::new(Mutex::new(emulator::create(rows, cols, scrollback_len)));

        let mut cmd = CommandBuilder::new(command);
        cmd.args(args);
        cmd.cwd(std::env::current_dir()?);
        cmd.env("TERM", "xterm-256color");
        for (key, value) in env {
            cmd.env(key, value);
        }

        let child = pair.slave.spawn_command(cmd)?;
        drop(pair.slave);

        let reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;
        let master = Arc::new(Mutex::new(pair.master));
        let handle = PtyHandle::new(Arc::clone(&emu), writer, master);

        let reader_emu = Arc::clone(&emu);
        // Capture a ring buffer of PTY output. On child exit we flush the
        // tail to startup.log so diagnosing "child died before writing its
        // own log file" regressions doesn't require re-running with a
        // separate tracer. Kept to 4 KiB so it's cheap.
        let pty_tail = Arc::new(Mutex::new(Vec::<u8>::with_capacity(4096)));
        let pty_tail_r = Arc::clone(&pty_tail);
        let reader_handle = thread::spawn(move || {
            let mut reader = reader;
            let mut buffer = [0u8; 8192];
            let mut had_error = false;

            loop {
                let count = match reader.read(&mut buffer) {
                    Ok(0) => break, // Clean EOF
                    Ok(count) => count,
                    Err(e) => {
                        // Report read error
                        let _ = notifier.send(AppEvent::PtyError(PtyError::ReadError {
                            error: e.to_string(),
                        }));
                        had_error = true;
                        break;
                    }
                };

                reader_emu.lock().process(&buffer[..count]);
                // Append to tail ring buffer (4 KiB)
                {
                    let mut tail = pty_tail_r.lock();
                    let chunk = &buffer[..count];
                    if chunk.len() >= 4096 {
                        tail.clear();
                        tail.extend_from_slice(&chunk[chunk.len() - 4096..]);
                    } else {
                        tail.extend_from_slice(chunk);
                        if tail.len() > 4096 {
                            let drop = tail.len() - 4096;
                            tail.drain(..drop);
                        }
                    }
                }
                let _ = notifier.send(AppEvent::PtyOutput);
            }
            // Notify UI that the child process has exited (only if no error already sent)
            if !had_error {
                // Flush PTY tail to the startup trace log so "child died
                // before writing its own log" cases are debuggable.
                let tail_bytes = pty_tail_r.lock().clone();
                let tail_str = String::from_utf8_lossy(&tail_bytes);
                // Strip ANSI escape sequences crudely to keep the log readable.
                let cleaned = strip_ansi(&tail_str);
                crate::ui::runtime_trace(&format!(
                    "PTY child exited (generation={}); last {} bytes of output (ansi-stripped):\n---BEGIN-PTY-TAIL---\n{}\n---END-PTY-TAIL---",
                    generation,
                    tail_bytes.len(),
                    cleaned.trim_end()
                ));
                let _ = notifier.send(AppEvent::ProcessExit { pty_generation: generation });
            }
        });

        Ok(Self {
            handle,
            child,
            reader_handle: Some(reader_handle),
        })
    }

    pub fn handle(&self) -> PtyHandle {
        self.handle.clone()
    }

    pub fn shutdown(&mut self) -> Result<(), Box<dyn Error>> {
        // Close stdin to signal EOF to child
        self.handle.close_writer();

        // Give child a chance to exit gracefully with SIGTERM
        #[cfg(unix)]
        if let Some(pid) = self.child.process_id() {
            // SAFETY: kill() with SIGTERM is safe to call here because we obtained the pid
            // from the child process handle, guaranteeing it was valid at spawn time.
            // Sending SIGTERM to a potentially-exited process is harmless (returns ESRCH).
            unsafe {
                libc::kill(pid as i32, libc::SIGTERM);
            }
        }

        // Wait with timeout for graceful exit
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(300);
        loop {
            match self.child.try_wait()? {
                Some(_) => break,
                None if std::time::Instant::now() >= deadline => {
                    // Force kill after timeout
                    let _ = self.child.kill();
                    let _ = self.child.wait();
                    break;
                }
                None => std::thread::sleep(std::time::Duration::from_millis(10)),
            }
        }

        // Join reader thread with timeout to prevent hanging on shutdown
        if let Some(reader_handle) = self.reader_handle.take() {
            let join_deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            while !reader_handle.is_finished() {
                if std::time::Instant::now() >= join_deadline {
                    // Reader thread is stuck — abandon it rather than block forever
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            if reader_handle.is_finished() {
                let _ = reader_handle.join();
            }
        }
        Ok(())
    }
}

impl Drop for PtySession {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}
