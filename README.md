# frascii

A terminal ASCII-art renderer for realtime fractals — a live, colourful ASCII rendering of escape-time fractals drawn straight into your terminal.

> **Status: it explores.** Mandelbrot and Julia sets, live in glyph mode, with pan, zoom and nine truecolour palettes. Half-block mode and the motion modes are next; see [CLAUDE.md](CLAUDE.md#not-yet-designed).

```
:::::::::::::::::::::::::::::::::::::::::::::-------------------===+-----------:::::::::::::::::
::::::::::::::::::::::::::::::::::::::----------------------===+  +=+=-------------:::::::::::::
::::::::::::::::::::::::::::::-----------------------==--=====+     =====-----=-------::::::::::
:::::::::::::::::::::::::--------------------------==+                    +  *=--------:::::::::
::::::::::::::::::::-------------==-------------===*                         #===-------::::::::
::::::::::::::::------------------==+ +@+#*%*===+                              ==--------:::::::
:::::::::::---------=------====                                              +-----------:::::::
::::::::::::::::------------------==+ +@+#*%*===+                              ==--------:::::::
::::::::::::::::::::-------------==-------------===*                         #===-------::::::::
:::::::::::::::::::::::--------------------------==+                    +  *=--------:::::::::::
::::::::::::::::::::::::::::::-----------------------==--=====+     =====-----=-------::::::::::
::::::::::::::::::::::::::::::::::::::----------------------===+  +=+=-------------:::::::::::::
```

(Glyphs only — every cell also carries a 24-bit colour.)

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
      --log <FILE>         Write logs to FILE
  -v, --verbose...         Increase log verbosity (-v debug, -vv trace)
      --headless           Render frames without a terminal and report timing
      --frames <N>         Frames to render [default: 60]
      --cols <N>           Samples across [default: 200]
      --rows <N>           Samples down [default: 100]
      --limit <N>          Iteration limit (default: scaled to the magnification)
      --magnification <X>  Magnification to render at [default: 1]
      --fractal <FRACTAL>  Which fractal to render [mandelbrot, julia]
      --ppm <FILE>         Write the last frame as a binary PPM
  -h, --help               Print help
  -V, --version            Print version
```

### Headless mode

`--headless` renders frames with no terminal involved and reports what it measured:

```sh
$ frascii --headless --cols 200 --rows 100 --frames 30 --magnification 1e6
20000 samples/frame, limit 2691, backend cpu
30 frames: mean 5.27ms, worst 5.84ms -> 189.7 fps
meets 30fps
frame was 11.1% interior
```

It exists for two reasons beyond benchmarking. It reports the **interior fraction** because a run that renders a solid black frame can look 85× faster than real work — the cardioid shortcut answers an all-interior view instantly — so the number is printed and flagged when the view is not representative. And it consumes `frascii-core` without touching the frontend at all, which keeps core honest about being usable on its own. (A source-scan test enforces that, not the compiler: cargo dependencies are per-package, so the frontend is technically in scope there.)

`--ppm` writes the last frame as a binary greyscale PGM, for checking geometry.

`--log` takes a file rather than a stream because the TUI owns the alternate screen: a log line on stdout or stderr is painted over the render. Tail it from a second terminal:

```sh
cargo run -- --log target/frascii.log -vv
tail -f target/frascii.log      # ... in another terminal
```

## Keyboard shortcuts

| Key | Action |
|-----|--------|
| `h` `j` `k` `l`, arrows | Pan |
| `+` / `=`, `-` | Zoom in / out |
| `.` / `,` | Raise / lower the iteration limit |
| `r` | Reset the view |
| `Tab` / `f` | Next fractal (Mandelbrot, Julia) |
| `p` | Next palette |
| `q` / `Esc` / `Ctrl+C` | Quit |

Zoom is about the centre — scroll-to-cursor needs mouse capture, which is not wired up yet. Half-block mode and the animation controls are also still to come.

Panning moves by a fraction of the view rather than a fixed number of samples, so it feels the same at every depth. Zooming in eventually stops: at about `4e12` magnification `f64` can no longer separate adjacent samples, and the viewport refuses to go further rather than dissolving into rounding error. The iteration limit tracks depth automatically; `.` and `,` bias it up or down from there.

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
