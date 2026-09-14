//! The event loop, the terminal's lifecycle, and the frame.

use std::time::{Duration, Instant};

use frascii_core::{Mandelbrot, SampleGrid, Viewport, sample_into};
use ratatui::Frame;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::Rect;

use crate::error::Result;
use crate::render::Grid;
use crate::shade::{CellMode, Shader, shade_into};

/// The frame period: how long one iteration of the loop should take in total.
///
/// A genuine budget, not a sleep. The loop subtracts the time drawing took from
/// the wait, so the period stays ~33ms (30fps) rather than becoming
/// `draw + 33ms`. That distinction is worth about a third of the frame rate
/// once drawing is not free: an 8.7ms frame plus a flat 33ms wait is 24fps, and
/// the cost is in the loop's structure rather than anywhere measurable in the
/// renderer.
const TICK: Duration = Duration::from_millis(33);

/// How many times taller a character cell is than it is wide.
///
/// A setting rather than a measurement. Terminals do not report it portably —
/// crossterm's `window_size` documents its pixel fields as possibly "not
/// reliably implemented or default to 0" — and 2:1 is the near-universal
/// convention. Named rather than inlined as a literal `2.0` because it is the
/// knob a user with an unusual font would reach for.
const CELL_ASPECT: f64 = 2.0;

/// What the loop should do after handling an event.
///
/// An enum rather than a `bool` so a third loop-level outcome is an addition
/// rather than a second flag to keep consistent with the first.
///
/// It must **not** become a command channel. Pan, zoom, reset and
/// fractal-switch belong in `App`'s own state, mutated by `handle_key`
/// directly; routing them through here would make `run` re-interpret a decision
/// `handle_key` already made, and `handle_key` would stop being the single
/// place a binding is defined — which is the property that makes the
/// keybindings testable without a terminal. A variant is warranted only when
/// the *loop* must genuinely behave differently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    /// Keep running.
    Continue,
    /// Leave the loop and restore the terminal.
    Quit,
}

/// Everything that decides what the samples *are*.
///
/// The counterpart to [`Shader`], which holds everything deciding how they are
/// coloured. That division is the whole mechanism behind skipping work: if this
/// compares equal to what was last sampled, the kernel need not run again.
///
/// `Copy + PartialEq` so the check is a *derived* comparison rather than a
/// `bool` somebody has to remember to set. The rule for a new field is one
/// question: if it changes, must the kernel run again? If yes it belongs here;
/// if no, on the shader.
///
/// The fractal is not a field yet because there is only one. When switching
/// arrives it joins this struct as an **enum** — `&dyn Fractal` is neither
/// `Copy` nor `PartialEq`, so a trait object here would quietly break the
/// comparison this type exists for.
#[derive(Debug, Clone, Copy, PartialEq)]
struct SampleParams {
    viewport: Viewport,
    limit: u32,
}

/// The frontend's state.
#[derive(Debug)]
pub struct App {
    params: SampleParams,
    /// The parameters the sample grid was last filled for, if any.
    sampled_for: Option<SampleParams>,
    samples: SampleGrid,
    shader: Shader,
    cells: Grid,
}

impl App {
    /// A fresh app at the home view.
    #[must_use]
    pub fn new() -> Self {
        let mode = CellMode::default();
        Self {
            params: SampleParams {
                // Zero-sized until the first `update` learns the terminal's
                // size. Sampling a zero grid is legal and costs nothing, so
                // there is no special first-frame path to get wrong.
                viewport: Viewport::home(0, 0, mode.sample_aspect(CELL_ASPECT)),
                limit: 0,
            },
            sampled_for: None,
            samples: SampleGrid::new(0, 0),
            shader: Shader {
                mode,
                ..Shader::default()
            },
            cells: Grid::new(0, 0),
        }
    }

    /// The palette currently in use, for a status line.
    #[must_use]
    pub fn palette_name(&self) -> &'static str {
        self.shader.palette.name()
    }

    /// Decide what a key press means.
    ///
    /// Pure with respect to the terminal, and separate from [`App::run`] on
    /// purpose: this is the half worth testing, and a test that had to spawn a
    /// terminal to reach it would not get written. Every binding this grows
    /// should gain a case here.
    #[must_use]
    pub fn handle_key(&mut self, key: KeyEvent) -> Flow {
        // Only presses, because a key-up must not re-trigger the action its
        // key-down already ran: acting on `KeyEventKind::Release` would quit on
        // the *release* of the key that had already quit, and treating Repeat
        // as a press would make a held key fire twice.
        //
        // Where the non-Press kinds actually come from, since this is easy to
        // get wrong: on Windows, unprompted — crossterm's console reader emits
        // Release on key-up (`crossterm-0.29.0
        // src/event/sys/windows/parse.rs:226,289`). On a Unix terminal they
        // arrive only if the application pushes `REPORT_EVENT_TYPES`
        // (`crossterm-0.29.0 src/event.rs:296-298`), and `ratatui::try_init`
        // pushes no keyboard-enhancement flags (`ratatui-0.30.2
        // src/init.rs:397-403`), so frascii does not see them there today.
        // Windows is out of scope, so this is inert until the kitty protocol is
        // enabled — which a modifier-chord binding would want.
        if key.kind != KeyEventKind::Press {
            return Flow::Continue;
        }

        match (key.code, key.modifiers) {
            (KeyCode::Char('c' | 'C'), KeyModifiers::CONTROL) => Flow::Quit,
            (KeyCode::Char('q' | 'Q') | KeyCode::Esc, _) => Flow::Quit,
            _ => Flow::Continue,
        }
    }

    /// Bring the frame up to date: resize, sample if needed, shade.
    ///
    /// **All the per-frame cost lives here**, deliberately, so that
    /// [`App::draw`] stays a pure function of already-computed state. Two
    /// things follow: the render snapshots are stable because drawing cannot
    /// depend on the clock, and sample time, shade time and ratatui's own
    /// buffer diff stay separately measurable instead of collapsing into one
    /// unattributable call.
    pub fn update(&mut self, area: Rect) {
        let (cols, rows) = self
            .shader
            .mode
            .lattice(usize::from(area.width), usize::from(area.height));

        if self.params.viewport.cols != cols || self.params.viewport.rows != rows {
            self.params
                .viewport
                .resize(cols, rows, self.shader.mode.sample_aspect(CELL_ASPECT));
        }
        self.params.limit = self.params.viewport.suggested_limit();

        // The entire caching policy. Palette cycling changes the shader and
        // never the params, so it never reaches the kernel.
        if self.sampled_for != Some(self.params) {
            sample_into(
                &mut self.samples,
                &self.params.viewport,
                &Mandelbrot,
                self.params.limit,
            );
            self.sampled_for = Some(self.params);
        }

        shade_into(&self.shader, &self.samples, &mut self.cells);
    }

    /// Put the finished frame on screen.
    ///
    /// Pure and cheap: it blits an already-shaded grid. See [`App::update`] for
    /// why the cost is not here.
    pub fn draw(&self, frame: &mut Frame<'_>) {
        frame.render_widget(&self.cells, frame.area());
    }

    /// Own the terminal until the user quits.
    ///
    /// Restoration is the caller's, via [`crate::run`]: a `run` that both
    /// initialised and restored the terminal could not be driven by a test
    /// harness's backend, and one that restored on the happy path only would
    /// leave a raw-mode terminal behind on the first error.
    pub fn run(&mut self, terminal: &mut ratatui::DefaultTerminal) -> Result<()> {
        let mut next_frame = Instant::now();

        loop {
            // `autoresize` then `get_frame().area()` is what `draw` does
            // internally, and it is how `update` learns the size *before*
            // drawing. Deliberately not `Terminal::size()`, whose own docs say
            // it reports the backend size without updating ratatui's viewport
            // bookkeeping — so after a resize it disagrees with `Frame::area()`
            // for one frame, and the grid would be built for the wrong shape.
            terminal.autoresize()?;
            let area = terminal.get_frame().area();
            self.update(area);
            terminal.draw(|frame| self.draw(frame))?;

            next_frame += TICK;
            let now = Instant::now();
            if next_frame < now {
                // Drawing overran the budget. Drop the backlog rather than
                // chase it: catching up would spend the next several frames
                // rendering stale views with no input handled in between.
                next_frame = now;
            }

            // Drain every event that has arrived, rather than one per frame. A
            // held key repeats faster than the frame rate, and handling one
            // event per draw would run the renderer at the key-repeat rate
            // instead of the frame rate.
            while let Some(remaining) = next_frame.checked_duration_since(Instant::now()) {
                // The short-circuit is load-bearing: `read` blocks, so it must
                // only be reached once `poll` has said an event is waiting.
                if !event::poll(remaining)? {
                    break;
                }
                if let Event::Key(key) = event::read()?
                    && self.handle_key(key) == Flow::Quit
                {
                    tracing::debug!("quit requested");
                    return Ok(());
                }
            }
        }
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn updated(width: u16, height: u16) -> App {
        let mut app = App::new();
        app.update(Rect::new(0, 0, width, height));
        app
    }

    #[test]
    fn q_and_escape_quit() {
        let mut app = App::new();
        assert_eq!(app.handle_key(press(KeyCode::Char('q'))), Flow::Quit);
        assert_eq!(app.handle_key(press(KeyCode::Char('Q'))), Flow::Quit);
        assert_eq!(app.handle_key(press(KeyCode::Esc)), Flow::Quit);
    }

    #[test]
    fn ctrl_c_quits() {
        let mut app = App::new();
        let key = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(app.handle_key(key), Flow::Quit);
    }

    #[test]
    fn a_bare_c_does_not_quit() {
        let mut app = App::new();
        assert_eq!(app.handle_key(press(KeyCode::Char('c'))), Flow::Continue);
    }

    #[test]
    fn unbound_keys_continue() {
        let mut app = App::new();
        assert_eq!(app.handle_key(press(KeyCode::Char('x'))), Flow::Continue);
        assert_eq!(app.handle_key(press(KeyCode::Up)), Flow::Continue);
    }

    #[test]
    fn release_and_repeat_events_are_ignored() {
        let mut app = App::new();
        for kind in [KeyEventKind::Release, KeyEventKind::Repeat] {
            let mut key = press(KeyCode::Char('q'));
            key.kind = kind;
            assert_eq!(app.handle_key(key), Flow::Continue, "{kind:?}");
        }
    }

    #[test]
    fn updating_fills_the_grid_to_the_terminals_size() {
        let app = updated(60, 20);
        assert_eq!((app.cells.width(), app.cells.height()), (60, 20));
        assert_eq!((app.samples.cols(), app.samples.rows()), (60, 20));
    }

    #[test]
    fn the_first_frame_renders_the_set_rather_than_a_blank() {
        // The home view has a large interior and a large outside, so a frame
        // that is all one glyph means the viewport or the shading is wrong —
        // which is the failure a snapshot would show but not explain.
        let app = updated(80, 30);
        let glyphs: std::collections::BTreeSet<char> = app.cells.cells().map(|c| c.glyph).collect();
        assert!(
            glyphs.len() >= 5,
            "only {} distinct glyphs in the first frame: {glyphs:?}",
            glyphs.len()
        );
        let colours: std::collections::BTreeSet<_> = app
            .cells
            .cells()
            .map(|c| (c.colour.r, c.colour.g, c.colour.b))
            .collect();
        assert!(
            colours.len() >= 5,
            "only {} distinct colours",
            colours.len()
        );
    }

    #[test]
    fn a_second_update_at_the_same_size_does_not_re_sample() {
        // The caching policy, asserted rather than assumed.
        let mut app = updated(40, 15);
        let before = app.samples.clone();
        let params = app.sampled_for;
        app.update(Rect::new(0, 0, 40, 15));
        assert_eq!(app.sampled_for, params);
        assert_eq!(app.samples, before);
    }

    #[test]
    fn a_resize_does_re_sample() {
        let mut app = updated(40, 15);
        let before = app.sampled_for;
        app.update(Rect::new(0, 0, 41, 15));
        assert_ne!(app.sampled_for, before, "a resize must re-sample");
        assert_eq!((app.cells.width(), app.cells.height()), (41, 15));
    }

    #[test]
    fn changing_only_the_phase_never_re_samples() {
        // Palette cycling's whole claim: a re-shade and no kernel work. If this
        // fails, cycling has become the most expensive mode rather than the
        // cheapest.
        let mut app = updated(30, 12);
        let samples_before = app.samples.clone();
        let cells_before = app.cells.clone();

        app.shader.phase += 8.0;
        app.update(Rect::new(0, 0, 30, 12));

        assert_eq!(app.samples, samples_before, "the samples were recomputed");
        assert_ne!(app.cells, cells_before, "the colours did not change");
    }

    #[test]
    fn a_zero_sized_terminal_does_not_panic() {
        // Reported during a resize, and the frame must survive it.
        let app = updated(0, 0);
        assert_eq!(app.cells.cells().count(), 0);
    }

    #[test]
    fn the_iteration_limit_follows_the_viewport() {
        let app = updated(50, 20);
        assert_eq!(app.params.limit, app.params.viewport.suggested_limit());
        assert!(app.params.limit >= 100);
    }
}
