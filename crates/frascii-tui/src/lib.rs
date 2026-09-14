//! The terminal frontend for frascii.
//!
//! This crate holds everything presentational: the rendered [`Grid`] and its
//! cells, the ratatui blit, the terminal's lifecycle, and key dispatch.
//! Frontend-agnostic fractal logic lives in [`frascii_core`], which this crate
//! consumes and any other frontend could too.
//!
//! # Layering
//!
//! Rendering and the terminal are deliberately in one crate for now. The rule
//! that still holds is the *other* direction: nothing presentational and
//! nothing terminal-shaped may drift down into `frascii-core`, because that is
//! what a second frontend would consume unchanged.
//!
//! Within this crate the seam is kept as a file boundary instead: `render.rs`
//! holds the pipeline's data types and takes no ratatui dependency, and
//! `widget.rs` holds the ratatui impl. If a second frontend arrives, `render`
//! and the ramps and palettes that will join it are what move out, and keeping
//! it ratatui-free is what makes that a file move rather than a rewrite.

// Module names above are code spans rather than intra-doc links on purpose:
// both modules are private, so a link from this crate's public docs would be a
// `private_intra_doc_links` warning and fail the doc lane.

mod app;
mod clock;
mod error;
mod oklab;
mod palette;
mod ramp;
mod render;
mod shade;
mod widget;

#[cfg(test)]
mod render_tests;

pub use app::{App, Drive, Flow};
pub use error::{Result, TuiError};
pub use palette::Palette;
pub use ramp::RAMP;
pub use render::{Cell, Grid, Rgb};
pub use shade::{CellMode, Shader, shade_into};

/// Run the frontend: set the terminal up, drive the loop, and restore it.
///
/// The terminal is restored on every exit path, error included. `ratatui`'s
/// init also installs a panic hook that restores it before unwinding
/// (`ratatui-0.30.2 src/init.rs:398`, `try_init` calling `set_panic_hook`), so
/// a panic inside the loop does not leave a raw-mode terminal behind either.
pub fn run() -> Result<()> {
    let mut terminal = ratatui::try_init()?;
    let outcome = App::new().run(&mut terminal);
    // Restore first, then propagate: reporting an error into a terminal still
    // in raw mode and the alternate screen means the message is wiped by the
    // screen restore a moment later.
    let restored = ratatui::try_restore();
    outcome?;
    restored.map_err(TuiError::from)
}
