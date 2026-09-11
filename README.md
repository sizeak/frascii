# frascii

A terminal ASCII-art renderer for realtime fractals — a live, colourful ASCII rendering of escape-time fractals drawn straight into your terminal.

> **Status: bootstrap.** The workspace, the task runner and the crate boundaries are in place. The fractal kernels and the rendering pipeline are not implemented yet: `frascii` currently opens a terminal, draws a splash frame, and quits on `q`. What is deliberately deferred is listed in [CLAUDE.md](CLAUDE.md#not-yet-designed).

## Build and run

frascii needs current stable Rust via [rustup](https://rustup.rs), and nothing else. It tracks stable rather than supporting a range, so `rustup update stable` is the only setup step; the floor is declared in `Cargo.toml` (currently 1.98), and an older compiler fails with a clear cargo error rather than a confusing one.

Linux and macOS. Windows is not supported.

```sh
git clone https://github.com/sizeak/frascii
cd frascii
cargo run
```

`frascii` is the workspace's default binary, so `cargo run` needs no `-p`.

For anything performance-related use the release profile — the dev profile keeps debug assertions and does not apply link-time optimisation:

```sh
cargo run --release
```

## Usage

```
frascii [OPTIONS]

Options:
      --log <FILE>  Write logs to FILE
  -v, --verbose...  Increase log verbosity (-v debug, -vv trace)
  -h, --help        Print help
  -V, --version     Print version
```

`--log` takes a file rather than a stream because the TUI owns the alternate screen: a log line on stdout or stderr is painted over the render. Tail it from a second terminal:

```sh
cargo run -- --log target/frascii.log -vv
tail -f target/frascii.log      # ... in another terminal
```

## Keyboard shortcuts

| Key | Action |
|-----|--------|
| `q` / `Esc` | Quit |
| `Ctrl+C` | Quit |

Pan, zoom, fractal selection and animation controls are not implemented yet.

## How it fits together

Three crates, with dependencies pointing one way only:

| Crate | Responsibility |
|-------|----------------|
| `frascii-core` | Frontend-agnostic fractal logic: the kernels, the plane↔sample geometry, and the sampler that drives one over the other. No dependencies at all. |
| `frascii-tui` | The terminal frontend: the cell grid, glyph ramps and palettes, the ratatui blit, terminal lifecycle, key dispatch. |
| `frascii` | The binary: argument parsing and wiring. |

`frascii-core` is what a second frontend — an image exporter, a GPU surface — could consume unchanged, so nothing presentational or host-bound is allowed into it; what differs between frontends is the pixel aspect ratio, and that is a parameter rather than a reason to duplicate the geometry. Rendering currently sits with the terminal frontend as a deliberate simplification, and `frascii-tui`'s `render.rs` takes no ratatui dependency — enforced by a test — so extracting it later stays a file move. [CLAUDE.md](CLAUDE.md#architecture) has the rules.

`xtask/` is the repo's task runner, not part of the product.

## Development

```sh
pre-commit install                # once per clone

cargo verify                      # every lane: fmt, clippy, build, test, doc
cargo verify --only clippy        # one lane
cargo verify --list               # the lane / exit-code table
cargo test -p frascii-tui         # one crate's tests (plain cargo)
cargo run                         # the TUI
```

`cargo verify` is an alias for the `xtask` crate — a normal workspace member, so the task runner is Rust, built by cargo and tested by `cargo test`. Every lane runs even after an earlier one fails, output is captured to `target/verify-logs/`, and the process exits with the first failing lane's reserved code (10 fmt, 11 clippy, 12 build, 13 test, 14 doc).

CI runs `cargo verify --only <lane>`, one job per lane, over the same lane definition — so a green `cargo verify` locally means a green PR, and a test fails if the CI matrix and the lane list ever disagree. See [CONTRIBUTING.md](CONTRIBUTING.md) for the change workflow.

## Licence

MIT — see [LICENSE](LICENSE).
