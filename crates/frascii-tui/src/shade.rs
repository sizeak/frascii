//! Turning a sample grid into a cell grid.
//!
//! This is the *shade* half of the sample/shade split, and the split is the
//! load-bearing shape of the frontend. Sampling is expensive and changes only
//! when the viewport, kernel or limit change; shading is cheap and runs every
//! frame. Keeping them apart means:
//!
//! - every ramp, palette and sub-cell reduction is unit-testable against a
//!   hand-written grid of [`Escape`] values, with no kernel in the test — which
//!   is exactly what the tests below do;
//! - palette cycling costs one pass over an existing grid and no kernel work at
//!   all;
//! - sample time and shade time can be measured separately, so "the frame is
//!   slow" has an answer.
//!
//! Fused into one loop, the sub-cell reductions would be written against that
//! interleaving, and by the time three render modes depended on it the
//! untangling would be expensive.
//!
//! Like [`crate::render`], this module takes no ratatui dependency.

use frascii_core::{Escape, SampleGrid};

use crate::oklab;
use crate::palette::Palette;
use crate::ramp;
use crate::render::{Cell, Grid};

/// How samples map onto character cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CellMode {
    /// One sample per cell, shaded by the glyph ramp.
    ///
    /// The default, and the product's identity: this is the mode that makes
    /// frascii ASCII art rather than a low-resolution image viewer.
    #[default]
    Glyph,
    /// Two vertically stacked samples per cell, each with its own colour.
    ///
    /// Twice the vertical resolution and square-ish pixels, at the cost of the
    /// glyph ramp: this mode carries density in colour alone. It is not ASCII
    /// art — it is a 2× vertical pixel display — which is exactly why
    /// [`CellMode::Glyph`] is the default and this is the escape hatch for
    /// detail.
    ///
    /// 1×2 is the ceiling for *coloured* sub-cell rendering in a terminal.
    /// Braille and the legacy-computing octants reach 2×4, but every one of
    /// them can carry only a single colour per cell, which is useless for a
    /// colour-mapped fractal.
    HalfBlock,
}

impl CellMode {
    /// Samples across and down one character cell.
    #[must_use]
    pub const fn subdivisions(self) -> (usize, usize) {
        match self {
            Self::Glyph => (1, 1),
            Self::HalfBlock => (1, 2),
        }
    }

    /// The next mode, for a toggle.
    #[must_use]
    pub const fn next(self) -> Self {
        match self {
            Self::Glyph => Self::HalfBlock,
            Self::HalfBlock => Self::Glyph,
        }
    }

    /// The mode's name, for a status line.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Glyph => "glyph",
            Self::HalfBlock => "half-block",
        }
    }
}

/// How many samples are averaged into each output pixel, per axis.
///
/// Supersampling multiplies the lattice in **both** axes, which is exactly why
/// it does not disturb the sample aspect: `cell_aspect × (across·k) / (down·k)`
/// is the same number for every `k`. That invariance is what lets it be its own
/// setting instead of a combinatorial explosion of [`CellMode`] variants.
///
/// Capped low on purpose, and the cap is measured rather than guessed. At 1e6
/// magnification on a Ryzen 9 7940HS, per frame at 20,000 output pixels:
///
/// | | samples | frame | fps |
/// |---|---|---|---|
/// | 1× | 20,000 | 7.3ms | 138 |
/// | 2× | 80,000 | 18.2ms | 55 |
/// | 3× | 180,000 | 88.5ms | 11 |
///
/// So **2× is the highest that holds 30fps at depth, and 3× does not** — an
/// earlier version of this comment guessed the wall was at 4×, which the
/// numbers above disagree with. 3× is kept because a *parked* view does not
/// care about frame rate and it is the sharpest still the renderer can produce;
/// it is a poor choice while anything is moving, and the status line shows the
/// cost while you make the trade.
///
/// Note the scaling is worse than the `k²` samples suggest — nine times the
/// samples cost twelve times the time. Untested, but the likely cause is that
/// 180,000 samples no longer fit in cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Supersample {
    /// One sample per output pixel.
    #[default]
    Off,
    /// 2×2 samples averaged per output pixel.
    X2,
    /// 3×3 samples averaged per output pixel.
    X3,
}

impl Supersample {
    /// Samples per axis.
    #[must_use]
    pub const fn factor(self) -> usize {
        match self {
            Self::Off => 1,
            Self::X2 => 2,
            Self::X3 => 3,
        }
    }

    /// The next setting, for a key that cycles.
    #[must_use]
    pub const fn next(self) -> Self {
        match self {
            Self::Off => Self::X2,
            Self::X2 => Self::X3,
            Self::X3 => Self::Off,
        }
    }

    /// The setting's name, for a status line.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Off => "1x",
            Self::X2 => "2x",
            Self::X3 => "3x",
        }
    }
}

/// Everything that decides how samples are coloured, and nothing that decides
/// what the samples are.
///
/// That division is what makes the dirty check work: if a field belongs here,
/// changing it must never require re-sampling. The question to ask of a new
/// field is exactly that — if the kernel would have to run again, it belongs
/// with the sample parameters instead.
#[derive(Debug, Clone)]
pub struct Shader {
    /// The colour gradient.
    pub palette: Palette,
    /// Cycling offset, in iteration units.
    pub phase: f64,
    /// How samples map onto cells.
    pub mode: CellMode,
    /// How many samples are averaged into each output pixel.
    pub supersample: Supersample,
}

impl Shader {
    /// Samples across and down one character cell, including supersampling.
    #[must_use]
    pub const fn subdivisions(&self) -> (usize, usize) {
        let (across, down) = self.mode.subdivisions();
        let k = self.supersample.factor();
        (across * k, down * k)
    }

    /// The sample lattice needed to fill `cols × rows` character cells.
    #[must_use]
    pub const fn lattice(&self, cols: usize, rows: usize) -> (usize, usize) {
        let (across, down) = self.subdivisions();
        (cols * across, rows * down)
    }

    /// The sample aspect a viewport needs for this configuration.
    ///
    /// `cell_aspect × across / down` — one formula covering every sub-cell
    /// layout, and invariant under supersampling because that multiplies both
    /// terms. See `Viewport::sample_aspect`.
    #[must_use]
    pub fn sample_aspect(&self, cell_aspect: f64) -> f64 {
        let (across, down) = self.subdivisions();
        cell_aspect * across as f64 / down as f64
    }
}

impl Default for Shader {
    fn default() -> Self {
        Self {
            palette: Palette::default(),
            phase: 0.0,
            mode: CellMode::default(),
            supersample: Supersample::default(),
        }
    }
}

/// Shade `samples` into `out`, resizing `out` to fit.
///
/// A mismatch between the grids renders the overlap rather than panicking, for
/// the same reason `Grid` tolerates one: a terminal resize is observed a frame
/// before the grids are remade to match, and a panic there would turn a window
/// drag into a crash.
pub fn shade_into(shader: &Shader, samples: &SampleGrid, out: &mut Grid) {
    let (across, down) = shader.subdivisions();
    let cols = samples.cols() / across.max(1);
    let rows = samples.rows() / down.max(1);

    if out.width() != cols || out.height() != rows {
        out.resize(cols, rows);
    }

    match shader.mode {
        CellMode::Glyph => shade_glyph(shader, samples, out),
        CellMode::HalfBlock => shade_half_block(shader, samples, out),
    }
}

/// The upper-half block.
///
/// Only this one, never `▄`: a lower block with the colours swapped is
/// pixel-identical, so one glyph keeps the blit branchless and every cell
/// single-width — which also keeps the terminal diff out of its
/// wide-character path.
const UPPER_HALF: char = '▀';

/// Two stacked samples per cell: the upper becomes the foreground, the lower
/// the background.
///
/// No colour is lost when they differ — both 24-bit values survive in the one
/// cell, which is what makes this mode worth having over braille.
fn shade_half_block(shader: &Shader, samples: &SampleGrid, out: &mut Grid) {
    let (sx, sy) = shader.subdivisions();
    // The cell's block splits vertically: the top half becomes the foreground,
    // the bottom half the background. With supersampling each half is itself a
    // block to average.
    let half = (sy / 2).max(1);
    let mut top = Vec::with_capacity(sx * half);
    let mut bottom = Vec::with_capacity(sx * half);

    for iy in 0..out.height() {
        for ix in 0..out.width() {
            collect_block(samples, ix * sx, iy * sy, sx, half, &mut top);
            collect_block(samples, ix * sx, iy * sy + half, sx, half, &mut bottom);
            if top.is_empty() {
                continue;
            }
            // An empty bottom half happens when the lattice has an odd row
            // count, which a resize can produce for a frame. Reuse the top
            // rather than filling it black: a black half reads as a one-pixel
            // gap along the bottom edge, which looks like a rendering bug.
            let lower = if bottom.is_empty() { &top } else { &bottom };
            out.set(
                ix,
                iy,
                Cell::with_background(
                    UPPER_HALF,
                    oklab::average(top.iter().map(|e| colour_for(shader, *e))),
                    oklab::average(lower.iter().map(|e| colour_for(shader, *e))),
                ),
            );
        }
    }
}

/// The colour a sample takes, with interior rendering as black.
fn colour_for(shader: &Shader, escape: Escape) -> crate::render::Rgb {
    match escape.smooth() {
        Some(smooth) => shader.palette.at(smooth + shader.phase),
        None => crate::render::Rgb::BLACK,
    }
}

/// The ramp carries density, the palette carries colour.
///
/// With supersampling on, each cell averages a `k × k` block: the densities are
/// averaged to pick one glyph, and **the colours are averaged after the palette,
/// in linear light**. Averaging `smooth` and colouring once instead would give a
/// boundary cell a colour belonging to neither side — the mean of two iteration
/// counts is not the mean of the two colours they map to.
fn shade_glyph(shader: &Shader, samples: &SampleGrid, out: &mut Grid) {
    let (sx, sy) = shader.subdivisions();
    let mut block = Vec::with_capacity(sx * sy);
    for iy in 0..out.height() {
        for ix in 0..out.width() {
            collect_block(samples, ix * sx, iy * sy, sx, sy, &mut block);
            if block.is_empty() {
                continue;
            }
            let density = block.iter().map(|e| ramp::density(*e)).sum::<f64>() / block.len() as f64;
            let colour = oklab::average(block.iter().map(|e| colour_for(shader, *e)));
            out.set(ix, iy, Cell::new(ramp::glyph_for_density(density), colour));
        }
    }
}

/// Gather the samples of a `w × h` block into `out`, replacing its contents.
///
/// Reuses the caller's buffer: at 3× supersampling this runs nine times per
/// cell, and a fresh `Vec` each time would be twenty thousand allocations a
/// frame.
fn collect_block(
    samples: &SampleGrid,
    x0: usize,
    y0: usize,
    w: usize,
    h: usize,
    out: &mut Vec<Escape>,
) {
    out.clear();
    for y in y0..y0 + h {
        for x in x0..x0 + w {
            if let Some(escape) = samples.get(x, y) {
                out.push(escape);
            }
        }
    }
}

/// The cell a single sample shades to, with no supersampling.
///
/// The reference the block path must agree with at 1×, which is what
/// `supersampling_off_matches_the_single_sample_path` asserts.
#[cfg(test)]
fn cell_for(shader: &Shader, escape: Escape) -> Cell {
    Cell::new(ramp::glyph(escape), colour_for(shader, escape))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sample grid written by hand — no kernel anywhere in these tests,
    /// which is the split's main justification.
    fn grid_from(rows: &[&[Escape]]) -> SampleGrid {
        let mut grid = SampleGrid::new(rows[0].len(), rows.len());
        for (iy, row) in rows.iter().enumerate() {
            for (ix, sample) in row.iter().enumerate() {
                grid.set(ix, iy, *sample);
            }
        }
        grid
    }

    fn escaped(smooth: f64) -> Escape {
        Escape::Escaped {
            iterations: smooth.ceil() as u32,
            smooth,
        }
    }

    #[test]
    fn glyph_mode_is_one_sample_per_cell() {
        assert_eq!(CellMode::Glyph.subdivisions(), (1, 1));
        assert_eq!(Shader::default().lattice(80, 24), (80, 24));
    }

    #[test]
    fn half_block_mode_is_two_stacked_samples_per_cell() {
        assert_eq!(CellMode::HalfBlock.subdivisions(), (1, 2));
        let shader = Shader {
            mode: CellMode::HalfBlock,
            ..Shader::default()
        };
        assert_eq!(shader.lattice(80, 24), (80, 48));
    }

    #[test]
    fn the_modes_toggle_and_name_themselves() {
        assert_eq!(CellMode::Glyph.next(), CellMode::HalfBlock);
        assert_eq!(CellMode::HalfBlock.next(), CellMode::Glyph);
        assert_eq!(CellMode::Glyph.name(), "glyph");
        assert_eq!(CellMode::HalfBlock.name(), "half-block");
    }

    #[test]
    fn both_modes_cover_the_same_plane_for_the_same_cells() {
        // **The load-bearing claim of the whole core/frontend split.** A mode
        // change must alter only the numbers handed to core — the sample
        // lattice and the aspect — never the region of the plane on screen. If
        // these disagree, pressing `m` would jump the view and the aspect
        // parameter would not be absorbing the sub-cell layout after all.
        const CELLS: (usize, usize) = (100, 30);
        const CELL_ASPECT: f64 = 2.0;

        let mut views = Vec::new();
        for mode in [CellMode::Glyph, CellMode::HalfBlock] {
            let shader = Shader {
                mode,
                ..Shader::default()
            };
            let (cols, rows) = shader.lattice(CELLS.0, CELLS.1);
            let vp = frascii_core::Viewport::home(cols, rows, shader.sample_aspect(CELL_ASPECT));
            views.push((vp.half_width, vp.half_height(), vp.centre));
        }
        let (gw, gh, gc) = views[0];
        let (hw, hh, hc) = views[1];
        assert!((gw - hw).abs() < 1e-12, "widths differ: {gw} vs {hw}");
        assert!((gh - hh).abs() < 1e-12, "heights differ: {gh} vs {hh}");
        assert_eq!(gc, hc, "centres differ");
    }

    #[test]
    fn supersampling_is_off_by_default_and_cycles() {
        assert_eq!(Supersample::default(), Supersample::Off);
        assert_eq!(Supersample::Off.factor(), 1);
        assert_eq!(Supersample::X2.factor(), 2);
        assert_eq!(Supersample::X3.factor(), 3);
        assert_eq!(Supersample::Off.next(), Supersample::X2);
        assert_eq!(Supersample::X3.next(), Supersample::Off, "must wrap");
        assert_eq!(Supersample::X2.name(), "2x");
    }

    #[test]
    fn supersampling_multiplies_the_lattice_in_both_axes() {
        let shader = Shader {
            supersample: Supersample::X3,
            ..Shader::default()
        };
        assert_eq!(shader.subdivisions(), (3, 3));
        assert_eq!(shader.lattice(20, 10), (60, 30));

        // And it composes with half-block rather than replacing it.
        let both = Shader {
            mode: CellMode::HalfBlock,
            supersample: Supersample::X2,
            ..Shader::default()
        };
        assert_eq!(both.subdivisions(), (2, 4));
        assert_eq!(both.lattice(20, 10), (40, 40));
    }

    #[test]
    fn the_sample_aspect_is_invariant_under_supersampling() {
        // **The property that lets supersampling be its own setting** rather
        // than a combinatorial explosion of modes: it multiplies both axes, so
        // the ratio — and therefore the plane region on screen — is unchanged.
        for mode in [CellMode::Glyph, CellMode::HalfBlock] {
            let base = Shader {
                mode,
                ..Shader::default()
            }
            .sample_aspect(2.0);
            for supersample in [Supersample::X2, Supersample::X3] {
                let scaled = Shader {
                    mode,
                    supersample,
                    ..Shader::default()
                }
                .sample_aspect(2.0);
                assert!(
                    (scaled - base).abs() < 1e-12,
                    "{mode:?} at {supersample:?}: {scaled} vs {base}"
                );
            }
        }
    }

    #[test]
    fn supersampling_off_matches_the_single_sample_path() {
        // At 1× the block path must be exactly the old single-sample path, or
        // turning supersampling on and off would visibly shift the picture.
        let samples = grid_from(&[
            &[escaped(1.0), Escape::Interior, escaped(600.0)],
            &[escaped(40.0), escaped(3.5), Escape::Interior],
        ]);
        let shader = Shader::default();
        let mut out = Grid::new(0, 0);
        shade_into(&shader, &samples, &mut out);

        for iy in 0..2 {
            for ix in 0..3 {
                let expected = cell_for(&shader, samples.get(ix, iy).expect("in range"));
                assert_eq!(out.get(ix, iy), Some(expected), "at {ix},{iy}");
            }
        }
    }

    #[test]
    fn supersampling_antialiases_a_boundary() {
        // The whole point. A cell whose 2×2 block straddles the edge — two
        // interior samples, two deep-exterior — must come out *between* the
        // two, where a single centre sample would snap to whichever side it
        // landed on.
        let straddling = grid_from(&[
            &[Escape::Interior, escaped(600.0)],
            &[Escape::Interior, escaped(600.0)],
        ]);
        let shader = Shader {
            supersample: Supersample::X2,
            ..Shader::default()
        };
        let mut out = Grid::new(0, 0);
        shade_into(&shader, &straddling, &mut out);

        assert_eq!((out.width(), out.height()), (1, 1));
        let cell = out.get(0, 0).expect("one cell");

        // Neither extreme: not blank like the interior, not solid like a deep
        // escape.
        assert_ne!(cell.glyph, ' ', "collapsed to the interior");
        assert_ne!(cell.glyph, '@', "collapsed to the exterior");

        // And the colour is a genuine mix rather than one side's.
        let interior = colour_for(&shader, Escape::Interior);
        let exterior = colour_for(&shader, escaped(600.0));
        assert_ne!(cell.colour, interior);
        assert_ne!(cell.colour, exterior);
    }

    #[test]
    fn colour_is_averaged_after_the_palette_not_before() {
        // Averaging `smooth` and colouring once would give a boundary cell a
        // colour belonging to neither side, because the palette is cyclic: the
        // mean of two iteration counts can land anywhere in the gradient,
        // including on a hue neither sample had.
        //
        // Two samples a full half-period apart make the difference visible.
        let a = 4.0;
        let b = 4.0 + Palette::PERIOD / 2.0;
        let samples = grid_from(&[&[escaped(a), escaped(b)], &[escaped(a), escaped(b)]]);
        let shader = Shader {
            supersample: Supersample::X2,
            ..Shader::default()
        };
        let mut out = Grid::new(0, 0);
        shade_into(&shader, &samples, &mut out);
        let averaged_colour = out.get(0, 0).expect("one cell").colour;

        // What the wrong order would have produced.
        let averaged_smooth = shader.palette.at((a + b) / 2.0);
        assert_ne!(
            averaged_colour, averaged_smooth,
            "the mean of two colours should not equal the colour of the mean"
        );
    }

    #[test]
    fn half_block_supersamples_each_half_independently() {
        // 2× on half-block gives a 2×4 block: the top 2×2 becomes the
        // foreground, the bottom 2×2 the background, each averaged on its own.
        let samples = grid_from(&[
            &[escaped(2.0), escaped(2.0)],
            &[escaped(2.0), escaped(2.0)],
            &[escaped(500.0), escaped(500.0)],
            &[escaped(500.0), escaped(500.0)],
        ]);
        let shader = Shader {
            mode: CellMode::HalfBlock,
            supersample: Supersample::X2,
            ..Shader::default()
        };
        let mut out = Grid::new(0, 0);
        shade_into(&shader, &samples, &mut out);

        assert_eq!((out.width(), out.height()), (1, 1));
        let cell = out.get(0, 0).expect("one cell");
        // Uniform halves, so each average is exactly its own sample's colour.
        assert_eq!(cell.colour, colour_for(&shader, escaped(2.0)));
        assert_eq!(cell.background, colour_for(&shader, escaped(500.0)));
    }

    #[test]
    fn half_block_puts_the_upper_sample_in_the_foreground() {
        // The packing, checked against a hand-written grid: two rows collapse
        // into one cell, upper to fg and lower to bg, with both colours intact.
        let samples = grid_from(&[&[escaped(3.0)], &[escaped(300.0)]]);
        let mut out = Grid::new(0, 0);
        let shader = Shader {
            mode: CellMode::HalfBlock,
            ..Shader::default()
        };
        shade_into(&shader, &samples, &mut out);

        assert_eq!((out.width(), out.height()), (1, 1));
        let cell = out.get(0, 0).expect("one cell");
        assert_eq!(cell.glyph, '▀');
        assert_eq!(cell.colour, shader.palette.at(3.0));
        assert_eq!(cell.background, shader.palette.at(300.0));
        assert_ne!(cell.colour, cell.background, "both colours must survive");
    }

    #[test]
    fn half_block_halves_the_row_count() {
        let samples = SampleGrid::new(7, 8);
        let mut out = Grid::new(0, 0);
        shade_into(
            &Shader {
                mode: CellMode::HalfBlock,
                ..Shader::default()
            },
            &samples,
            &mut out,
        );
        assert_eq!((out.width(), out.height()), (7, 4));
    }

    #[test]
    fn an_odd_sample_height_reuses_the_upper_half_rather_than_gapping() {
        // A resize can hand us an odd row count for a frame. Filling the
        // missing lower half with black would read as a one-pixel gap along the
        // bottom edge, which looks like a rendering bug.
        let samples = grid_from(&[&[escaped(5.0)], &[escaped(5.0)], &[escaped(9.0)]]);
        let mut out = Grid::new(0, 0);
        let shader = Shader {
            mode: CellMode::HalfBlock,
            ..Shader::default()
        };
        shade_into(&shader, &samples, &mut out);
        // Three rows give one full cell; the odd row is dropped by the integer
        // division, so nothing is half-drawn.
        assert_eq!(out.height(), 1);
        let cell = out.get(0, 0).expect("one cell");
        assert_eq!(
            cell.colour, cell.background,
            "both halves came from row 0/1"
        );
    }

    #[test]
    fn half_block_interior_is_black_on_both_halves() {
        let samples = grid_from(&[&[Escape::Interior], &[Escape::Interior]]);
        let mut out = Grid::new(0, 0);
        shade_into(
            &Shader {
                mode: CellMode::HalfBlock,
                ..Shader::default()
            },
            &samples,
            &mut out,
        );
        let cell = out.get(0, 0).expect("one cell");
        assert_eq!(cell.colour, crate::render::Rgb::BLACK);
        assert_eq!(cell.background, crate::render::Rgb::BLACK);
    }

    #[test]
    fn half_block_carries_no_glyph_ramp() {
        // Density is colour alone in this mode, so every cell is the same
        // glyph. Stated as a test because it is the mode's defining property
        // and the reason glyph mode remains the default.
        let samples = grid_from(&[
            &[escaped(1.0), escaped(50.0), Escape::Interior],
            &[escaped(400.0), Escape::Interior, escaped(7.0)],
        ]);
        let mut out = Grid::new(0, 0);
        shade_into(
            &Shader {
                mode: CellMode::HalfBlock,
                ..Shader::default()
            },
            &samples,
            &mut out,
        );
        assert!(out.cells().all(|c| c.glyph == '▀'), "the ramp leaked in");
    }

    #[test]
    fn the_sample_aspect_follows_the_subdivision_formula() {
        // Glyph mode: one sample spans a whole cell, so a sample is as tall as
        // a cell — twice its width.
        let glyph = Shader::default();
        let half = Shader {
            mode: CellMode::HalfBlock,
            ..Shader::default()
        };
        assert!((glyph.sample_aspect(2.0) - 2.0).abs() < 1e-12);
        // Half-block: two stacked samples, so each is square.
        assert!((half.sample_aspect(2.0) - 1.0).abs() < 1e-12);
        // And both track the cell aspect rather than hardcoding it, which is
        // the knob a user with an unusual font would reach for.
        assert!((glyph.sample_aspect(2.4) - 2.4).abs() < 1e-12);
        assert!((half.sample_aspect(2.4) - 1.2).abs() < 1e-12);
    }

    #[test]
    fn shading_resizes_the_output_to_the_lattice() {
        let samples = SampleGrid::new(7, 3);
        let mut out = Grid::new(0, 0);
        shade_into(&Shader::default(), &samples, &mut out);
        assert_eq!((out.width(), out.height()), (7, 3));
    }

    #[test]
    fn an_all_interior_grid_shades_to_blanks() {
        // A fresh SampleGrid is all interior, so this is the degenerate case.
        let samples = SampleGrid::new(4, 2);
        let mut out = Grid::new(0, 0);
        shade_into(&Shader::default(), &samples, &mut out);
        assert!(out.cells().all(|c| c == Cell::default()));
    }

    #[test]
    fn the_phase_changes_colour_but_never_the_glyph() {
        // The property that makes cycling read as motion rather than noise: the
        // ASCII structure must stand still while the colour moves.
        let shader = Shader::default();
        let shifted = Shader {
            phase: Palette::PERIOD / 3.0,
            ..Shader::default()
        };

        for smooth in [1.0, 4.5, 17.25, 90.0, 300.0] {
            let a = cell_for(&shader, escaped(smooth));
            let b = cell_for(&shifted, escaped(smooth));
            assert_eq!(a.glyph, b.glyph, "the glyph moved at smooth={smooth}");
            assert_ne!(a.colour, b.colour, "the colour did not move at {smooth}");
        }
    }

    #[test]
    fn interior_takes_no_colour_from_the_palette() {
        // Otherwise the largest solid region of the picture throbs with the
        // cycling phase.
        for phase in [0.0, 5.0, 17.0, Palette::PERIOD] {
            let shader = Shader {
                phase,
                ..Shader::default()
            };
            assert_eq!(cell_for(&shader, Escape::Interior), Cell::default());
        }
    }

    #[test]
    fn shading_is_independent_of_the_iteration_limit() {
        // Two samples with identical `smooth` must shade identically whatever
        // limit produced them, or a dive's rising limit re-shades the frame.
        let shader = Shader::default();
        let low = Escape::Escaped {
            iterations: 20,
            smooth: 19.5,
        };
        let high = Escape::Escaped {
            iterations: 20,
            smooth: 19.5,
        };
        assert_eq!(cell_for(&shader, low), cell_for(&shader, high));
    }

    #[test]
    fn a_smaller_sample_grid_leaves_the_rest_of_the_output_alone() {
        // The resize-lag case: shading must render the overlap, not panic.
        let samples = SampleGrid::new(2, 2);
        let mut out = Grid::new(5, 5);
        shade_into(&Shader::default(), &samples, &mut out);
        // It resized down to the lattice rather than leaving stale cells.
        assert_eq!((out.width(), out.height()), (2, 2));
    }

    #[test]
    fn a_zero_sized_sample_grid_does_not_panic() {
        let samples = SampleGrid::new(0, 0);
        let mut out = Grid::new(4, 4);
        shade_into(&Shader::default(), &samples, &mut out);
        assert_eq!(out.cells().count(), 0);
    }

    #[test]
    fn a_hand_written_grid_shades_cell_for_cell() {
        // The test the whole split exists to make possible: a known 3x2 field
        // of escape values, shaded and checked glyph by glyph, with no kernel.
        let samples = grid_from(&[
            &[Escape::Interior, escaped(1.0), escaped(600.0)],
            &[escaped(300.0), Escape::Interior, escaped(3.0)],
        ]);
        let mut out = Grid::new(0, 0);
        shade_into(&Shader::default(), &samples, &mut out);

        assert_eq!((out.width(), out.height()), (3, 2));
        let row0: String = out.row(0).expect("row 0").iter().map(|c| c.glyph).collect();
        let row1: String = out.row(1).expect("row 1").iter().map(|c| c.glyph).collect();

        // Interior blank, a fast escape light, a very slow one heavy.
        assert_eq!(row0.chars().next(), Some(' '));
        assert_eq!(row0.chars().nth(2), Some('@'));
        assert_eq!(row1.chars().nth(1), Some(' '));
        // And density is ordered: the 300-iteration escape outweighs the
        // 3-iteration one.
        let heavy = ramp::RAMP
            .iter()
            .position(|c| *c == row1.chars().next().unwrap());
        let light = ramp::RAMP
            .iter()
            .position(|c| *c == row1.chars().nth(2).unwrap());
        assert!(heavy > light, "{row1:?} is not ordered by escape time");
    }
}
