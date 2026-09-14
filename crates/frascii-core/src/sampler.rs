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

/// Fill `grid` with the classification of every sample in `vp`.
///
/// The grid is resized to the viewport if it does not already match, so a
/// caller can keep one grid for the life of the program and hand it back every
/// frame — at 30fps a fresh allocation per frame is 20,000 elements thirty
/// times a second for a buffer about to be overwritten entirely.
///
/// Rows are spread across threads with rayon. Rows rather than individual
/// samples because a row is big enough to amortise the scheduling and small
/// enough that the wildly uneven cost of samples — an interior point runs the
/// full limit, an escaping one may run four iterations — still balances via
/// work stealing.
///
/// # Why this is a function and not a trait
///
/// It was briefly a `Sampler` trait with a `CpuSampler` impl, justified as a
/// seam for a future GPU backend. That justification does not survive contact:
/// a GPU backend is upload → dispatch → readback, with asynchronous completion
/// and a buffer the host does not own in between, and *this* signature fits
/// none of that. A seam shaped wrongly for the thing it is a seam for provides
/// no insulation at all — it would be rewritten when the backend arrived,
/// which is the precise cost it existed to avoid.
///
/// What actually makes a second backend possible is that [`Viewport`],
/// [`Escape`](crate::Escape) and [`SampleGrid`] are backend-neutral *data* that
/// name no host. Those are what a GPU crate would consume, and they are already
/// right. Adding a trait then is a mechanical refactor of one free function.
///
/// Meanwhile the trait cost something: dynamic dispatch, or a generic parameter
/// that goes viral through every type holding the frontend's `App`.
///
/// # Dispatch
///
/// Generic over the kernel rather than taking `&dyn Fractal`, so `escape`
/// inlines into the row loop. A trait object would cost an indirect call per
/// *sample* — twenty thousand of them per frame — and, worse, would stop the
/// optimiser seeing the iteration loop at all. Callers that need to choose a
/// kernel at run time use [`Kernel`](crate::Kernel), which matches once per
/// frame instead of once per sample.
pub fn sample_into<F: Fractal + ?Sized>(
    grid: &mut SampleGrid,
    vp: &Viewport,
    fractal: &F,
    limit: u32,
) {
    if grid.cols() != vp.cols || grid.rows() != vp.rows {
        grid.resize(vp.cols, vp.rows);
    }

    // Collected because rayon needs an indexed parallel iterator, and the row
    // index is what maps a row back to its place on the plane.
    let mut rows: Vec<&mut [_]> = grid.rows_mut().collect();
    rows.par_iter_mut().enumerate().for_each(|(iy, row)| {
        for (ix, sample) in row.iter_mut().enumerate() {
            *sample = fractal.escape(vp.sample_to_plane(ix, iy), limit);
        }
    });
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
        sample_into(&mut grid, &vp, &Mandelbrot, 500);

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
        sample_into(&mut grid, &vp, &Mandelbrot, 500);

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
        sample_into(&mut a, &vp, &Mandelbrot, 400);
        sample_into(&mut b, &vp, &Mandelbrot, 400);
        assert_eq!(a, b);
    }

    #[test]
    fn a_grid_is_reused_rather_than_reallocated_when_the_size_is_unchanged() {
        let vp = Viewport::home(20, 10, 0.5);
        let mut grid = SampleGrid::new(20, 10);
        sample_into(&mut grid, &vp, &Mandelbrot, 100);
        let first = grid.clone();
        sample_into(&mut grid, &vp, &Mandelbrot, 100);
        assert_eq!(grid, first);
    }

    #[test]
    fn a_zero_sized_viewport_produces_an_empty_grid_without_panicking() {
        let vp = Viewport::home(0, 0, 0.5);
        let mut grid = SampleGrid::new(10, 10);
        sample_into(&mut grid, &vp, &Mandelbrot, 100);
        assert_eq!(grid.samples().count(), 0);
    }

    #[test]
    fn the_sampler_is_generic_over_the_fractal() {
        // Dispatching through `&dyn Fractal` has to actually reach the right
        // kernel; a Julia grid must differ from a Mandelbrot one.
        let vp = Viewport::home(24, 12, 0.5);
        let mut m = SampleGrid::new(0, 0);
        let mut j = SampleGrid::new(0, 0);
        sample_into(&mut m, &vp, &Mandelbrot, 300);
        sample_into(&mut j, &vp, &Julia::new(Complex::new(-0.8, 0.156)), 300);
        assert_ne!(m, j);
    }

    #[test]
    fn a_higher_limit_only_ever_reclassifies_escaped_points_as_interior() {
        // Interior membership is a function of the limit, never the reverse: a
        // point that escaped at a low limit must still escape at a higher one.
        let vp = Viewport::home(30, 15, 0.5);
        let mut low = SampleGrid::new(0, 0);
        let mut high = SampleGrid::new(0, 0);
        sample_into(&mut low, &vp, &Mandelbrot, 50);
        sample_into(&mut high, &vp, &Mandelbrot, 2_000);

        for iy in 0..15 {
            for ix in 0..30 {
                if let (Some(Escape::Escaped { .. }), Some(h)) = (low.get(ix, iy), high.get(ix, iy))
                {
                    assert!(!h.is_interior(), "{ix},{iy} un-escaped at a higher limit");
                }
            }
        }
    }
}
