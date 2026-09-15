//! Walking a Julia parameter around a closed loop.
//!
//! This is the motion that changes a fractal's *shape*. Zooming changes scale
//! and runs into `f64` after a few decades; a parameter orbit runs forever,
//! because it never leaves the region the loop was measured in.
//!
//! In core rather than the frontend for the same reason as [`target`]: "which
//! parameters are worth looking at" is maths over escape data with no
//! presentation in it, and a tool rendering a parameter sweep to video would
//! want exactly this.
//!
//! [`target`]: crate::target

use crate::complex::Complex;
use crate::formula::Formula;

/// The Julia parameter `turns` of the way around `F`'s orbit loop.
///
/// `turns` is in revolutions and wraps, so a caller can accumulate it forever
/// without normalising — which matters, because the alternative is an animation
/// that stutters and then stalls after a day of uptime once the per-frame
/// increment goes small relative to the accumulated total.
#[must_use]
pub fn julia_parameter<F: Formula>(turns: f64) -> Complex {
    parameter_on(F::JULIA_ORBIT, turns)
}

/// The parameter `turns` of the way around an explicit closed loop.
///
/// Split from [`julia_parameter`] so the interpolation is testable against a
/// hand-written loop, with no formula and no iteration in the test.
#[must_use]
pub fn parameter_on(points: &[(f64, f64)], turns: f64) -> Complex {
    // A loop with nothing in it has no parameter to give. Returning the origin
    // rather than panicking because this runs once a frame inside the render
    // loop, and `0` is in the main component of every formula here, so the
    // fallback is a real Julia set rather than a blank screen.
    if points.is_empty() {
        return Complex::ZERO;
    }
    let count = points.len();
    // `rem_euclid` rather than `%`: a negative `turns` must wrap forward, or
    // running an orbit backwards indexes from the wrong end of the loop.
    let position = turns.rem_euclid(1.0) * count as f64;
    // `rem_euclid(1.0)` can return exactly 1.0 for a tiny negative input, and
    // NaN propagates, so the index is clamped rather than trusted.
    let index = if position.is_finite() {
        (position as usize).min(count - 1)
    } else {
        0
    };
    let fraction = if position.is_finite() {
        position - index as f64
    } else {
        0.0
    };

    let (ar, ai) = points[index];
    // Wrapping to 0 is what closes the loop: the last point interpolates back
    // to the first, so a full revolution returns exactly where it started.
    let (br, bi) = points[(index + 1) % count];
    Complex::new(ar + (br - ar) * fraction, ai + (bi - ai) * fraction)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::escape::BAILOUT;
    use crate::formula::{BurningShip, Celtic, Cubic, Quadratic, Tricorn};

    /// Whether `c`'s critical orbit stays bounded — i.e. `c` is in the
    /// connectedness locus, so its Julia set is connected rather than dust.
    fn in_locus<F: Formula>(c: Complex, limit: u32) -> bool {
        let (mut zr, mut zi) = (0.0, 0.0);
        for _ in 0..limit {
            if zr * zr + zi * zi > BAILOUT {
                return false;
            }
            let next = F::step(zr, zi, c.re, c.im);
            zr = next.0;
            zi = next.1;
        }
        true
    }

    /// How much the rendered picture varies between neighbouring samples.
    ///
    /// The fraction of adjacent sample pairs landing in different log-spaced
    /// escape bands, which is what the glyph ramp keys off. It is the measure
    /// the baked loops were chosen against, reproduced here without reaching
    /// for the frontend. On a 72x32 grid at limit 200, dust scores 0.0595
    /// because it is a smooth gradient and Douady's rabbit 0.1292 because it
    /// has filigree; the baked loops' weakest phases run 0.0657 to 0.0879.
    pub(super) fn structure<F: Formula>(c: Complex, half_width: f64) -> f64 {
        const COLS: usize = 72;
        const ROWS: usize = 32;
        const LIMIT: u32 = 200;
        let band = |zr0: f64, zi0: f64| -> usize {
            let (mut zr, mut zi) = (zr0, zi0);
            for n in 0..LIMIT {
                let mag = zr * zr + zi * zi;
                if mag > BAILOUT {
                    // The frontend's exact bucketing, reproduced rather than
                    // approximated: ten ramp steps over a log-compressed count
                    // with a fixed 512 reference. An invented band spacing gave
                    // this measure far more bands than the ramp has, which
                    // scored a smooth dust gradient as though it were filigree
                    // and put one loop's minimum *below* the dust baseline.
                    let t = (1.0 + f64::from(n)).ln() / (1.0 + 512.0_f64).ln();
                    return ((t * 10.0) as usize).min(9) + 1;
                }
                let next = F::step(zr, zi, c.re, c.im);
                zr = next.0;
                zi = next.1;
            }
            0
        };
        let at = |ix: usize, iy: usize| {
            let x = -half_width + 2.0 * half_width * ix as f64 / (COLS - 1) as f64;
            let y = -half_width + 2.0 * half_width * iy as f64 / (ROWS - 1) as f64;
            band(x, y)
        };
        let mut differing = 0;
        let mut pairs = 0;
        for iy in 0..ROWS {
            for ix in 0..COLS {
                let here = at(ix, iy);
                if ix + 1 < COLS {
                    pairs += 1;
                    differing += usize::from(here != at(ix + 1, iy));
                }
                if iy + 1 < ROWS {
                    pairs += 1;
                    differing += usize::from(here != at(ix, iy + 1));
                }
            }
        }
        differing as f64 / pairs as f64
    }

    #[test]
    fn a_full_revolution_returns_to_the_start() {
        // The loop has to be closed or the animation jerks once per cycle, and
        // a gap between the last point and the first is invisible in a diff.
        let points = &[(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)];
        assert_eq!(parameter_on(points, 0.0), parameter_on(points, 1.0));
        assert_eq!(parameter_on(points, 0.0), parameter_on(points, 7.0));
        // And approaching a full turn from below arrives at the start rather
        // than somewhere else, which is what "closed" actually means.
        let nearly = parameter_on(points, 1.0 - 1e-9);
        assert!(
            nearly.re.abs() < 1e-6 && nearly.im.abs() < 1e-6,
            "the loop does not close: {nearly:?}"
        );
    }

    #[test]
    fn the_parameter_moves_continuously() {
        // A jump mid-cycle would read as the fractal being replaced rather
        // than morphing, which is the whole point of the motion.
        let points = &[(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)];
        let mut previous = parameter_on(points, 0.0);
        for i in 1..=400 {
            let here = parameter_on(points, f64::from(i) / 400.0);
            let hop = (here.re - previous.re).hypot(here.im - previous.im);
            assert!(hop < 0.02, "jumped {hop} at turn {}", f64::from(i) / 400.0);
            previous = here;
        }
    }

    #[test]
    fn interpolation_walks_each_edge_in_order() {
        let points = &[(0.0, 0.0), (2.0, 0.0)];
        // A two-point loop is out and back: a quarter turn is halfway out.
        assert_eq!(parameter_on(points, 0.25), Complex::new(1.0, 0.0));
        assert_eq!(parameter_on(points, 0.5), Complex::new(2.0, 0.0));
        assert_eq!(parameter_on(points, 0.75), Complex::new(1.0, 0.0));
    }

    #[test]
    fn a_negative_turn_wraps_forward_rather_than_indexing_backwards() {
        let points = &[(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)];
        assert_eq!(parameter_on(points, -0.25), parameter_on(points, 0.75));
        assert_eq!(parameter_on(points, -1.0), parameter_on(points, 0.0));
    }

    #[test]
    fn a_degenerate_loop_yields_a_usable_parameter_rather_than_panicking() {
        // This runs once a frame in the render loop, so it must not panic. The
        // origin is in the main component of every formula here, so the
        // fallback is a real Julia set and not a blank screen.
        assert_eq!(parameter_on(&[], 0.3), Complex::ZERO);
        assert_eq!(parameter_on(&[(0.5, 0.5)], 0.9), Complex::new(0.5, 0.5));
        // A NaN must not index out of bounds.
        let points = &[(0.0, 0.0), (1.0, 0.0)];
        let _ = parameter_on(points, f64::NAN);
        let _ = parameter_on(points, f64::INFINITY);
    }

    #[test]
    fn every_baked_orbit_hugs_its_connectedness_locus() {
        // The property that stops the orbit showing dust, and the reason this
        // feature exists. Outside the locus a Julia set is a Cantor set with no
        // interior, which the renderer draws as a smooth featureless blob —
        // exactly what the orbit this replaced did for 13 of 24 sampled phases.
        //
        // "In the locus" is not decidable, so this asserts what is: no vertex
        // escapes within 1,500 iterations. Measured margins — the quadratic,
        // tricorn and cubic loops survive 20,000 outright; the burning ship and
        // celtic each have two vertices a hair outside, escaping at 3,705 and
        // 5,349. Both are far beyond `suggested_limit`'s ceiling, so those
        // Julia sets render as connected at any depth this program reaches.
        //
        // The 1,500 is therefore a floor with 2.5x headroom, not a measurement:
        // it fails if a future loop wanders genuinely outside, which is the
        // regression worth catching. An earlier table did exactly that — one
        // vertex sat at 0.255, just past the quadratic's real limit of 0.25 —
        // and this test is what found it.
        macro_rules! check {
            ($formula:ty) => {{
                let orbit = <$formula as Formula>::JULIA_ORBIT;
                assert!(orbit.len() >= 8, "too few points to interpolate smoothly");
                for &(re, im) in orbit {
                    let c = Complex::new(re, im);
                    assert!(
                        in_locus::<$formula>(c, 1_500),
                        "{} orbit leaves the locus at {c:?}",
                        <$formula as Formula>::NAME
                    );
                }
            }};
        }
        check!(Quadratic);
        check!(BurningShip);
        check!(Tricorn);
        check!(Celtic);
        check!(Cubic);
    }

    #[test]
    fn every_baked_orbit_is_closed_without_a_long_hop() {
        // Interpolation is linear between neighbours, so a long edge is a
        // stretch of the cycle crossing parameter space quickly — and it is
        // where the path is most likely to leave the locus.
        macro_rules! check {
            ($formula:ty, $limit:expr) => {{
                let orbit = <$formula as Formula>::JULIA_ORBIT;
                let worst = (0..orbit.len())
                    .map(|i| {
                        let (ar, ai) = orbit[i];
                        let (br, bi) = orbit[(i + 1) % orbit.len()];
                        (br - ar).hypot(bi - ai)
                    })
                    .fold(0.0_f64, f64::max);
                assert!(
                    worst <= $limit,
                    "{} has a {worst:.3} hop, over the {} budget",
                    <$formula as Formula>::NAME,
                    $limit
                );
            }};
        }
        // Measured worst hops: quadratic 0.316, cubic 0.408, tricorn 0.684,
        // celtic 0.821, burning ship 0.928. The asymmetric sets need a looser
        // budget because their loops genuinely have to cross further — the
        // ship's locus is not star-shaped about the origin the rays are cast
        // from, so one edge of its loop has real distance to cover.
        check!(Quadratic, 0.45);
        check!(Cubic, 0.45);
        check!(Tricorn, 0.75);
        check!(Celtic, 0.95);
        check!(BurningShip, 1.05);
    }

    #[test]
    fn no_phase_of_any_orbit_renders_as_a_featureless_blob() {
        // The regression guard for the failure this feature exists to fix.
        // Every phase must carry visibly more structure than dust does, or the
        // animation goes flat part of the way round and looks broken.
        //
        // The dust baseline is measured in the same breath rather than
        // hardcoded, so the threshold cannot drift away from what it means.
        // `c = 0.7` is real and just past the quadratic's limit of 0.25, so it
        // is squarely outside the locus: the frame it renders is the smooth
        // oval this test exists to keep off the screen.
        let dust = structure::<Quadratic>(Complex::new(0.7, 0.0), 1.7);
        // And a known-good set must clearly beat it, or the measure is not
        // separating anything and the assertion below would be vacuous.
        let rabbit = structure::<Quadratic>(Complex::new(-0.123, 0.745), 1.7);
        assert!(
            rabbit > dust * 1.8,
            "the measure does not separate dust {dust:.4} from filigree {rabbit:.4}"
        );

        macro_rules! check {
            ($formula:ty) => {{
                let half = <$formula as Formula>::JULIA_HALF_WIDTH;
                for phase in 0..24 {
                    let turns = f64::from(phase) / 24.0;
                    let c = julia_parameter::<$formula>(turns);
                    let score = structure::<$formula>(c, half);
                    // Only 1.05x, and the slim margin is deliberate rather
                    // than lazy: measured minima are quadratic 0.0728,
                    // tricorn 0.0728, celtic 0.0844, cubic 0.0879 and burning
                    // ship 0.0657 against a 0.0595 baseline — so the ship's
                    // weakest phase really is within 10% of dust on this
                    // coarse measure. A tighter bound would fail on a loop
                    // that is visually fine. The sharp guard on dust is
                    // `every_baked_orbit_hugs_its_connectedness_locus`; this
                    // one catches a loop that goes flat while staying in the
                    // locus, which the other cannot see.
                    assert!(
                        score > dust * 1.05,
                        "{} at turn {turns:.2} (c = {c:?}) scores {score:.4}, \
                         at the {dust:.4} dust baseline",
                        <$formula as Formula>::NAME
                    );
                }
            }};
        }
        check!(Quadratic);
        check!(BurningShip);
        check!(Tricorn);
        check!(Celtic);
        check!(Cubic);
    }

    #[test]
    fn the_orbit_visits_genuinely_different_julia_sets() {
        // An orbit that barely moved would be a still picture with extra
        // machinery. Opposite phases must differ substantially.
        let near = julia_parameter::<Quadratic>(0.0);
        let far = julia_parameter::<Quadratic>(0.5);
        let apart = (near.re - far.re).hypot(near.im - far.im);
        assert!(apart > 0.5, "the orbit only spans {apart}");
    }
}

#[cfg(test)]
mod calibration {
    use super::*;
    use crate::complex::Complex;
    use crate::formula::{BurningShip, Celtic, Cubic, Quadratic, Tricorn};

    #[test]
    #[ignore = "diagnostic: prints the structure scale rather than asserting"]
    fn print_structure_scale() {
        let s = |c: Complex, h: f64| super::tests::structure::<Quadratic>(c, h);
        println!("dust   c=+0.70      {:.4}", s(Complex::new(0.7, 0.0), 1.7));
        println!("good   c=-0.70      {:.4}", s(Complex::new(-0.7, 0.0), 1.7));
        println!(
            "rabbit c=-0.123+.745i {:.4}",
            s(Complex::new(-0.123, 0.745), 1.7)
        );
        macro_rules! scan {
            ($f:ty, $name:expr) => {{
                let half = <$f as Formula>::JULIA_HALF_WIDTH;
                let mut lo = f64::MAX;
                let mut sum = 0.0;
                for p in 0..24 {
                    let t = f64::from(p) / 24.0;
                    let v = super::tests::structure::<$f>(julia_parameter::<$f>(t), half);
                    lo = lo.min(v);
                    sum += v;
                }
                println!("{:<14} min {lo:.4}  mean {:.4}", $name, sum / 24.0);
            }};
        }
        scan!(Quadratic, "quadratic");
        scan!(BurningShip, "burning ship");
        scan!(Tricorn, "tricorn");
        scan!(Celtic, "celtic");
        scan!(Cubic, "cubic");
    }
}
