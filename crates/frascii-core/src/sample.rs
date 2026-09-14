//! The sample grid: this crate's output, and what every frontend consumes.

use crate::Escape;

/// A rectangular field of classified points, in row-major order.
///
/// The distinction worth holding onto: this is a **sample** grid of `Escape`
/// values and is frontend-agnostic. A grid of glyphs and colours is a *cell*
/// grid, and that belongs to whatever is drawing.
#[derive(Debug, Clone, PartialEq)]
pub struct SampleGrid {
    cols: usize,
    rows: usize,
    samples: Vec<Escape>,
}

impl SampleGrid {
    /// A grid of the given size, every sample interior.
    #[must_use]
    pub fn new(cols: usize, rows: usize) -> Self {
        Self {
            cols,
            rows,
            samples: vec![Escape::Interior; cols * rows],
        }
    }

    /// Samples across.
    #[must_use]
    pub const fn cols(&self) -> usize {
        self.cols
    }

    /// Samples down.
    #[must_use]
    pub const fn rows(&self) -> usize {
        self.rows
    }

    /// Resize in place, reusing the allocation where it is large enough.
    ///
    /// The reason this exists rather than constructing a fresh grid each frame:
    /// at 30fps a full-screen grid is a 20,000-element allocation thirty times
    /// a second, for a buffer whose contents are about to be overwritten
    /// entirely anyway.
    pub fn resize(&mut self, cols: usize, rows: usize) {
        self.cols = cols;
        self.rows = rows;
        self.samples.clear();
        self.samples.resize(cols * rows, Escape::Interior);
    }

    /// The sample at `(ix, iy)`, or `None` outside the grid.
    #[must_use]
    pub fn get(&self, ix: usize, iy: usize) -> Option<Escape> {
        (ix < self.cols && iy < self.rows).then(|| self.samples[iy * self.cols + ix])
    }

    /// Overwrite the sample at `(ix, iy)`.
    ///
    /// Out-of-bounds writes are dropped rather than panicking, matching the
    /// rest of the grid's edge behaviour.
    ///
    /// The sampler does not use this — it fills rows in parallel through
    /// `rows_mut`. It exists so a *consumer* can build a grid by hand: every
    /// glyph ramp, palette and sub-cell reduction in a frontend is a function of
    /// a `SampleGrid`, and being able to write a 4×4 one in a test is what lets
    /// those be checked without running a kernel.
    pub fn set(&mut self, ix: usize, iy: usize, sample: Escape) {
        if ix < self.cols && iy < self.rows {
            self.samples[iy * self.cols + ix] = sample;
        }
    }

    /// One row of samples, or `None` when `iy` is past the last row.
    #[must_use]
    pub fn row(&self, iy: usize) -> Option<&[Escape]> {
        if iy >= self.rows {
            return None;
        }
        let start = iy * self.cols;
        Some(&self.samples[start..start + self.cols])
    }

    /// Every sample, row-major.
    pub fn samples(&self) -> impl Iterator<Item = Escape> + '_ {
        self.samples.iter().copied()
    }

    /// The rows, as mutable slices, for a sampler to fill in parallel.
    pub(crate) fn rows_mut(&mut self) -> impl Iterator<Item = &mut [Escape]> {
        // `chunks_mut` on a zero width would panic, and a zero-width grid is a
        // real thing a terminal reports during a resize.
        let cols = self.cols.max(1);
        self.samples.chunks_mut(cols)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_grid_is_the_requested_size_and_all_interior() {
        let g = SampleGrid::new(4, 3);
        assert_eq!((g.cols(), g.rows()), (4, 3));
        assert_eq!(g.samples().count(), 12);
        assert!(g.samples().all(Escape::is_interior));
    }

    #[test]
    fn indexing_is_row_major_and_bounded() {
        let g = SampleGrid::new(3, 2);
        assert!(g.get(2, 1).is_some());
        assert!(g.get(3, 1).is_none());
        assert!(g.get(2, 2).is_none());
        assert_eq!(g.row(1).map(<[Escape]>::len), Some(3));
        assert!(g.row(2).is_none());
    }

    #[test]
    fn resizing_reshapes_and_blanks() {
        let mut g = SampleGrid::new(2, 2);
        g.resize(5, 1);
        assert_eq!((g.cols(), g.rows()), (5, 1));
        assert_eq!(g.samples().count(), 5);
        assert!(g.samples().all(Escape::is_interior));
    }

    #[test]
    fn a_zero_dimension_is_legal_and_yields_no_samples() {
        // A terminal genuinely reports this mid-resize.
        let g = SampleGrid::new(0, 40);
        assert_eq!(g.samples().count(), 0);
        assert!(g.get(0, 0).is_none());
    }

    #[test]
    fn set_and_get_round_trip_within_bounds() {
        let mut g = SampleGrid::new(3, 2);
        let sample = Escape::Escaped {
            iterations: 7,
            smooth: 6.5,
        };
        g.set(2, 1, sample);
        assert_eq!(g.get(2, 1), Some(sample));
        assert_eq!(g.get(0, 0), Some(Escape::Interior));
    }

    #[test]
    fn out_of_bounds_writes_are_dropped() {
        let mut g = SampleGrid::new(2, 2);
        g.set(
            9,
            9,
            Escape::Escaped {
                iterations: 1,
                smooth: 0.5,
            },
        );
        assert!(g.samples().all(Escape::is_interior));
    }

    #[test]
    fn rows_mut_yields_one_slice_per_row() {
        let mut g = SampleGrid::new(3, 4);
        assert_eq!(g.rows_mut().count(), 4);
        assert!(g.rows_mut().all(|r| r.len() == 3));
    }

    #[test]
    fn rows_mut_does_not_panic_on_a_zero_width_grid() {
        let mut g = SampleGrid::new(0, 5);
        assert_eq!(g.rows_mut().count(), 0);
    }
}
