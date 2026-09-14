//! The mapping between sample indices and the complex plane.
//!
//! This is the geometry every frontend shares, and the reason it is here rather
//! than in a renderer: a terminal, an image exporter and a GPU surface differ
//! only in how tall a sample is relative to its width. That difference is the
//! `pixel_aspect` field — a number — not a different mapping.

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
    /// This is the whole of what a frontend's geometry contributes. One sample
    /// per terminal cell means a sample is about twice as tall as it is wide,
    /// so the caller passes ~0.5 to keep the plane square; two stacked samples
    /// per cell means ~1.0. Core never learns which of those is happening.
    pub pixel_aspect: f64,
}

impl Viewport {
    /// The home view at the given sample dimensions and aspect.
    #[must_use]
    pub const fn home(cols: usize, rows: usize, pixel_aspect: f64) -> Self {
        Self {
            centre: HOME_CENTRE,
            half_width: HOME_HALF_WIDTH,
            cols,
            rows,
            pixel_aspect,
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
        self.sample_width() * self.pixel_aspect
    }

    /// Half the window's height, in plane units.
    #[must_use]
    pub fn half_height(&self) -> f64 {
        0.5 * self.rows as f64 * self.sample_height()
    }

    /// The plane point at the centre of sample `(ix, iy)`.
    ///
    /// `iy` counts downward from the top row, as screen coordinates do, while
    /// the imaginary axis runs upward — so the row index is subtracted.
    #[must_use]
    pub fn sample_to_plane(&self, ix: usize, iy: usize) -> Complex {
        let re = self.centre.re - self.half_width + (ix as f64 + 0.5) * self.sample_width();
        let im = self.centre.im + self.half_height() - (iy as f64 + 0.5) * self.sample_height();
        Complex::new(re, im)
    }

    /// Scale the view by `factor` about the sample `(ix, iy)`, leaving the
    /// plane point under that sample where it is.
    ///
    /// Anchoring matters for scroll-to-zoom: zooming about the centre makes the
    /// thing you were pointing at slide away, which is the difference between a
    /// map that follows the cursor and one that fights it.
    pub fn zoom_about(&mut self, factor: f64, ix: usize, iy: usize) {
        let anchor = self.sample_to_plane(ix, iy);
        self.half_width *= factor;
        self.centre = anchor + (self.centre - anchor).scale(factor);
    }

    /// Scale the view by `factor` about its centre.
    pub fn zoom_centre(&mut self, factor: f64) {
        self.half_width *= factor;
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
    pub const fn resize(&mut self, cols: usize, rows: usize, pixel_aspect: f64) {
        self.cols = cols;
        self.rows = rows;
        self.pixel_aspect = pixel_aspect;
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
            v.pixel_aspect = 1.0;
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
    fn a_zero_width_viewport_does_not_divide_by_zero() {
        // A terminal reports a zero dimension mid-resize; this must not be the
        // thing that takes the renderer down.
        let v = Viewport::home(0, 0, 0.5);
        assert_eq!(v.sample_width(), 0.0);
        assert_eq!(v.half_height(), 0.0);
        assert_eq!(v.precision(), Precision::Exhausted);
    }
}
