//! The event loop, the terminal's lifecycle, and the frame.

use std::time::{Duration, Instant};

use frascii_core::{Kernel, Precision, SampleGrid, Viewport};
use ratatui::Frame;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

use crate::error::Result;
use crate::palette::Palette;
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
/// The kernel is an **enum**, not a `Box<dyn Fractal>`: a trait object is
/// neither `Copy` nor `PartialEq`, so it would quietly break the comparison
/// this type exists for.
#[derive(Debug, Clone, Copy, PartialEq)]
struct SampleParams {
    viewport: Viewport,
    kernel: Kernel,
    limit: u32,
}

/// How far one pan press moves the view, as a fraction of its width.
///
/// A fraction rather than a fixed number of samples, so panning feels the same
/// at every zoom depth — the alternative crawls when zoomed in and leaps when
/// zoomed out.
const PAN_FRACTION: f64 = 1.0 / 12.0;

/// How much one zoom press scales the view.
const ZOOM_STEP: f64 = 0.8;

/// The furthest the iteration limit can be biased from the viewport's
/// suggestion, in doublings either way.
///
/// Bounded because the suggestion already tracks depth: the bias is for
/// *taste* — trading detail against frame time — not for reaching depths the
/// automatic value cannot. Unbounded, it would be a way to hang the renderer.
const LIMIT_BIAS_RANGE: i32 = 4;

/// The frontend's state.
#[derive(Debug)]
pub struct App {
    params: SampleParams,
    /// The parameters the sample grid was last filled for, if any.
    sampled_for: Option<SampleParams>,
    samples: SampleGrid,
    shader: Shader,
    cells: Grid,
    /// Which palette, as an index so `p` can cycle.
    palette_index: usize,
    /// Iteration limit bias, in doublings from the viewport's suggestion.
    limit_bias: i32,
    /// Whether the status line is shown.
    ///
    /// Off by default so the fractal gets the whole terminal, and so the render
    /// snapshots capture the picture rather than a line of changing numbers.
    hud: bool,
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
                kernel: Kernel::default(),
                limit: 0,
            },
            sampled_for: None,
            samples: SampleGrid::new(0, 0),
            shader: Shader {
                mode,
                ..Shader::default()
            },
            cells: Grid::new(0, 0),
            palette_index: 0,
            limit_bias: 0,
            hud: false,
        }
    }

    /// The palette currently in use, for a status line.
    #[must_use]
    pub fn palette_name(&self) -> &'static str {
        self.shader.palette.name()
    }

    /// The fractal currently in use, for a status line.
    #[must_use]
    pub const fn kernel_name(&self) -> &'static str {
        self.params.kernel.name()
    }

    /// How far the view is zoomed, relative to the home view.
    #[must_use]
    pub fn magnification(&self) -> f64 {
        self.params.viewport.magnification()
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
            (KeyCode::Char('c' | 'C'), KeyModifiers::CONTROL) => return Flow::Quit,
            (KeyCode::Char('q' | 'Q') | KeyCode::Esc, _) => return Flow::Quit,

            // Panning. `Flow` stays `Continue` for all of these: a command must
            // not travel through the return value, or `run` would have to
            // re-interpret a decision made here and this would stop being the
            // one place a binding is defined.
            (KeyCode::Char('h') | KeyCode::Left, _) => self.pan(-1, 0),
            (KeyCode::Char('l') | KeyCode::Right, _) => self.pan(1, 0),
            (KeyCode::Char('k') | KeyCode::Up, _) => self.pan(0, -1),
            (KeyCode::Char('j') | KeyCode::Down, _) => self.pan(0, 1),

            // Zoom. `=` because it is the unshifted `+` on most layouts, so
            // zooming in does not need a modifier.
            (KeyCode::Char('+' | '='), _) => self.zoom(ZOOM_STEP),
            (KeyCode::Char('-' | '_'), _) => self.zoom(1.0 / ZOOM_STEP),

            // Iteration limit, biased against the viewport's suggestion.
            (KeyCode::Char('.' | '>'), _) => self.bias_limit(1),
            (KeyCode::Char(',' | '<'), _) => self.bias_limit(-1),

            (KeyCode::Char('r' | 'R'), _) => self.reset(),
            (KeyCode::Tab | KeyCode::Char('f'), _) => self.next_kernel(),
            (KeyCode::Char('p' | 'P'), _) => self.next_palette(),
            (KeyCode::Char('m' | 'M'), _) => self.next_mode(),
            (KeyCode::Char('i' | 'I'), _) => self.hud = !self.hud,

            _ => {}
        }
        Flow::Continue
    }

    /// Shift the view by one step, in units of `(dx, dy)` presses.
    fn pan(&mut self, dx: isize, dy: isize) {
        let step = self.pan_step();
        self.params.viewport.pan_samples(dx * step, dy * step);
    }

    /// One pan press, in samples.
    ///
    /// At least one sample, so panning still works in a terminal narrow enough
    /// that the fraction rounds to zero.
    fn pan_step(&self) -> isize {
        let cols = self.params.viewport.cols as f64;
        ((cols * PAN_FRACTION).round() as isize).max(1)
    }

    /// Scale the view about its centre.
    ///
    /// Zoom is about the centre rather than a cursor because there is no cursor
    /// yet: mouse capture has to be enabled explicitly *and* disabled in both
    /// the teardown and the panic hook, which is its own piece of work.
    fn zoom(&mut self, factor: f64) {
        if self.params.viewport.zoom_centre(factor) {
            // Clamped at the precision wall. Nothing to report yet — a status
            // line is the next increment — but the viewport refused rather
            // than dissolving into rounding error, which is the point.
            tracing::debug!(
                magnification = self.params.viewport.magnification(),
                "zoom clamped: f64 precision exhausted"
            );
        }
    }

    /// Move the iteration limit away from the viewport's suggestion.
    fn bias_limit(&mut self, delta: i32) {
        self.limit_bias = (self.limit_bias + delta).clamp(-LIMIT_BIAS_RANGE, LIMIT_BIAS_RANGE);
    }

    /// Back to the home view, keeping the terminal's size.
    ///
    /// The palette and fractal are deliberately left alone: `r` means "I have
    /// zoomed somewhere useless, take me back", not "undo everything".
    fn reset(&mut self) {
        let viewport = &mut self.params.viewport;
        *viewport = Viewport::home(viewport.cols, viewport.rows, viewport.sample_aspect);
        self.limit_bias = 0;
    }

    /// Switch to the next fractal.
    fn next_kernel(&mut self) {
        self.params.kernel = self.params.kernel.next();
        // Home, because a view framed on one fractal says nothing about where
        // the next one is interesting — and a Julia set at a Mandelbrot's deep
        // zoom is usually a blank screen.
        self.reset();
    }

    /// Switch to the next palette.
    ///
    /// Only the shader changes, so this never re-samples.
    fn next_palette(&mut self) {
        self.palette_index = (self.palette_index + 1) % Palette::count();
        self.shader.palette = Palette::nth(self.palette_index);
    }

    /// Switch between glyph and half-block rendering.
    ///
    /// This does re-sample, and unavoidably: the mode changes the sample
    /// lattice as well as the reduction. What it must *not* change is the
    /// region of the plane on screen — the sample aspect absorbs the different
    /// lattice exactly, which is the claim
    /// `switching_mode_keeps_the_same_view` pins.
    fn next_mode(&mut self) {
        self.shader.mode = self.shader.mode.next();
        // `update` derives the lattice and the aspect from the mode on the next
        // frame, so nothing needs recomputing here.
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
        self.params.limit = self.biased_limit();

        // The entire caching policy. Palette cycling changes the shader and
        // never the params, so it never reaches the kernel.
        if self.sampled_for != Some(self.params) {
            self.params.kernel.sample_into(
                &mut self.samples,
                &self.params.viewport,
                self.params.limit,
            );
            self.sampled_for = Some(self.params);
        }

        shade_into(&self.shader, &self.samples, &mut self.cells);
    }

    /// The iteration limit: the viewport's suggestion, biased by `,`/`.`.
    ///
    /// The suggestion still tracks depth, so the bias rides on top of the
    /// automatic behaviour rather than replacing it — zooming deeper still
    /// raises the limit whatever the bias.
    fn biased_limit(&self) -> u32 {
        let suggested = f64::from(self.params.viewport.suggested_limit());
        let scaled = suggested * 2f64.powi(self.limit_bias);
        scaled.clamp(16.0, 100_000.0) as u32
    }

    /// Put the finished frame on screen.
    ///
    /// Pure and cheap: it blits an already-shaded grid. See [`App::update`] for
    /// why the cost is not here.
    pub fn draw(&self, frame: &mut Frame<'_>) {
        let area = frame.area();
        frame.render_widget(&self.cells, area);

        if self.hud && area.height > 0 {
            // Overlaid on the bottom row rather than given a row of its own:
            // stealing a row would change the sample lattice every time the
            // status line was toggled, and re-sample for a caption.
            let row = Rect::new(area.x, area.y + area.height - 1, area.width, 1);
            // `Clear` first: a `Paragraph`'s style recolours the row but only
            // overwrites the cells its text occupies, so without this the
            // fractal's glyphs show through to the right of the status text —
            // which reads as corruption rather than as an overlay.
            frame.render_widget(Clear, row);
            frame.render_widget(self.status_line(), row);
        }
    }

    /// The status line: what the renderer is currently doing.
    ///
    /// Cheap, and the point of it is that a regression gets noticed while using
    /// the thing rather than three weeks later. The magnification and limit are
    /// what to cross-check against `--headless` at the same size and depth; if
    /// they diverge, the event loop is costing something the benchmark cannot
    /// see.
    fn status_line(&self) -> Paragraph<'static> {
        let viewport = &self.params.viewport;
        let dim = Style::default().fg(Color::Rgb(0x88, 0x88, 0x88));
        let bright = Style::default()
            .fg(Color::Rgb(0xe8, 0xe8, 0xe8))
            .add_modifier(Modifier::BOLD);

        let mut spans = vec![
            Span::styled(self.params.kernel.name().to_owned(), bright),
            Span::styled("  ", dim),
            Span::styled(self.shader.mode.name().to_owned(), dim),
            Span::styled("  ", dim),
            Span::styled(self.shader.palette.name().to_owned(), dim),
            Span::styled(format!("  {:.3e}x  ", viewport.magnification()), dim),
            Span::styled(format!("iter {}", self.params.limit), dim),
        ];

        // Only shown when it means something. At `Ample` the user does not need
        // to know the precision wall exists.
        match viewport.precision() {
            Precision::Ample => {}
            Precision::Marginal => spans.push(Span::styled(
                "  precision: marginal".to_owned(),
                Style::default().fg(Color::Rgb(0xd0, 0xa0, 0x30)),
            )),
            Precision::Exhausted => spans.push(Span::styled(
                "  precision: exhausted (max zoom)".to_owned(),
                Style::default().fg(Color::Rgb(0xd0, 0x50, 0x40)),
            )),
        }

        Paragraph::new(Line::from(spans)).style(Style::default().bg(Color::Rgb(0x10, 0x10, 0x18)))
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
    fn panning_moves_the_view_and_reverses_exactly() {
        let mut app = updated(60, 20);
        let home = app.params.viewport.centre;

        let _ = app.handle_key(press(KeyCode::Char('l')));
        let right = app.params.viewport.centre;
        assert!(right.re > home.re, "l must move right");
        assert!(
            (right.im - home.im).abs() < 1e-12,
            "l must not move vertically"
        );

        let _ = app.handle_key(press(KeyCode::Char('h')));
        assert!((app.params.viewport.centre.re - home.re).abs() < 1e-12);

        // And down is toward smaller imaginary parts, matching screen order.
        let _ = app.handle_key(press(KeyCode::Char('j')));
        assert!(app.params.viewport.centre.im < home.im, "j must move down");
        let _ = app.handle_key(press(KeyCode::Char('k')));
        assert!((app.params.viewport.centre.im - home.im).abs() < 1e-12);
    }

    #[test]
    fn the_arrow_keys_pan_identically_to_hjkl() {
        let mut letters = updated(60, 20);
        let mut arrows = updated(60, 20);
        for (letter, arrow) in [
            (KeyCode::Char('h'), KeyCode::Left),
            (KeyCode::Char('j'), KeyCode::Down),
            (KeyCode::Char('k'), KeyCode::Up),
            (KeyCode::Char('l'), KeyCode::Right),
        ] {
            let _ = letters.handle_key(press(letter));
            let _ = arrows.handle_key(press(arrow));
            assert_eq!(
                letters.params.viewport, arrows.params.viewport,
                "{letter:?}"
            );
        }
    }

    #[test]
    fn panning_feels_the_same_at_every_depth() {
        // The reason the step is a fraction of the view: a fixed number of
        // samples would crawl when zoomed in and leap when zoomed out. The
        // *plane* distance must scale with the view.
        let mut shallow = updated(60, 20);
        let mut deep = updated(60, 20);
        for _ in 0..10 {
            let _ = deep.handle_key(press(KeyCode::Char('+')));
        }
        deep.update(Rect::new(0, 0, 60, 20));

        let before_shallow = shallow.params.viewport.centre.re;
        let before_deep = deep.params.viewport.centre.re;
        let _ = shallow.handle_key(press(KeyCode::Char('l')));
        let _ = deep.handle_key(press(KeyCode::Char('l')));

        let shallow_move = shallow.params.viewport.centre.re - before_shallow;
        let deep_move = deep.params.viewport.centre.re - before_deep;
        assert!(deep_move < shallow_move, "a deep pan must cover less plane");
        // Both as a fraction of their own view: the same fraction.
        let a = shallow_move / shallow.params.viewport.half_width;
        let b = deep_move / deep.params.viewport.half_width;
        assert!((a - b).abs() < 1e-9, "{a} vs {b}");
    }

    #[test]
    fn panning_works_in_a_terminal_too_narrow_for_the_fraction() {
        // One twelfth of four columns rounds to zero; the step must still move.
        let mut app = updated(4, 3);
        let before = app.params.viewport.centre.re;
        let _ = app.handle_key(press(KeyCode::Char('l')));
        assert!(app.params.viewport.centre.re > before);
    }

    #[test]
    fn zooming_in_and_out_returns_to_where_it_started() {
        let mut app = updated(60, 20);
        let home = app.params.viewport.half_width;
        let _ = app.handle_key(press(KeyCode::Char('+')));
        assert!(app.params.viewport.half_width < home, "+ must zoom in");
        let _ = app.handle_key(press(KeyCode::Char('-')));
        assert!((app.params.viewport.half_width - home).abs() < 1e-12);
    }

    #[test]
    fn equals_zooms_in_so_no_modifier_is_needed() {
        // `+` is shifted on most layouts; `=` is the same key unshifted.
        let mut app = updated(40, 15);
        let before = app.params.viewport.half_width;
        let _ = app.handle_key(press(KeyCode::Char('=')));
        assert!(app.params.viewport.half_width < before);
    }

    #[test]
    fn zooming_in_forever_stops_at_the_precision_wall_rather_than_dissolving() {
        // The clamp, reached through the keybinding: a user holding `+` must
        // end up at the deepest resolvable view, not at rounding error.
        let mut app = updated(40, 15);
        for _ in 0..400 {
            let _ = app.handle_key(press(KeyCode::Char('+')));
        }
        let viewport = app.params.viewport;
        assert!((viewport.half_width - viewport.min_half_width()).abs() < 1e-30);
        // And adjacent samples are still distinct, which is what the wall
        // protects.
        assert!(viewport.sample_to_plane(0, 0).re != viewport.sample_to_plane(1, 0).re);
    }

    #[test]
    fn the_limit_bias_moves_the_limit_and_is_bounded() {
        let mut app = updated(50, 20);
        let automatic = app.params.limit;

        let _ = app.handle_key(press(KeyCode::Char('.')));
        app.update(Rect::new(0, 0, 50, 20));
        assert!(app.params.limit > automatic, "`.` must raise the limit");

        let _ = app.handle_key(press(KeyCode::Char(',')));
        let _ = app.handle_key(press(KeyCode::Char(',')));
        app.update(Rect::new(0, 0, 50, 20));
        assert!(app.params.limit < automatic, "`,` must lower it");

        // Bounded: the suggestion already tracks depth, so the bias is for
        // taste and must not be a way to hang the renderer.
        for _ in 0..50 {
            let _ = app.handle_key(press(KeyCode::Char('.')));
        }
        assert_eq!(app.limit_bias, LIMIT_BIAS_RANGE);
        for _ in 0..100 {
            let _ = app.handle_key(press(KeyCode::Char(',')));
        }
        assert_eq!(app.limit_bias, -LIMIT_BIAS_RANGE);
    }

    #[test]
    fn the_bias_rides_on_top_of_the_automatic_limit() {
        // Zooming deeper must still raise the limit whatever the bias, or the
        // bias would have replaced the depth tracking rather than adjusting it.
        let mut app = updated(50, 20);
        let _ = app.handle_key(press(KeyCode::Char(',')));
        app.update(Rect::new(0, 0, 50, 20));
        let shallow = app.params.limit;

        for _ in 0..20 {
            let _ = app.handle_key(press(KeyCode::Char('+')));
        }
        app.update(Rect::new(0, 0, 50, 20));
        assert!(app.params.limit > shallow, "depth must still raise it");
    }

    #[test]
    fn reset_returns_the_view_but_keeps_the_palette_and_fractal() {
        // `r` means "take me back", not "undo everything".
        let mut app = updated(60, 20);
        let _ = app.handle_key(press(KeyCode::Char('p')));
        let _ = app.handle_key(press(KeyCode::Tab));
        let palette = app.palette_name();
        let kernel = app.kernel_name();

        for _ in 0..5 {
            let _ = app.handle_key(press(KeyCode::Char('+')));
            let _ = app.handle_key(press(KeyCode::Char('l')));
        }
        let _ = app.handle_key(press(KeyCode::Char('.')));
        let _ = app.handle_key(press(KeyCode::Char('r')));

        let home = Viewport::home(60, 20, app.params.viewport.sample_aspect);
        assert_eq!(app.params.viewport, home);
        assert_eq!(app.limit_bias, 0);
        assert_eq!(app.palette_name(), palette, "reset changed the palette");
        assert_eq!(app.kernel_name(), kernel, "reset changed the fractal");
    }

    #[test]
    fn tab_cycles_the_fractal_and_reframes() {
        let mut app = updated(60, 20);
        assert_eq!(app.kernel_name(), "mandelbrot");

        // Zoom somewhere first: a view framed on one fractal says nothing about
        // where the next is interesting, so switching must reframe.
        for _ in 0..6 {
            let _ = app.handle_key(press(KeyCode::Char('+')));
        }
        let _ = app.handle_key(press(KeyCode::Tab));
        assert_eq!(app.kernel_name(), "julia");
        assert!((app.params.viewport.magnification() - 1.0).abs() < 1e-12);

        let _ = app.handle_key(press(KeyCode::Tab));
        assert_eq!(app.kernel_name(), "mandelbrot");
    }

    #[test]
    fn switching_the_fractal_changes_the_picture() {
        let mut app = updated(50, 20);
        let mandel = app.cells.clone();
        let _ = app.handle_key(press(KeyCode::Tab));
        app.update(Rect::new(0, 0, 50, 20));
        assert_ne!(app.cells, mandel, "julia rendered the same as mandelbrot");
        // And is not blank.
        let glyphs: std::collections::BTreeSet<char> = app.cells.cells().map(|c| c.glyph).collect();
        assert!(glyphs.len() >= 4, "julia looks empty: {glyphs:?}");
    }

    #[test]
    fn p_cycles_the_palette_without_re_sampling() {
        // Only the shader changes, so the kernel must not run — the same
        // property palette cycling relies on.
        let mut app = updated(40, 15);
        let first = app.palette_name();
        let samples = app.samples.clone();
        let cells = app.cells.clone();

        let _ = app.handle_key(press(KeyCode::Char('p')));
        app.update(Rect::new(0, 0, 40, 15));

        assert_ne!(app.palette_name(), first, "the palette did not change");
        assert_eq!(app.samples, samples, "changing the palette re-sampled");
        assert_ne!(app.cells, cells, "the colours did not change");
    }

    #[test]
    fn cycling_the_palette_returns_to_the_first() {
        let mut app = updated(30, 10);
        let first = app.palette_name();
        for _ in 0..Palette::count() {
            let _ = app.handle_key(press(KeyCode::Char('p')));
        }
        assert_eq!(app.palette_name(), first);
    }

    #[test]
    fn a_pan_re_samples_but_a_palette_change_does_not() {
        // The two sides of the dirty check, through the bindings.
        let mut app = updated(40, 15);
        let before = app.sampled_for;
        let _ = app.handle_key(press(KeyCode::Char('l')));
        app.update(Rect::new(0, 0, 40, 15));
        assert_ne!(app.sampled_for, before, "a pan must re-sample");

        let after_pan = app.sampled_for;
        let _ = app.handle_key(press(KeyCode::Char('p')));
        app.update(Rect::new(0, 0, 40, 15));
        assert_eq!(app.sampled_for, after_pan, "a palette change re-sampled");
    }

    #[test]
    fn m_toggles_the_render_mode_and_changes_the_glyphs() {
        let mut app = updated(40, 16);
        assert!(
            app.cells.cells().any(|c| c.glyph != '▀'),
            "glyph mode should use the ramp"
        );

        let _ = app.handle_key(press(KeyCode::Char('m')));
        app.update(Rect::new(0, 0, 40, 16));
        assert!(
            app.cells.cells().all(|c| c.glyph == '▀'),
            "half-block should be one glyph"
        );
        // Twice the samples, same cells.
        assert_eq!((app.cells.width(), app.cells.height()), (40, 16));
        assert_eq!((app.samples.cols(), app.samples.rows()), (40, 32));

        let _ = app.handle_key(press(KeyCode::Char('m')));
        app.update(Rect::new(0, 0, 40, 16));
        assert_eq!((app.samples.cols(), app.samples.rows()), (40, 16));
    }

    #[test]
    fn switching_mode_keeps_the_same_view() {
        // **The claim the whole core/frontend split rests on**, checked through
        // the real binding rather than by hand-picked numbers: pressing `m`
        // must change the sample lattice and the reduction while leaving the
        // region of the plane on screen exactly where it was.
        let mut app = updated(80, 24);
        for _ in 0..5 {
            let _ = app.handle_key(press(KeyCode::Char('+')));
        }
        let _ = app.handle_key(press(KeyCode::Char('l')));
        app.update(Rect::new(0, 0, 80, 24));

        let before = app.params.viewport;
        let (w, h, c) = (before.half_width, before.half_height(), before.centre);

        let _ = app.handle_key(press(KeyCode::Char('m')));
        app.update(Rect::new(0, 0, 80, 24));
        let after = app.params.viewport;

        assert_eq!(after.centre, c, "the view moved");
        assert!((after.half_width - w).abs() < 1e-12, "the width changed");
        assert!(
            (after.half_height() - h).abs() < 1e-12,
            "the height changed: {} vs {h}",
            after.half_height()
        );
        // And it really did change the lattice, or the test proves nothing.
        assert_ne!(after.rows, before.rows);
        assert!((after.sample_aspect - before.sample_aspect / 2.0).abs() < 1e-12);
    }

    #[test]
    fn half_block_resolves_detail_glyph_mode_cannot() {
        // The reason the mode exists: twice the vertical samples. A thin
        // feature that lands between glyph rows should show up in half-block.
        let glyph = updated(60, 20);
        let mut half = updated(60, 20);
        let _ = half.handle_key(press(KeyCode::Char('m')));
        half.update(Rect::new(0, 0, 60, 20));

        assert_eq!(half.samples.rows(), glyph.samples.rows() * 2);
        // Distinct colours are the signal in half-block, since the glyph is
        // constant: two per cell rather than one.
        let colours = |app: &App| {
            app.cells
                .cells()
                .flat_map(|c| {
                    [
                        (c.colour.r, c.colour.g, c.colour.b),
                        (c.background.r, c.background.g, c.background.b),
                    ]
                })
                .collect::<std::collections::BTreeSet<_>>()
                .len()
        };
        assert!(
            colours(&half) > colours(&glyph),
            "half-block {} vs glyph {}",
            colours(&half),
            colours(&glyph)
        );
    }

    #[test]
    fn i_toggles_the_status_line_and_it_is_off_by_default() {
        let mut app = updated(40, 12);
        assert!(!app.hud, "the status line must start hidden");
        let _ = app.handle_key(press(KeyCode::Char('i')));
        assert!(app.hud);
        let _ = app.handle_key(press(KeyCode::Char('i')));
        assert!(!app.hud);
    }

    #[test]
    fn the_status_line_only_warns_when_precision_is_actually_short() {
        // At the home view there is nothing to warn about, and a permanent
        // warning is a warning nobody reads.
        let app = updated(60, 20);
        assert_eq!(app.params.viewport.precision(), Precision::Ample);

        let mut deep = updated(60, 20);
        for _ in 0..400 {
            let _ = deep.handle_key(press(KeyCode::Char('+')));
        }
        deep.update(Rect::new(0, 0, 60, 20));
        assert_ne!(
            deep.params.viewport.precision(),
            Precision::Ample,
            "at the wall the user should be told why zooming stopped"
        );
    }

    #[test]
    fn toggling_the_status_line_never_re_samples() {
        // It is overlaid on the bottom row rather than given a row of its own,
        // precisely so that showing it does not change the sample lattice.
        let mut app = updated(50, 18);
        let before = app.sampled_for;
        let samples = app.samples.clone();
        let _ = app.handle_key(press(KeyCode::Char('i')));
        app.update(Rect::new(0, 0, 50, 18));
        assert_eq!(app.sampled_for, before);
        assert_eq!(app.samples, samples);
    }

    #[test]
    fn every_binding_reports_continue_rather_than_a_command() {
        // `Flow` must not become a command channel: only quitting is a
        // loop-level outcome, and everything else mutates `App` in place.
        let mut app = updated(30, 12);
        for code in [
            KeyCode::Char('h'),
            KeyCode::Char('j'),
            KeyCode::Char('k'),
            KeyCode::Char('l'),
            KeyCode::Left,
            KeyCode::Right,
            KeyCode::Up,
            KeyCode::Down,
            KeyCode::Char('+'),
            KeyCode::Char('='),
            KeyCode::Char('-'),
            KeyCode::Char('.'),
            KeyCode::Char(','),
            KeyCode::Char('r'),
            KeyCode::Tab,
            KeyCode::Char('f'),
            KeyCode::Char('p'),
            KeyCode::Char('m'),
            KeyCode::Char('i'),
        ] {
            assert_eq!(app.handle_key(press(code)), Flow::Continue, "{code:?}");
        }
    }

    #[test]
    fn the_iteration_limit_follows_the_viewport() {
        let app = updated(50, 20);
        assert_eq!(app.params.limit, app.biased_limit());
        assert!(app.params.limit >= 100);
    }
}
