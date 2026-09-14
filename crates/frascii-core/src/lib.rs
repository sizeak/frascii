//! Frontend-agnostic fractal logic.
//!
//! This is the crate any frontend can consume: escape-time kernels, the
//! geometry that maps samples onto the complex plane, and the sampler that
//! drives one over the other. It has no opinion on colour, on glyphs, on what a
//! cell is, or on where an answer is drawn.
//!
//! ```
//! use frascii_core::{CpuSampler, Mandelbrot, SampleGrid, Sampler, Viewport};
//!
//! let viewport = Viewport::home(80, 40, 0.5);
//! let mut grid = SampleGrid::new(0, 0);
//! CpuSampler.sample_into(&mut grid, &viewport, &Mandelbrot, 500);
//!
//! assert_eq!((grid.cols(), grid.rows()), (80, 40));
//! ```
//!
//! # Layering
//!
//! The rule is a negative one: **nothing here may be about presentation, and
//! nothing here may be host-bound.** No colour, no glyph, no cell type, no
//! terminal, no `std::io`. `rayon` is the only dependency, and it is here
//! because the sampler owns the decision to go wide so that the kernels do not
//! have to.
//!
//! What belongs here is anything a second frontend would need *unchanged*: the
//! kernels, the plane↔sample mapping, the sampler, and the search for somewhere
//! worth zooming. An image exporter needs all of those exactly as a terminal
//! does — what differs between them is a *number*, [`Viewport::pixel_aspect`],
//! which the frontend supplies.
//!
//! Note the two senses of "grid", because conflating them is what would pull
//! presentation down here: a **sample grid** ([`SampleGrid`]) of [`Escape`]
//! values is this crate's output and frontend-agnostic; a **cell grid** of
//! glyphs and colours is presentation and lives in the frontend.

mod complex;
mod escape;
mod fractal;
mod sample;
mod sampler;
mod target;
mod viewport;

pub use complex::Complex;
pub use escape::{BAILOUT, escape_time};
pub use fractal::{Fractal, Julia, Mandelbrot};
pub use sample::SampleGrid;
pub use sampler::{CpuSampler, Sampler};
pub use target::{boundary_target, interior_fraction, is_interesting};
pub use viewport::{HOME_CENTRE, HOME_HALF_WIDTH, Precision, Viewport};

/// How the orbit of one point behaved under iteration.
///
/// This is the seam between the kernels and whatever draws them, and it is
/// deliberately the smallest thing that can be: an iteration count says which
/// band a point falls in, and `smooth` says where *within* the band it sits, so
/// a renderer can interpolate a palette instead of drawing visible contour
/// steps.
///
/// It is provisional. Shading techniques that need more than an escape time —
/// distance estimation, orbit traps, derivative-based normals — would extend or
/// replace this type, and that decision belongs with the rendering design
/// rather than here.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Escape {
    /// The orbit escaped the bailout radius.
    Escaped {
        /// Iterations completed before the orbit escaped.
        iterations: u32,
        /// Continuous iteration count, in the range `iterations ..
        /// iterations + 1`.
        ///
        /// Renderers should prefer this over `iterations`: mapping the integer
        /// count straight onto a palette produces the banded look that
        /// continuous colouring exists to avoid.
        smooth: f64,
    },
    /// The orbit was still bounded when the iteration limit was reached, so the
    /// point is treated as interior.
    ///
    /// "Treated as" rather than "is": a higher limit can reclassify a point, so
    /// interior membership is a function of the limit and not a property of the
    /// point alone.
    Interior,
}

impl Escape {
    /// Whether the point was classified as interior at the limit it was
    /// evaluated with.
    #[must_use]
    pub const fn is_interior(self) -> bool {
        matches!(self, Self::Interior)
    }

    /// The continuous iteration count, or `None` for an interior point.
    #[must_use]
    pub const fn smooth(self) -> Option<f64> {
        match self {
            Self::Escaped { smooth, .. } => Some(smooth),
            Self::Interior => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interior_has_no_smooth_value() {
        assert!(Escape::Interior.is_interior());
        assert_eq!(Escape::Interior.smooth(), None);
    }

    #[test]
    fn escaped_carries_its_continuous_count() {
        let escape = Escape::Escaped {
            iterations: 12,
            smooth: 12.5,
        };
        assert!(!escape.is_interior());
        assert_eq!(escape.smooth(), Some(12.5));
    }
}
