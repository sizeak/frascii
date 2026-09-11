//! The frascii repo's task runner.
//!
//! `cargo verify` is the whole interface; it is an alias for
//! `cargo run -p xtask -- verify` (see `.cargo/config.toml`).
//!
//! This is a normal workspace member, which is the point: the lane table and
//! its exit codes are typed, its pure parts are tested by
//! `cargo test --workspace` alongside everything else, and the repo stays
//! single-language with no extra tool to install.
//!
//! What it adds over typing the five cargo commands by hand: every lane runs
//! even after an earlier one fails, output is captured per lane with only a
//! tail printed, and the process exits with the first failing lane's reserved
//! code so a caller can tell what broke without reading a log.
//!
//! Deliberately *not* here: tiers, and a `-p` wrapper around `cargo test -p`.
//! A subset is `--only`, and cargo already tests one crate perfectly well.

mod lane;
mod runner;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;

use lane::Lane;
use runner::Runner;

/// Reserved for "you invoked me wrong", kept clear of every lane code so a
/// caller can always tell that apart from "a check failed".
const EXIT_USAGE: u8 = 2;

/// One command, so no subcommand: a `verify` subcommand under a single-variant
/// enum would be scaffolding for a second task that does not exist. If one ever
/// does, that is when the enum earns its place.
#[derive(Debug, Parser)]
#[command(
    name = "xtask",
    about = "Run the repo's checks",
    long_about = "Run the repo's checks.\n\n\
        With no arguments, runs every lane -- the same set CI runs, so a green \
        sweep means a green PR. Every selected lane runs even after an earlier \
        one fails. Output goes to target/verify-logs/<lane>.log and only the \
        last 40 lines are printed. Exits with the first failing lane's \
        reserved code: 10 fmt, 11 clippy, 12 build, 13 test, 14 doc. Plus 2 \
        for bad arguments."
)]
struct Cli {
    /// Run just this lane. Repeatable. Each CI job runs one.
    #[arg(long, value_name = "LANE", conflicts_with = "list")]
    only: Vec<String>,

    /// Print the lane / exit-code table and exit.
    #[arg(long)]
    list: bool,
}

fn main() -> ExitCode {
    let Cli { only, list } = Cli::parse();

    if list {
        print_lane_table();
        return ExitCode::SUCCESS;
    }

    let lanes = match select_lanes(&only) {
        Ok(lanes) => lanes,
        Err(message) => {
            eprintln!("xtask: {message}");
            return ExitCode::from(EXIT_USAGE);
        }
    };

    let mut runner = Runner::new(&workspace_root());
    println!(
        "==> running {} lane(s): {}",
        lanes.len(),
        lanes
            .iter()
            .copied()
            .map(Lane::name)
            .collect::<Vec<_>>()
            .join(" ")
    );
    for lane in lanes {
        runner.run(lane);
    }
    ExitCode::from(runner.summarise())
}

/// Which lanes `--only` selects, or every lane when it is empty.
///
/// Always returned in canonical order: this filters [`Lane::ALL`], so
/// `--only test --only fmt` still runs fmt first and neither flag order nor a
/// repeat can change that.
fn select_lanes(only: &[String]) -> Result<Vec<Lane>, String> {
    if only.is_empty() {
        return Ok(Lane::ALL.to_vec());
    }

    let mut unknown = Vec::new();
    let mut selected = Vec::new();
    for name in only {
        match Lane::parse(name) {
            Some(lane) => selected.push(lane),
            None => unknown.push(name.as_str()),
        }
    }
    if !unknown.is_empty() {
        let known: Vec<_> = Lane::ALL.iter().map(|l| l.name()).collect();
        return Err(format!(
            "unknown lane(s): {} (known: {})",
            unknown.join(", "),
            known.join(", ")
        ));
    }
    Ok(Lane::ALL
        .iter()
        .copied()
        .filter(|l| selected.contains(l))
        .collect())
}

fn print_lane_table() {
    // Widths line up with the rows below.
    println!("LANE     EXIT  CHECK");
    for lane in Lane::ALL {
        println!(
            "{:<8} {:<5} {}",
            lane.name(),
            lane.exit_code(),
            lane.description()
        );
    }
}

/// The workspace root, from this crate's manifest directory.
///
/// `CARGO_MANIFEST_DIR` rather than the current directory, so `cargo verify`
/// logs to the same place regardless of which subdirectory it was run from.
fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        // clap validates the whole tree here and panics on a conflicting or
        // duplicated argument, which is otherwise only found at runtime.
        Cli::command().debug_assert();
    }

    #[test]
    fn no_arguments_runs_every_lane() {
        // The load-bearing default: `cargo verify` has to be the same set CI
        // runs, or "green locally means green PR" is false.
        assert_eq!(select_lanes(&[]).expect("no flags is valid"), Lane::ALL);
    }

    #[test]
    fn only_selects_named_lanes_in_canonical_order() {
        // Named out of order on purpose: the sweep must stay cheapest-first so
        // a fast failure is not hidden behind a slow lane.
        let lanes = select_lanes(&["test".into(), "fmt".into()]).expect("both lanes are known");
        assert_eq!(lanes, vec![Lane::Fmt, Lane::Test]);
    }

    #[test]
    fn only_deduplicates_a_repeated_lane() {
        let lanes = select_lanes(&["fmt".into(), "fmt".into()]).expect("fmt is known");
        assert_eq!(lanes, vec![Lane::Fmt]);
    }

    #[test]
    fn only_rejects_an_unknown_lane_and_lists_the_known_ones() {
        let error = select_lanes(&["nope".into()]).expect_err("nope is not a lane");
        assert!(error.contains("nope"), "{error}");
        assert!(error.contains("clippy"), "{error}");
    }

    #[test]
    fn a_partly_unknown_only_list_fails_rather_than_running_the_known_half() {
        // Running what it understood and ignoring the rest would report success
        // for a sweep that never ran the lane the caller asked for.
        let error = select_lanes(&["fmt".into(), "nope".into()]).expect_err("nope is not a lane");
        assert!(error.contains("nope"), "{error}");
    }
}
