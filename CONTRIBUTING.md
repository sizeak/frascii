# Contributing to frascii

## Setup

frascii needs current stable Rust via [rustup](https://rustup.rs) and nothing else. There is no `rust-toolchain.toml`, no dev shell to enter and no task runner to install — `cargo verify` is a workspace crate. The project tracks stable rather than supporting a range, so if `cargo` reports "not supported by the following packages", run `rustup update stable`.

Linux and macOS; Windows is not supported.

```sh
git clone https://github.com/sizeak/frascii
cd frascii
rustup update stable
pre-commit install     # runs fmt + clippy on every commit
cargo build --workspace
```

`pre-commit` itself comes from your package manager or `pipx install pre-commit`. The hook is the cheap subset of `cargo verify` on purpose: a hook slow enough to be bypassed with `--no-verify` protects nothing. Its fmt step *fixes* formatting rather than checking it — the commit still fails, so restage and reattempt.

One optional extra: `cargo install cargo-insta`, needed only to accept or review render-snapshot changes.

## Verifying a change

```sh
cargo verify                          # every lane: fmt, clippy, build, test, doc
cargo verify --only clippy            # one lane
cargo verify --list                   # the lane / exit-code table
cargo test -p frascii-core smooth      # one crate and a filter — plain cargo
```

`cargo verify`'s default is the whole set on purpose: it matches CI's matrix, so a green sweep means a green PR. There are no tiers and no `-p` wrapper — a subset is `--only`, and cargo already tests one crate perfectly well.

Every lane runs even after an earlier one fails. Full output goes to `target/verify-logs/<lane>.log`; only a 40-line tail is printed. The process exits with the **first failing lane's** reserved code, so you can tell what broke without opening a log:

| Exit | Meaning |
|------|---------|
| 0 | every selected lane passed |
| 2 | bad arguments |
| 10 / 11 / 12 / 13 / 14 | fmt / clippy / build / test / doc failed |

## The change workflow

1. **Write a failing test first.** Red–green TDD, for bug fixes as much as features: a bug fix without a regression test is a fix that can silently come undone. If the fix seems to live somewhere untestable, that is a signal to push the logic down into a library crate rather than to skip the test.
2. **Make the minimal change** that turns the test green.
3. **Run `cargo verify`** in full. Don't infer success — read the exit code.
4. **Request a review** and address the findings.
5. **Open the PR**, watch CI, and merge only when every check is green.

## Things worth knowing before your first change

- **Which crate does this belong in?** The test to apply is: *would a second frontend need this unchanged?* If yes it belongs in `frascii-core` — which is why that crate's only dependency is `rayon` and allows nothing presentational or host-bound. The kernels, the plane↔sample geometry and the sampler are all core's; the glyphs and colours are the frontend's. Rendering lives with the terminal frontend for now, and `render.rs` stays ratatui-free so it can be extracted later — a test enforces that, so don't import ratatui into it for convenience. [CLAUDE.md](CLAUDE.md#architecture) has the full rules, and each `Cargo.toml` says what must never appear in its `[dependencies]`.
- **Never `println!` in the TUI.** The alternate screen is live; anything written to stdout or stderr is painted over the render. Use `tracing` and `--log`.
- **Read snapshot diffs.** Render snapshots fail only when rendering genuinely changed. `cargo insta review` to accept; never blind-accept, and never delete a snapshot file to make a test pass — re-point it at the new UI instead.
- **Every public item needs a doc comment.** `missing_docs` is warn-level workspace-wide and CI denies warnings. The `doc` lane also catches broken intra-doc links, which `cargo build` never sees.
- **Keep a clap doc comment to one line.** With a blank line in the comment, clap prints the first paragraph as `-h` help and the *whole* comment as `--help`; with no blank line it merges every line into one `-h` string. Either way rationale in a doc comment ends up in front of users — put it in a `//` comment.
- **Don't add `clippy::pedantic`.** It looks like an improvement and is not — see the reasoning in [CLAUDE.md](CLAUDE.md#lints).
- **Never force push.** Not `--force`, not `--force-with-lease`, and don't amend a pushed commit. Add a new commit.
- **Commits are GPG-signed.** Don't skip signing.

## Adding a verify lane

Lanes are data. Add a `Lane` variant in `xtask/src/lane.rs` — name, exit code, description, command — and a matrix entry in `.github/workflows/ci.yml`.

You cannot forget the second half: `the_ci_matrix_lists_exactly_these_lanes` reads the workflow at compile time and fails if it and `Lane::ALL` disagree. The other tests in that module keep every exit code distinct and clear of the usage code, and check that each lane's description names the cargo subcommand it actually runs.
