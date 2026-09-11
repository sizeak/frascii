//! Frontend-agnostic fractal logic.
//!
//! This is the crate any frontend can consume: escape-time kernels and the
//! shared, non-presentational decisions around them. It has no opinion on
//! colour, on glyphs, on what a cell is, or on where an answer is drawn.
//!
//! # Layering
//!
//! The rule is a negative one: **nothing here may be about presentation, and
//! nothing here may be host-bound.** No colour, no glyph ramp, no terminal, no
//! `std::io`. `[dependencies]` in this crate's manifest is empty and is the
//! whole check.
//!
//! What belongs here is anything a second frontend would need *unchanged*: the
//! kernels, their parameters, the plane↔sample-index mapping, and the sampler
//! that drives one over the other. An image exporter needs all of those exactly
//! as a terminal does — what differs between them is a *number* (the pixel
//! aspect ratio: about 2.0 for a terminal cell, 1.0 for a square pixel), which
//! is an input the frontend supplies, not a reason to duplicate the geometry.
//!
//! Note the two senses of "grid", because conflating them is what would pull
//! presentation down here: a **sample grid** of [`Escape`] values is this
//! crate's output and frontend-agnostic; a **cell grid** of glyphs and colours
//! is presentation and lives in the frontend.

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
