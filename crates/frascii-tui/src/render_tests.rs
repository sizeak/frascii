//! Render snapshots.
//!
//! These catch what targeted assertions miss: a border that stops joining, a
//! line that shifts by a column, a frame that loses a row. They capture
//! **symbols only, not styles** — that is `TestBackend`'s limitation, so colour
//! and emphasis stay the job of targeted assertions elsewhere.
//!
//! Run `cargo insta review` to accept or update a diff, and read it rather than
//! blind-accepting: these are the tests that only fail when rendering genuinely
//! changed. If a change invalidates a snapshot, re-point it at the new UI —
//! never delete the file, because a deleted snapshot leaves nothing to notice
//! when the UI comes back.

use ratatui::Terminal;
use ratatui::backend::TestBackend;

use crate::App;

/// Draw one frame of `app` at the given size and snapshot it.
fn frame(app: &App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height))
        .expect("TestBackend cannot fail to initialise");
    terminal
        .draw(|f| app.draw(f))
        .expect("TestBackend cannot fail to draw");
    terminal.backend().to_string()
}

#[test]
fn splash_frame() {
    insta::assert_snapshot!(frame(&App::new(), 60, 12));
}

#[test]
fn splash_frame_in_a_narrow_terminal() {
    insta::assert_snapshot!(frame(&App::new(), 20, 6));
}

#[test]
fn splash_frame_in_a_terminal_too_small_for_the_text() {
    insta::assert_snapshot!(frame(&App::new(), 8, 3));
}
