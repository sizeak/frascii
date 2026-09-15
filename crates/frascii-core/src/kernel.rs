//! A run-time choice of fractal that is still cheap to compare.

use crate::complex::Complex;
use crate::formula;
use crate::fractal::{Fractal, Julia, Parameter};
use crate::orbit::julia_parameter;
use crate::sample::SampleGrid;
use crate::sampler::sample_into;
use crate::viewport::Viewport;

/// Which formula to iterate.
///
/// Separate from [`Kernel`] because the formula and the *plane* are independent
/// choices: every formula has both a parameter plane and a family of Julia sets,
/// and conflating them is what left four of the five formulas with no Julia at
/// all. Five formulas times two planes is ten fractals from two small enums.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FormulaKind {
    /// `z → z² + c`.
    #[default]
    Quadratic,
    /// `z → (|Re z| + i|Im z|)² + c`.
    BurningShip,
    /// `z → conj(z)² + c`.
    Tricorn,
    /// `z → |Re(z²)| + i·Im(z²) + c`.
    Celtic,
    /// `z → z³ + c`.
    Cubic,
}

impl FormulaKind {
    /// Every formula, in switching order.
    pub const ALL: &'static [Self] = &[
        Self::Quadratic,
        Self::BurningShip,
        Self::Tricorn,
        Self::Celtic,
        Self::Cubic,
    ];

    /// The next formula in switching order.
    #[must_use]
    pub fn next(self) -> Self {
        let index = Self::ALL.iter().position(|f| *f == self).unwrap_or(0);
        Self::ALL[(index + 1) % Self::ALL.len()]
    }

    /// The name of this formula's *parameter plane* set.
    ///
    /// The quadratic's is "mandelbrot" and the cubic's "multibrot³" because
    /// those are what people call them; the rest are named after the formula.
    #[must_use]
    pub const fn parameter_name(self) -> &'static str {
        match self {
            Self::Quadratic => "mandelbrot",
            Self::BurningShip => "burning ship",
            Self::Tricorn => "tricorn",
            Self::Celtic => "celtic",
            Self::Cubic => "multibrot³",
        }
    }

    /// The name of this formula's Julia family.
    #[must_use]
    pub const fn julia_name(self) -> &'static str {
        match self {
            Self::Quadratic => "julia",
            Self::BurningShip => "burning ship julia",
            Self::Tricorn => "tricorn julia",
            Self::Celtic => "celtic julia",
            Self::Cubic => "cubic julia",
        }
    }

    /// The Julia parameter `turns` of the way around this formula's loop.
    ///
    /// The dispatch that makes the orbit work for every formula rather than
    /// only the quadratic.
    #[must_use]
    pub fn julia_parameter(self, turns: f64) -> Complex {
        match self {
            Self::Quadratic => julia_parameter::<formula::Quadratic>(turns),
            Self::BurningShip => julia_parameter::<formula::BurningShip>(turns),
            Self::Tricorn => julia_parameter::<formula::Tricorn>(turns),
            Self::Celtic => julia_parameter::<formula::Celtic>(turns),
            Self::Cubic => julia_parameter::<formula::Cubic>(turns),
        }
    }
}

/// Which fractal to sample: a formula, and which of its two planes.
///
/// A closed enum rather than a boxed [`Fractal`](crate::Fractal), and the
/// reason is not taste. A frontend has to answer "did anything about the
/// samples change?" every frame, which means the choice of fractal must be
/// `Copy + PartialEq` — and a trait object is neither. Using `Box<dyn Fractal>`
/// there would silently break the comparison that decides whether the kernel
/// needs to run again.
///
/// It also puts the dispatch in the right place: [`Kernel::sample_into`]
/// matches once per frame and hands a concrete type to the sampler, so the
/// kernel inlines into the row loop rather than costing an indirect call per
/// sample.
///
/// The trait still exists and is still the extension point — this is the subset
/// a *caller* can hold in a comparable struct.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kernel {
    /// A formula's parameter plane: `z₀ = 0`, and the sampled point is `c`.
    Parameter(FormulaKind),
    /// One of a formula's Julia sets: the sampled point is `z₀`.
    Julia(FormulaKind, Complex),
}

impl Default for Kernel {
    fn default() -> Self {
        Self::MANDELBROT
    }
}

impl Kernel {
    /// The Mandelbrot set — the quadratic's parameter plane.
    pub const MANDELBROT: Self = Self::Parameter(FormulaKind::Quadratic);

    /// Which formula this kernel iterates.
    #[must_use]
    pub const fn formula(self) -> FormulaKind {
        match self {
            Self::Parameter(formula) | Self::Julia(formula, _) => formula,
        }
    }

    /// Whether this kernel has a live `c` the UI can move.
    ///
    /// A parameter plane has none — the sampled point *is* `c` — so the orbit
    /// drive is meaningless there and the UI needs to know before offering it.
    #[must_use]
    pub const fn is_julia(self) -> bool {
        matches!(self, Self::Julia(..))
    }

    /// The same formula seen in its other plane.
    ///
    /// Switching to a Julia set lands at the start of that formula's orbit
    /// rather than at whatever `c` was left over, so the flip always shows a
    /// set worth looking at.
    #[must_use]
    pub fn flip_plane(self) -> Self {
        match self {
            Self::Parameter(formula) => Self::julia_of(formula),
            Self::Julia(formula, _) => Self::Parameter(formula),
        }
    }

    /// This formula's Julia set at the start of its orbit.
    #[must_use]
    pub fn julia_of(formula: FormulaKind) -> Self {
        Self::Julia(formula, formula.julia_parameter(0.0))
    }

    /// The same plane, with the next formula.
    #[must_use]
    pub fn next_formula(self) -> Self {
        let formula = self.formula().next();
        match self {
            Self::Parameter(_) => Self::Parameter(formula),
            // A Julia `c` belongs to the formula it was measured for, so
            // carrying it across would land on an arbitrary parameter of the
            // new formula — very often outside its locus, which renders as a
            // featureless blob. Re-aim at the new formula's own orbit instead.
            Self::Julia(..) => Self::julia_of(formula),
        }
    }

    /// Whether two kernels are the same fractal, ignoring parameters.
    ///
    /// Distinct from `PartialEq`, which compares parameters too: the UI needs
    /// to recognise "the Julia of this formula" even when `c` has since moved.
    #[must_use]
    pub const fn same_kind(self, other: Self) -> bool {
        matches!(
            (self, other),
            (Self::Parameter(a), Self::Parameter(b)) | (Self::Julia(a, _), Self::Julia(b, _))
                if a as u8 == b as u8
        )
    }

    /// A view framing this fractal's whole set.
    ///
    /// Each one sits somewhere different on the plane, so a single shared home
    /// view would open several of them off-centre or empty.
    ///
    /// Matched here rather than routed through a shared helper. A helper taking
    /// `&dyn Fractal` would be a second, *different* match from the one in
    /// [`Kernel::sample_into`] — which has to stay concrete so the sampler
    /// monomorphises — so it would add indirection without becoming the single
    /// place the enum is handled. Two honest matches beat one that claims to be
    /// the only one.
    #[must_use]
    pub fn home(self) -> (Complex, f64) {
        match self {
            Self::Parameter(FormulaKind::Quadratic) => {
                Parameter::<formula::Quadratic>::new().home()
            }
            Self::Parameter(FormulaKind::BurningShip) => {
                Parameter::<formula::BurningShip>::new().home()
            }
            Self::Parameter(FormulaKind::Tricorn) => Parameter::<formula::Tricorn>::new().home(),
            Self::Parameter(FormulaKind::Celtic) => Parameter::<formula::Celtic>::new().home(),
            Self::Parameter(FormulaKind::Cubic) => Parameter::<formula::Cubic>::new().home(),
            Self::Julia(FormulaKind::Quadratic, c) => Julia::<formula::Quadratic>::new(c).home(),
            Self::Julia(FormulaKind::BurningShip, c) => {
                Julia::<formula::BurningShip>::new(c).home()
            }
            Self::Julia(FormulaKind::Tricorn, c) => Julia::<formula::Tricorn>::new(c).home(),
            Self::Julia(FormulaKind::Celtic, c) => Julia::<formula::Celtic>::new(c).home(),
            Self::Julia(FormulaKind::Cubic, c) => Julia::<formula::Cubic>::new(c).home(),
        }
    }

    /// The kernel's name, for a status line.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Parameter(formula) => formula.parameter_name(),
            Self::Julia(formula, _) => formula.julia_name(),
        }
    }

    /// Sample this kernel over `vp` into `grid`.
    ///
    /// The match happens here — once per frame — so the sampler receives a
    /// concrete type and the kernel inlines into its inner loop.
    pub fn sample_into(self, grid: &mut SampleGrid, vp: &Viewport, limit: u32) {
        match self {
            // The quadratic's parameter plane is the one kernel with an exact
            // interior shortcut, so it gets its own type rather than the
            // generic `Parameter`.
            Self::Parameter(FormulaKind::Quadratic) => {
                sample_into(grid, vp, &crate::fractal::Mandelbrot, limit);
            }
            Self::Parameter(FormulaKind::BurningShip) => {
                sample_into(grid, vp, &Parameter::<formula::BurningShip>::new(), limit);
            }
            Self::Parameter(FormulaKind::Tricorn) => {
                sample_into(grid, vp, &Parameter::<formula::Tricorn>::new(), limit);
            }
            Self::Parameter(FormulaKind::Celtic) => {
                sample_into(grid, vp, &Parameter::<formula::Celtic>::new(), limit);
            }
            Self::Parameter(FormulaKind::Cubic) => {
                sample_into(grid, vp, &Parameter::<formula::Cubic>::new(), limit);
            }
            Self::Julia(FormulaKind::Quadratic, c) => {
                sample_into(grid, vp, &Julia::<formula::Quadratic>::new(c), limit);
            }
            Self::Julia(FormulaKind::BurningShip, c) => {
                sample_into(grid, vp, &Julia::<formula::BurningShip>::new(c), limit);
            }
            Self::Julia(FormulaKind::Tricorn, c) => {
                sample_into(grid, vp, &Julia::<formula::Tricorn>::new(c), limit);
            }
            Self::Julia(FormulaKind::Celtic, c) => {
                sample_into(grid, vp, &Julia::<formula::Celtic>::new(c), limit);
            }
            Self::Julia(FormulaKind::Cubic, c) => {
                sample_into(grid, vp, &Julia::<formula::Cubic>::new(c), limit);
            }
        }
    }
}

/// The Julia parameter a fresh switch to the *quadratic* Julia lands on:
/// Douady's rabbit.
///
/// Chosen by measurement rather than fame — over the default view it is 16.9%
/// interior, where the equally famous dendrite at `-0.8 + 0.156i` is 1.1%. A
/// renderer that shades by density has almost nothing to shade there.
///
/// Note this is *not* where the orbit starts; [`Kernel::julia_of`] uses turn
/// zero of the formula's own loop, so every formula lands somewhere measured
/// for it rather than on a quadratic-specific constant.
pub const JULIA_DEFAULT: Complex = Complex::new(-0.123, 0.745);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formula::Formula;

    /// Every kernel the UI can reach: both planes of every formula.
    fn all_kernels() -> Vec<Kernel> {
        FormulaKind::ALL
            .iter()
            .flat_map(|f| [Kernel::Parameter(*f), Kernel::julia_of(*f)])
            .collect()
    }

    #[test]
    fn cycling_formulas_visits_every_one_and_returns() {
        let mut formula = FormulaKind::default();
        let mut seen = vec![formula.parameter_name()];
        for _ in 1..FormulaKind::ALL.len() {
            formula = formula.next();
            seen.push(formula.parameter_name());
        }
        assert_eq!(
            seen,
            vec![
                "mandelbrot",
                "burning ship",
                "tricorn",
                "celtic",
                "multibrot³"
            ]
        );
        assert_eq!(formula.next(), FormulaKind::default(), "did not wrap");
    }

    #[test]
    fn every_formula_has_both_planes_and_they_are_named_distinctly() {
        // The whole point of splitting formula from plane: ten fractals, all
        // reachable, none sharing a name with another.
        let kernels = all_kernels();
        assert_eq!(kernels.len(), 10);
        let mut names: Vec<_> = kernels.iter().map(|k| k.name()).collect();
        names.sort_unstable();
        let count = names.len();
        names.dedup();
        assert_eq!(names.len(), count, "duplicate kernel name");
    }

    #[test]
    fn flipping_the_plane_keeps_the_formula_and_is_reversible() {
        for &formula in FormulaKind::ALL {
            let parameter = Kernel::Parameter(formula);
            let julia = parameter.flip_plane();
            assert!(
                julia.is_julia(),
                "{} did not flip",
                formula.parameter_name()
            );
            assert_eq!(julia.formula(), formula, "flip changed the formula");
            // Back again lands on exactly the parameter plane it came from.
            assert_eq!(julia.flip_plane(), parameter);
        }
    }

    #[test]
    fn a_parameter_plane_has_no_live_parameter() {
        // The UI keys the orbit drive off this: offering an orbit on a
        // parameter plane would animate nothing at all.
        for &formula in FormulaKind::ALL {
            assert!(!Kernel::Parameter(formula).is_julia());
            assert!(Kernel::julia_of(formula).is_julia());
        }
    }

    #[test]
    fn switching_formula_on_a_julia_re_aims_rather_than_carrying_c_across() {
        // A `c` measured for one formula is an arbitrary parameter of the next,
        // and very often outside its locus — which renders as a featureless
        // blob. Each formula must land on its own orbit.
        let quadratic = Kernel::julia_of(FormulaKind::Quadratic);
        let next = quadratic.next_formula();
        assert_eq!(next.formula(), FormulaKind::BurningShip);
        assert!(next.is_julia(), "the plane must survive the switch");
        let Kernel::Julia(_, c) = next else {
            unreachable!("just asserted it is a Julia")
        };
        assert_eq!(
            c,
            FormulaKind::BurningShip.julia_parameter(0.0),
            "carried the old formula's parameter across"
        );
    }

    #[test]
    fn switching_formula_on_a_parameter_plane_stays_on_the_parameter_plane() {
        let kernel = Kernel::MANDELBROT.next_formula();
        assert!(!kernel.is_julia());
        assert_eq!(kernel, Kernel::Parameter(FormulaKind::BurningShip));
    }

    #[test]
    fn same_kind_ignores_the_parameter_but_equality_does_not() {
        let a = Kernel::Julia(FormulaKind::Quadratic, Complex::ZERO);
        let b = Kernel::Julia(FormulaKind::Quadratic, JULIA_DEFAULT);
        assert!(a.same_kind(b));
        assert_ne!(a, b, "equality must see the parameter");
        // Different formula, same plane, is a different kind.
        assert!(!a.same_kind(Kernel::Julia(FormulaKind::Tricorn, Complex::ZERO)));
        // Same formula, different plane, is a different kind.
        assert!(!a.same_kind(Kernel::MANDELBROT));
    }

    #[test]
    fn each_kernel_frames_itself_rather_than_borrowing_another_framing() {
        // A kernel using the wrong `home` opens off-centre or empty, and the
        // interior-fraction check below is too loose to catch which one — so
        // compare against each formula's own constants directly.
        for &formula in FormulaKind::ALL {
            let (centre, half_width) = Kernel::Parameter(formula).home();
            let expected = match formula {
                FormulaKind::Quadratic => (
                    formula::Quadratic::HOME_CENTRE,
                    formula::Quadratic::HOME_HALF_WIDTH,
                ),
                FormulaKind::BurningShip => (
                    formula::BurningShip::HOME_CENTRE,
                    formula::BurningShip::HOME_HALF_WIDTH,
                ),
                FormulaKind::Tricorn => (
                    formula::Tricorn::HOME_CENTRE,
                    formula::Tricorn::HOME_HALF_WIDTH,
                ),
                FormulaKind::Celtic => (
                    formula::Celtic::HOME_CENTRE,
                    formula::Celtic::HOME_HALF_WIDTH,
                ),
                FormulaKind::Cubic => {
                    (formula::Cubic::HOME_CENTRE, formula::Cubic::HOME_HALF_WIDTH)
                }
            };
            assert_eq!(
                (centre.re, centre.im),
                expected.0,
                "{} parameter plane is framed on the wrong centre",
                formula.parameter_name()
            );
            assert_eq!(half_width, expected.1);
        }
    }

    #[test]
    fn every_kernel_renders_something_worth_looking_at_from_its_home() {
        // The point of per-kernel framing: none of the ten should open on a
        // blank screen or a solid block.
        for kernel in all_kernels() {
            let (centre, half_width) = kernel.home();
            let mut vp = Viewport::home(70, 34, 2.0);
            vp.centre = centre;
            vp.half_width = half_width;

            let mut grid = SampleGrid::new(0, 0);
            kernel.sample_into(&mut grid, &vp, 400);
            let interior = grid.samples().filter(|e| e.is_interior()).count();
            let total = grid.samples().count();
            let fraction = interior as f64 / total as f64;
            assert!(
                (0.01..0.80).contains(&fraction),
                "{} opens {:.1}% interior",
                kernel.name(),
                fraction * 100.0
            );
        }
    }

    #[test]
    fn a_real_parameter_collapses_three_of_the_julia_families_into_one() {
        // Not a bug, and worth pinning so it never reads as one. With `c` real,
        // the burning ship's `2|zr||zi|` and the tricorn's `-2·zr·zi` have the
        // same *magnitude* as the quadratic's `2·zr·zi`, while the real part
        // `zr² - zi²` depends only on the squares. So |zr| and |zi| evolve
        // identically in all three, escape depends only on magnitude, and the
        // three Julia sets are pixel-for-pixel the same picture. Measured: zero
        // disagreements over 4,000 random seeds at each of four real `c`.
        //
        // The celtic escapes this because its `abs` is on the *real* part,
        // which genuinely changes the orbit.
        //
        // This is why the baked orbits are rotated 45° off the real axis: turn
        // zero is where `julia_of` lands, and three identical fractals there
        // would look like the formula switch was broken.
        let vp = Viewport::home(30, 15, 2.0);
        let real = Complex::new(-0.5, 0.0);
        let mut quadratic = SampleGrid::new(0, 0);
        Kernel::Julia(FormulaKind::Quadratic, real).sample_into(&mut quadratic, &vp, 400);
        for formula in [FormulaKind::BurningShip, FormulaKind::Tricorn] {
            let mut other = SampleGrid::new(0, 0);
            Kernel::Julia(formula, real).sample_into(&mut other, &vp, 400);
            assert_eq!(
                quadratic,
                other,
                "{} should coincide with the quadratic at a real c",
                formula.julia_name()
            );
        }
        let mut celtic = SampleGrid::new(0, 0);
        Kernel::Julia(FormulaKind::Celtic, real).sample_into(&mut celtic, &vp, 400);
        assert_ne!(quadratic, celtic, "the celtic's abs is on the real part");
    }

    #[test]
    fn no_orbit_starts_on_the_real_axis() {
        // The guard for the rotation the test above explains. A future rebake
        // that put turn zero back on the axis would silently make three of the
        // five formula switches show the identical picture.
        for &formula in FormulaKind::ALL {
            let c = formula.julia_parameter(0.0);
            assert!(
                c.im.abs() > 0.05,
                "{} starts at {c:?}, effectively on the real axis",
                formula.julia_name()
            );
        }
    }

    #[test]
    fn every_kernel_is_wired_to_its_own_formula() {
        // The mutation this catches: a kernel whose `sample_into` arm reaches
        // for the wrong formula. Comparing kernels at their *own* homes cannot
        // see it — two kernels running the same formula at different framings
        // still differ — so sample every one over an identical viewport.
        let vp = Viewport::home(40, 20, 2.0);
        let mut grids = Vec::new();
        for kernel in all_kernels() {
            let mut grid = SampleGrid::new(0, 0);
            kernel.sample_into(&mut grid, &vp, 300);
            grids.push((kernel.name(), grid));
        }
        for (i, (name_a, a)) in grids.iter().enumerate() {
            for (name_b, b) in &grids[i + 1..] {
                assert_ne!(a, b, "{name_a} and {name_b} render identically");
            }
        }
    }

    #[test]
    fn a_kernel_is_copy_and_comparable() {
        // The property the whole type exists for: a frontend must be able to
        // put this in a `Copy + PartialEq` struct and compare frames.
        let a = Kernel::default();
        let b = a;
        assert_eq!(a, b);
    }

    #[test]
    fn dispatching_through_the_enum_matches_the_kernel_directly() {
        // The enum must be a pure dispatch layer: same viewport, same answers.
        let vp = Viewport::home(20, 10, 2.0);
        let mut through_enum = SampleGrid::new(0, 0);
        Kernel::MANDELBROT.sample_into(&mut through_enum, &vp, 300);

        let mut direct = SampleGrid::new(0, 0);
        sample_into(&mut direct, &vp, &crate::fractal::Mandelbrot, 300);
        assert_eq!(through_enum, direct);
    }

    #[test]
    fn every_julia_starts_somewhere_that_is_not_a_blank_screen() {
        // `julia_of` is what the UI lands on when flipping plane or switching
        // formula, so every one of the five must open on something.
        for &formula in FormulaKind::ALL {
            let kernel = Kernel::julia_of(formula);
            let (centre, half_width) = kernel.home();
            let mut vp = Viewport::home(50, 25, 2.0);
            vp.centre = centre;
            vp.half_width = half_width;
            let mut grid = SampleGrid::new(0, 0);
            kernel.sample_into(&mut grid, &vp, 500);
            let interior = grid.samples().filter(|e| e.is_interior()).count();
            let escaped = grid.samples().count() - interior;
            assert!(
                interior > 5 && escaped > 5,
                "{} opens with {interior} interior and {escaped} escaped",
                kernel.name()
            );
        }
    }
}
