# frascii

A terminal ASCII-art renderer for realtime fractals — a live, colourful ASCII rendering of escape-time fractals drawn straight into your terminal.

> **Status: it runs.** Launch it and the Mandelbrot set dives forever with the colours drifting. Mandelbrot and Julia sets, glyph or half-block rendering, optional supersampling, pan and zoom, nine truecolour palettes, a Julia parameter orbit. What is deliberately *not* here is listed in [CLAUDE.md](CLAUDE.md#not-yet-designed).

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
| `m` | Glyph / half-block rendering |
| `s` | Supersampling: 1× / 2× / 3× |
| `?` | Full control list |
| `i` | Status bar |
| `c` | Palette cycling on/off |
| `o` | Julia parameter orbit |
| `z` | Unattended auto-zoom |
| `Space` | Pause all motion |
| `q` / `Esc` / `Ctrl+C` | Quit |

`s` supersamples: each output pixel averages a `k × k` block of samples, which antialiases the boundary instead of snapping each cell to whichever sample landed at its centre. Colours are averaged *after* the palette and in linear light — averaging iteration counts and colouring once would give a boundary cell a colour belonging to neither side, and averaging sRGB bytes would make every mixed edge too dark.

It costs `k²` times the samples, measured at 1e6 magnification on a 20,000-pixel frame: 7.3ms at 1×, 18.2ms at 2×, 88.5ms at 3×. So 2× still holds 30fps and **3× does not** — use 3× on a parked view, where frame rate does not matter and it is the sharpest still available.

### Motion

**All of this is running when you launch.** The dive and the palette drift are on by default, because a static first frame is not what a realtime renderer should open with. Touching any navigation key stops the dive and hands you control; `z` starts it again.

`c` drifts the colour mapping without touching the samples, so it costs one pass over an existing grid and no fractal maths at all — it composes with everything else.

`o` walks the Julia parameter around a circle near the Mandelbrot boundary, which is the band where Julia sets have structure rather than being a filled disc or dust. It switches to Julia if you were on Mandelbrot, because orbiting a parameter the current fractal does not have would look like the key did nothing.

`z` dives forever: descend toward the boundary, re-aim every eight-fold magnification, and on reaching the limit of `f64` reset to the whole set and pick somewhere new. It stops one notch *short* of the hard precision clamp — diving all the way would show several visibly mushy frames before every cut.

Every rate is per second and applied per frame, and zoom is geometric in elapsed time, so a dive covers the same ground in the same wall-clock time on a slow machine — in fewer, chunkier frames rather than more slowly.

A status bar sits at the bottom showing what is being rendered — fractal, mode, palette, magnification, iteration limit, what is moving — and the common controls. `?` opens the full list; `i` hides the bar for a full-screen view. The hints shrink by tier as the terminal narrows rather than being cut off mid-word, and the narrowest still points at `? help`.

**Pan before you zoom.** Zoom is about the centre of the view, and the home view is centred on `-0.75`, which is *inside* the set — so pressing `+` from a fresh start dives into the solid interior and the screen goes blank. Pan to some boundary filigree first. Scroll-to-cursor would fix this properly and needs mouse capture, which is not wired up yet; the unattended auto-zoom mode will use core's `boundary_target` to pick somewhere worth diving into.

`m` switches between one sample per cell shaded by the glyph ramp, and two vertically stacked samples per cell each with its own colour. Half-block doubles the vertical resolution but carries density in colour alone — it is a pixel display rather than ASCII art, which is why glyph mode is the default. 1×2 is the ceiling for *coloured* sub-cell rendering: braille and the legacy-computing octants reach 2×4 but can only carry one colour per cell.

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
