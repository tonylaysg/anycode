mod common;

use anycode::clipboard::ClipboardHandler;

#[test]
fn test_clipboard_handler_creation() {
    // This may fail in CI without display, so just check it compiles
    let _handler = ClipboardHandler::new();
}
