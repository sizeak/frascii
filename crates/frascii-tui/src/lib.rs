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

/// The terminal frascii draws to.
///
/// A `BufWriter` under the backend, which is the point of hand-rolling this
/// instead of calling `ratatui::init`. `DefaultTerminal` writes through
/// `stdout()` — a `LineWriter`, which flushes on every newline and holds a
/// small buffer. A full redraw where every cell changes both colours is about
/// thirty bytes per cell, and palette cycling makes *every* frame a full
/// redraw: measured at 1.7MB over three seconds on a 60×14 terminal, or roughly
/// 566KB/s. That write volume, not ratatui, is the ceiling.
type BufferedTerminal =
    ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::BufWriter<std::io::Stdout>>>;

/// Run the frontend: set the terminal up, drive the loop, and restore it.
///
/// The terminal is restored on every exit path, error included, and a panic
/// hook restores it before unwinding — so a panic inside the loop does not
/// leave a raw-mode terminal behind either.
pub fn run() -> Result<()> {
    let mut terminal = init()?;
    let outcome = App::new().run(&mut terminal);
    // Restore first, then propagate: reporting an error into a terminal still
    // in raw mode and the alternate screen means the message is wiped by the
    // screen restore a moment later.
    let restored = restore();
    outcome?;
    restored
}

/// Enter raw mode and the alternate screen, and build the buffered terminal.
fn init() -> Result<BufferedTerminal> {
    use ratatui::crossterm::execute;
    use ratatui::crossterm::terminal::{EnterAlternateScreen, enable_raw_mode};

    // Before anything is changed, so a failure during setup still restores.
    install_panic_hook();
    enable_raw_mode()?;
    execute!(std::io::stdout(), EnterAlternateScreen)?;

    let writer = std::io::BufWriter::with_capacity(1 << 20, std::io::stdout());
    let backend = ratatui::backend::CrosstermBackend::new(writer);
    Ok(ratatui::Terminal::new(backend)?)
}

/// Undo what [`init`] did.
fn restore() -> Result<()> {
    use ratatui::crossterm::execute;
    use ratatui::crossterm::terminal::{LeaveAlternateScreen, disable_raw_mode};

    disable_raw_mode()?;
    execute!(std::io::stdout(), LeaveAlternateScreen)?;
    Ok(())
}

/// Restore the terminal before a panic reaches the user's screen.
///
/// Hand-written because `ratatui::init`'s hook is private, and taking a
/// `BufWriter` means not using `ratatui::init`. The previous hook is chained
/// rather than replaced, so the panic message still prints — into a terminal
/// that is by then usable.
///
/// Both calls ignore their errors deliberately: this runs while already
/// panicking, and a second failure here would replace a useful message with a
/// useless one.
fn install_panic_hook() {
    use ratatui::crossterm::execute;
    use ratatui::crossterm::terminal::{LeaveAlternateScreen, disable_raw_mode};

    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(std::io::stdout(), LeaveAlternateScreen);
        previous(info);
    }));
}
