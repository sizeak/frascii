//! Driving a kernel over a viewport.
//!
//! The sampler owns iteration order and parallelism, which is exactly why the
//! kernels do not: a `Fractal` answers for one point and knows nothing about
//! threads, so there is one place to change how the work is spread and it is
//! here.

use rayon::prelude::*;

use crate::fractal::Fractal;
use crate::sample::SampleGrid;
use crate::viewport::Viewport;

/// Something that can fill a sample grid from a viewport and a fractal.
///
/// A trait for one implementation, deliberately. It is the seam that lets a
/// SIMD or GPU backend arrive later without `frascii-core` gaining a host
/// dependency — a GPU device is host-bound, so a backend needing one lives in
/// its own crate and is selected by the binary, never linked in here.
pub trait Sampler {
    /// Fill `grid` with the classification of every sample in `vp`.
    ///
    /// The grid is resized to the viewport if it does not already match, so a
    /// caller can keep one grid for the life of the program and hand it back
    /// every frame.
    fn sample_into(&self, grid: &mut SampleGrid, vp: &Viewport, fractal: &dyn Fractal, limit: u32);

    /// The backend's name, for a status line or a benchmark report.
    fn name(&self) -> &'static str;
}

/// The CPU sampler: rayon across rows.
///
/// Rows rather than individual samples because a row is a big enough unit to
/// amortise the scheduling, and small enough that the wildly uneven cost of
/// samples — an interior point runs the full limit, an escaping one may run
/// four iterations — still balances across threads via work stealing.
#[derive(Debug, Clone, Copy, Default)]
pub struct CpuSampler;

impl Sampler for CpuSampler {
    fn sample_into(&self, grid: &mut SampleGrid, vp: &Viewport, fractal: &dyn Fractal, limit: u32) {
        if grid.cols() != vp.cols || grid.rows() != vp.rows {
            grid.resize(vp.cols, vp.rows);
        }

        // Collected because rayon needs an indexed parallel iterator and the
        // row index is what maps a row back to its place on the plane.
        let mut rows: Vec<&mut [_]> = grid.rows_mut().collect();
        rows.par_iter_mut().enumerate().for_each(|(iy, row)| {
            for (ix, sample) in row.iter_mut().enumerate() {
                *sample = fractal.escape(vp.sample_to_plane(ix, iy), limit);
            }
        });
    }

    fn name(&self) -> &'static str {
        "cpu"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Escape;
    use crate::complex::Complex;
    use crate::fractal::{Julia, Mandelbrot};

    #[test]
    fn sampling_fills_the_grid_and_matches_point_by_point() {
        // The sampler must be a pure fan-out over the kernel: same viewport,
        // same answers, regardless of how the work was split.
        let vp = Viewport::home(40, 20, 0.5);
        let mut grid = SampleGrid::new(0, 0);
        CpuSampler.sample_into(&mut grid, &vp, &Mandelbrot, 500);

        assert_eq!((grid.cols(), grid.rows()), (40, 20));
        for iy in 0..20 {
            for ix in 0..40 {
                let expected = Mandelbrot.escape(vp.sample_to_plane(ix, iy), 500);
                assert_eq!(grid.get(ix, iy), Some(expected), "at {ix},{iy}");
            }
        }
    }

    #[test]
    fn the_home_view_is_neither_empty_nor_solid() {
        // The cheapest possible guard against a mapping bug that renders a flat
        // field: the classic view has a large set and a large outside.
        let vp = Viewport::home(80, 40, 0.5);
        let mut grid = SampleGrid::new(0, 0);
        CpuSampler.sample_into(&mut grid, &vp, &Mandelbrot, 500);

        let interior = grid.samples().filter(|e| e.is_interior()).count();
        let escaped = grid.samples().count() - interior;
        assert!(interior > 100, "only {interior} interior samples");
        assert!(escaped > 100, "only {escaped} escaped samples");
    }

    #[test]
    fn sampling_is_deterministic_across_runs() {
        // Parallelism must not reach the result. If work stealing could affect
        // a sample, the render snapshots downstream would be flaky.
        let vp = Viewport::home(30, 15, 0.5);
        let mut a = SampleGrid::new(0, 0);
        let mut b = SampleGrid::new(0, 0);
        CpuSampler.sample_into(&mut a, &vp, &Mandelbrot, 400);
        CpuSampler.sample_into(&mut b, &vp, &Mandelbrot, 400);
        assert_eq!(a, b);
    }

    #[test]
    fn a_grid_is_reused_rather_than_reallocated_when_the_size_is_unchanged() {
        let vp = Viewport::home(20, 10, 0.5);
        let mut grid = SampleGrid::new(20, 10);
        CpuSampler.sample_into(&mut grid, &vp, &Mandelbrot, 100);
        let first = grid.clone();
        CpuSampler.sample_into(&mut grid, &vp, &Mandelbrot, 100);
        assert_eq!(grid, first);
    }

    #[test]
    fn a_zero_sized_viewport_produces_an_empty_grid_without_panicking() {
        let vp = Viewport::home(0, 0, 0.5);
        let mut grid = SampleGrid::new(10, 10);
        CpuSampler.sample_into(&mut grid, &vp, &Mandelbrot, 100);
        assert_eq!(grid.samples().count(), 0);
    }

    #[test]
    fn the_sampler_is_generic_over_the_fractal() {
        // Dispatching through `&dyn Fractal` has to actually reach the right
        // kernel; a Julia grid must differ from a Mandelbrot one.
        let vp = Viewport::home(24, 12, 0.5);
        let mut m = SampleGrid::new(0, 0);
        let mut j = SampleGrid::new(0, 0);
        CpuSampler.sample_into(&mut m, &vp, &Mandelbrot, 300);
        CpuSampler.sample_into(&mut j, &vp, &Julia::new(Complex::new(-0.8, 0.156)), 300);
        assert_ne!(m, j);
    }

    #[test]
    fn a_higher_limit_only_ever_reclassifies_escaped_points_as_interior() {
        // Interior membership is a function of the limit, never the reverse: a
        // point that escaped at a low limit must still escape at a higher one.
        let vp = Viewport::home(30, 15, 0.5);
        let mut low = SampleGrid::new(0, 0);
        let mut high = SampleGrid::new(0, 0);
        CpuSampler.sample_into(&mut low, &vp, &Mandelbrot, 50);
        CpuSampler.sample_into(&mut high, &vp, &Mandelbrot, 2_000);

        for iy in 0..15 {
            for ix in 0..30 {
                if let (Some(Escape::Escaped { .. }), Some(h)) = (low.get(ix, iy), high.get(ix, iy))
                {
                    assert!(!h.is_interior(), "{ix},{iy} un-escaped at a higher limit");
                }
            }
        }
    }

    #[test]
    fn the_backend_names_itself() {
        assert_eq!(CpuSampler.name(), "cpu");
    }
}
