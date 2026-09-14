//! The event loop, the terminal's lifecycle, and the frame.

use std::time::{Duration, Instant};

use frascii_core::{
    Complex, Kernel, Precision, SampleGrid, Viewport, boundary_target, is_interesting,
};
use ratatui::Frame;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::clock::Clock;
use crate::error::Result;
use crate::palette::Palette;
use crate::render::Grid;
use crate::shade::{CellMode, Shader, Supersample, shade_into};

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

/// Palette phase advance, in iterations per second.
///
/// With [`Palette::PERIOD`] at 32 iterations this is a full cycle in a little
/// over five seconds — slow enough to read as drift rather than strobing.
const CYCLE_RATE: f64 = 6.0;

/// How long the Julia parameter takes to travel once around its path.
const ORBIT_PERIOD: Duration = Duration::from_secs(24);

/// The radius of the Julia parameter's path.
///
/// Chosen to stay near the Mandelbrot set's boundary, which is where Julia sets
/// are interesting: well inside and they are a filled disc, well outside and
/// they are dust.
const ORBIT_RADIUS: f64 = 0.7;

/// How much the auto-zoom shrinks the view per second.
///
/// `1/1.2`, so the magnification grows about 1.2× a second and a full dive from
/// the whole set to the `f64` wall takes roughly two and a half minutes. The
/// first version was 0.55 — nearly 1.8× a second, a complete dive in under a
/// minute — which is a fly-through rather than something you can watch: detail
/// resolves and is gone before the eye settles on it. This is an unattended
/// display, so the rate wants to be slow enough to look at.
///
/// Applied geometrically — `factor.powf(dt)`, never `factor * dt` — so the dive
/// advances at the same rate in *plane* terms whatever the frame rate. On a
/// slow machine it takes the same wall-clock time in fewer, chunkier frames
/// rather than slowing down, which matters precisely because nobody is watching
/// the frame counter.
const ZOOM_PER_SECOND: f64 = 1.0 / 1.2;

/// How far the dive descends before choosing a fresh target.
///
/// A target must be re-chosen periodically, not once per dive: a point on the
/// boundary at one scale is not on the boundary several decades down, because
/// the filament it sits on resolves into structure that moves away from it.
/// Diving on a single target renders solid interior within a few decades —
/// measured at 898 blank frames out of 1200 before this existed.
///
/// Eight is the factor the original validation used when it reached 2.1e6 over
/// eight steps without ever losing detail.
const RETARGET_EVERY: f64 = 8.0;

/// The furthest the iteration limit can be biased from the viewport's
/// suggestion, in doublings either way.
///
/// Bounded because the suggestion already tracks depth: the bias is for
/// *taste* — trading detail against frame time — not for reaching depths the
/// automatic value cannot. Unbounded, it would be a way to hang the renderer.
const LIMIT_BIAS_RANGE: i32 = 4;

/// What is moving the view by itself.
///
/// Exclusive, because these fight: an orbit reframes home on every parameter
/// change while a dive is trying to descend, so running both would produce
/// neither.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Drive {
    /// Nothing moves unless a key says so.
    #[default]
    Still,
    /// The Julia parameter travels around a path.
    JuliaOrbit,
    /// Dive toward the boundary forever, resetting at the precision wall.
    AutoZoom,
}

impl Drive {
    /// The drive's name, for a status line.
    const fn name(self) -> &'static str {
        match self {
            Self::Still => "still",
            Self::JuliaOrbit => "orbit",
            Self::AutoZoom => "auto-zoom",
        }
    }
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
    /// Which palette, as an index so `p` can cycle.
    palette_index: usize,
    /// Iteration limit bias, in doublings from the viewport's suggestion.
    limit_bias: i32,
    /// Whether the status bar is shown.
    ///
    /// On by default: it is where the controls are, and a renderer whose
    /// bindings are invisible is one nobody finds the bindings of. `i` hides it
    /// for a full-screen view.
    hud: bool,
    /// Whether the help overlay is shown.
    help: bool,
    /// Animation time.
    clock: Clock,
    /// Whether the palette drifts on its own.
    ///
    /// On at startup. Independent of [`Drive`] rather than a variant of it,
    /// because cycling composes with everything: it changes only the shader, so
    /// it can run during a dive, an orbit, or a still frame.
    cycling: bool,
    /// What is moving the view.
    drive: Drive,
    /// Where the Julia parameter is on its path, in turns.
    orbit_turns: f64,
    /// The magnification at which the dive last chose a target.
    dived_from: f64,
    /// Set when the dive wants a fresh grid to re-target from.
    ///
    /// Two-phase because picking a target needs *samples*: the frame that
    /// resets the view has only the old deep grid, so the choice waits one
    /// frame for the home view to be sampled. The viewer sees the whole set
    /// briefly between dives, which is the right thing to show anyway.
    retarget_pending: bool,
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
                viewport: Viewport::home(
                    0,
                    0,
                    Shader {
                        mode,
                        ..Shader::default()
                    }
                    .sample_aspect(CELL_ASPECT),
                ),
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
            hud: true,
            help: false,
            clock: Clock::new(),
            // Both on by default, because "realtime fractal renderer" is the
            // whole premise and a static first frame does not deliver it.
            //
            // Cycling alone is not enough, and that distinction is the point:
            // it moves the *colour* while the shape stands still, which reads
            // as a tinted photograph rather than a live render. The dive is
            // what makes the geometry move.
            //
            // Touching any navigation key stops the dive — see `take_control`
            // — so this costs an explorer one keypress and gives everyone else
            // the thing the program is for.
            cycling: true,
            drive: Drive::AutoZoom,
            orbit_turns: 0.0,
            dived_from: 1.0,
            retarget_pending: false,
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
            (KeyCode::Char('h') | KeyCode::Left, _) => self.navigate(|a| a.pan(-1, 0)),
            (KeyCode::Char('l') | KeyCode::Right, _) => self.navigate(|a| a.pan(1, 0)),
            (KeyCode::Char('k') | KeyCode::Up, _) => self.navigate(|a| a.pan(0, -1)),
            (KeyCode::Char('j') | KeyCode::Down, _) => self.navigate(|a| a.pan(0, 1)),

            // Zoom. `=` because it is the unshifted `+` on most layouts, so
            // zooming in does not need a modifier.
            (KeyCode::Char('+' | '='), _) => self.navigate(|a| a.zoom(ZOOM_STEP)),
            (KeyCode::Char('-' | '_'), _) => self.navigate(|a| a.zoom(1.0 / ZOOM_STEP)),

            // Iteration limit, biased against the viewport's suggestion.
            (KeyCode::Char('.' | '>'), _) => self.bias_limit(1),
            (KeyCode::Char(',' | '<'), _) => self.bias_limit(-1),

            (KeyCode::Char('r' | 'R'), _) => self.navigate(App::reset),
            (KeyCode::Tab | KeyCode::Char('f'), _) => self.next_kernel(),
            (KeyCode::Char('p' | 'P'), _) => self.next_palette(),
            (KeyCode::Char('m' | 'M'), _) => self.next_mode(),
            (KeyCode::Char('s' | 'S'), _) => self.next_supersample(),
            (KeyCode::Char('i' | 'I'), _) => self.hud = !self.hud,
            (KeyCode::Char('?'), _) => self.help = !self.help,
            (KeyCode::Char('c'), _) => self.cycling = !self.cycling,
            (KeyCode::Char('o' | 'O'), _) => self.toggle_drive(Drive::JuliaOrbit),
            (KeyCode::Char('z' | 'Z'), _) => self.toggle_drive(Drive::AutoZoom),
            (KeyCode::Char(' '), _) => self.clock.toggle_pause(),

            _ => {}
        }
        Flow::Continue
    }

    /// Run a manual navigation, taking control from whatever was driving.
    ///
    /// Panning or zooming while the dive is descending would be a fight the
    /// user cannot win — the next frame moves the view back. So touching a
    /// navigation key stops the driver, which is also the only sensible reading
    /// of the input: you would not press `h` unless you wanted to steer.
    /// Palette cycling is left running, since it does not move the view.
    fn navigate(&mut self, action: impl FnOnce(&mut Self)) {
        self.drive = Drive::Still;
        action(self);
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

    /// Turn a drive on, or back to still if it was already running.
    ///
    /// Switching to the orbit also switches to Julia: orbiting a parameter the
    /// current fractal does not have would look like the key did nothing.
    fn toggle_drive(&mut self, drive: Drive) {
        self.drive = if self.drive == drive {
            Drive::Still
        } else {
            drive
        };

        if self.drive == Drive::JuliaOrbit && !self.params.kernel.same_kind(julia_at(0.0)) {
            self.params.kernel = julia_at(self.orbit_turns);
            self.reset();
        }
    }

    /// Advance whatever is moving by `dt`.
    fn advance(&mut self, dt: Duration) {
        let seconds = dt.as_secs_f64();

        if self.cycling {
            // Wrapped every frame. Left to accumulate, the value eventually
            // dwarfs the increment and cycling first stutters, then stalls —
            // a failure that only shows after a day of the unattended mode.
            self.shader.phase = (self.shader.phase + seconds * CYCLE_RATE) % Palette::PERIOD;
        }

        match self.drive {
            Drive::Still => {}
            Drive::JuliaOrbit => {
                self.orbit_turns =
                    (self.orbit_turns + seconds / ORBIT_PERIOD.as_secs_f64()).fract();
                self.params.kernel = julia_at(self.orbit_turns);
            }
            Drive::AutoZoom => self.dive(seconds),
        }
    }

    /// One frame of the unattended dive.
    ///
    /// Geometric in `dt`, so the descent covers the same plane distance per
    /// second whatever the frame rate.
    fn dive(&mut self, seconds: f64) {
        // Stop at `Marginal`, not at the hard clamp. Diving to the clamp would
        // show several visibly mushy frames before *every* cut — forever, on a
        // loop — where stopping a couple of decades early keeps every frame
        // sharp. The clamp still exists to refuse a manual zoom.
        if self.params.viewport.precision() != Precision::Ample {
            self.params.viewport = Viewport::home(
                self.params.viewport.cols,
                self.params.viewport.rows,
                self.params.viewport.sample_aspect,
            );
            self.limit_bias = 0;
            self.dived_from = 1.0;
            self.retarget_pending = true;
            return;
        }

        self.params
            .viewport
            .zoom_centre(ZOOM_PER_SECOND.powf(seconds));

        // Re-aim periodically. Without this the dive holds one target all the
        // way down and ends up inside a solid region — the boundary it aimed at
        // is no longer where it was several decades ago.
        if self.params.viewport.magnification() / self.dived_from >= RETARGET_EVERY {
            self.retarget_pending = true;
        }
    }

    /// Aim the dive at somewhere that will still hold detail, if the current
    /// grid offers one.
    ///
    /// Called after sampling, so it reads the grid just produced. A view with
    /// no boundary in it — solid interior or empty exterior — leaves the flag
    /// set and tries again next frame rather than diving into a flat field.
    fn retarget(&mut self) {
        if !is_interesting(&self.samples) {
            return;
        }
        if let Some(point) = boundary_target(&self.samples, &self.params.viewport) {
            self.params.viewport.centre = point;
            self.dived_from = self.params.viewport.magnification();
            self.retarget_pending = false;
        }
    }

    /// Cycle the supersampling factor.
    ///
    /// Re-samples, and must: the factor multiplies the lattice. It costs `k²`
    /// times the samples, which is why it stops at 3× — the status line shows
    /// the frame's iteration limit so the trade is visible while making it.
    fn next_supersample(&mut self) {
        self.shader.supersample = self.shader.supersample.next();
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
    pub fn update(&mut self, area: Rect, now: Instant) {
        let dt = self.clock.tick(now);
        self.advance(dt);

        let canvas = self.canvas_area(area);
        let (cols, rows) = self
            .shader
            .lattice(usize::from(canvas.width), usize::from(canvas.height));

        if self.params.viewport.cols != cols || self.params.viewport.rows != rows {
            self.params
                .viewport
                .resize(cols, rows, self.shader.sample_aspect(CELL_ASPECT));
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

            // After sampling, so the choice reads the grid just produced.
            if self.retarget_pending {
                self.retarget();
            }
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
        frame.render_widget(&self.cells, self.canvas_area(area));

        if let Some(row) = self.bar_area(area) {
            // `Clear` first: a `Paragraph`'s style recolours the row but only
            // overwrites the cells its text occupies, so without this whatever
            // was underneath shows through to the right of the text — which
            // reads as corruption rather than as a bar.
            frame.render_widget(Clear, row);
            frame.render_widget(self.status_bar(row.width), row);
        }

        if self.help {
            let overlay = centred(area, 46, 16);
            frame.render_widget(Clear, overlay);
            frame.render_widget(help_overlay(), overlay);
        }
    }

    /// The part of the terminal the fractal is drawn into.
    ///
    /// The bar takes a row rather than being overlaid on one, so nothing shows
    /// through behind it and the sample lattice matches what is visible.
    /// Toggling the bar therefore re-samples — which is the honest cost of the
    /// picture actually changing size.
    fn canvas_area(&self, area: Rect) -> Rect {
        match self.bar_area(area) {
            Some(_) => Rect {
                height: area.height - 1,
                ..area
            },
            None => area,
        }
    }

    /// Where the status bar goes, if there is room for it.
    ///
    /// `None` in a terminal one row tall: a bar that consumed the only row
    /// would leave nowhere for the fractal, which is the wrong trade.
    fn bar_area(&self, area: Rect) -> Option<Rect> {
        (self.hud && area.height > 1)
            .then(|| Rect::new(area.x, area.y + area.height - 1, area.width, 1))
    }

    /// The status bar: what the renderer is doing, and how to drive it.
    ///
    /// Both halves matter. The state is how a regression gets noticed while
    /// using the thing rather than three weeks later — cross-check the
    /// magnification and limit against `--headless` at the same size and depth.
    /// The controls are there because a renderer whose bindings are invisible
    /// is one nobody finds the bindings of.
    fn status_bar(&self, width: u16) -> Paragraph<'static> {
        let dim = Style::default().fg(Color::Rgb(0x80, 0x80, 0x88));
        let bright = Style::default()
            .fg(Color::Rgb(0xe8, 0xe8, 0xe8))
            .add_modifier(Modifier::BOLD);
        let motion_style = Style::default().fg(Color::Rgb(0x60, 0xc0, 0x90));
        let key = Style::default().fg(Color::Rgb(0xc8, 0xb0, 0x60));

        let state = self.state_text();
        let motion = self.motion_text();
        // The hints get whatever the state leaves, with two spaces between.
        let used = state.chars().count() + motion.chars().count() + 2;
        let hints = hints_for(usize::from(width).saturating_sub(used + 2));

        let mut spans = vec![Span::styled(state, bright)];
        if !motion.is_empty() {
            spans.push(Span::styled(format!("  {motion}"), motion_style));
        }
        if !hints.is_empty() {
            // Right-aligned by padding, so the two halves do not jiggle against
            // each other as the magnification's digits change.
            let pad = usize::from(width).saturating_sub(used + hints.chars().count());
            spans.push(Span::styled(" ".repeat(pad), dim));
            spans.push(Span::styled(hints.to_owned(), key));
        }

        Paragraph::new(Line::from(spans)).style(Style::default().bg(Color::Rgb(0x14, 0x14, 0x1c)))
    }

    /// The left half: what is being rendered.
    fn state_text(&self) -> String {
        let viewport = &self.params.viewport;
        let mut text = format!(
            "{}  {}  {}",
            self.params.kernel.name(),
            self.shader.mode.name(),
            self.shader.palette.name()
        );
        if self.shader.supersample != Supersample::Off {
            text.push_str(&format!("  ss{}", self.shader.supersample.name()));
        }
        text.push_str(&format!(
            "  {:.3e}x  iter {}",
            viewport.magnification(),
            self.params.limit
        ));
        // Only when it means something: at `Ample` there is nothing to warn
        // about, and a permanent warning is one nobody reads.
        //
        // "max zoom" keys off the clamp rather than off `Exhausted`, because
        // `Exhausted` is unreachable by zooming: the clamp refuses to go below
        // `min_half_width`, which sits exactly at the `Marginal` boundary. An
        // earlier version tested for `Exhausted` here and so never said
        // anything at the one moment the user most needs told.
        if viewport.half_width <= viewport.min_half_width() {
            text.push_str("  max zoom");
        } else if viewport.precision() != Precision::Ample {
            text.push_str("  precision: marginal");
        }
        text
    }

    /// What is currently moving, if anything.
    fn motion_text(&self) -> String {
        let mut parts = Vec::new();
        if self.clock.is_paused() {
            parts.push("paused");
        }
        if self.drive != Drive::Still {
            parts.push(self.drive.name());
        }
        if self.cycling {
            parts.push("cycling");
        }
        parts.join(" ")
    }

    /// Own the terminal until the user quits.
    ///
    /// Restoration is the caller's, via [`crate::run`]: a `run` that both
    /// initialised and restored the terminal could not be driven by a test
    /// harness's backend, and one that restored on the happy path only would
    /// leave a raw-mode terminal behind on the first error.
    pub fn run(&mut self, terminal: &mut crate::BufferedTerminal) -> Result<()> {
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
            self.update(area, Instant::now());
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

/// The control hints that fit in `width` columns.
///
/// Tiered rather than truncated mid-word: a hint cut off halfway is worse than
/// one that is absent, because it reads as a rendering bug. Each tier is a
/// complete, useful set, and the shortest still points at the full list.
fn hints_for(width: usize) -> &'static str {
    const TIERS: &[&str] = &[
        "hjkl pan  +/- zoom  z dive  p palette  m mode  s ss  ? help",
        "hjkl  +/-  z dive  p palette  ? help",
        "hjkl  +/-  z  p  ? help",
        "? help",
    ];
    TIERS
        .iter()
        .copied()
        .find(|tier| tier.chars().count() <= width)
        .unwrap_or("")
}

/// The full binding list.
fn help_overlay() -> Paragraph<'static> {
    let key = Style::default().fg(Color::Rgb(0xc8, 0xb0, 0x60));
    let text = Style::default().fg(Color::Rgb(0xd8, 0xd8, 0xd8));
    let row = |k: &'static str, what: &'static str| {
        Line::from(vec![
            Span::styled(format!(" {k:<12}"), key),
            Span::styled(what, text),
        ])
    };

    Paragraph::new(vec![
        row("h j k l", "pan (also arrows)"),
        row("+ / -", "zoom in / out"),
        row(". / ,", "iteration limit"),
        row("r", "reset the view"),
        row("Tab / f", "next fractal"),
        row("p", "next palette"),
        row("m", "glyph / half-block"),
        row("s", "supersampling 1x/2x/3x"),
        row("z", "auto-zoom (dives forever)"),
        row("o", "Julia parameter orbit"),
        row("c", "palette cycling"),
        row("Space", "pause motion"),
        row("i", "status bar"),
        row("q / Esc", "quit"),
    ])
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(" controls ")
            .style(Style::default().bg(Color::Rgb(0x14, 0x14, 0x1c))),
    )
}

/// A `width` x `height` rectangle centred in `area`, clamped to fit.
///
/// Clamped rather than asserted: a terminal smaller than the overlay is a
/// normal thing to encounter, and the box should shrink rather than the
/// arithmetic underflow.
fn centred(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    }
}

/// The Julia parameter at a point on its circular path.
///
/// A circle near the Mandelbrot boundary rather than an arbitrary sweep: that
/// is the band where Julia sets have structure, so the whole orbit is worth
/// watching instead of just the part that crosses it.
fn julia_at(turns: f64) -> Kernel {
    let theta = turns * std::f64::consts::TAU;
    Kernel::Julia {
        c: Complex::new(ORBIT_RADIUS * theta.cos(), ORBIT_RADIUS * theta.sin()),
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

    /// An app sized and sampled once, with all motion stopped.
    ///
    /// Parked by default because most of these tests are about a single frame,
    /// and the shipped defaults move: a dive would change the view between
    /// updates and make every assertion about "the same frame" a race. Motion
    /// tests turn what they need back on.
    fn updated(width: u16, height: u16) -> App {
        let mut app = App::new();
        app.drive = Drive::Still;
        app.cycling = false;
        app.update(Rect::new(0, 0, width, height), Instant::now());
        app
    }

    /// An app with the shipped defaults, for testing what a user actually gets.
    fn launched(width: u16, height: u16) -> App {
        let mut app = App::new();
        app.update(Rect::new(0, 0, width, height), Instant::now());
        app
    }

    /// Drive the app for `frames` frames of exactly `step`, from a fixed
    /// origin.
    ///
    /// Controlled time rather than wall time: an animation test that depended
    /// on how long the test process happened to take would be flaky, and the
    /// rates here are all per-second.
    fn animate(app: &mut App, area: Rect, frames: u32, step: Duration) {
        let start = Instant::now();
        for i in 0..=frames {
            app.update(area, start + step * i);
        }
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
        // One row shorter than the terminal: the status bar takes it.
        let app = updated(60, 20);
        assert_eq!((app.cells.width(), app.cells.height()), (60, 19));
        assert_eq!((app.samples.cols(), app.samples.rows()), (60, 19));
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
        app.update(Rect::new(0, 0, 40, 15), Instant::now());
        assert_eq!(app.sampled_for, params);
        assert_eq!(app.samples, before);
    }

    #[test]
    fn a_resize_does_re_sample() {
        let mut app = updated(40, 15);
        let before = app.sampled_for;
        app.update(Rect::new(0, 0, 41, 15), Instant::now());
        assert_ne!(app.sampled_for, before, "a resize must re-sample");
        // One row goes to the status bar.
        assert_eq!((app.cells.width(), app.cells.height()), (41, 14));
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
        app.update(Rect::new(0, 0, 30, 12), Instant::now());

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
        deep.update(Rect::new(0, 0, 60, 20), Instant::now());

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
        app.update(Rect::new(0, 0, 50, 20), Instant::now());
        assert!(app.params.limit > automatic, "`.` must raise the limit");

        let _ = app.handle_key(press(KeyCode::Char(',')));
        let _ = app.handle_key(press(KeyCode::Char(',')));
        app.update(Rect::new(0, 0, 50, 20), Instant::now());
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
        app.update(Rect::new(0, 0, 50, 20), Instant::now());
        let shallow = app.params.limit;

        for _ in 0..20 {
            let _ = app.handle_key(press(KeyCode::Char('+')));
        }
        app.update(Rect::new(0, 0, 50, 20), Instant::now());
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

        let home = Viewport::home(60, 19, app.params.viewport.sample_aspect);
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
        app.update(Rect::new(0, 0, 50, 20), Instant::now());
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
        app.update(Rect::new(0, 0, 40, 15), Instant::now());

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
        app.update(Rect::new(0, 0, 40, 15), Instant::now());
        assert_ne!(app.sampled_for, before, "a pan must re-sample");

        let after_pan = app.sampled_for;
        let _ = app.handle_key(press(KeyCode::Char('p')));
        app.update(Rect::new(0, 0, 40, 15), Instant::now());
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
        app.update(Rect::new(0, 0, 40, 16), Instant::now());
        assert!(
            app.cells.cells().all(|c| c.glyph == '▀'),
            "half-block should be one glyph"
        );
        // Twice the samples, same cells. 15 rows, not 16: the bar takes one.
        assert_eq!((app.cells.width(), app.cells.height()), (40, 15));
        assert_eq!((app.samples.cols(), app.samples.rows()), (40, 30));

        let _ = app.handle_key(press(KeyCode::Char('m')));
        app.update(Rect::new(0, 0, 40, 16), Instant::now());
        assert_eq!((app.samples.cols(), app.samples.rows()), (40, 15));
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
        app.update(Rect::new(0, 0, 80, 24), Instant::now());

        let before = app.params.viewport;
        let (w, h, c) = (before.half_width, before.half_height(), before.centre);

        let _ = app.handle_key(press(KeyCode::Char('m')));
        app.update(Rect::new(0, 0, 80, 24), Instant::now());
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
        half.update(Rect::new(0, 0, 60, 20), Instant::now());

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
    fn the_status_bar_is_shown_by_default_and_i_hides_it() {
        // On by default because it is where the controls are, and bindings
        // nobody can see are bindings nobody finds.
        let mut app = updated(40, 12);
        assert!(app.hud, "the status bar should start visible");
        let _ = app.handle_key(press(KeyCode::Char('i')));
        assert!(!app.hud);
        let _ = app.handle_key(press(KeyCode::Char('i')));
        assert!(app.hud);
    }

    #[test]
    fn hiding_the_bar_gives_its_row_back_to_the_fractal() {
        let area = Rect::new(0, 0, 40, 12);
        let mut app = updated(40, 12);
        assert_eq!(app.cells.height(), 11);

        let _ = app.handle_key(press(KeyCode::Char('i')));
        app.update(area, Instant::now());
        assert_eq!(app.cells.height(), 12, "the row was not returned");
    }

    #[test]
    fn the_bar_is_dropped_rather_than_taking_the_only_row() {
        // In a one-row terminal a bar would leave nowhere for the fractal.
        let app = updated(20, 1);
        assert_eq!(app.bar_area(Rect::new(0, 0, 20, 1)), None);
        assert_eq!(app.canvas_area(Rect::new(0, 0, 20, 1)).height, 1);
    }

    #[test]
    fn the_status_bar_shows_the_state_and_the_controls() {
        let app = updated(100, 20);
        let state = app.state_text();
        assert!(state.contains("mandelbrot"), "{state}");
        assert!(state.contains("glyph"), "{state}");
        assert!(state.contains("iter"), "{state}");
        // And the hints are there when there is room.
        assert!(hints_for(80).contains("? help"));
        assert!(hints_for(80).contains("hjkl"));
    }

    #[test]
    fn the_hints_degrade_by_tier_rather_than_truncating_mid_word() {
        // A hint cut off halfway reads as a rendering bug, so each tier is a
        // complete set and the narrowest still points at the full list.
        let mut previous = usize::MAX;
        for width in [80, 50, 30, 10, 6, 0] {
            let hint = hints_for(width);
            assert!(hint.chars().count() <= width, "width {width} got {hint:?}");
            assert!(hint.chars().count() <= previous, "tiers must shrink");
            previous = hint.chars().count();
            if !hint.is_empty() {
                assert!(hint.contains("? help"), "width {width}: {hint:?}");
                assert!(!hint.ends_with(' '), "width {width}: trailing space");
            }
        }
        assert_eq!(hints_for(0), "", "nothing fits in no columns");
    }

    #[test]
    fn the_state_only_mentions_precision_when_it_is_short() {
        let app = updated(60, 20);
        assert!(!app.state_text().contains("precision"));
        assert!(!app.state_text().contains("max zoom"));

        let mut deep = updated(60, 20);
        for _ in 0..400 {
            let _ = deep.handle_key(press(KeyCode::Char('+')));
        }
        deep.update(Rect::new(0, 0, 60, 20), Instant::now());
        assert!(
            deep.state_text().contains("max zoom"),
            "{}",
            deep.state_text()
        );
    }

    #[test]
    fn the_motion_text_names_only_what_is_running() {
        let app = updated(30, 10);
        assert_eq!(app.motion_text(), "", "nothing is moving");

        let mut live = launched(30, 10);
        assert_eq!(live.motion_text(), "auto-zoom cycling");
        let _ = live.handle_key(press(KeyCode::Char(' ')));
        assert!(live.motion_text().starts_with("paused"));
    }

    #[test]
    fn question_mark_toggles_the_help_overlay() {
        let mut app = updated(40, 14);
        assert!(!app.help, "help should start closed");
        let _ = app.handle_key(press(KeyCode::Char('?')));
        assert!(app.help);
        let _ = app.handle_key(press(KeyCode::Char('?')));
        assert!(!app.help);
    }

    #[test]
    fn the_help_overlay_is_clamped_into_a_small_terminal() {
        // Smaller than the box is normal; it must shrink rather than underflow.
        let area = Rect::new(0, 0, 10, 4);
        let overlay = centred(area, 46, 16);
        assert_eq!((overlay.width, overlay.height), (10, 4));
        assert_eq!((overlay.x, overlay.y), (0, 0));
    }

    #[test]
    fn the_help_overlay_is_centred_in_a_large_terminal() {
        let overlay = centred(Rect::new(0, 0, 100, 40), 46, 16);
        assert_eq!((overlay.width, overlay.height), (46, 16));
        assert_eq!((overlay.x, overlay.y), (27, 12));
    }

    #[test]
    fn showing_help_does_not_disturb_the_render() {
        // It is an overlay, not a layout change: the sample grid must be
        // untouched, or opening help would re-render the fractal.
        let area = Rect::new(0, 0, 50, 18);
        let mut app = updated(50, 18);
        let samples = app.samples.clone();
        let _ = app.handle_key(press(KeyCode::Char('?')));
        app.update(area, Instant::now());
        assert_eq!(app.samples, samples);
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
        deep.update(Rect::new(0, 0, 60, 20), Instant::now());
        assert_ne!(
            deep.params.viewport.precision(),
            Precision::Ample,
            "at the wall the user should be told why zooming stopped"
        );
    }

    #[test]
    fn toggling_the_bar_re_samples_because_the_picture_changes_size() {
        // The bar takes a row rather than being overlaid, so the fractal really
        // does get bigger when it is hidden. Re-sampling is the honest cost of
        // that, and asserting it here stops anyone "optimising" the toggle by
        // overlaying the bar again — which would put the fractal behind it.
        let mut app = updated(50, 18);
        let before = app.sampled_for;
        let _ = app.handle_key(press(KeyCode::Char('i')));
        app.update(Rect::new(0, 0, 50, 18), Instant::now());
        assert_ne!(app.sampled_for, before);
    }

    #[test]
    fn palette_cycling_moves_the_colours_and_never_the_samples() {
        // The cheapest motion mode, and its whole claim: no kernel work. If
        // this fails, cycling has become the most expensive mode rather than
        // the least.
        let area = Rect::new(0, 0, 40, 15);
        let mut app = updated(40, 15);
        app.cycling = true;
        let samples = app.samples.clone();
        let cells = app.cells.clone();
        let sampled_for = app.sampled_for;

        animate(&mut app, area, 20, Duration::from_millis(33));

        assert_eq!(app.samples, samples, "cycling re-sampled");
        assert_eq!(app.sampled_for, sampled_for, "cycling touched the params");
        assert_ne!(app.cells, cells, "the colours did not move");
    }

    #[test]
    fn the_cycling_phase_stays_bounded_however_long_it_runs() {
        // Unwrapped, the phase eventually dwarfs its own increment and cycling
        // stutters then stalls — a failure that only appears after a day of the
        // unattended mode running.
        let area = Rect::new(0, 0, 20, 8);
        let mut app = updated(20, 8);
        app.cycling = true;
        // Twelve hours of animation, at a second per frame.
        animate(&mut app, area, 43_200, Duration::from_secs(1));
        assert!(
            app.shader.phase >= 0.0 && app.shader.phase < Palette::PERIOD,
            "phase escaped its period: {}",
            app.shader.phase
        );
    }

    #[test]
    fn pausing_stops_the_animation_and_resuming_does_not_lurch() {
        let area = Rect::new(0, 0, 30, 10);
        let mut app = updated(30, 10);
        app.cycling = true;
        animate(&mut app, area, 5, Duration::from_millis(33));
        let phase = app.shader.phase;

        let _ = app.handle_key(press(KeyCode::Char(' ')));
        assert!(app.clock.is_paused());
        // A long pause must contribute nothing.
        animate(&mut app, area, 3, Duration::from_secs(10));
        assert_eq!(app.shader.phase, phase, "the phase moved while paused");

        let _ = app.handle_key(press(KeyCode::Char(' ')));
        animate(&mut app, area, 1, Duration::from_millis(33));
        let moved = app.shader.phase - phase;
        assert!(
            moved > 0.0 && moved < 1.0,
            "resumed with a lurch of {moved} iterations"
        );
    }

    #[test]
    fn the_julia_orbit_moves_the_parameter_and_switches_fractal() {
        // Orbiting a parameter the current fractal does not have would look
        // like the key did nothing, so `o` switches to Julia too.
        let area = Rect::new(0, 0, 40, 14);
        let mut app = updated(40, 14);
        assert_eq!(app.kernel_name(), "mandelbrot");

        let _ = app.handle_key(press(KeyCode::Char('o')));
        assert_eq!(app.kernel_name(), "julia");
        assert_eq!(app.drive, Drive::JuliaOrbit);

        let first = app.params.kernel;
        animate(&mut app, area, 30, Duration::from_millis(100));
        assert_ne!(app.params.kernel, first, "the parameter did not move");
        assert!(app.params.kernel.same_kind(first), "it left julia");
    }

    #[test]
    fn the_orbit_stays_on_its_path_forever() {
        // The path is a circle near the Mandelbrot boundary, which is the band
        // where Julia sets have structure. Drifting off it — through
        // accumulated error, or a `fract` that was forgotten — would end in
        // dust or a filled disc.
        let area = Rect::new(0, 0, 24, 8);
        let mut app = updated(24, 8);
        let _ = app.handle_key(press(KeyCode::Char('o')));
        // Fifty laps.
        animate(&mut app, area, 1_200, Duration::from_secs(1));

        let Kernel::Julia { c } = app.params.kernel else {
            panic!("left julia");
        };
        let radius = c.norm_sqr().sqrt();
        assert!(
            (radius - ORBIT_RADIUS).abs() < 1e-9,
            "drifted off the path: radius {radius}"
        );
        assert!(app.orbit_turns >= 0.0 && app.orbit_turns < 1.0);
    }

    #[test]
    fn auto_zoom_descends_and_the_rate_is_geometric_in_time() {
        // Geometric, not linear: the same wall-clock time must cover the same
        // *plane* distance whatever the frame rate, so an unattended dive looks
        // the same on a slow machine in fewer, chunkier frames.
        let area = Rect::new(0, 0, 40, 14);

        let mut fast = updated(40, 14);
        fast.drive = Drive::AutoZoom;
        animate(&mut fast, area, 60, Duration::from_millis(50));

        let mut slow = updated(40, 14);
        slow.drive = Drive::AutoZoom;
        animate(&mut slow, area, 10, Duration::from_millis(300));

        assert!(fast.magnification() > 1.0, "the dive did not descend");
        let ratio = fast.magnification() / slow.magnification();
        assert!(
            (0.97..1.03).contains(&ratio),
            "three seconds of diving differed by frame rate: {:.4e} vs {:.4e}",
            fast.magnification(),
            slow.magnification()
        );
    }

    #[test]
    fn auto_zoom_runs_unattended_without_stalling_or_going_blank() {
        // The mode's whole promise. It must dive, hit the precision limit,
        // reset, re-target and dive again — indefinitely — and never sit on a
        // featureless frame.
        let area = Rect::new(0, 0, 50, 18);
        let mut app = updated(50, 18);
        app.drive = Drive::AutoZoom;

        // Long enough for at least two complete dives: at ~1.2x magnification
        // a second, one dive to the wall takes about three minutes, so this is
        // roughly seven minutes of simulated time. Tied to the rate rather than
        // a round number, because a slower dive would otherwise silently stop
        // testing the reset at all.
        let start = Instant::now();
        let step = Duration::from_millis(100);
        let mut resets = 0;
        let mut previous = app.magnification();
        let mut featureless = 0;

        for i in 0..=4_200 {
            app.update(area, start + step * i);
            if app.magnification() < previous {
                resets += 1;
            }
            previous = app.magnification();
            // A frame is allowed to be featureless only while re-targeting.
            if !is_interesting(&app.samples) && !app.retarget_pending {
                featureless += 1;
            }
        }

        assert!(
            resets >= 2,
            "only {resets} resets in seven minutes of diving"
        );
        assert!(app.magnification() > 1.0, "it ended up stalled at home");
        assert!(
            featureless < 200,
            "{featureless} frames were featureless outside a re-target"
        );
    }

    #[test]
    fn auto_zoom_stops_short_of_the_hard_clamp() {
        // It resets at `Marginal`, not at the clamp: diving all the way would
        // show several visibly mushy frames before every cut, forever.
        let area = Rect::new(0, 0, 40, 14);
        let mut app = updated(40, 14);
        app.drive = Drive::AutoZoom;

        let start = Instant::now();
        let step = Duration::from_millis(100);
        // Two minutes of simulated diving, which at ~1.2x a second reaches
        // about 1e9 — deep enough to prove it descends without reaching the
        // clamp this test exists to check it avoids.
        let mut deepest = 1.0_f64;
        for i in 0..=1_400 {
            app.update(area, start + step * i);
            deepest = deepest.max(app.magnification());
            assert_ne!(
                app.params.viewport.precision(),
                Precision::Exhausted,
                "the dive reached the hard clamp"
            );
        }
        assert!(deepest > 1e6, "it never got deep: {deepest:.3e}");
    }

    #[test]
    fn the_drives_are_exclusive_and_toggle_off() {
        // An orbit reframes home on every parameter change while a dive is
        // descending, so running both would produce neither.
        let mut app = launched(30, 10);
        assert_eq!(app.drive, Drive::AutoZoom, "the shipped default");
        let _ = app.handle_key(press(KeyCode::Char('o')));
        assert_eq!(app.drive, Drive::JuliaOrbit);
        let _ = app.handle_key(press(KeyCode::Char('o')));
        assert_eq!(app.drive, Drive::Still, "a second press must turn it off");
    }

    #[test]
    fn cycling_composes_with_a_drive_rather_than_replacing_it() {
        // It changes only the shader, so it can run during a dive.
        let area = Rect::new(0, 0, 30, 10);
        let mut app = launched(30, 10);
        animate(&mut app, area, 20, Duration::from_millis(50));
        assert!(app.cycling && app.drive == Drive::AutoZoom);
        assert!(app.shader.phase > 0.0, "the palette stopped cycling");
        assert!(app.magnification() > 1.0, "the dive stopped");
    }

    #[test]
    fn a_still_app_does_not_move_on_its_own() {
        // With cycling off and no drive, an idle frame must be identical — or
        // the renderer would burn the kernel on a static picture forever.
        let area = Rect::new(0, 0, 30, 10);
        let mut app = updated(30, 10);
        assert!(!app.cycling && app.drive == Drive::Still);
        let cells = app.cells.clone();
        let sampled_for = app.sampled_for;
        animate(&mut app, area, 30, Duration::from_millis(33));
        assert_eq!(app.cells, cells);
        assert_eq!(app.sampled_for, sampled_for);
    }

    #[test]
    fn s_cycles_supersampling_and_multiplies_the_lattice() {
        let area = Rect::new(0, 0, 40, 16);
        let mut app = updated(40, 16);
        // 15 rows of fractal; the bar has the sixteenth.
        assert_eq!((app.samples.cols(), app.samples.rows()), (40, 15));

        let _ = app.handle_key(press(KeyCode::Char('s')));
        app.update(area, Instant::now());
        assert_eq!(app.shader.supersample, Supersample::X2);
        assert_eq!((app.samples.cols(), app.samples.rows()), (80, 30));
        // The cell grid is unchanged — it is the *samples* that multiply.
        assert_eq!((app.cells.width(), app.cells.height()), (40, 15));

        let _ = app.handle_key(press(KeyCode::Char('s')));
        app.update(area, Instant::now());
        assert_eq!((app.samples.cols(), app.samples.rows()), (120, 45));

        let _ = app.handle_key(press(KeyCode::Char('s')));
        app.update(area, Instant::now());
        assert_eq!(app.shader.supersample, Supersample::Off, "must wrap");
    }

    #[test]
    fn supersampling_does_not_move_the_view() {
        // Same invariance as the mode toggle, and for the same reason: the
        // factor multiplies both lattice axes, so the plane region is
        // untouched. Turning it on should sharpen the picture, not reframe it.
        let area = Rect::new(0, 0, 60, 20);
        let mut app = updated(60, 20);
        for _ in 0..4 {
            let _ = app.handle_key(press(KeyCode::Char('+')));
        }
        app.update(area, Instant::now());
        let before = app.params.viewport;

        let _ = app.handle_key(press(KeyCode::Char('s')));
        app.update(area, Instant::now());
        let after = app.params.viewport;

        assert_eq!(after.centre, before.centre);
        assert!((after.half_width - before.half_width).abs() < 1e-12);
        assert!((after.half_height() - before.half_height()).abs() < 1e-12);
        assert_ne!(after.cols, before.cols, "the lattice should have grown");
    }

    #[test]
    fn a_fresh_launch_moves_the_shape_not_just_the_colours() {
        // What the user actually gets, and the reason the defaults are what
        // they are: cycling alone moves the colour while the geometry stands
        // still, which reads as a tinted photograph rather than a live render.
        let area = Rect::new(0, 0, 50, 18);
        let mut app = launched(50, 18);
        assert!(app.cycling, "colour should drift");
        assert_eq!(app.drive, Drive::AutoZoom, "the shape should move");

        let magnification = app.magnification();
        let glyphs: Vec<char> = app.cells.cells().map(|c| c.glyph).collect();
        animate(&mut app, area, 20, Duration::from_millis(50));

        assert!(
            app.magnification() > magnification,
            "the view did not descend"
        );
        let after: Vec<char> = app.cells.cells().map(|c| c.glyph).collect();
        assert_ne!(glyphs, after, "the shape did not change, only the colour");
    }

    #[test]
    fn navigating_takes_control_from_the_automatic_drive() {
        // Panning while the dive descends would be a fight the user cannot
        // win: the next frame moves the view back. Any navigation key stops it.
        for code in [
            KeyCode::Char('h'),
            KeyCode::Char('l'),
            KeyCode::Char('j'),
            KeyCode::Char('k'),
            KeyCode::Left,
            KeyCode::Char('+'),
            KeyCode::Char('-'),
            KeyCode::Char('r'),
        ] {
            let mut app = launched(30, 12);
            assert_eq!(app.drive, Drive::AutoZoom);
            let _ = app.handle_key(press(code));
            assert_eq!(app.drive, Drive::Still, "{code:?} did not take control");
            // Cycling is left alone: it does not move the view.
            assert!(app.cycling, "{code:?} stopped the palette too");
        }
    }

    #[test]
    fn a_manual_pan_actually_moves_after_taking_control() {
        // Taking control must not swallow the keypress that took it.
        let mut app = launched(40, 14);
        let before = app.params.viewport.centre.re;
        let _ = app.handle_key(press(KeyCode::Char('l')));
        assert!(app.params.viewport.centre.re > before, "the pan was lost");
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
            KeyCode::Char('s'),
            KeyCode::Char('?'),
            KeyCode::Char('c'),
            KeyCode::Char('o'),
            KeyCode::Char('z'),
            KeyCode::Char(' '),
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
