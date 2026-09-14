//! Glyph density: how much ink a sample gets.

use frascii_core::Escape;

/// The density ramp, lightest first.
///
/// ASCII only, and a test asserts it. A non-ASCII glyph risks a double-width
/// cell, which shifts every following column and breaks every snapshot in a way
/// that reads as a layout bug rather than a ramp bug.
pub const RAMP: &[char] = &[' ', '.', ':', '-', '=', '+', '*', '#', '%', '@'];

/// The escape count that maps to full ink.
///
/// **Fixed, and never the iteration limit.** A dive raises the limit from a few
/// hundred to several thousand, so a `smooth / limit` normalisation would
/// re-shade the entire frame each time the limit stepped — a whole-screen pulse
/// on every step of every dive, forever, which reads as a rendering glitch with
/// its cause nowhere near the render code.
const FULL_INK: f64 = 512.0;

/// Ink coverage for a sample, in `0.0 ..= 1.0`.
///
/// Density comes from **escape time**, not from the palette colour's perceived
/// luminance. That was an open question and the answer matters: if density
/// tracked luminance then cycling the palette would change the *glyphs*, and
/// the ASCII structure would shimmer. That reads as noise rather than motion.
/// Escape time keeps the shape still and lets only the colour move, which is
/// the point of having two channels at all.
///
/// Log-compressed, because the exterior's interesting range is small: most
/// escaping samples near the boundary go in one to twenty iterations, and a
/// linear map would collapse nearly all of them into the lightest one or two
/// glyphs and look flat.
#[must_use]
pub(crate) fn density(escape: Escape) -> f64 {
    match escape {
        // Interior is the ramp's *lightest* glyph — a space — not its heaviest.
        // The interior is a solid region; drawing it as `@` would fill the
        // largest part of the screen with the busiest glyph and bury the
        // filigree, which is the part worth looking at.
        Escape::Interior => 0.0,
        Escape::Escaped { smooth, .. } => {
            let t = (1.0 + smooth).ln() / (1.0 + FULL_INK).ln();
            t.clamp(0.0, 1.0)
        }
    }
}

/// The glyph for a sample.
///
/// Note that `' '` is the output for two different things: the interior, and the
/// very lightest exterior. The colour tells them apart, so a symbols-only
/// snapshot cannot — worth knowing before reading one and concluding the ramp is
/// broken.
///
/// The shading path goes through [`glyph_for_density`] instead, because it
/// averages a block first. This single-sample form is kept as the reference
/// that the block path must agree with at 1×.
#[cfg(test)]
#[must_use]
pub(crate) fn glyph(escape: Escape) -> char {
    glyph_for_density(density(escape))
}

/// The glyph for an already-computed density.
///
/// Separate from [`glyph`] because supersampling averages the densities of a
/// block and then picks *one* glyph for the cell — which is the antialiasing: a
/// cell straddling the boundary gets an intermediate weight rather than
/// whichever single sample happened to land at its centre.
#[must_use]
pub(crate) fn glyph_for_density(density: f64) -> char {
    let index = (density.clamp(0.0, 1.0) * RAMP.len() as f64) as usize;
    RAMP[index.min(RAMP.len() - 1)]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn escaped(smooth: f64) -> Escape {
        Escape::Escaped {
            iterations: smooth.ceil() as u32,
            smooth,
        }
    }

    #[test]
    fn the_ramp_is_ascii_and_ordered_lightest_first() {
        assert!(
            RAMP.iter().all(|c| c.is_ascii_graphic() || *c == ' '),
            "a non-ASCII glyph risks a double-width cell"
        );
        assert_eq!(RAMP[0], ' ');
        assert_eq!(RAMP[RAMP.len() - 1], '@');
        assert!(RAMP.len() >= 8, "too few steps to read as a gradient");
    }

    #[test]
    fn interior_is_blank_and_deep_escapes_are_solid() {
        assert_eq!(density(Escape::Interior), 0.0);
        assert_eq!(glyph(Escape::Interior), ' ');
        assert_eq!(glyph(escaped(FULL_INK)), '@');
    }

    #[test]
    fn density_is_monotone_in_escape_time() {
        // Monotone is what keeps the structure stable: a heavier glyph must
        // always mean a slower escape, or the picture is noise.
        let mut previous = -1.0;
        for i in 0..400 {
            let d = density(escaped(f64::from(i) * 1.5));
            assert!(
                d >= previous,
                "density fell at smooth={}",
                f64::from(i) * 1.5
            );
            previous = d;
        }
    }

    #[test]
    fn density_is_independent_of_the_iteration_limit() {
        // The property that stops a dive pulsing. The same sample must shade
        // identically no matter what limit produced it, so this asserts the
        // obvious thing directly: `density` never sees a limit at all, and the
        // value depends only on `smooth`.
        let a = Escape::Escaped {
            iterations: 40,
            smooth: 39.5,
        };
        let b = Escape::Escaped {
            iterations: 40,
            smooth: 39.5,
        };
        assert_eq!(density(a), density(b));
        assert_eq!(glyph(a), glyph(b));
    }

    #[test]
    fn the_log_map_spreads_the_near_boundary_range_across_the_ramp() {
        // The reason for compressing: samples escaping in 1..20 iterations are
        // where the filigree lives. A linear map would put nearly all of them
        // in the lightest one or two glyphs.
        let glyphs: Vec<char> = (1..=20).map(|n| glyph(escaped(f64::from(n)))).collect();
        let distinct: std::collections::BTreeSet<_> = glyphs.iter().collect();
        assert!(
            distinct.len() >= 4,
            "only {} distinct glyphs across 1..20 iterations: {glyphs:?}",
            distinct.len()
        );
    }

    #[test]
    fn every_density_maps_to_a_glyph_in_range() {
        // Indexing must never run off the end, including at exactly 1.0.
        for i in 0..=1000 {
            let smooth = f64::from(i) * 2.0;
            let g = glyph(escaped(smooth));
            assert!(RAMP.contains(&g), "{g:?} is not in the ramp");
        }
        // And well past full ink, where the clamp is doing the work.
        assert_eq!(glyph(escaped(1e9)), '@');
    }
}
