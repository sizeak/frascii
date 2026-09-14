//! Choosing where to zoom next.
//!
//! This is maths over `Escape` data with no presentation in it, which is why it
//! is in core rather than the frontend: a tool rendering a zoom *video* wants
//! exactly the same answer to "where is there still detail?" as the interactive
//! explorer does.

use crate::Escape;
use crate::complex::Complex;
use crate::sample::SampleGrid;
use crate::viewport::Viewport;

/// Below this fraction of interior samples, a view is effectively empty.
const MIN_INTERESTING: f64 = 0.005;

/// Above this fraction, it is effectively solid.
const MAX_INTERESTING: f64 = 0.995;

/// The fraction of samples classified interior.
///
/// The "is this worth looking at" metric. A view is interesting when it
/// contains *both* the set and its outside — all one or all the other means the
/// boundary, which is the only part with any structure, is off screen.
#[must_use]
pub fn interior_fraction(grid: &SampleGrid) -> f64 {
    let total = grid.cols() * grid.rows();
    if total == 0 {
        return 0.0;
    }
    let interior = grid.samples().filter(|e| e.is_interior()).count();
    interior as f64 / total as f64
}

/// Whether a view has both the set and its outside in frame.
#[must_use]
pub fn is_interesting(grid: &SampleGrid) -> bool {
    (MIN_INTERESTING..=MAX_INTERESTING).contains(&interior_fraction(grid))
}

/// A point on the set's boundary that will still hold detail after zooming in.
///
/// The rule: among samples that escaped, take the one with the highest
/// iteration count that is directly adjacent to an interior sample. Escaping
/// slowly means being close to the set; touching an interior neighbour means
/// being on the boundary rather than merely near it. Together they find the
/// filigree, which is the part that survives magnification.
///
/// Returns `None` when the grid has no boundary at all — an entirely solid or
/// entirely empty view — which is the caller's signal to give up on this view
/// and look elsewhere rather than dive into a flat field.
///
/// Verified before it was written: diving with this rule, zooming 8× and
/// re-picking each time, reached 2.1×10⁶ magnification over eight steps with
/// the interior fraction never leaving 3.6%–92.8%, and twelve independent dives
/// of six steps produced no collapses.
#[must_use]
pub fn boundary_target(grid: &SampleGrid, vp: &Viewport) -> Option<Complex> {
    let mut best: Option<(u32, usize, usize)> = None;

    for iy in 0..grid.rows() {
        for ix in 0..grid.cols() {
            let Some(Escape::Escaped { iterations, .. }) = grid.get(ix, iy) else {
                continue;
            };
            if !touches_interior(grid, ix, iy) {
                continue;
            }
            if best.is_none_or(|(best_iterations, _, _)| iterations > best_iterations) {
                best = Some((iterations, ix, iy));
            }
        }
    }

    best.map(|(_, ix, iy)| vp.sample_to_plane(ix, iy))
}

/// Whether any of the four neighbours of `(ix, iy)` is interior.
///
/// Four-neighbour rather than eight: a diagonal-only touch happens at a corner
/// of the sample lattice and is as often an artefact of the sampling as a real
/// adjacency.
fn touches_interior(grid: &SampleGrid, ix: usize, iy: usize) -> bool {
    let neighbours = [
        (ix.wrapping_sub(1), iy),
        (ix + 1, iy),
        (ix, iy.wrapping_sub(1)),
        (ix, iy + 1),
    ];
    // `wrapping_sub` at an edge yields `usize::MAX`, which `get` rejects as out
    // of bounds — so edges simply have fewer neighbours, with no special case.
    neighbours
        .iter()
        .filter_map(|&(nx, ny)| grid.get(nx, ny))
        .any(Escape::is_interior)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fractal::{Fractal, Mandelbrot};
    use crate::sampler::sample_into;

    fn sampled(vp: &Viewport, limit: u32) -> SampleGrid {
        let mut grid = SampleGrid::new(0, 0);
        sample_into(&mut grid, vp, &Mandelbrot, limit);
        grid
    }

    #[test]
    fn the_home_view_is_interesting() {
        let vp = Viewport::home(60, 30, 0.5);
        let grid = sampled(&vp, 300);
        let fraction = interior_fraction(&grid);
        assert!(
            (0.05..0.60).contains(&fraction),
            "home view interior fraction was {fraction}"
        );
        assert!(is_interesting(&grid));
    }

    #[test]
    fn an_all_interior_view_is_not_interesting_and_has_no_target() {
        // Deep inside the cardioid: no boundary anywhere in frame.
        let mut vp = Viewport::home(30, 15, 0.5);
        vp.centre = Complex::new(-0.2, 0.0);
        vp.half_width = 0.05;
        let grid = sampled(&vp, 300);

        assert!((interior_fraction(&grid) - 1.0).abs() < 1e-9);
        assert!(!is_interesting(&grid));
        assert_eq!(boundary_target(&grid, &vp), None);
    }

    #[test]
    fn an_all_exterior_view_has_no_target() {
        let mut vp = Viewport::home(30, 15, 0.5);
        vp.centre = Complex::new(3.0, 3.0);
        vp.half_width = 0.5;
        let grid = sampled(&vp, 300);

        assert_eq!(interior_fraction(&grid), 0.0);
        assert!(!is_interesting(&grid));
        assert_eq!(boundary_target(&grid, &vp), None);
    }

    #[test]
    fn the_chosen_target_lies_on_the_boundary() {
        let vp = Viewport::home(60, 30, 0.5);
        let grid = sampled(&vp, 500);
        let target = boundary_target(&grid, &vp).expect("the home view has a boundary");

        // Within the view...
        assert!((target.re - vp.centre.re).abs() <= vp.half_width);
        assert!((target.im - vp.centre.im).abs() <= vp.half_height());

        // ...and genuinely near the set: it should take many iterations to
        // escape, not a handful.
        let escape = Mandelbrot.escape(target, 5_000);
        match escape {
            Escape::Escaped { iterations, .. } => {
                assert!(iterations > 20, "target escaped in only {iterations}");
            }
            Escape::Interior => {}
        }
    }

    #[test]
    fn a_full_dive_to_the_precision_wall_never_collapses() {
        // The previous version of this test stopped at 1e5 — about a third of
        // the log-depth a real dive covers, and the shallow third at that. The
        // deep regime is where the boundary degenerates into a filament and a
        // target's neighbourhood can resolve away under the next
        // magnification, so it is the only part worth testing.
        //
        // Both ends are thresholded. Near zero means the dive fell into empty
        // exterior; near one means it fell inside a bulb and the frame is solid
        // — equally broken, and the adjacency rule does not exclude it on its
        // own, because a bulb's rim satisfies "escaped sample touching an
        // interior sample" perfectly.
        let mut vp = Viewport::home(50, 25, 2.0);
        let mut steps = 0;
        let mut worst_low = 1.0_f64;
        let mut worst_high = 0.0_f64;

        loop {
            let grid = sampled(&vp, vp.suggested_limit());
            let fraction = interior_fraction(&grid);
            worst_low = worst_low.min(fraction);
            worst_high = worst_high.max(fraction);
            assert!(
                (MIN_INTERESTING..=MAX_INTERESTING).contains(&fraction),
                "step {steps} at {:.3e}x collapsed to {fraction:.4} interior",
                vp.magnification()
            );

            let Some(point) = boundary_target(&grid, &vp) else {
                panic!(
                    "step {steps} at {:.3e}x found no target",
                    vp.magnification()
                );
            };
            vp.centre = point;
            steps += 1;

            // Stop where the renderer would: `f64` exhausted.
            if vp.zoom_centre(0.125) {
                break;
            }
            assert!(steps < 64, "dive did not terminate");
        }

        // It must actually have gone deep, or the test proves nothing.
        assert!(
            vp.magnification() > 1e10,
            "only reached {:.3e}x in {steps} steps",
            vp.magnification()
        );
        assert!(steps >= 10, "only {steps} steps to the wall");
        // Recorded so a regression shows as a changed margin, not just a pass.
        assert!(
            worst_low > MIN_INTERESTING,
            "closest to empty: {worst_low:.4}"
        );
        assert!(
            worst_high < MAX_INTERESTING,
            "closest to solid: {worst_high:.4}"
        );
    }

    #[test]
    fn the_target_choice_is_deterministic() {
        // Ties at the highest iteration count are routine at depth. The scan
        // keeps the first maximum in row-major order (the comparison is strictly
        // greater), which makes dives reproducible — without that, the hang
        // fixed in the dive loop could not have had a regression test.
        let vp = Viewport::home(40, 20, 2.0);
        let grid = sampled(&vp, 400);
        let first = boundary_target(&grid, &vp);
        for _ in 0..5 {
            assert_eq!(boundary_target(&grid, &vp), first);
        }
    }

    #[test]
    fn diving_by_repeatedly_re_targeting_keeps_finding_detail() {
        // The property measured by hand before this was implemented, and the
        // regression guard for unattended auto-zoom: every step must stay
        // interesting and keep offering somewhere to go.
        let mut vp = Viewport::home(40, 20, 0.5);
        for step in 0..6 {
            let grid = sampled(&vp, vp.suggested_limit());
            let fraction = interior_fraction(&grid);
            assert!(
                (MIN_INTERESTING..=MAX_INTERESTING).contains(&fraction),
                "step {step}: view collapsed, interior fraction {fraction}"
            );
            let target = boundary_target(&grid, &vp)
                .unwrap_or_else(|| panic!("step {step}: no boundary target"));
            vp.centre = target;
            vp.zoom_centre(0.125);
        }
        assert!(vp.magnification() > 1e5, "{}", vp.magnification());
    }

    #[test]
    fn an_empty_grid_is_handled_rather_than_dividing_by_zero() {
        let vp = Viewport::home(0, 0, 0.5);
        let grid = SampleGrid::new(0, 0);
        assert_eq!(interior_fraction(&grid), 0.0);
        assert!(!is_interesting(&grid));
        assert_eq!(boundary_target(&grid, &vp), None);
    }
}
