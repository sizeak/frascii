//! Oklab, for interpolating between colours without passing through mud.
//!
//! Why this exists at all: interpolating in sRGB between two saturated,
//! near-complementary colours takes the midpoint through desaturated grey. That
//! matters here specifically because several of the palettes are built from
//! syntax themes, and a syntax theme's entries are chosen to be *maximally
//! distinguishable* — which is very often near-complementary. Interpolating
//! those in sRGB is exactly the case that looks worst.
//!
//! Oklab is a perceptual space where a straight line between two colours stays
//! saturated and the lightness reads evenly. The conversion is not cheap, but it
//! runs once per palette at startup while the lookup table is baked, so its cost
//! is irrelevant — see [`crate::palette`].
//!
//! Constants and matrices are Björn Ottosson's, from the original Oklab post
//! (<https://bottosson.github.io/posts/oklab/>). They are reproduced rather than
//! derived, so the round-trip tests at the bottom are what stands behind them:
//! if a digit were mistyped, `srgb_round_trips_through_oklab` would fail.

use crate::render::Rgb;

/// A colour in Oklab: perceptual lightness plus two opponent axes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Oklab {
    /// Perceptual lightness, roughly `0.0 ..= 1.0`.
    pub(crate) l: f64,
    /// Green–red axis.
    pub(crate) a: f64,
    /// Blue–yellow axis.
    pub(crate) b: f64,
}

impl Oklab {
    /// Linear interpolation toward `other`.
    ///
    /// Straight-line in Oklab, which is the whole point: the same interpolation
    /// in sRGB is what produces the grey midpoint this module exists to avoid.
    pub(crate) fn lerp(self, other: Self, t: f64) -> Self {
        Self {
            l: self.l + (other.l - self.l) * t,
            a: self.a + (other.a - self.a) * t,
            b: self.b + (other.b - self.b) * t,
        }
    }
}

/// One sRGB channel, 0–255, to linear light, 0.0–1.0.
fn channel_to_linear(value: u8) -> f64 {
    let c = f64::from(value) / 255.0;
    if c <= 0.040_45 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// One linear-light channel back to sRGB, 0–255.
fn channel_from_linear(c: f64) -> u8 {
    let encoded = if c <= 0.003_130_8 {
        12.92 * c
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    };
    // Clamped because a straight line in Oklab can leave the sRGB gamut, and
    // out-of-gamut channels must be brought back rather than wrapped — wrapping
    // turns a slightly-too-bright cyan into a dark red.
    (encoded.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// Convert an sRGB colour to Oklab.
pub(crate) fn to_oklab(rgb: Rgb) -> Oklab {
    let r = channel_to_linear(rgb.r);
    let g = channel_to_linear(rgb.g);
    let b = channel_to_linear(rgb.b);

    let l = 0.412_221_470_8 * r + 0.536_332_536_3 * g + 0.051_445_992_9 * b;
    let m = 0.211_903_498_2 * r + 0.680_699_545_1 * g + 0.107_396_956_6 * b;
    let s = 0.088_302_461_9 * r + 0.281_718_837_6 * g + 0.629_978_700_5 * b;

    let l_ = l.cbrt();
    let m_ = m.cbrt();
    let s_ = s.cbrt();

    Oklab {
        l: 0.210_454_255_3 * l_ + 0.793_617_785_0 * m_ - 0.004_072_046_8 * s_,
        a: 1.977_998_495_1 * l_ - 2.428_592_205_0 * m_ + 0.450_593_709_9 * s_,
        b: 0.025_904_037_1 * l_ + 0.782_771_766_2 * m_ - 0.808_675_766_0 * s_,
    }
}

/// Convert an Oklab colour back to sRGB, clamping into gamut.
pub(crate) fn from_oklab(lab: Oklab) -> Rgb {
    let l_ = lab.l + 0.396_337_777_4 * lab.a + 0.215_803_757_3 * lab.b;
    let m_ = lab.l - 0.105_561_345_8 * lab.a - 0.063_854_172_8 * lab.b;
    let s_ = lab.l - 0.089_484_177_5 * lab.a - 1.291_485_548_0 * lab.b;

    let l = l_ * l_ * l_;
    let m = m_ * m_ * m_;
    let s = s_ * s_ * s_;

    Rgb::new(
        channel_from_linear(4.076_741_662_1 * l - 3.307_711_591_3 * m + 0.230_969_929_2 * s),
        channel_from_linear(-1.268_438_004_6 * l + 2.609_757_401_1 * m - 0.341_319_396_5 * s),
        channel_from_linear(-0.004_196_086_3 * l - 0.703_418_614_7 * m + 1.707_614_701_0 * s),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srgb_round_trips_through_oklab() {
        // The test the matrices rest on: they are transcribed constants, so a
        // mistyped digit shows up here and nowhere else.
        for r in [0u8, 1, 17, 64, 128, 200, 254, 255] {
            for g in [0u8, 33, 128, 191, 255] {
                for b in [0u8, 7, 128, 222, 255] {
                    let original = Rgb::new(r, g, b);
                    let back = from_oklab(to_oklab(original));
                    // One unit of tolerance per channel: the trip is through
                    // cube roots and a power, so exact equality would be
                    // asserting something about f64 rounding rather than about
                    // the conversion.
                    for (a, b) in [
                        (original.r, back.r),
                        (original.g, back.g),
                        (original.b, back.b),
                    ] {
                        assert!(
                            a.abs_diff(b) <= 1,
                            "{original:?} -> {back:?} differs by more than one unit"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn black_and_white_land_where_they_should() {
        let black = to_oklab(Rgb::BLACK);
        assert!(black.l.abs() < 1e-6, "black lightness {}", black.l);

        let white = to_oklab(Rgb::new(255, 255, 255));
        assert!((white.l - 1.0).abs() < 1e-3, "white lightness {}", white.l);
        // Neutral colours sit on the lightness axis.
        assert!(white.a.abs() < 1e-3 && white.b.abs() < 1e-3, "{white:?}");
    }

    #[test]
    fn interpolating_complements_stays_saturated() {
        // The reason this module exists. Blue to yellow is the worst case: in
        // sRGB the midpoint is grey, in Oklab it stays a colour.
        let blue = Rgb::new(0, 80, 255);
        let yellow = Rgb::new(255, 210, 0);

        let midpoint = from_oklab(to_oklab(blue).lerp(to_oklab(yellow), 0.5));
        let naive = Rgb::new(
            u8::try_from((u16::from(blue.r) + u16::from(yellow.r)) / 2).unwrap_or(255),
            u8::try_from((u16::from(blue.g) + u16::from(yellow.g)) / 2).unwrap_or(255),
            u8::try_from((u16::from(blue.b) + u16::from(yellow.b)) / 2).unwrap_or(255),
        );

        // Chroma, as the distance from the neutral axis in Oklab.
        let chroma = |c: Rgb| {
            let lab = to_oklab(c);
            lab.a.hypot(lab.b)
        };
        assert!(
            chroma(midpoint) > chroma(naive),
            "Oklab midpoint {midpoint:?} (chroma {:.4}) should beat sRGB {naive:?} (chroma {:.4})",
            chroma(midpoint),
            chroma(naive)
        );
    }

    #[test]
    fn lerp_hits_both_ends_exactly() {
        let a = to_oklab(Rgb::new(10, 20, 30));
        let b = to_oklab(Rgb::new(200, 100, 50));
        assert_eq!(a.lerp(b, 0.0), a);
        assert_eq!(a.lerp(b, 1.0), b);
    }

    #[test]
    fn out_of_gamut_results_are_clamped_not_wrapped() {
        // A straight line in Oklab can leave sRGB. Wrapping would turn a
        // slightly-too-bright colour into a dark one, which looks like a bug in
        // the palette rather than a gamut limit.
        let beyond = Oklab {
            l: 1.4,
            a: 0.3,
            b: 0.2,
        };
        let rgb = from_oklab(beyond);
        assert_eq!(rgb.r, 255, "{rgb:?}");
    }
}
