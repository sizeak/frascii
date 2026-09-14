//! The mapping between sample indices and the complex plane.
//!
//! This is the geometry every frontend shares, and the reason it is here rather
//! than in a renderer: a terminal, an image exporter and a GPU surface differ
//! only in how tall a sample is relative to its width. That difference is the
//! `sample_aspect` field — a number — not a different mapping.

use crate::complex::Complex;

/// The half-width of the home view, in plane units.
///
/// The classic framing: real from -2.25 to 0.75, which holds the whole set with
/// a margin.
pub const HOME_HALF_WIDTH: f64 = 1.75;

/// The centre of the home view.
pub const HOME_CENTRE: Complex = Complex::new(-0.75, 0.0);

/// How much precision is left at the current magnification.
///
/// A typed signal rather than a doc comment, because two callers need to *act*
/// on it: unattended auto-zoom has to stop diving somewhere, and the UI should
/// be able to say why the picture stopped getting sharper. Neither is
/// expressible in prose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Precision {
    /// Plenty of significant bits left; the image is exact.
    Ample,
    /// Approaching the limit. Still correct, but the end is in sight.
    Marginal,
    /// `f64` cannot separate adjacent samples any more. Zooming further
    /// enlarges rounding error rather than revealing detail.
    Exhausted,
}

/// A rectangular window onto the complex plane, in samples.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    /// The plane point at the centre of the window.
    pub centre: Complex,
    /// Half the window's width, in plane units.
    pub half_width: f64,
    /// Samples across.
    pub cols: usize,
    /// Samples down.
    pub rows: usize,
    /// A sample's height divided by its width, in plane units.
    ///
    /// This is the whole of what a frontend's geometry contributes, and the one
    /// number that must be right or every fractal renders squashed. The
    /// frontend computes it as
    ///
    /// ```text
    /// sample_aspect = cell_aspect × samples_across_a_cell / samples_down_a_cell
    /// ```
    ///
    /// where `cell_aspect` is how many times taller a character cell is than it
    /// is wide — about 2.0, and not discoverable portably, so it is a setting
    /// rather than a measurement (`crossterm-0.29.0 src/terminal.rs:150-152`
    /// says `window_size`'s pixel fields "may not be reliably implemented or
    /// default to 0"). One sample per cell gives 2.0; half-block's two stacked
    /// samples give 1.0; braille's 2×4 also gives 1.0. Supersampling multiplies
    /// both counts and so leaves it unchanged.
    ///
    /// Called *sample* aspect, never pixel aspect: video PAR is width:height,
    /// the opposite way round, and this is height:width.
    pub sample_aspect: f64,
}

impl Viewport {
    /// The home view at the given sample dimensions and aspect.
    #[must_use]
    pub const fn home(cols: usize, rows: usize, sample_aspect: f64) -> Self {
        Self {
            centre: HOME_CENTRE,
            half_width: HOME_HALF_WIDTH,
            cols,
            rows,
            sample_aspect,
        }
    }

    /// The width of one sample, in plane units.
    #[must_use]
    pub fn sample_width(&self) -> f64 {
        if self.cols == 0 {
            return 0.0;
        }
        2.0 * self.half_width / self.cols as f64
    }

    /// The height of one sample, in plane units.
    #[must_use]
    pub fn sample_height(&self) -> f64 {
        self.sample_width() * self.sample_aspect
    }

    /// Half the window's height, in plane units.
    #[must_use]
    pub fn half_height(&self) -> f64 {
        0.5 * self.rows as f64 * self.sample_height()
    }

    /// The plane point at continuous sample coordinates.
    ///
    /// `x` and `y` are in sample units, with integers on sample *edges*, so the
    /// centre of sample `(i, j)` is `(i + 0.5, j + 0.5)`. Continuous rather
    /// than indexed on purpose: supersampling, jitter and any rotated pattern
    /// need to ask about points between samples, and an integer-only signature
    /// is the one thing that would force this to be rewritten instead of called
    /// differently.
    ///
    /// `y` counts downward as screen coordinates do, while the imaginary axis
    /// runs upward — hence the subtraction.
    #[must_use]
    pub fn plane_at(&self, x: f64, y: f64) -> Complex {
        Complex::new(
            self.centre.re - self.half_width + x * self.sample_width(),
            self.centre.im + self.half_height() - y * self.sample_height(),
        )
    }

    /// The plane point at the centre of sample `(ix, iy)`.
    #[must_use]
    pub fn sample_to_plane(&self, ix: usize, iy: usize) -> Complex {
        self.plane_at(ix as f64 + 0.5, iy as f64 + 0.5)
    }

    /// Scale the view by `factor` about the sample `(ix, iy)`, leaving the
    /// plane point under that sample where it is.
    ///
    /// Anchoring matters for scroll-to-zoom: zooming about the centre makes the
    /// thing you were pointing at slide away, which is the difference between a
    /// map that follows the cursor and one that fights it.
    /// Returns `true` when the zoom was limited by [`Viewport::min_half_width`]
    /// rather than applied in full.
    pub fn zoom_about(&mut self, factor: f64, ix: usize, iy: usize) -> bool {
        let anchor = self.sample_to_plane(ix, iy);
        let before = self.half_width;
        let clamped = self.apply_zoom(factor);
        // The *effective* factor, not the requested one: when the zoom was
        // clamped the centre must move by what actually happened, or the
        // anchored point slides and scroll-to-zoom stops tracking the cursor.
        let effective = self.half_width / before;
        self.centre = anchor + (self.centre - anchor).scale(effective);
        clamped
    }

    /// Scale the view by `factor` about its centre.
    ///
    /// Returns `true` when the zoom was limited rather than applied in full.
    pub fn zoom_centre(&mut self, factor: f64) -> bool {
        self.apply_zoom(factor)
    }

    /// The narrowest view `f64` can still resolve at this centre and width.
    ///
    /// Derived rather than a constant, because the wall is not a fixed
    /// magnification: it depends on how many samples are being spread across
    /// the view and on how large the coordinates are, since `f64`'s spacing is
    /// relative. A hardcoded "1e13" would be wrong at a different terminal size
    /// or a different centre.
    #[must_use]
    pub fn min_half_width(&self) -> f64 {
        // The same factor `precision()` calls Exhausted, so the clamp and the
        // report cannot disagree about where the wall is.
        const SAFETY: f64 = 16.0;
        let magnitude = self.centre.re.abs().max(self.centre.im.abs()).max(1.0);
        0.5 * self.cols.max(1) as f64 * f64::EPSILON * magnitude * SAFETY
    }

    /// Scale `half_width`, refusing to go below what `f64` can resolve.
    fn apply_zoom(&mut self, factor: f64) -> bool {
        let wanted = self.half_width * factor;
        let floor = self.min_half_width();
        if wanted < floor {
            self.half_width = floor;
            true
        } else {
            self.half_width = wanted;
            false
        }
    }

    /// Shift the view by whole samples.
    pub fn pan_samples(&mut self, dx: isize, dy: isize) {
        self.centre = Complex::new(
            self.centre.re + dx as f64 * self.sample_width(),
            // Down the screen is toward smaller imaginary parts.
            self.centre.im - dy as f64 * self.sample_height(),
        );
    }

    /// Change the sample dimensions without moving or rescaling the view.
    pub const fn resize(&mut self, cols: usize, rows: usize, sample_aspect: f64) {
        self.cols = cols;
        self.rows = rows;
        self.sample_aspect = sample_aspect;
    }

    /// How far in the view is zoomed, relative to the home view.
    #[must_use]
    pub fn magnification(&self) -> f64 {
        HOME_HALF_WIDTH / self.half_width
    }

    /// How much `f64` precision remains at this magnification.
    ///
    /// The test is how many times larger one sample is than the smallest
    /// difference `f64` can represent near the view's centre. Once a sample
    /// step is within a few of those, neighbouring samples round to the same
    /// value and the image dissolves rather than sharpens.
    #[must_use]
    pub fn precision(&self) -> Precision {
        let magnitude = self.centre.re.abs().max(self.centre.im.abs()).max(1.0);
        let smallest_step = f64::EPSILON * magnitude;
        if smallest_step == 0.0 {
            return Precision::Ample;
        }
        match self.sample_width() / smallest_step {
            r if r >= 1024.0 => Precision::Ample,
            r if r >= 16.0 => Precision::Marginal,
            _ => Precision::Exhausted,
        }
    }

    /// An iteration limit that keeps pace with the magnification.
    ///
    /// Detail near the boundary needs more iterations the further in you go, so
    /// a fixed limit either wastes work when zoomed out or flattens the picture
    /// when zoomed in. The growth is logarithmic because the interesting
    /// structure is self-similar across scales; the constants were chosen by
    /// diving eight decades and watching the picture stay resolved.
    #[must_use]
    pub fn suggested_limit(&self) -> u32 {
        let magnification = self.magnification().max(1.0);
        let limit = 300.0 + 120.0 * magnification.log2();
        limit.clamp(100.0, 20_000.0) as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vp() -> Viewport {
        Viewport::home(100, 50, 0.5)
    }

    #[test]
    fn the_middle_sample_of_an_odd_grid_is_the_centre() {
        // With an odd count the centre falls exactly on a sample, which makes
        // this an equality rather than a tolerance.
        let v = Viewport::home(3, 3, 1.0);
        let p = v.sample_to_plane(1, 1);
        assert!((p.re - v.centre.re).abs() < 1e-12, "{p:?}");
        assert!((p.im - v.centre.im).abs() < 1e-12, "{p:?}");
    }

    #[test]
    fn the_window_spans_exactly_twice_the_half_width() {
        let v = vp();
        let left = v.sample_to_plane(0, 0).re;
        let right = v.sample_to_plane(v.cols - 1, 0).re;
        // Sample centres sit half a step inside each edge.
        let span = right - left + v.sample_width();
        assert!((span - 2.0 * v.half_width).abs() < 1e-12, "span {span}");
    }

    #[test]
    fn rows_run_downward_in_screen_order() {
        // Row 0 is the top of the screen and so the largest imaginary part.
        let v = vp();
        assert!(v.sample_to_plane(0, 0).im > v.sample_to_plane(0, 49).im);
    }

    #[test]
    fn the_aspect_ratio_changes_height_without_moving_the_centre() {
        let mut wide = vp();
        let tall = {
            let mut v = vp();
            v.sample_aspect = 1.0;
            v
        };
        assert!((tall.half_height() - 2.0 * wide.half_height()).abs() < 1e-12);
        wide.resize(wide.cols, wide.rows, 1.0);
        assert_eq!(wide.centre, tall.centre);
        assert_eq!(wide.half_width, tall.half_width);
    }

    #[test]
    fn zooming_about_a_sample_leaves_that_point_fixed() {
        // The invariant behind scroll-to-zoom. Checked at a corner, where an
        // error in the anchoring would be largest.
        for (ix, iy) in [(0, 0), (99, 49), (37, 11)] {
            let mut v = vp();
            let before = v.sample_to_plane(ix, iy);
            v.zoom_about(0.5, ix, iy);
            let after = v.sample_to_plane(ix, iy);
            assert!((after.re - before.re).abs() < 1e-12, "{ix},{iy}: {after:?}");
            assert!((after.im - before.im).abs() < 1e-12, "{ix},{iy}: {after:?}");
        }
    }

    #[test]
    fn zooming_in_then_out_about_the_same_point_returns_the_view() {
        let mut v = vp();
        let original = v;
        v.zoom_about(0.25, 10, 10);
        v.zoom_about(4.0, 10, 10);
        assert!((v.half_width - original.half_width).abs() < 1e-12);
        assert!((v.centre.re - original.centre.re).abs() < 1e-12);
        assert!((v.centre.im - original.centre.im).abs() < 1e-12);
    }

    #[test]
    fn panning_and_panning_back_is_the_identity() {
        let mut v = vp();
        let original = v.centre;
        v.pan_samples(7, -3);
        assert!(v.centre != original);
        v.pan_samples(-7, 3);
        assert!((v.centre.re - original.re).abs() < 1e-12);
        assert!((v.centre.im - original.im).abs() < 1e-12);
    }

    #[test]
    fn panning_one_sample_moves_the_view_by_exactly_one_sample() {
        let v = vp();
        let before = v.sample_to_plane(5, 5);
        let mut moved = v;
        moved.pan_samples(1, 0);
        // Shifting the window right by one sample means the point that was at
        // column 6 is now at column 5.
        let after = moved.sample_to_plane(5, 5);
        assert!((after.re - v.sample_to_plane(6, 5).re).abs() < 1e-12);
        assert!((after.im - before.im).abs() < 1e-12);
    }

    #[test]
    fn magnification_is_one_at_home_and_grows_as_you_zoom() {
        let mut v = vp();
        assert!((v.magnification() - 1.0).abs() < 1e-12);
        v.zoom_centre(0.5);
        assert!((v.magnification() - 2.0).abs() < 1e-12);
    }

    #[test]
    fn precision_degrades_as_the_view_shrinks() {
        let mut v = vp();
        assert_eq!(v.precision(), Precision::Ample);

        // Far enough in that f64 cannot separate adjacent samples.
        v.half_width = 1e-16;
        assert_eq!(v.precision(), Precision::Exhausted);

        // And there is a band in between, or the warning would have nothing to
        // warn about before it was too late. It is genuinely narrow: at these
        // dimensions 1e-12 is Marginal and 1e-13 is already Exhausted.
        v.half_width = 1e-12;
        assert_eq!(v.precision(), Precision::Marginal);
        v.half_width = 1e-13;
        assert_eq!(v.precision(), Precision::Exhausted);
    }

    #[test]
    fn the_suggested_limit_rises_with_depth_and_stays_bounded() {
        let mut v = vp();
        let home = v.suggested_limit();
        v.zoom_centre(1e-6);
        let deep = v.suggested_limit();
        assert!(deep > home, "{deep} should exceed {home}");
        assert!(deep <= 20_000);

        // Zoomed out past home, it must not collapse to nothing.
        v.half_width = 1e6;
        assert!(v.suggested_limit() >= 100);
    }

    #[test]
    fn doubling_the_lattice_and_halving_the_aspect_covers_the_same_plane() {
        // The load-bearing claim of the whole core/frontend split: switching
        // render mode changes only the numbers a frontend passes, never the
        // region of the plane on screen. Glyph mode is (cols, rows, 2.0);
        // half-block is (cols, 2*rows, 1.0). If these two disagree, the `m`
        // toggle would jump the view and the aspect parameter would not be
        // absorbing the sub-cell layout after all.
        //
        // Cheap, pure viewport maths, and deliberately here rather than waiting
        // for half-block to be implemented.
        let glyph = Viewport::home(200, 50, 2.0);
        let half_block = Viewport::home(200, 100, 1.0);

        assert_eq!(glyph.centre, half_block.centre);
        assert_eq!(glyph.half_width, half_block.half_width);
        assert!(
            (glyph.half_height() - half_block.half_height()).abs() < 1e-12,
            "{} vs {}",
            glyph.half_height(),
            half_block.half_height()
        );

        // And the corners land on the same plane points.
        assert!((glyph.plane_at(0.0, 0.0).im - half_block.plane_at(0.0, 0.0).im).abs() < 1e-12);
        assert!(
            (glyph.plane_at(200.0, 50.0).im - half_block.plane_at(200.0, 100.0).im).abs() < 1e-12
        );
    }

    #[test]
    fn braille_blocking_also_covers_the_same_plane() {
        // 2x4 sub-samples per cell: two across as well as four down, which a
        // rows-only rule could not express. sample_aspect = 2.0 * 2/4 = 1.0.
        let glyph = Viewport::home(100, 50, 2.0);
        let braille = Viewport::home(200, 200, 1.0);
        assert!((glyph.half_height() - braille.half_height()).abs() < 1e-12);
        assert!((glyph.sample_width() * 2.0 - braille.sample_width() * 4.0).abs() < 1e-12);
    }

    #[test]
    fn plane_at_and_sample_to_plane_agree_at_sample_centres() {
        let v = vp();
        for (ix, iy) in [(0, 0), (17, 3), (99, 49)] {
            let indexed = v.sample_to_plane(ix, iy);
            let continuous = v.plane_at(ix as f64 + 0.5, iy as f64 + 0.5);
            assert_eq!(indexed, continuous, "at {ix},{iy}");
        }
    }

    #[test]
    fn plane_at_puts_integers_on_sample_edges() {
        // Which is what makes a half-sample offset mean "the centre", and what
        // a jittered supersampling pattern would rely on.
        let v = vp();
        let left_edge = v.plane_at(0.0, 0.0);
        assert!((left_edge.re - (v.centre.re - v.half_width)).abs() < 1e-12);
        assert!((left_edge.im - (v.centre.im + v.half_height())).abs() < 1e-12);
    }

    #[test]
    fn zooming_past_the_precision_wall_clamps_and_reports_it() {
        let mut v = vp();
        assert!(!v.zoom_centre(0.5), "an ordinary zoom is not clamped");

        // Ask for far more than f64 can deliver.
        let clamped = v.zoom_centre(1e-30);
        assert!(clamped, "a zoom past the wall must report itself clamped");
        assert!((v.half_width - v.min_half_width()).abs() < 1e-30);
    }

    #[test]
    fn adjacent_samples_stay_distinct_at_the_clamp() {
        // The property the wall exists to protect: at the narrowest permitted
        // view, neighbouring samples must still be different plane points. If
        // they collapse, the picture is rounding error rather than fractal.
        let mut v = vp();
        v.zoom_centre(1e-30);
        for ix in [0, 1, 50, 98] {
            let a = v.sample_to_plane(ix, 10);
            let b = v.sample_to_plane(ix + 1, 10);
            assert!(a.re != b.re, "samples {ix} and {} coincide", ix + 1);
        }
    }

    #[test]
    fn a_clamped_zoom_still_anchors_the_point_under_the_cursor() {
        // Clamping must not break the anchoring invariant, or a scroll at max
        // zoom would slide the view sideways.
        let mut v = vp();
        v.zoom_centre(1e-20);
        let before = v.sample_to_plane(30, 20);
        let clamped = v.zoom_about(0.5, 30, 20);
        assert!(clamped, "already at the wall, so this must clamp");
        let after = v.sample_to_plane(30, 20);
        assert!(
            (after.re - before.re).abs() < 1e-30,
            "{after:?} vs {before:?}"
        );
        assert!(
            (after.im - before.im).abs() < 1e-30,
            "{after:?} vs {before:?}"
        );
    }

    #[test]
    fn the_clamp_and_the_precision_report_agree_about_where_the_wall_is() {
        // Two mechanisms describing one limit; if they drift, the UI says
        // "exhausted" while the zoom keeps going, or the reverse.
        let mut v = vp();
        v.zoom_centre(1e-30);
        assert_eq!(v.precision(), Precision::Marginal);
    }

    #[test]
    fn a_zero_width_viewport_does_not_divide_by_zero() {
        // A terminal reports a zero dimension mid-resize; this must not be the
        // thing that takes the renderer down.
        let v = Viewport::home(0, 0, 0.5);
        assert_eq!(v.sample_width(), 0.0);
        assert_eq!(v.half_height(), 0.0);
        assert_eq!(v.precision(), Precision::Exhausted);
    }
}
