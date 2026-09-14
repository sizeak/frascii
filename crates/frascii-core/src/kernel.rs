//! A run-time choice of fractal that is still cheap to compare.

use crate::complex::Complex;
use crate::fractal::{BurningShip, Celtic, Fractal, Julia, Mandelbrot, Multibrot3, Tricorn};
use crate::sample::SampleGrid;
use crate::sampler::sample_into;
use crate::viewport::Viewport;

/// Which fractal to sample.
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
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Kernel {
    /// The Mandelbrot set, `z → z² + c`.
    #[default]
    Mandelbrot,
    /// A Julia set for `z → z² + c`, with the given parameter.
    Julia {
        /// The parameter selecting which Julia set this is.
        c: Complex,
    },
    /// The Burning Ship, `z → (|Re z| + i|Im z|)² + c`.
    BurningShip,
    /// The Tricorn, `z → conj(z)² + c`.
    Tricorn,
    /// The Celtic, `z → |Re(z²)| + i·Im(z²) + c`.
    Celtic,
    /// The degree-3 Multibrot, `z → z³ + c`.
    Multibrot3,
}

impl Kernel {
    /// Every kernel, in switching order.
    ///
    /// Julia carries its default parameter here; switching to it should land on
    /// a recognisable set rather than whatever `c` happened to be left over.
    pub const ALL: &'static [Self] = &[
        Self::Mandelbrot,
        Self::Julia { c: JULIA_DEFAULT },
        Self::BurningShip,
        Self::Tricorn,
        Self::Celtic,
        Self::Multibrot3,
    ];

    /// The next kernel in switching order.
    #[must_use]
    pub fn next(self) -> Self {
        let position = Self::ALL.iter().position(|k| k.same_kind(self));
        // An unknown kernel (a Julia with a moved parameter) counts as its own
        // kind, so cycling from it still advances rather than sticking.
        let index = position.unwrap_or(0);
        Self::ALL[(index + 1) % Self::ALL.len()]
    }

    /// Whether two kernels are the same fractal, ignoring parameters.
    ///
    /// Distinct from `PartialEq`, which compares parameters too: cycling needs
    /// to find "the Julia entry" even when `c` has since been moved.
    #[must_use]
    pub const fn same_kind(self, other: Self) -> bool {
        matches!(
            (self, other),
            (Self::Mandelbrot, Self::Mandelbrot)
                | (Self::Julia { .. }, Self::Julia { .. })
                | (Self::BurningShip, Self::BurningShip)
                | (Self::Tricorn, Self::Tricorn)
                | (Self::Celtic, Self::Celtic)
                | (Self::Multibrot3, Self::Multibrot3)
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
            Self::Mandelbrot => Mandelbrot.home(),
            Self::Julia { c } => Julia::new(c).home(),
            Self::BurningShip => BurningShip.home(),
            Self::Tricorn => Tricorn.home(),
            Self::Celtic => Celtic.home(),
            Self::Multibrot3 => Multibrot3.home(),
        }
    }

    /// The kernel's name, for a status line.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Mandelbrot => "mandelbrot",
            Self::Julia { .. } => "julia",
            Self::BurningShip => "burning ship",
            Self::Tricorn => "tricorn",
            Self::Celtic => "celtic",
            Self::Multibrot3 => "multibrot³",
        }
    }

    /// Sample this kernel over `vp` into `grid`.
    ///
    /// The match happens here — once per frame — so the sampler receives a
    /// concrete type and the kernel inlines into its inner loop.
    pub fn sample_into(self, grid: &mut SampleGrid, vp: &Viewport, limit: u32) {
        match self {
            Self::Mandelbrot => sample_into(grid, vp, &Mandelbrot, limit),
            Self::Julia { c } => sample_into(grid, vp, &Julia::new(c), limit),
            Self::BurningShip => sample_into(grid, vp, &BurningShip, limit),
            Self::Tricorn => sample_into(grid, vp, &Tricorn, limit),
            Self::Celtic => sample_into(grid, vp, &Celtic, limit),
            Self::Multibrot3 => sample_into(grid, vp, &Multibrot3, limit),
        }
    }
}

/// The Julia parameter a fresh switch lands on: Douady's rabbit.
///
/// Chosen by measurement rather than fame — over the default view it is 16.9%
/// interior, where the equally famous dendrite at `-0.8 + 0.156i` is 1.1%. A
/// renderer that shades by density has almost nothing to shade there.
pub const JULIA_DEFAULT: Complex = Complex::new(-0.123, 0.745);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cycling_visits_every_kernel_and_returns() {
        let mut kernel = Kernel::default();
        let mut seen = vec![kernel.name()];
        for _ in 1..Kernel::ALL.len() {
            kernel = kernel.next();
            seen.push(kernel.name());
        }
        assert_eq!(
            seen,
            vec![
                "mandelbrot",
                "julia",
                "burning ship",
                "tricorn",
                "celtic",
                "multibrot³"
            ]
        );
        assert!(kernel.next().same_kind(Kernel::default()), "did not wrap");
    }

    #[test]
    fn every_kernel_is_distinct_and_named() {
        // A duplicated name makes the status line ambiguous; a duplicated
        // `same_kind` arm makes cycling stick.
        let mut names: Vec<_> = Kernel::ALL.iter().map(|k| k.name()).collect();
        names.sort_unstable();
        let count = names.len();
        names.dedup();
        assert_eq!(names.len(), count, "duplicate kernel name");

        for kernel in Kernel::ALL {
            let matches = Kernel::ALL
                .iter()
                .filter(|other| kernel.same_kind(**other))
                .count();
            assert_eq!(matches, 1, "{} matches {matches} kinds", kernel.name());
        }
    }

    #[test]
    fn every_kernel_renders_something_worth_looking_at_from_its_home() {
        // The point of per-kernel framing: none of them should open on a blank
        // screen or a solid block.
        for kernel in Kernel::ALL {
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
                (0.02..0.75).contains(&fraction),
                "{} opens {:.1}% interior",
                kernel.name(),
                fraction * 100.0
            );
        }
    }

    #[test]
    fn the_kernels_render_differently_from_one_another() {
        // Six variants that produced the same picture would mean a formula was
        // wired to the wrong step.
        let vp = Viewport::home(40, 20, 2.0);
        let mut grids = Vec::new();
        for kernel in Kernel::ALL {
            let (centre, half_width) = kernel.home();
            let mut framed = vp;
            framed.centre = centre;
            framed.half_width = half_width;
            let mut grid = SampleGrid::new(0, 0);
            kernel.sample_into(&mut grid, &framed, 300);
            grids.push((kernel.name(), grid));
        }
        for (i, (name_a, a)) in grids.iter().enumerate() {
            for (name_b, b) in &grids[i + 1..] {
                assert_ne!(a, b, "{name_a} and {name_b} render identically");
            }
        }
    }

    #[test]
    fn cycling_from_a_moved_julia_still_advances() {
        // `PartialEq` would not find this in ALL, because `c` has moved. If the
        // lookup used equality rather than kind, switching would stick here.
        let moved = Kernel::Julia {
            c: Complex::new(0.285, 0.01),
        };
        // Whatever follows Julia in the list, not Mandelbrot specifically —
        // that only coincided while there were two kernels.
        let after_julia = Kernel::ALL[2];
        assert!(moved.next().same_kind(after_julia), "stuck on julia");
        assert!(!moved.next().same_kind(moved), "did not advance at all");
    }

    #[test]
    fn same_kind_ignores_parameters_but_equality_does_not() {
        let a = Kernel::Julia {
            c: Complex::new(0.0, 0.0),
        };
        let b = Kernel::Julia { c: JULIA_DEFAULT };
        assert!(a.same_kind(b));
        assert_ne!(a, b, "equality must see the parameter");
        assert!(!a.same_kind(Kernel::Mandelbrot));
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
    fn each_kernel_samples_a_distinct_picture() {
        let vp = Viewport::home(30, 15, 2.0);
        let mut mandel = SampleGrid::new(0, 0);
        let mut julia = SampleGrid::new(0, 0);
        Kernel::Mandelbrot.sample_into(&mut mandel, &vp, 400);
        Kernel::Julia { c: JULIA_DEFAULT }.sample_into(&mut julia, &vp, 400);
        assert_ne!(mandel, julia);
        // And both are worth looking at, rather than one being blank.
        for grid in [&mandel, &julia] {
            let interior = grid.samples().filter(|e| e.is_interior()).count();
            assert!(interior > 0 && interior < grid.samples().count());
        }
    }

    #[test]
    fn dispatching_through_the_enum_matches_the_kernel_directly() {
        // The enum must be a pure dispatch layer: same viewport, same answers.
        let vp = Viewport::home(20, 10, 2.0);
        let mut through_enum = SampleGrid::new(0, 0);
        Kernel::Mandelbrot.sample_into(&mut through_enum, &vp, 300);

        let mut direct = SampleGrid::new(0, 0);
        sample_into(&mut direct, &vp, &Mandelbrot, 300);
        assert_eq!(through_enum, direct);
    }

    #[test]
    fn the_default_julia_is_not_a_blank_screen() {
        let vp = Viewport::home(40, 20, 2.0);
        let mut grid = SampleGrid::new(0, 0);
        Kernel::Julia { c: JULIA_DEFAULT }.sample_into(&mut grid, &vp, 500);
        let interior = grid.samples().filter(|e| e.is_interior()).count();
        assert!(interior > 10, "only {interior} interior samples");
    }
}
