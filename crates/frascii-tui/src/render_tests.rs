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
//!
//! Against a fractal these earn more than they did against the splash: a
//! symbols-only capture pins exactly the glyph ramp and the plane↔sample
//! geometry, which are the two things most likely to shift silently. The tiny
//! sizes are kept for the same reason they were added — they are what caught
//! the border-overpaint bug.

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;

use crate::App;

/// Draw one frame at the given size and snapshot it.
///
/// `update` then `draw`, in that order, because that is the split the app is
/// built around and a snapshot of `draw` alone would capture an empty grid.
/// Nothing here consults a clock, so these are deterministic — which is the
/// property that makes them worth having.
fn frame(width: u16, height: u16) -> String {
    let mut app = App::new();
    app.update(Rect::new(0, 0, width, height));

    let mut terminal = Terminal::new(TestBackend::new(width, height))
        .expect("TestBackend cannot fail to initialise");
    terminal
        .draw(|f| app.draw(f))
        .expect("TestBackend cannot fail to draw");
    terminal.backend().to_string()
}

// Half-block mode has **no snapshot**, deliberately. `TestBackend` captures
// symbols only, and every cell in that mode is `▀` — so a snapshot of it is a
// uniform rectangle that would pass whatever the picture did, while still
// failing noisily on any geometry change. It would be a test that looks like
// coverage and is not.
//
// What covers that mode instead: `shade.rs` asserts the packing directly
// (upper sample to foreground, lower to background, both 24-bit values intact),
// and `app.rs` asserts that toggling the mode leaves the plane region
// unchanged and doubles the sample rows. Those check the things that can
// actually break.

#[test]
fn mandelbrot_home_view() {
    insta::assert_snapshot!(frame(60, 12));
}

#[test]
fn mandelbrot_home_view_in_a_narrow_terminal() {
    insta::assert_snapshot!(frame(20, 6));
}

#[test]
fn mandelbrot_home_view_in_a_tiny_terminal() {
    insta::assert_snapshot!(frame(8, 3));
}
