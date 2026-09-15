//! The command-line interface.
//!
//! The clap tree lives in the binary rather than a library crate — see this
//! crate's manifest for why — and the pure decisions it feeds (log level,
//! log destination) are functions here with tests, so `main` stays a wiring
//! layer with nothing testable in it.

use std::path::PathBuf;

use clap::Parser;
use frascii_core::{FormulaKind, Kernel};

/// A terminal ASCII-art renderer for realtime fractals.
#[derive(Debug, Parser)]
#[command(version, about, long_about = None)]
pub(crate) struct Cli {
    // A doc comment here is user-facing, and the exact rule is worth knowing
    // (`clap_derive-4.6.4 src/utils/doc_comments.rs:123-135`): if the comment
    // contains a blank line, the first *paragraph* becomes the short help and
    // the *whole* comment becomes `--help`'s long help; with no blank line,
    // every line is merged into one and there is no long help. So a multi-line
    // doc comment either prints in full under `--help` or collapses into one
    // run-on short help — neither is what prose wants. Keep the doc comment to
    // a single line and the reasoning in `//` comments like this one.
    //
    // A file, never a stream: the TUI owns the alternate screen, so a log line
    // on stdout or stderr is painted over the render and scrambles the frame.
    // With no `--log`, logging is off entirely rather than dropped silently to
    // a default path a user would not know to look in.
    /// Write logs to FILE
    #[arg(long, value_name = "FILE")]
    pub(crate) log: Option<PathBuf>,

    /// Increase log verbosity (-v debug, -vv trace)
    #[arg(short, long, action = clap::ArgAction::Count)]
    pub(crate) verbose: u8,

    // Benchmark the renderer with no terminal involved. Grouped under
    // `Headless` in the help so the interactive flags above stay legible; all
    // of them require `--headless`, which clap enforces so a run that looks
    // like a benchmark cannot silently open a TUI instead.
    /// Render frames without a terminal and report timing
    #[arg(long)]
    pub(crate) headless: bool,

    /// Frames to render
    #[arg(long, value_name = "N", default_value_t = 60, requires = "headless")]
    pub(crate) frames: u32,

    /// Samples across
    #[arg(long, value_name = "N", default_value_t = 200, requires = "headless")]
    pub(crate) cols: usize,

    /// Samples down
    #[arg(long, value_name = "N", default_value_t = 100, requires = "headless")]
    pub(crate) rows: usize,

    /// Iteration limit (default: scaled to the magnification)
    #[arg(long, value_name = "N", requires = "headless")]
    pub(crate) limit: Option<u32>,

    /// Magnification to render at
    #[arg(long, value_name = "X", default_value_t = 1.0, requires = "headless")]
    pub(crate) magnification: f64,

    /// Which formula to render
    #[arg(long, value_enum, default_value_t = FractalArg::Mandelbrot, requires = "headless")]
    pub(crate) fractal: FractalArg,

    /// Render the Julia sets of that formula rather than its parameter plane
    #[arg(long, requires = "headless")]
    pub(crate) julia: bool,

    /// Write the last frame as a binary PGM
    #[arg(long, value_name = "FILE", requires = "headless")]
    pub(crate) ppm: Option<PathBuf>,
}

/// The fractal a headless run should render.
///
/// Defined here rather than beside the renderer so `headless.rs` needs no
/// argument parser: it is a second frontend, and a frontend that had to depend
/// on the CLI to be callable would not be much of one.
///
/// One variant per *formula*, with `--julia` selecting the plane, rather than
/// ten variants naming every combination. The formula and the plane are
/// independent choices, so spelling out the product would mean a new formula
/// needed two new variants and two new match arms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum FractalArg {
    /// `z² + c` — the Mandelbrot set, or the classic Julia sets.
    Mandelbrot,
    /// `(|Re z| + i|Im z|)² + c` — the Burning Ship.
    BurningShip,
    /// `conj(z)² + c` — the Tricorn, or Mandelbar.
    Tricorn,
    /// `|Re(z²)| + i·Im(z²) + c` — the Celtic.
    Celtic,
    /// `z³ + c` — the degree-3 Multibrot.
    Multibrot3,
}

impl FractalArg {
    /// Which formula this names.
    pub(crate) const fn formula(self) -> FormulaKind {
        match self {
            Self::Mandelbrot => FormulaKind::Quadratic,
            Self::BurningShip => FormulaKind::BurningShip,
            Self::Tricorn => FormulaKind::Tricorn,
            Self::Celtic => FormulaKind::Celtic,
            Self::Multibrot3 => FormulaKind::Cubic,
        }
    }

    /// The kernel for this formula in the requested plane.
    ///
    /// Here rather than in `main.rs` because `main` cannot be tested without
    /// spawning the binary, so a match that lived there would not be covered —
    /// and getting one arm wrong renders a different fractal than was asked
    /// for, silently.
    pub(crate) fn kernel(self, julia: bool) -> Kernel {
        if julia {
            Kernel::julia_of(self.formula())
        } else {
            Kernel::Parameter(self.formula())
        }
    }
}

/// The `tracing` filter directive for a `-v` count.
///
/// Own crates only. A blanket `debug` pulls in every dependency's internals —
/// at trace level, a terminal backend logs per keystroke and drowns out
/// anything frascii said.
#[must_use]
pub(crate) fn log_directive(verbose: u8) -> &'static str {
    match verbose {
        0 => "frascii=info,frascii_tui=info",
        1 => "frascii=debug,frascii_tui=debug",
        _ => "frascii=trace,frascii_tui=trace,frascii_core=trace",
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn every_fractal_argument_maps_to_its_own_formula() {
        // The match this replaced lived in `main.rs`, which cannot be tested
        // without spawning the binary — so a wrong arm rendered the wrong
        // fractal silently. Each argument must reach a distinct formula.
        // A `Vec` and `contains` rather than a set, so `FormulaKind` does not
        // have to carry an `Ord` it has no meaning for.
        let mut seen: Vec<FormulaKind> = Vec::new();
        for arg in [
            FractalArg::Mandelbrot,
            FractalArg::BurningShip,
            FractalArg::Tricorn,
            FractalArg::Celtic,
            FractalArg::Multibrot3,
        ] {
            let formula = arg.formula();
            assert!(!seen.contains(&formula), "{arg:?} duplicates a formula");
            seen.push(formula);
        }
        assert_eq!(
            seen.len(),
            FormulaKind::ALL.len(),
            "a formula is unreachable from the CLI"
        );
    }

    #[test]
    fn the_julia_flag_selects_the_plane_and_nothing_else() {
        for arg in [FractalArg::Mandelbrot, FractalArg::Celtic] {
            let parameter = arg.kernel(false);
            let julia = arg.kernel(true);
            assert!(!parameter.is_julia(), "{arg:?} without --julia");
            assert!(julia.is_julia(), "{arg:?} with --julia");
            // Same formula either way: the flag changes the plane alone.
            assert_eq!(parameter.formula(), julia.formula());
            assert_eq!(parameter.formula(), arg.formula());
        }
    }

    #[test]
    fn the_default_fractal_argument_is_the_mandelbrot_set() {
        assert_eq!(FractalArg::Mandelbrot.kernel(false), Kernel::MANDELBROT);
    }
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        // clap validates the whole tree here and panics on a conflicting or
        // duplicated argument, which is otherwise only discovered at runtime.
        Cli::command().debug_assert();
    }

    #[test]
    fn defaults_are_quiet_and_unlogged() {
        let cli = Cli::parse_from(["frascii"]);
        assert_eq!(cli.verbose, 0);
        assert_eq!(cli.log, None);
    }

    #[test]
    fn verbosity_counts_repeats() {
        assert_eq!(Cli::parse_from(["frascii", "-v"]).verbose, 1);
        assert_eq!(Cli::parse_from(["frascii", "-vv"]).verbose, 2);
        assert_eq!(Cli::parse_from(["frascii", "-vvv"]).verbose, 3);
    }

    #[test]
    fn log_takes_a_path() {
        let cli = Cli::parse_from(["frascii", "--log", "/tmp/frascii.log"]);
        assert_eq!(cli.log, Some(PathBuf::from("/tmp/frascii.log")));
    }

    #[test]
    fn directives_scale_with_verbosity_and_never_go_blanket() {
        for verbose in 0..4 {
            let directive = log_directive(verbose);
            assert!(
                directive.starts_with("frascii="),
                "{verbose} -v: {directive}"
            );
            // A bare level with no target would enable every dependency.
            assert!(
                directive.split(',').all(|d| d.contains('=')),
                "{verbose} -v: {directive}"
            );
        }
        assert_ne!(log_directive(0), log_directive(1));
        assert_eq!(log_directive(2), log_directive(9));
    }
}
