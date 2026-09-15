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

use std::time::Instant;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;

use crate::App;

/// Draw one frame at the given size and snapshot it.
///
/// `update` then `draw`, in that order, because that is the split the app is
/// built around and a snapshot of `draw` alone would capture an empty grid.
///
/// The kernel and the drive are pinned rather than inherited from `App::new`.
/// These snapshots are named for the Mandelbrot and exist to pin *its* glyph
/// ramp and plane↔sample geometry, so they must not move when the shipped
/// default moves — which it has: the launch view is now an orbiting Julia set.
/// Pinning also makes the determinism real instead of incidental. `update`
/// takes `Instant::now()`, and these only came out stable because `Clock`'s
/// first tick returns zero and `powf(0.0)` is 1; with the motion switched off
/// there is nothing for a clock to feed.
fn frame(width: u16, height: u16) -> String {
    let mut app = App::new();
    app.pin_for_snapshot();
    // The status bar is hidden for these: it carries a magnification and an
    // iteration count, so a snapshot including it would fail on any change to
    // the defaults rather than on a change to the *rendering*, which is what
    // these exist to catch.
    let _ = app.handle_key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE));
    app.update(Rect::new(0, 0, width, height), Instant::now());

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
