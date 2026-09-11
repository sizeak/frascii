//! The lanes: what a verification sweep runs, and what each one costs.
//!
//! Lanes are data. Adding one means adding a variant here and a matrix entry in
//! `.github/workflows/ci.yml` — and the tests below fail until both are done.

use std::fmt;

/// One check in a verification sweep.
///
/// Ordered cheapest-first, and [`Lane::ALL`] is that order: a one-second `fmt`
/// failure should surface before a multi-minute build has been paid for. The
/// runner always walks `ALL` and filters, so no combination of flags can
/// reorder lanes or run one twice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Lane {
    /// `cargo fmt --all -- --check`
    Fmt,
    /// `cargo clippy --workspace --all-targets -- -D warnings`
    Clippy,
    /// `cargo build --workspace --all-targets`
    Build,
    /// `cargo test --workspace`
    Test,
    /// `cargo doc --workspace --no-deps`, with warnings denied.
    Doc,
}

impl Lane {
    /// Every lane, cheapest first.
    pub(crate) const ALL: &'static [Self] =
        &[Self::Fmt, Self::Clippy, Self::Build, Self::Test, Self::Doc];

    /// The lane's name, as it appears on the command line and in logs.
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Fmt => "fmt",
            Self::Clippy => "clippy",
            Self::Build => "build",
            Self::Test => "test",
            Self::Doc => "doc",
        }
    }

    /// Parse a lane from its name.
    pub(crate) fn parse(name: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|l| l.name() == name)
    }

    /// The process exit code reserved for this lane's failure.
    ///
    /// One code per lane, so a caller can tell "clippy is dirty" (11) from "the
    /// tests broke" (13) without parsing stdout. The consumer is anything
    /// running the sweep non-interactively — a hook, a CI wrapper, an agent —
    /// which has an exit status to hand and would otherwise have to scrape the
    /// summary table. They start at 10 to stay clear of the usage code, and
    /// they are documented in `CLAUDE.md`, so keep them stable.
    pub(crate) const fn exit_code(self) -> u8 {
        match self {
            Self::Fmt => 10,
            Self::Clippy => 11,
            Self::Build => 12,
            Self::Test => 13,
            Self::Doc => 14,
        }
    }

    /// One line for `--list` and the run header: the command this lane runs.
    ///
    /// Rendered from [`Lane::command`] rather than written out beside it. The
    /// two used to be hand-mirrored strings with a test asserting they agreed;
    /// deriving one from the other makes that class of drift impossible
    /// instead of merely detected.
    pub(crate) fn description(self) -> String {
        let (args, env) = self.command();
        let prefix = match env {
            Some((key, value)) => format!("{key}='{value}' "),
            None => String::new(),
        };
        format!("{prefix}cargo {}", args.join(" "))
    }

    /// The cargo arguments this lane runs, and any environment it needs.
    ///
    /// Returned as data rather than executed here so a test can check the
    /// description above against the command below.
    pub(crate) fn command(self) -> (Vec<&'static str>, Option<(&'static str, &'static str)>) {
        match self {
            Self::Fmt => (vec!["fmt", "--all", "--", "--check"], None),
            Self::Clippy => (
                vec![
                    "clippy",
                    "--workspace",
                    "--all-targets",
                    "--",
                    "-D",
                    "warnings",
                ],
                None,
            ),
            Self::Build => (vec!["build", "--workspace", "--all-targets"], None),
            Self::Test => (vec!["test", "--workspace"], None),
            // `cargo doc` has no `--` passthrough, so the flag goes through the
            // environment. Not a formality: a broken intra-doc link, or a
            // `<lane>` that rustdoc reads as an HTML tag, is a *rustdoc*
            // warning that neither `cargo build` nor clippy ever sees. (The
            // `missing_docs` lint is a different mechanism — that one is the
            // clippy lane's `-D warnings`.)
            Self::Doc => (
                vec!["doc", "--workspace", "--no-deps"],
                Some(("RUSTDOCFLAGS", "-D warnings")),
            ),
        }
    }
}

impl fmt::Display for Lane {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// The CI workflow, read at compile time.
    const CI_WORKFLOW: &str = include_str!("../../.github/workflows/ci.yml");

    #[test]
    fn every_lane_has_a_distinct_exit_code_clear_of_the_usage_code() {
        let mut seen = HashSet::new();
        for lane in Lane::ALL {
            assert!(
                seen.insert(lane.exit_code()),
                "{lane} reuses exit code {}",
                lane.exit_code()
            );
            // Below 10 is reserved for "you invoked me wrong", so a caller can
            // always tell that apart from "a check failed".
            assert!(lane.exit_code() >= 10, "{lane} collides with a usage code");
        }
    }

    #[test]
    fn every_lane_round_trips_through_its_name() {
        for lane in Lane::ALL {
            assert_eq!(Lane::parse(lane.name()), Some(*lane));
        }
        assert_eq!(Lane::parse("nope"), None);
        assert_eq!(Lane::parse(""), None);
    }

    #[test]
    fn every_lane_describes_itself_as_the_cargo_command_it_runs() {
        for lane in Lane::ALL {
            let (args, _) = lane.command();
            assert!(!args.is_empty(), "{lane} runs nothing");
            assert!(
                lane.description()
                    .contains(&format!("cargo {}", args.join(" "))),
                "{lane}: {:?}",
                lane.description()
            );
        }
    }

    #[test]
    fn a_lanes_environment_shows_up_in_its_description() {
        // The doc lane's `-D warnings` is the whole reason that lane catches
        // anything, so `--list` must not hide it.
        assert_eq!(
            Lane::Doc.description(),
            "RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps"
        );
    }

    #[test]
    fn the_ci_matrix_lists_exactly_these_lanes() {
        // The claim this repo makes is that a green `cargo verify` means a
        // green PR, and that only holds if CI runs every lane. Nothing else
        // checks it: a lane added here but not to the workflow would simply
        // never run in CI, silently.
        //
        // `include_str!` reads the workflow at compile time, so the check costs
        // no filesystem access when the test runs.
        let line = CI_WORKFLOW
            .lines()
            .map(str::trim)
            .find(|l| l.starts_with("lane: ["))
            .expect("ci.yml declares a `lane: [...]` matrix axis");

        let listed: Vec<&str> = line
            .trim_start_matches("lane: [")
            .trim_end_matches(']')
            .split(',')
            .map(str::trim)
            .collect();

        let expected: Vec<&str> = Lane::ALL.iter().map(|l| l.name()).collect();
        assert_eq!(
            listed, expected,
            "ci.yml's matrix and Lane::ALL disagree; add the lane to both"
        );
    }
}
