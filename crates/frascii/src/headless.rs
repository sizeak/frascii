//! Rendering without a terminal: the benchmark, and the boundary's proof.
//!
//! This module imports **`frascii_core` and never `frascii_tui`**, which is the
//! point of it existing here rather than inside the frontend: it is a second
//! consumer of core that needs no frontend at all.
//!
//! That is *not* compile-time proof, and calling it so would be exactly the
//! receipt-less boundary claim this repo's conventions warn about. Cargo
//! dependencies are per-**package**, and this package depends on
//! `frascii-tui`, so `frascii_tui` is in scope here and the compiler would
//! happily accept an import of it. The only thing enforcing the rule is
//! `the_headless_path_never_reaches_for_the_frontend` below — a source scan, the
//! same grade of evidence `this_module_stays_free_of_ratatui` provides for the
//! render boundary. (Genuine compile-time proof is available if ever wanted: an
//! example under `frascii-core` sees only that package's dependencies and so
//! *cannot* name the frontend. It would mean moving the feature out of the CLI.)
//!
//! What *is* compile-time enforced is the other direction — core's manifest has
//! no frontend dependency, so core cannot reach upward.
//!
//! It is also the only thing in the repo that can answer "is this fast enough?"
//! with a number.

use std::fmt::Write as _;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::time::{Duration, Instant};

use frascii_core::{
    Escape, Fractal, Julia, Mandelbrot, SampleGrid, Viewport, boundary_target, interior_fraction,
    sample_into,
};

/// Which fractal a headless run should render.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Which {
    /// The Mandelbrot set.
    Mandelbrot,
    /// The default Julia set.
    Julia,
}

impl Which {
    /// The kernel this names.
    fn fractal(self) -> Box<dyn Fractal> {
        match self {
            Self::Mandelbrot => Box::new(Mandelbrot),
            Self::Julia => Box::new(Julia::default()),
        }
    }
}

/// What a headless run should do.
#[derive(Debug, Clone)]
pub(crate) struct Options {
    /// Samples across.
    pub(crate) cols: usize,
    /// Samples down.
    pub(crate) rows: usize,
    /// How many frames to render.
    pub(crate) frames: u32,
    /// Iteration limit, or `None` to take the viewport's suggestion.
    pub(crate) limit: Option<u32>,
    /// Magnification to render at, which is what makes a run representative of
    /// deep zoom rather than the cheap home view.
    pub(crate) magnification: f64,
    /// Which fractal.
    pub(crate) which: Which,
    /// Where to write a PPM of the last frame, if anywhere.
    pub(crate) ppm: Option<std::path::PathBuf>,
}

/// What a headless run measured.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Report {
    /// Time for each frame, in order.
    pub(crate) frames: Vec<Duration>,
    /// The iteration limit actually used.
    pub(crate) limit: u32,
    /// Samples per frame.
    pub(crate) samples: usize,
    /// The magnification actually reached, which is not the one requested when
    /// `f64` ran out first.
    pub(crate) magnification: f64,
    /// The fraction of the measured frame that was interior.
    ///
    /// Reported because a benchmark that cannot tell you it measured a blank
    /// screen is worse than no benchmark. The first version of this tool zoomed
    /// straight in on the home centre and produced a 100%-interior frame, which
    /// the cardioid shortcut answers instantly — it claimed 16,000 fps for
    /// rendering nothing.
    pub(crate) interior: f64,
}

impl Report {
    /// The mean frame time.
    pub(crate) fn mean(&self) -> Duration {
        if self.frames.is_empty() {
            return Duration::ZERO;
        }
        self.frames.iter().sum::<Duration>() / u32::try_from(self.frames.len()).unwrap_or(u32::MAX)
    }

    /// The slowest frame, which is what a dropped frame would come from.
    pub(crate) fn worst(&self) -> Duration {
        self.frames.iter().copied().max().unwrap_or(Duration::ZERO)
    }

    /// Mean frames per second.
    pub(crate) fn fps(&self) -> f64 {
        let mean = self.mean().as_secs_f64();
        if mean <= 0.0 {
            f64::INFINITY
        } else {
            1.0 / mean
        }
    }

    /// A human-readable summary.
    pub(crate) fn summary(&self, backend: &str) -> String {
        let mut out = String::new();
        let _ = writeln!(
            out,
            "{} samples/frame, limit {}, magnification {:.3e}, backend {}",
            self.samples, self.limit, self.magnification, backend
        );
        let _ = writeln!(
            out,
            "{} frames: mean {:.2?}, worst {:.2?} -> {:.1} fps",
            self.frames.len(),
            self.mean(),
            self.worst(),
            self.fps()
        );
        let verdict = if self.fps() >= 30.0 {
            "meets 30fps"
        } else {
            "BELOW 30fps"
        };
        let _ = writeln!(out, "{verdict}");

        // A frame that is entirely one thing measured the shortcut, not the
        // renderer, so say so rather than let the number stand unqualified.
        if !(0.005..=0.995).contains(&self.interior) {
            let _ = writeln!(
                out,
                "WARNING: frame was {:.1}% interior — not a representative view",
                self.interior * 100.0
            );
        } else {
            let _ = writeln!(out, "frame was {:.1}% interior", self.interior * 100.0);
        }
        out
    }
}

/// Render `frames` frames and report how long each took.
pub(crate) fn run(options: &Options) -> std::io::Result<Report> {
    let fractal = options.which.fractal();
    let mut grid = SampleGrid::new(0, 0);

    let mut viewport = Viewport::home(options.cols, options.rows, 1.0);
    if options.magnification > 1.0 {
        dive_to(
            &mut viewport,
            &mut grid,
            fractal.as_ref(),
            options.magnification,
        );
    }
    let limit = options.limit.unwrap_or_else(|| viewport.suggested_limit());

    let mut timings = Vec::with_capacity(options.frames as usize);

    for _ in 0..options.frames {
        let started = Instant::now();
        sample_into(&mut grid, &viewport, fractal.as_ref(), limit);
        timings.push(started.elapsed());
    }

    if let Some(path) = options.ppm.as_deref() {
        write_ppm(path, &grid)?;
    }

    Ok(Report {
        frames: timings,
        limit,
        samples: options.cols * options.rows,
        interior: interior_fraction(&grid),
        magnification: viewport.magnification(),
    })
}

/// Zoom toward the set's boundary until reaching `magnification`.
///
/// Emphatically not `zoom_centre` in one step: the home centre sits inside the
/// set, so scaling about it lands in a solid interior region where every sample
/// is answered by the cardioid shortcut. That renders an entirely black frame
/// very fast and makes the benchmark a lie. Following `boundary_target` keeps
/// the filigree in frame, which is the workload a real deep view has.
fn dive_to(viewport: &mut Viewport, grid: &mut SampleGrid, fractal: &dyn Fractal, target: f64) {
    // A factor per step small enough that the next view still contains the
    // structure the last one aimed at.
    const STEP: f64 = 8.0;

    while viewport.magnification() < target {
        sample_into(grid, viewport, fractal, viewport.suggested_limit());
        let Some(point) = boundary_target(grid, viewport) else {
            // Nowhere left to go; stop here rather than dive into a flat field.
            break;
        };
        viewport.centre = point;
        let remaining = target / viewport.magnification();
        if viewport.zoom_centre(1.0 / remaining.min(STEP)) {
            // The viewport refused to go narrower because `f64` cannot resolve
            // it. Without this the loop spins forever: the clamp holds
            // `half_width` fixed, so the magnification never reaches the
            // target. Any unattended zoom needs the same guard.
            break;
        }
    }

    // Leave the grid holding the view that will actually be measured.
    sample_into(grid, viewport, fractal, viewport.suggested_limit());
}

/// Write the grid as a binary greyscale PGM.
///
/// Greyscale by escape time, deliberately plain: this is an image to *check the
/// geometry with*, not a rendering. Palettes and glyph ramps are the frontend's
/// business, and reaching for them here would make this a second driver of the
/// first renderer rather than an independent consumer of core — which is the
/// thing it exists to be.
///
/// `P5` (one byte per sample), not `P6`: the data is greyscale, so writing three
/// identical bytes per sample would be two thirds waste.
fn write_ppm(path: &Path, grid: &SampleGrid) -> std::io::Result<()> {
    let mut file = BufWriter::new(File::create(path)?);
    writeln!(file, "P5\n{} {}\n255", grid.cols(), grid.rows())?;

    // Scale against the brightest escape actually present, so the image uses
    // its full range at any iteration limit.
    let peak = grid
        .samples()
        .filter_map(|e| e.smooth())
        .fold(1.0_f64, f64::max);

    for iy in 0..grid.rows() {
        for ix in 0..grid.cols() {
            let value = match grid.get(ix, iy) {
                Some(Escape::Escaped { smooth, .. }) => {
                    let t = (smooth / peak).clamp(0.0, 1.0);
                    (t * 255.0) as u8
                }
                _ => 0,
            };
            file.write_all(&[value])?;
        }
    }
    file.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// This module's own source, read at compile time.
    const SOURCE: &str = include_str!("headless.rs");

    fn options() -> Options {
        Options {
            cols: 40,
            rows: 20,
            frames: 2,
            limit: Some(200),
            magnification: 1.0,
            which: Which::Mandelbrot,
            ppm: None,
        }
    }

    #[test]
    fn the_headless_path_never_reaches_for_the_frontend() {
        // The whole reason this module lives in the binary: it consumes core
        // and links no frontend, so the layering is proved by the build rather
        // than asserted in prose. Code lines only — the docs above necessarily
        // name the crate.
        let offenders: Vec<&str> = SOURCE
            .lines()
            .take_while(|line| !line.trim_start().starts_with("#[cfg(test)]"))
            .filter(|line| !line.trim_start().starts_with("//"))
            .filter(|line| line.contains("frascii_tui"))
            .collect();
        assert!(
            offenders.is_empty(),
            "headless must not depend on the frontend: {offenders:?}"
        );
    }

    #[test]
    fn a_run_reports_one_timing_per_frame() {
        let report = run(&options()).expect("no I/O without a ppm path");
        assert_eq!(report.frames.len(), 2);
        assert_eq!(report.samples, 800);
        assert_eq!(report.limit, 200);
    }

    #[test]
    fn the_limit_falls_back_to_the_viewports_suggestion() {
        let mut o = options();
        o.limit = None;
        o.magnification = 1024.0;
        let report = run(&o).expect("no I/O");
        // Deep views need more iterations; the suggestion must reflect that.
        assert!(report.limit > 300, "limit was {}", report.limit);
    }

    #[test]
    fn zero_frames_is_legal_and_reports_nothing() {
        let mut o = options();
        o.frames = 0;
        let report = run(&o).expect("no I/O");
        assert!(report.frames.is_empty());
        assert_eq!(report.mean(), Duration::ZERO);
        assert_eq!(report.worst(), Duration::ZERO);
    }

    #[test]
    fn the_summary_states_whether_the_frame_budget_was_met() {
        let fast = Report {
            frames: vec![Duration::from_millis(10)],
            limit: 500,
            samples: 800,
            interior: 0.25,
            magnification: 1.0,
        };
        assert!(fast.summary("cpu").contains("meets 30fps"));

        let slow = Report {
            frames: vec![Duration::from_millis(100)],
            limit: 500,
            samples: 800,
            interior: 0.25,
            magnification: 1.0,
        };
        assert!(slow.summary("cpu").contains("BELOW 30fps"));
    }

    #[test]
    fn mean_and_worst_describe_the_spread() {
        let report = Report {
            frames: vec![
                Duration::from_millis(10),
                Duration::from_millis(30),
                Duration::from_millis(20),
            ],
            limit: 100,
            samples: 1,
            interior: 0.25,
            magnification: 1.0,
        };
        assert_eq!(report.mean(), Duration::from_millis(20));
        assert_eq!(report.worst(), Duration::from_millis(30));
        assert!((report.fps() - 50.0).abs() < 1e-9);
    }

    #[test]
    fn a_ppm_is_written_with_a_valid_header_and_body() {
        let dir = std::env::temp_dir().join(format!("frascii-ppm-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("out.ppm");

        let mut o = options();
        o.ppm = Some(path.clone());
        run(&o).expect("writes a ppm");

        let bytes = std::fs::read(&path).expect("pgm exists");
        assert!(bytes.starts_with(b"P5\n40 20\n255\n"), "bad header");
        // Header plus one byte per sample — greyscale, so no triplets.
        assert_eq!(bytes.len(), b"P5\n40 20\n255\n".len() + 40 * 20);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_unreachable_magnification_stops_at_the_wall_instead_of_hanging() {
        // The clamp holds `half_width` fixed once `f64` is exhausted, so a dive
        // loop that waits for the magnification to reach its target never
        // terminates. This asks for far more than is representable and must
        // still return.
        let mut o = options();
        o.magnification = 1e20;
        o.frames = 1;
        let report = run(&o).expect("no I/O");
        assert!(report.magnification > 1.0, "it should have dived at all");
        assert!(
            report.magnification < 1e20,
            "it cannot have reached {:.3e}",
            report.magnification
        );
    }

    #[test]
    fn both_fractals_can_be_rendered_headlessly() {
        for which in [Which::Mandelbrot, Which::Julia] {
            let mut o = options();
            o.which = which;
            assert_eq!(run(&o).expect("no I/O").frames.len(), 2);
        }
    }
}
