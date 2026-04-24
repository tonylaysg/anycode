//! Regression tests for the alacritty emulator grid-underflow panic.
//!
//! `alacritty_terminal::Term::new` calls `.last()` on the grid storage during
//! its initial resize, which underflows when rows=0 or cols=0 and panics with
//! `index out of bounds: the len is 0 but the index is usize::MAX`.
//!
//! Some terminal environments (fresh `pty.fork()` children, certain SSH
//! clients, tmux nested sessions) report a 0×0 window size that we previously
//! forwarded straight into the emulator, causing anycopilot / anycode to
//! crash before the TUI ever rendered. The emulator must floor the size to
//! 1×1 at construction and on every resize call.

use anycode::pty::emulator::create;

fn assert_does_not_panic(rows: u16, cols: u16) {
    // Construction must not panic.
    let mut emu = create(rows, cols, 0);
    // set_size must also tolerate 0×0 (e.g. a bogus SIGWINCH).
    emu.set_size(0, 0);
    emu.set_size(rows, cols);
    // Feeding a byte after init shouldn't panic either.
    emu.process(b"hi");
}

#[test]
fn create_with_zero_rows_does_not_panic() {
    assert_does_not_panic(0, 80);
}

#[test]
fn create_with_zero_cols_does_not_panic() {
    assert_does_not_panic(24, 0);
}

#[test]
fn create_with_zero_size_does_not_panic() {
    assert_does_not_panic(0, 0);
}

#[test]
fn create_with_normal_size_works() {
    assert_does_not_panic(24, 80);
}
