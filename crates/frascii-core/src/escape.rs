//! The escape-time loop every kernel shares.

use crate::Escape;
use crate::complex::Complex;

/// The bailout radius.
///
/// 256, not 2, and the size is load-bearing rather than cautious. The
/// continuous iteration count below divides by `ln|z|`, and that estimate is
/// only smooth once `|z|` is far outside the escape radius — at a bailout of 2
/// the correction term is visibly wrong and the palette bands along the escape
/// contours, which is the exact artefact continuous colouring exists to remove.
/// Verified by computing the correction across an escape band: monotonic at
/// 2^8, lumpy at 2.
pub const BAILOUT: f64 = 256.0;

/// `BAILOUT` squared, so the loop compares against a squared magnitude and
/// never takes a square root.
const BAILOUT_SQ: f64 = BAILOUT * BAILOUT;

/// Iterate `z -> z² + c` from `z0` until it escapes or `limit` is reached.
///
/// This is the whole of the maths. A kernel's job is only to decide what `z0`
/// and `c` are for a given point on the plane — Mandelbrot fixes `z0` and
/// varies `c`, Julia does the reverse — which is why there is one loop here and
/// not one per fractal.
///
/// # Smooth iteration count
///
/// `Escaped::smooth` is the continuous escape time,
/// `n - log₂(ln|z| / ln BAILOUT)`, which places a point *within* its escape
/// band instead of only naming the band. A renderer that colours by the integer
/// count draws visible contour steps; this is what lets it interpolate.
///
/// The value is clamped to be non-negative. It can only go negative when `z0`
/// is already outside the bailout radius before a single iteration runs — which
/// Mandelbrot cannot do (`z0` is the origin) but Julia can, since there `z0` is
/// the sampled point and the view may extend past `|z| = 256`.
#[must_use]
pub fn escape_time(z0: Complex, c: Complex, limit: u32) -> Escape {
    // Raw f64 rather than `Complex` arithmetic: the expanded form reuses
    // `zr2`/`zi2` for both the escape test and the next `z`, where going
    // through `Mul` would square each component twice per iteration.
    let mut zr = z0.re;
    let mut zi = z0.im;
    let (cr, ci) = (c.re, c.im);

    for n in 0..limit {
        let zr2 = zr * zr;
        let zi2 = zi * zi;
        let mag_sq = zr2 + zi2;

        if mag_sq > BAILOUT_SQ {
            return Escape::Escaped {
                iterations: n,
                smooth: smooth_count(n, mag_sq),
            };
        }

        // `zi` first: it reads the pre-update `zr`.
        zi = 2.0 * zr * zi + ci;
        zr = zr2 - zi2 + cr;
    }

    Escape::Interior
}

/// The continuous escape time for a point that escaped on iteration `n` with
/// squared magnitude `mag_sq`.
fn smooth_count(n: u32, mag_sq: f64) -> f64 {
    // ln|z| = ln(√mag_sq) = ½·ln(mag_sq), which avoids the square root.
    let log_zn = 0.5 * mag_sq.ln();
    let nu = f64::from(n) - (log_zn / BAILOUT.ln()).ln() / core::f64::consts::LN_2;
    nu.max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mandelbrot's binding: `z0` is the origin, `c` is the sampled point.
    fn mandel(re: f64, im: f64, limit: u32) -> Escape {
        escape_time(Complex::ZERO, Complex::new(re, im), limit)
    }

    #[test]
    fn known_interior_points_do_not_escape() {
        // Every one of these was verified by iterating to 20,000 before being
        // written down. `0.3 + 0.5i` in particular *looks* like it should
        // escape and does not — its orbit is a bounded cycle.
        for (re, im) in [(0.0, 0.0), (-1.0, 0.0), (-0.5, 0.5), (0.3, 0.5)] {
            assert!(
                mandel(re, im, 5_000).is_interior(),
                "{re}+{im}i should be interior"
            );
        }
    }

    #[test]
    fn a_point_far_outside_escapes_almost_at_once() {
        let Escape::Escaped { iterations, .. } = mandel(2.0, 0.0, 100) else {
            panic!("2+0i escapes");
        };
        assert_eq!(
            iterations, 4,
            "2+0i exceeds a bailout of 256 on iteration 4"
        );
    }

    #[test]
    fn the_smooth_count_matches_the_reference_value() {
        // Computed independently before implementing: c = 2 gives 3.6080.
        let Escape::Escaped { smooth, .. } = mandel(2.0, 0.0, 100) else {
            panic!("2+0i escapes");
        };
        assert!((smooth - 3.6080).abs() < 1e-3, "smooth was {smooth}");

        // And c = 0.26, just outside the cardioid's cusp, gives 31.99.
        let Escape::Escaped { smooth, .. } = mandel(0.26, 0.0, 5_000) else {
            panic!("0.26 escapes");
        };
        assert!((smooth - 31.9917).abs() < 1e-3, "smooth was {smooth}");
    }

    #[test]
    fn the_smooth_count_sits_inside_its_own_escape_band() {
        // The invariant that makes it useful for interpolation: it refines the
        // integer count rather than replacing it.
        for i in 0..40 {
            let re = 0.25 + 0.004 * f64::from(i);
            if let Escape::Escaped { iterations, smooth } = mandel(re, 0.0, 5_000) {
                let n = f64::from(iterations);
                assert!(
                    (n - 1.0..=n + 1.0).contains(&smooth),
                    "n={iterations} smooth={smooth} for c={re}"
                );
            }
        }
    }

    #[test]
    fn the_smooth_count_decreases_monotonically_away_from_the_set() {
        // Escaping sooner must mean a smaller continuous count, with no
        // reversals — a reversal would show up as a seam in the palette.
        let mut previous = f64::INFINITY;
        for i in 0..12 {
            let re = 0.2504 + 0.0004 * f64::from(i);
            let Escape::Escaped { smooth, .. } = mandel(re, 0.0, 20_000) else {
                continue;
            };
            assert!(smooth < previous, "not monotonic at c={re}: {smooth}");
            previous = smooth;
        }
    }

    #[test]
    fn a_seed_already_past_the_bailout_is_clamped_to_zero() {
        // Julia can sample a point outside |z| = 256; the smooth count must not
        // go negative there.
        let escape = escape_time(Complex::new(1e6, 0.0), Complex::new(0.0, 0.0), 50);
        let Escape::Escaped { iterations, smooth } = escape else {
            panic!("a huge seed escapes immediately");
        };
        assert_eq!(iterations, 0);
        assert_eq!(smooth, 0.0);
    }

    #[test]
    fn a_zero_limit_reports_interior_rather_than_iterating() {
        assert!(mandel(2.0, 0.0, 0).is_interior());
    }
}
