//! The per-iteration step, which is all that differs between escape-time
//! fractals.
//!
//! Every fractal here iterates *something* and asks when it escapes. Only the
//! something differs, so that is the only thing a formula supplies — the loop,
//! the bailout and the continuous escape count are shared and cannot drift
//! between kernels.
//!
//! A trait with associated constants rather than a function pointer or an enum
//! matched inside the loop: each formula monomorphises into its own
//! specialised loop, so a cubic costs no branch for being a cubic.

/// One iteration of an escape-time fractal.
pub trait Formula {
    /// The name of the formula, for a status line.
    const NAME: &'static str;

    /// The degree of the iteration.
    ///
    /// **Load-bearing, and easy to get silently wrong.** The continuous escape
    /// count divides by `ln(DEGREE)`, because that is what makes it invariant
    /// under one more iteration: the count rises by one while `ln|z|` is
    /// multiplied by the degree. Use 2 for a cubic and the value jumps by 0.585
    /// at every band boundary — measured — which appears as banding in the
    /// render and as nothing at all in the maths.
    const DEGREE: f64;

    /// Advance `z` once, given the parameter `c`.
    ///
    /// Raw components rather than a `Complex`: the expanded form reuses the
    /// squares between the escape test and the next `z`, where going through
    /// operators would compute each one twice per iteration.
    fn step(zr: f64, zi: f64, cr: f64, ci: f64) -> (f64, f64);

    /// Where this formula's set sits on the plane, as a centre.
    ///
    /// Each formula needs its own framing or it opens off-centre. These were
    /// measured by finding the bounding box of the set rather than guessed.
    const HOME_CENTRE: (f64, f64);

    /// Half the width of a view that frames the whole set with a margin.
    const HOME_HALF_WIDTH: f64;

    /// Half the width of a view that frames this formula's Julia sets.
    ///
    /// A Julia set lives in the *dynamical* plane, so it needs its own framing
    /// and not [`HOME_HALF_WIDTH`](Formula::HOME_HALF_WIDTH), which frames the
    /// parameter plane.
    const JULIA_HALF_WIDTH: f64;

    /// A closed loop of parameters whose Julia sets are worth watching.
    ///
    /// Every formula with a `c` has a Julia family, and animating `c` around a
    /// loop is the one motion that changes the *shape* without ever running
    /// out of precision. Which loop matters enormously, and the obvious choice
    /// is wrong: a circle leaves the connectedness locus, and outside it the
    /// Julia set is Cantor dust that renders as a smooth featureless blob. The
    /// orbit this replaced was `0.7·(cos θ, sin θ)`, which spent **13 of 24
    /// sampled phases** at dust.
    ///
    /// So these hug the locus boundary instead. Each point is the radius along
    /// its ray — cast from `c = 0`, which is inside the main hyperbolic
    /// component of all five formulas because `step(0, 0, 0) = (0, 0)` — whose
    /// rendered Julia set carried the most structure, measured as the fraction
    /// of adjacent glyph pairs that differ. On that scale the old orbit's dust
    /// frames score 0.036 and Douady's rabbit 0.110; the weakest phase of
    /// every loop here clears 0.046 and the means run 0.11–0.23.
    ///
    /// Interpolated linearly and wrapped, so the loop must be *closed*: the
    /// last point joins the first, and a gap there would jerk once per cycle.
    const JULIA_ORBIT: &'static [(f64, f64)];
}

/// `z → z² + c`: the Mandelbrot and Julia sets.
#[derive(Debug, Clone, Copy, Default)]
pub struct Quadratic;

impl Formula for Quadratic {
    const NAME: &'static str = "quadratic";
    const DEGREE: f64 = 2.0;
    const HOME_CENTRE: (f64, f64) = (-0.75, 0.0);
    const HOME_HALF_WIDTH: f64 = 1.75;

    const JULIA_HALF_WIDTH: f64 = 1.7;

    #[rustfmt::skip]
    const JULIA_ORBIT: &'static [(f64, f64)] = &[
        (0.34652, 0.34652), (0.29574, 0.44261), (0.21704, 0.52398),
        (0.12423, 0.62456), (0.00000, 0.62865), (-0.16320, 0.82048),
        (-0.26083, 0.62969), (-0.38972, 0.58326), (-0.49978, 0.49978),
        (-0.63703, 0.42565), (-0.68598, 0.28414), (-0.81356, 0.16183),
        (-1.08460, 0.00000), (-0.81356, -0.16183), (-0.68598, -0.28414),
        (-0.63703, -0.42565), (-0.49978, -0.49978), (-0.38972, -0.58326),
        (-0.26083, -0.62969), (-0.16320, -0.82048), (-0.00000, -0.62865),
        (0.12423, -0.62456), (0.21704, -0.52398), (0.29574, -0.44261),
        (0.34652, -0.34652), (0.37229, -0.24876), (0.36311, -0.15040),
        (0.34644, -0.06891), (0.23250, 0.00000), (0.34644, 0.06891),
        (0.36311, 0.15040), (0.37229, 0.24876),
    ];

    fn step(zr: f64, zi: f64, cr: f64, ci: f64) -> (f64, f64) {
        (zr * zr - zi * zi + cr, 2.0 * zr * zi + ci)
    }
}

/// `z → (|Re z| + i|Im z|)² + c`: the Burning Ship.
///
/// The absolute values break the set's symmetry about the real axis, which is
/// the whole of its distinctive look — a measurement over the classic view puts
/// its mirror agreement at 69% where every other formula here is at 100%.
#[derive(Debug, Clone, Copy, Default)]
pub struct BurningShip;

impl Formula for BurningShip {
    const NAME: &'static str = "burning ship";
    const DEGREE: f64 = 2.0;
    const HOME_CENTRE: (f64, f64) = (-0.51, -0.54);
    const HOME_HALF_WIDTH: f64 = 1.9;

    const JULIA_HALF_WIDTH: f64 = 1.7;

    #[rustfmt::skip]
    const JULIA_ORBIT: &'static [(f64, f64)] = &[
        (0.37993, 0.37993), (0.10200, 0.15266), (0.18087, 0.43665),
        (0.06794, 0.34156), (0.00000, 0.26550), (-0.04963, 0.24951),
        (-0.08897, 0.21480), (-0.13820, 0.20683), (-0.18293, 0.18293),
        (-0.23165, 0.15478), (-0.29876, 0.12375), (-0.39074, 0.07772),
        (-1.08460, 0.00000), (-1.29934, -0.25846), (-1.03770, -0.42983),
        (-0.83438, -0.55751), (-0.66747, -0.66747), (-0.45329, -0.67840),
        (-0.31923, -0.77070), (-0.18541, -0.93214), (-0.00000, -0.99997),
        (0.21913, -1.10162), (0.44309, -1.06971), (0.71735, -1.07359),
        (0.26384, -0.26384), (0.25233, -0.16860), (0.23947, -0.09919),
        (0.24885, -0.04950), (0.23250, 0.00000), (0.31777, 0.06321),
        (0.39988, 0.16563), (0.17461, 0.11667),
    ];

    fn step(zr: f64, zi: f64, cr: f64, ci: f64) -> (f64, f64) {
        (zr * zr - zi * zi + cr, 2.0 * zr.abs() * zi.abs() + ci)
    }
}

/// `z → conj(z)² + c`: the Tricorn, or Mandelbar.
#[derive(Debug, Clone, Copy, Default)]
pub struct Tricorn;

impl Formula for Tricorn {
    const NAME: &'static str = "tricorn";
    const DEGREE: f64 = 2.0;
    const HOME_CENTRE: (f64, f64) = (-0.56, 0.0);
    const HOME_HALF_WIDTH: f64 = 1.9;

    const JULIA_HALF_WIDTH: f64 = 1.7;

    #[rustfmt::skip]
    const JULIA_ORBIT: &'static [(f64, f64)] = &[
        (0.25456, 0.25456), (0.32362, 0.48433), (0.17600, 0.42489),
        (0.06794, 0.34156), (0.00000, 0.28320), (-0.04808, 0.24171),
        (-0.08610, 0.20787), (-0.12500, 0.18708), (-0.18201, 0.18201),
        (-0.23165, 0.15478), (-0.24621, 0.10199), (-0.40499, 0.08056),
        (-1.08460, 0.00000), (-0.40499, -0.08056), (-0.24621, -0.10199),
        (-0.23165, -0.15478), (-0.18201, -0.18201), (-0.12500, -0.18708),
        (-0.08610, -0.20787), (-0.04808, -0.24171), (-0.00000, -0.28320),
        (0.06794, -0.34156), (0.17600, -0.42489), (0.32362, -0.48433),
        (0.25456, -0.25456), (0.25233, -0.16860), (0.24820, -0.10281),
        (0.24885, -0.04950), (0.23250, 0.00000), (0.24885, 0.04950),
        (0.24820, 0.10281), (0.25233, 0.16860),
    ];

    fn step(zr: f64, zi: f64, cr: f64, ci: f64) -> (f64, f64) {
        // Conjugating flips the sign of the cross term only.
        (zr * zr - zi * zi + cr, -2.0 * zr * zi + ci)
    }
}

/// `z → |Re(z²)| + i·Im(z²) + c`: the Celtic.
#[derive(Debug, Clone, Copy, Default)]
pub struct Celtic;

impl Formula for Celtic {
    const NAME: &'static str = "celtic";
    const DEGREE: f64 = 2.0;
    const HOME_CENTRE: (f64, f64) = (-0.84, 0.0);
    const HOME_HALF_WIDTH: f64 = 1.95;

    const JULIA_HALF_WIDTH: f64 = 1.95;

    #[rustfmt::skip]
    const JULIA_ORBIT: &'static [(f64, f64)] = &[
        (0.18187, 0.18187), (0.13682, 0.20476), (0.09377, 0.22637),
        (0.05099, 0.25634), (0.00000, 0.28859), (-0.06692, 0.33644),
        (-0.17801, 0.42975), (-0.70695, 1.05803), (-0.73177, 0.73177),
        (-0.73576, 0.49162), (-0.66566, 0.27572), (-1.15061, 0.22887),
        (-1.38985, 0.00000), (-1.17893, -0.23450), (-0.66566, -0.27572),
        (-0.75079, -0.50166), (-0.73924, -0.73924), (-0.69522, -1.04047),
        (-0.17801, -0.42975), (-0.06692, -0.33644), (-0.00000, -0.28859),
        (0.05099, -0.25634), (0.09377, -0.22637), (0.13682, -0.20476),
        (0.18187, -0.18187), (0.23206, -0.15506), (0.29680, -0.12294),
        (0.30100, -0.05987), (0.06435, 0.00000), (0.30100, 0.05987),
        (0.29680, 0.12294), (0.23206, 0.15506),
    ];

    fn step(zr: f64, zi: f64, cr: f64, ci: f64) -> (f64, f64) {
        ((zr * zr - zi * zi).abs() + cr, 2.0 * zr * zi + ci)
    }
}

/// `z → z³ + c`: the degree-3 Multibrot.
///
/// The one formula here that is not degree 2, which is why [`Formula::DEGREE`]
/// exists rather than the escape loop assuming a square.
#[derive(Debug, Clone, Copy, Default)]
pub struct Cubic;

impl Formula for Cubic {
    const NAME: &'static str = "cubic";
    const DEGREE: f64 = 3.0;
    const HOME_CENTRE: (f64, f64) = (0.03, 0.0);
    const HOME_HALF_WIDTH: f64 = 1.6;

    const JULIA_HALF_WIDTH: f64 = 1.4;

    #[rustfmt::skip]
    const JULIA_ORBIT: &'static [(f64, f64)] = &[
        (0.58819, 0.58819), (0.41128, 0.61552), (0.28786, 0.69496),
        (0.21811, 1.09652), (0.00000, 1.08256), (-0.21811, 1.09652),
        (-0.28786, 0.69496), (-0.41128, 0.61552), (-0.58819, 0.58819),
        (-0.54934, 0.36705), (-0.57473, 0.23806), (-0.50746, 0.10094),
        (-0.34560, 0.00000), (-0.50746, -0.10094), (-0.57473, -0.23806),
        (-0.54934, -0.36705), (-0.58819, -0.58819), (-0.41128, -0.61552),
        (-0.28786, -0.69496), (-0.21811, -1.09652), (-0.00000, -1.08256),
        (0.21811, -1.09652), (0.28786, -0.69496), (0.41128, -0.61552),
        (0.58819, -0.58819), (0.54934, -0.36705), (0.57473, -0.23806),
        (0.50746, -0.10094), (0.34560, 0.00000), (0.50746, 0.10094),
        (0.57473, 0.23806), (0.54934, 0.36705),
    ];

    fn step(zr: f64, zi: f64, cr: f64, ci: f64) -> (f64, f64) {
        // (a+bi)³ = a³ - 3ab² + (3a²b - b³)i
        let zr2 = zr * zr;
        let zi2 = zi * zi;
        (
            zr * zr2 - 3.0 * zr * zi2 + cr,
            3.0 * zr2 * zi - zi * zi2 + ci,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Iterate `F` from the origin and report whether it stayed bounded.
    fn bounded<F: Formula>(cr: f64, ci: f64, limit: u32) -> bool {
        let (mut zr, mut zi) = (0.0, 0.0);
        for _ in 0..limit {
            if zr * zr + zi * zi > 65536.0 {
                return false;
            }
            let next = F::step(zr, zi, cr, ci);
            zr = next.0;
            zi = next.1;
        }
        true
    }

    /// What fraction of a formula's home view is interior.
    fn interior_fraction<F: Formula>() -> f64 {
        let (cx, cy) = F::HOME_CENTRE;
        let half_w = F::HOME_HALF_WIDTH;
        let half_h = half_w / 2.0;
        let mut inside = 0;
        let mut total = 0;
        for j in 0..60 {
            for i in 0..60 {
                let cr = cx - half_w + 2.0 * half_w * f64::from(i) / 60.0;
                let ci = cy - half_h + 2.0 * half_h * f64::from(j) / 60.0;
                total += 1;
                if bounded::<F>(cr, ci, 300) {
                    inside += 1;
                }
            }
        }
        f64::from(inside) / f64::from(total)
    }

    #[test]
    fn every_formula_frames_a_set_worth_looking_at() {
        // A formula whose home view is empty or solid would open on a blank
        // screen. The framings were measured from each set's bounding box, and
        // this is what keeps them honest.
        let surveyed = [
            ("quadratic", interior_fraction::<Quadratic>()),
            ("burning ship", interior_fraction::<BurningShip>()),
            ("tricorn", interior_fraction::<Tricorn>()),
            ("celtic", interior_fraction::<Celtic>()),
            ("cubic", interior_fraction::<Cubic>()),
        ];
        for (name, fraction) in surveyed {
            assert!(
                (0.04..0.60).contains(&fraction),
                "{name} home view is {:.1}% interior",
                fraction * 100.0
            );
        }
    }

    #[test]
    fn the_quadratic_family_is_symmetric_about_the_real_axis() {
        // All of these conjugate cleanly, so `c` and its conjugate must agree.
        for (cr, ci) in [(-1.0, 0.3), (0.2, 0.5), (-1.7, 0.1), (0.3, 0.9)] {
            for (name, a, b) in [
                (
                    "quadratic",
                    bounded::<Quadratic>(cr, ci, 400),
                    bounded::<Quadratic>(cr, -ci, 400),
                ),
                (
                    "tricorn",
                    bounded::<Tricorn>(cr, ci, 400),
                    bounded::<Tricorn>(cr, -ci, 400),
                ),
                (
                    "celtic",
                    bounded::<Celtic>(cr, ci, 400),
                    bounded::<Celtic>(cr, -ci, 400),
                ),
            ] {
                assert_eq!(a, b, "{name} asymmetric at {cr}+{ci}i");
            }
        }
    }

    #[test]
    fn the_burning_ship_is_deliberately_not_symmetric() {
        // Its `|Im z|` breaks the mirror, and that asymmetry is the whole of
        // its distinctive shape. If this ever passes as symmetric, the absolute
        // values have been lost and it has quietly become a Mandelbrot.
        let asymmetric = (0..60)
            .filter(|i| {
                let ci = 0.02 + 0.03 * f64::from(*i % 20);
                let cr = -1.8 + 0.05 * f64::from(*i);
                bounded::<BurningShip>(cr, ci, 300) != bounded::<BurningShip>(cr, -ci, 300)
            })
            .count();
        assert!(asymmetric > 3, "only {asymmetric} asymmetric samples");
    }

    #[test]
    fn the_cubic_has_two_fold_rotational_symmetry() {
        // `z³` means `c` and `-c` give the same picture rotated, so membership
        // must agree. It is the cheapest check that the cubic expansion is
        // right rather than merely plausible.
        for i in 0..40 {
            let cr = -1.2 + 0.06 * f64::from(i);
            let ci = 0.31;
            assert_eq!(
                bounded::<Cubic>(cr, ci, 400),
                bounded::<Cubic>(-cr, -ci, 400),
                "cubic asymmetric at {cr}+{ci}i"
            );
        }
    }

    #[test]
    fn known_points_behave() {
        assert!(bounded::<Quadratic>(0.0, 0.0, 2_000));
        assert!(!bounded::<Quadratic>(2.0, 0.0, 2_000));
        assert!(bounded::<Tricorn>(0.0, 0.0, 2_000));
        assert!(!bounded::<Tricorn>(2.0, 0.0, 2_000));
        // The Burning Ship's hull sits around -1.755 on the real axis.
        assert!(bounded::<BurningShip>(-1.755, 0.0, 2_000));
    }

    #[test]
    fn only_the_cubic_is_degree_three() {
        assert_eq!(Quadratic::DEGREE, 2.0);
        assert_eq!(BurningShip::DEGREE, 2.0);
        assert_eq!(Tricorn::DEGREE, 2.0);
        assert_eq!(Celtic::DEGREE, 2.0);
        assert_eq!(Cubic::DEGREE, 3.0);
    }
}
