//! Colour: cyclic gradients, baked into a lookup table.

use crate::oklab::{from_oklab, to_oklab};
use crate::render::Rgb;

/// A named gradient, as the keypoints it is defined by.
///
/// Definitions are colour *lists*, never baked tables. A hand-written 512-entry
/// table cannot be reviewed, and a seam in one is invisible in a diff.
struct Definition {
    name: &'static str,
    keys: &'static [Rgb],
}

/// The gradient each palette interpolates through.
///
/// Several are read from terminal colour schemes. They are a *reading* of each
/// scheme's accent set as a gradient — ordered by lightness so the cycle reads
/// as motion rather than as flicker — and not a claim to be the scheme itself.
/// They suit the medium for a concrete reason: a scheme's colours are chosen to
/// stay legible against a terminal background, which is this renderer's exact
/// constraint.
const DEFINITIONS: &[Definition] = &[
    Definition {
        name: "classic",
        keys: &[
            Rgb::new(0, 7, 40),
            Rgb::new(0, 40, 110),
            Rgb::new(20, 110, 200),
            Rgb::new(110, 205, 235),
            Rgb::new(245, 250, 240),
            Rgb::new(240, 180, 40),
            Rgb::new(150, 80, 10),
        ],
    },
    Definition {
        name: "spectrum",
        keys: &[
            Rgb::new(200, 30, 60),
            Rgb::new(220, 150, 20),
            Rgb::new(190, 210, 40),
            Rgb::new(40, 190, 90),
            Rgb::new(30, 170, 200),
            Rgb::new(60, 80, 210),
            Rgb::new(150, 50, 180),
        ],
    },
    Definition {
        name: "fire",
        keys: &[
            Rgb::new(8, 0, 0),
            Rgb::new(110, 10, 5),
            Rgb::new(210, 60, 10),
            Rgb::new(245, 150, 30),
            Rgb::new(255, 230, 130),
            Rgb::new(255, 255, 245),
            Rgb::new(120, 30, 10),
        ],
    },
    Definition {
        name: "pastel",
        keys: &[
            Rgb::new(60, 70, 95),
            Rgb::new(150, 190, 220),
            Rgb::new(175, 225, 205),
            Rgb::new(245, 230, 180),
            Rgb::new(245, 195, 175),
            Rgb::new(215, 180, 220),
            Rgb::new(130, 130, 175),
        ],
    },
    Definition {
        name: "monokai",
        keys: &[
            Rgb::new(39, 40, 34),
            Rgb::new(102, 217, 239),
            Rgb::new(166, 226, 46),
            Rgb::new(230, 219, 116),
            Rgb::new(253, 151, 31),
            Rgb::new(249, 38, 114),
            Rgb::new(117, 113, 94),
        ],
    },
    Definition {
        name: "dracula",
        keys: &[
            Rgb::new(40, 42, 54),
            Rgb::new(139, 233, 253),
            Rgb::new(80, 250, 123),
            Rgb::new(241, 250, 140),
            Rgb::new(255, 184, 108),
            Rgb::new(255, 121, 198),
            Rgb::new(189, 147, 249),
        ],
    },
    Definition {
        name: "nord",
        keys: &[
            Rgb::new(46, 52, 64),
            Rgb::new(94, 129, 172),
            Rgb::new(136, 192, 208),
            Rgb::new(163, 190, 140),
            Rgb::new(235, 203, 139),
            Rgb::new(191, 97, 106),
            Rgb::new(76, 86, 106),
        ],
    },
    Definition {
        name: "gruvbox",
        keys: &[
            Rgb::new(40, 40, 40),
            Rgb::new(69, 133, 136),
            Rgb::new(152, 151, 26),
            Rgb::new(215, 153, 33),
            Rgb::new(254, 128, 25),
            Rgb::new(204, 36, 29),
            Rgb::new(80, 73, 69),
        ],
    },
    Definition {
        // The one that carries no colour at all, so the glyph ramp is the
        // entire picture. Worth having as a reference: if the shape does not
        // read here, colour was hiding a problem with the ramp.
        name: "mono",
        keys: &[
            Rgb::new(10, 10, 10),
            Rgb::new(255, 255, 255),
            Rgb::new(10, 10, 10),
        ],
    },
];

/// A cyclic colour gradient, pre-computed.
#[derive(Debug, Clone)]
pub struct Palette {
    name: &'static str,
    lut: Vec<Rgb>,
}

impl Palette {
    /// Entries in the lookup table.
    ///
    /// A power of two, so the wrap is a mask rather than a modulo.
    pub const LEN: usize = 512;

    /// Iterations per full colour cycle.
    ///
    /// **Fixed, and never derived from the iteration limit.** Auto-zoom raises
    /// the limit as it descends, so a period tied to it would re-shade the
    /// entire frame every time the limit stepped — a whole-screen pulse
    /// repeating for every step of every dive. With a fixed period a rising
    /// limit only adds bands out near the boundary and nothing already on
    /// screen moves, which is the behaviour you want anyway: the picture should
    /// sharpen as it descends, not recolour.
    pub const PERIOD: f64 = 32.0;

    /// How many palettes there are.
    #[must_use]
    pub fn count() -> usize {
        DEFINITIONS.len()
    }

    /// The palette at `index`, wrapping.
    ///
    /// Baking costs a few hundred Oklab conversions, done once. That is why the
    /// conversion's cost is irrelevant and why the *lookup* can be a plain
    /// index.
    #[must_use]
    pub fn nth(index: usize) -> Self {
        let definition = &DEFINITIONS[index % DEFINITIONS.len()];
        Self {
            name: definition.name,
            lut: bake(definition.keys),
        }
    }

    /// The palette with this name, if there is one.
    #[must_use]
    pub fn by_name(name: &str) -> Option<Self> {
        DEFINITIONS
            .iter()
            .position(|d| d.name == name)
            .map(Self::nth)
    }

    /// The palette's name, for a status line.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        self.name
    }

    /// The colour at a continuous iteration count.
    ///
    /// `t` is in iteration units and wraps every [`Palette::PERIOD`], so adding
    /// a phase to it cycles the colours without touching the samples.
    ///
    /// No interpolation between table entries, deliberately: 512 entries across
    /// a 32-iteration period is sixteen per iteration, finer than the eye
    /// resolves at terminal sizes, and a lerp here would be paid on every cell
    /// of every frame for no visible gain.
    #[must_use]
    pub fn at(&self, t: f64) -> Rgb {
        let position = t / Self::PERIOD * Self::LEN as f64;
        // `rem_euclid` rather than `%`: a negative phase must wrap forward, not
        // reflect.
        let index = position.rem_euclid(Self::LEN as f64) as usize;
        self.lut[index.min(Self::LEN - 1)]
    }
}

impl Default for Palette {
    fn default() -> Self {
        Self::nth(0)
    }
}

/// Interpolate `keys` into a [`Palette::LEN`]-entry table, in Oklab.
///
/// The loop is closed: the last key interpolates back to the first, so cycling
/// has no seam. An open gradient looks fine standing still and shows a hard edge
/// travelling across the screen the moment it moves — and cycling is one of the
/// motion modes, so it would be the first thing anyone noticed.
fn bake(keys: &[Rgb]) -> Vec<Rgb> {
    assert!(keys.len() >= 2, "a gradient needs at least two keys");
    let lab: Vec<_> = keys.iter().copied().map(to_oklab).collect();

    (0..Palette::LEN)
        .map(|i| {
            // Position along the closed loop: `keys.len()` segments, the last
            // of which returns to the start.
            let position = i as f64 / Palette::LEN as f64 * lab.len() as f64;
            let segment = position as usize % lab.len();
            let fraction = position - position.floor();
            let next = (segment + 1) % lab.len();
            from_oklab(lab[segment].lerp(lab[next], fraction))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_palette_bakes_to_a_full_table() {
        for i in 0..Palette::count() {
            let palette = Palette::nth(i);
            assert_eq!(palette.lut.len(), Palette::LEN, "{}", palette.name());
            assert!(!palette.name().is_empty());
        }
    }

    #[test]
    fn there_are_the_expected_palettes_and_they_are_reachable_by_name() {
        for name in [
            "classic", "spectrum", "fire", "pastel", "monokai", "dracula", "nord", "gruvbox",
            "mono",
        ] {
            assert!(Palette::by_name(name).is_some(), "{name} is missing");
        }
        assert_eq!(Palette::count(), 9);
        assert!(Palette::by_name("nonesuch").is_none());
    }

    #[test]
    fn palette_names_are_unique() {
        // Two palettes with one name makes `by_name` silently unreachable for
        // the second, and a status line ambiguous.
        let mut names: Vec<_> = DEFINITIONS.iter().map(|d| d.name).collect();
        names.sort_unstable();
        let count = names.len();
        names.dedup();
        assert_eq!(names.len(), count, "duplicate palette name");
    }

    #[test]
    fn nth_wraps_so_cycling_never_indexes_out_of_range() {
        let first = Palette::nth(0);
        assert_eq!(Palette::nth(Palette::count()).name(), first.name());
        assert_eq!(
            Palette::nth(Palette::count() * 7 + 2).name(),
            Palette::nth(2).name()
        );
    }

    #[test]
    fn the_cycle_is_closed_end_to_end() {
        // The seam test. Cycling rotates the gradient across the screen, so a
        // discontinuity between the last entry and the first is a visible edge
        // sweeping through the picture.
        for i in 0..Palette::count() {
            let palette = Palette::nth(i);
            let last = palette.lut[Palette::LEN - 1];
            let first = palette.lut[0];
            let gap = i32::from(last.r).abs_diff(i32::from(first.r))
                + i32::from(last.g).abs_diff(i32::from(first.g))
                + i32::from(last.b).abs_diff(i32::from(first.b));
            assert!(
                gap < 30,
                "{}: wrap gap of {gap} between {last:?} and {first:?}",
                palette.name()
            );
        }
    }

    #[test]
    fn the_gradient_is_continuous_everywhere() {
        // No step anywhere should be large: a big jump mid-gradient means a
        // mistyped key or a broken segment index, and it reads as a band.
        for i in 0..Palette::count() {
            let palette = Palette::nth(i);
            for w in palette.lut.windows(2) {
                let step = i32::from(w[0].r).abs_diff(i32::from(w[1].r))
                    + i32::from(w[0].g).abs_diff(i32::from(w[1].g))
                    + i32::from(w[0].b).abs_diff(i32::from(w[1].b));
                assert!(step < 24, "{}: step of {step} at {:?}", palette.name(), w);
            }
        }
    }

    #[test]
    fn lookups_wrap_by_period_rather_than_clamping() {
        let palette = Palette::default();
        // One full period on is the same colour.
        assert_eq!(palette.at(3.0), palette.at(3.0 + Palette::PERIOD));
        assert_eq!(palette.at(3.0), palette.at(3.0 + Palette::PERIOD * 9.0));
        // And a negative phase wraps forward rather than reflecting.
        assert_eq!(palette.at(3.0), palette.at(3.0 - Palette::PERIOD));
    }

    #[test]
    fn a_phase_offset_changes_the_colour_without_touching_anything_else() {
        // The mechanism behind palette cycling: the same sample, a different
        // colour, purely from the phase.
        let palette = Palette::default();
        assert_ne!(palette.at(10.0), palette.at(10.0 + Palette::PERIOD / 4.0));
    }

    #[test]
    fn extreme_inputs_do_not_panic() {
        let palette = Palette::default();
        for t in [0.0, -0.0, 1e12, -1e12, f64::MIN_POSITIVE] {
            let _ = palette.at(t);
        }
    }

    #[test]
    fn mono_is_actually_monochrome() {
        // Its whole job is to carry no hue, so the glyph ramp is the only
        // signal. If a key were mistyped this would drift toward colour.
        let mono = Palette::by_name("mono").expect("mono exists");
        for colour in &mono.lut {
            let max = colour.r.max(colour.g).max(colour.b);
            let min = colour.r.min(colour.g).min(colour.b);
            assert!(
                max.abs_diff(min) <= 6,
                "mono entry {colour:?} has a colour cast"
            );
        }
    }
}
