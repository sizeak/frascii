//! A run-time choice of fractal that is still cheap to compare.

use crate::complex::Complex;
use crate::fractal::{Julia, Mandelbrot};
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
    /// The Mandelbrot set.
    #[default]
    Mandelbrot,
    /// A Julia set with the given parameter.
    Julia {
        /// The parameter selecting which Julia set this is.
        c: Complex,
    },
}

impl Kernel {
    /// Every kernel, in switching order.
    ///
    /// Julia carries its default parameter here; switching to it should land on
    /// a recognisable set rather than whatever `c` happened to be left over.
    pub const ALL: &'static [Self] = &[Self::Mandelbrot, Self::Julia { c: JULIA_DEFAULT }];

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
            (Self::Mandelbrot, Self::Mandelbrot) | (Self::Julia { .. }, Self::Julia { .. })
        )
    }

    /// The kernel's name, for a status line.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Mandelbrot => "mandelbrot",
            Self::Julia { .. } => "julia",
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
        assert_eq!(seen, vec!["mandelbrot", "julia"]);
        assert!(kernel.next().same_kind(Kernel::default()), "did not wrap");
    }

    #[test]
    fn cycling_from_a_moved_julia_still_advances() {
        // `PartialEq` would not find this in ALL, because `c` has moved. If the
        // lookup used equality rather than kind, switching would stick here.
        let moved = Kernel::Julia {
            c: Complex::new(0.285, 0.01),
        };
        assert!(moved.next().same_kind(Kernel::Mandelbrot), "stuck on julia");
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
