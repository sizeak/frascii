//! The `frascii` binary.
//!
//! Thin by design: parse arguments, set up logging, hand off to the frontend,
//! print an error. Anything with a decision in it belongs in a library crate
//! where a unit test can reach it — `main` cannot be tested without spawning
//! the binary, so logic that lands here is logic that stops being covered.

mod cli_args;

use std::fs::File;
use std::process::ExitCode;

use clap::Parser;
use tracing_subscriber::EnvFilter;

use cli_args::{Cli, log_directive};

/// The binary's name, as users invoke it.
///
/// `CARGO_PKG_*` resolves per compiling package, so this is *this* package's
/// name — which is the one a user typed. A library crate reading the same macro
/// gets its own name instead, which is why user-facing output belongs here and
/// not down a layer.
const NAME: &str = env!("CARGO_PKG_NAME");

fn main() -> ExitCode {
    let cli = Cli::parse();

    if let Err(error) = init_logging(&cli) {
        eprintln!("{NAME}: {error}");
        return ExitCode::FAILURE;
    }

    match frascii_tui::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            // The terminal is already restored by the time this runs (see
            // `frascii_tui::run`), so stderr is safe to write to here.
            eprintln!("{NAME}: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Install a log subscriber writing to `--log`'s file, if one was given.
///
/// A no-op without `--log`: there is nowhere to write that would not corrupt
/// the alternate screen. `RUST_LOG` still wins over `-v` when set, which is the
/// convention every `tracing` user expects.
fn init_logging(cli: &Cli) -> std::io::Result<()> {
    let Some(path) = cli.log.as_deref() else {
        return Ok(());
    };

    let file = File::create(path)?;
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(log_directive(cli.verbose)));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(file)
        .with_ansi(false)
        .init();

    tracing::info!(version = env!("CARGO_PKG_VERSION"), "{NAME} starting");
    Ok(())
}
