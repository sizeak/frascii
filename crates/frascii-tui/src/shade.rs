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

use frascii_core::SampleGrid;

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
}

impl CellMode {
    /// Samples across and down one character cell.
    #[must_use]
    pub const fn subdivisions(self) -> (usize, usize) {
        match self {
            Self::Glyph => (1, 1),
        }
    }

    /// The sample aspect a viewport needs for this mode.
    ///
    /// `cell_aspect × across / down` — the one formula that covers every
    /// sub-cell layout, so a new mode is a row in `subdivisions` and nothing
    /// else. See `Viewport::sample_aspect`.
    #[must_use]
    pub fn sample_aspect(self, cell_aspect: f64) -> f64 {
        let (across, down) = self.subdivisions();
        cell_aspect * across as f64 / down as f64
    }

    /// The sample lattice needed to fill `cols × rows` character cells.
    #[must_use]
    pub const fn lattice(self, cols: usize, rows: usize) -> (usize, usize) {
        let (across, down) = self.subdivisions();
        (cols * across, rows * down)
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
}

impl Default for Shader {
    fn default() -> Self {
        Self {
            palette: Palette::default(),
            phase: 0.0,
            mode: CellMode::default(),
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
    let (across, down) = shader.mode.subdivisions();
    let cols = samples.cols() / across.max(1);
    let rows = samples.rows() / down.max(1);

    if out.width() != cols || out.height() != rows {
        out.resize(cols, rows);
    }

    match shader.mode {
        CellMode::Glyph => shade_glyph(shader, samples, out),
    }
}

/// One sample per cell: the ramp carries density, the palette carries colour.
fn shade_glyph(shader: &Shader, samples: &SampleGrid, out: &mut Grid) {
    for iy in 0..out.height() {
        for ix in 0..out.width() {
            let Some(escape) = samples.get(ix, iy) else {
                continue;
            };
            out.set(ix, iy, cell_for(shader, escape));
        }
    }
}

/// The cell a single sample shades to.
fn cell_for(shader: &Shader, escape: frascii_core::Escape) -> Cell {
    let glyph = ramp::glyph(escape);
    match escape.smooth() {
        // The phase is added here, and this is the whole of palette cycling:
        // the samples are untouched, so a cycling frame runs no kernel.
        Some(smooth) => Cell::new(glyph, shader.palette.at(smooth + shader.phase)),
        // Interior takes no colour from the palette. Colouring it would tie the
        // largest solid region of the picture to the cycling phase and make the
        // whole interior throb.
        None => Cell::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use frascii_core::Escape;

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
        assert_eq!(CellMode::Glyph.lattice(80, 24), (80, 24));
    }

    #[test]
    fn the_sample_aspect_follows_the_subdivision_formula() {
        // Glyph mode: one sample spans a whole cell, so a sample is as tall as
        // a cell — twice its width.
        assert!((CellMode::Glyph.sample_aspect(2.0) - 2.0).abs() < 1e-12);
        // And it tracks the cell aspect rather than hardcoding it, which is the
        // knob a user with an unusual font would reach for.
        assert!((CellMode::Glyph.sample_aspect(2.4) - 2.4).abs() < 1e-12);
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
