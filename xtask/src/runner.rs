//! Running lanes: capture, timing, the failure tail, and the summary table.

use std::fs::{self, File};
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::lane::Lane;

/// How many lines of a failing lane's log to print.
///
/// The full output is on disk; this is the part worth reading without asking.
const FAIL_TAIL: usize = 40;

/// ANSI escapes for the summary output, or empty strings when colour is not
/// wanted.
///
/// Held as strings rather than applied by a helper so call sites read as
/// ordinary formatting, and so a non-terminal run emits no escapes at all —
/// which is what keeps a captured log readable.
#[derive(Debug, Clone, Copy)]
struct Style {
    reset: &'static str,
    bold: &'static str,
    dim: &'static str,
    red: &'static str,
    green: &'static str,
}

impl Style {
    /// Colour if stdout is a terminal and `NO_COLOR` is unset, plain otherwise.
    ///
    /// `NO_COLOR` is honoured at any value, per the informal standard: its
    /// presence is the signal, not its content.
    fn detect() -> Self {
        if std::env::var_os("NO_COLOR").is_none() && io::stdout().is_terminal() {
            Self {
                reset: "\x1b[0m",
                bold: "\x1b[1m",
                dim: "\x1b[2m",
                red: "\x1b[31m",
                green: "\x1b[32m",
            }
        } else {
            Self {
                reset: "",
                bold: "",
                dim: "",
                red: "",
                green: "",
            }
        }
    }
}

/// What became of one lane.
#[derive(Debug)]
pub(crate) struct Outcome {
    /// The lane that ran.
    pub(crate) lane: Lane,
    /// Whether it passed.
    pub(crate) passed: bool,
    /// How long it took.
    pub(crate) elapsed: Duration,
}

/// Runs lanes and reports on them.
pub(crate) struct Runner {
    cargo: PathBuf,
    log_dir: PathBuf,
    style: Style,
    outcomes: Vec<Outcome>,
}

impl Runner {
    /// A runner logging under `<workspace>/target/verify-logs`.
    pub(crate) fn new(workspace_root: &Path) -> Self {
        Self {
            // The cargo that invoked us, so a sweep cannot silently use a
            // different toolchain than the one running it. rust-toolchain.toml
            // is applied by the rustup shim, and inheriting $CARGO keeps every
            // lane on that same resolved toolchain.
            cargo: std::env::var_os("CARGO")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("cargo")),
            log_dir: workspace_root.join("target/verify-logs"),
            style: Style::detect(),
            outcomes: Vec::new(),
        }
    }

    /// Run one lane, printing a header and — on failure — a tail of its log.
    ///
    /// Never returns an error for a failing lane: the sweep continues, and
    /// [`Runner::summarise`] decides the process exit code. A lane failure and
    /// an inability to *run* the lane are both just "did not pass" here, and
    /// the log says which.
    pub(crate) fn run(&mut self, lane: Lane) {
        let s = &self.style;
        println!(
            "{}==>{} {:<8} {}{}{}",
            s.bold,
            s.reset,
            lane.name(),
            s.dim,
            lane.description(),
            s.reset
        );

        let started = Instant::now();
        let passed = match self.execute(lane) {
            Ok(passed) => passed,
            Err(error) => {
                eprintln!("    {}could not run {lane}: {error}{}", s.red, s.reset);
                false
            }
        };
        let elapsed = started.elapsed();

        if !passed {
            self.print_tail(lane);
        }
        self.outcomes.push(Outcome {
            lane,
            passed,
            elapsed,
        });
    }

    /// Spawn the lane's cargo command, capturing its output to a log file.
    fn execute(&self, lane: Lane) -> io::Result<bool> {
        fs::create_dir_all(&self.log_dir)?;
        let log_path = self.log_path(lane);
        let log = File::create(&log_path)?;
        // Two handles to one file rather than piping and interleaving by hand:
        // cargo writes progress to stderr and test output to stdout, and the
        // reader wants them in the order they happened.
        let errors = log.try_clone()?;

        let (args, env) = lane.command();
        let mut command = Command::new(&self.cargo);
        command
            .args(&args)
            .stdout(Stdio::from(log))
            .stderr(Stdio::from(errors));
        if let Some((key, value)) = env {
            command.env(key, value);
        }

        Ok(command.status()?.success())
    }

    fn log_path(&self, lane: Lane) -> PathBuf {
        self.log_dir.join(format!("{}.log", lane.name()))
    }

    /// Print the last [`FAIL_TAIL`] lines of a lane's log, indented.
    fn print_tail(&self, lane: Lane) {
        let s = &self.style;
        let path = self.log_path(lane);
        let shown = path
            .strip_prefix(std::env::current_dir().unwrap_or_default())
            .unwrap_or(&path)
            .display()
            .to_string();

        println!("    {}last {FAIL_TAIL} lines of {shown}:{}", s.dim, s.reset);

        match fs::read_to_string(&path) {
            Ok(contents) => {
                let lines: Vec<&str> = contents.lines().collect();
                let start = lines.len().saturating_sub(FAIL_TAIL);
                let mut out = io::stdout().lock();
                for line in &lines[start..] {
                    let _ = writeln!(out, "    | {line}");
                }
            }
            Err(error) => println!("    | (could not read the log: {error})"),
        }
    }

    /// Print the summary table and return the process exit code.
    ///
    /// The code is the **first** failing lane's, so a sweep that breaks in two
    /// places still reports the cheapest thing to fix.
    pub(crate) fn summarise(&self) -> u8 {
        let s = &self.style;
        let width = self
            .outcomes
            .iter()
            .map(|o| o.lane.name().len())
            .max()
            .unwrap_or(0);

        println!("\n{}-- verify summary{}", s.bold, s.reset);
        let mut first_failure = None;
        for outcome in &self.outcomes {
            let (label, colour) = if outcome.passed {
                ("PASS", s.green)
            } else {
                ("FAIL", s.red)
            };
            let note = if outcome.passed {
                String::new()
            } else {
                format!("exit {}", outcome.lane.exit_code())
            };
            println!(
                "  {}{label}{}  {:<width$}  {:>8}  {note}",
                colour,
                s.reset,
                outcome.lane.name(),
                // Duration's own Debug formatting, which picks a sensible unit
                // per lane rather than flooring a fast one to "0s".
                format!("{:.1?}", outcome.elapsed),
            );
            if !outcome.passed {
                first_failure = first_failure.or(Some(outcome.lane.exit_code()));
            }
        }

        let total = self.outcomes.len();
        match first_failure {
            None => {
                println!("{}{total} lanes passed{}", s.green, s.reset);
                0
            }
            Some(code) => {
                let failed = self.outcomes.iter().filter(|o| !o.passed).count();
                println!(
                    "{}{failed} of {total} lanes failed -> exiting {code}{}",
                    s.red, s.reset
                );
                println!("{}logs: target/verify-logs{}", s.dim, s.reset);
                code
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_paths_are_named_after_their_lane() {
        let runner = Runner::new(Path::new("/tmp/does-not-need-to-exist"));
        assert!(
            runner
                .log_path(Lane::Clippy)
                .ends_with("target/verify-logs/clippy.log")
        );
    }
}
