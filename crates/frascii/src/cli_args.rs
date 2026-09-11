//! The command-line interface.
//!
//! The clap tree lives in the binary rather than a library crate — see this
//! crate's manifest for why — and the pure decisions it feeds (log level,
//! log destination) are functions here with tests, so `main` stays a wiring
//! layer with nothing testable in it.

use std::path::PathBuf;

use clap::Parser;

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
