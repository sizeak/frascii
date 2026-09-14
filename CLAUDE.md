# CLAUDE.md

Guidance for Claude Code (claude.ai/code) working in this repository. `AGENTS.md` is a symlink to it — edit this file, never the symlink.

frascii is a terminal ASCII-art renderer for realtime fractals: a live, colourful ASCII rendering of escape-time fractals (Mandelbrot, Julia, and others) drawn in a terminal window.

**Status: bootstrap.** The workspace, the task runner and the crate boundaries are in place. The fractal kernels and the rendering pipeline are *not designed yet* — see [Not yet designed](#not-yet-designed), which names every deferred decision and which crate owns it. Do not fill a gap in by inference; the design pass is separate work.

## Commands

Everything goes through cargo. The task runner is `xtask`, an ordinary workspace member, so there is no shell script, no Makefile and nothing to install.

```
cargo verify              every lane: fmt, clippy, build, test, doc
cargo verify --only doc   one lane (each CI job runs exactly this)
cargo verify --list       the lane / exit-code table
cargo test -p frascii-tui smooth     one crate, one filter — plain cargo
cargo run                            the TUI (the default binary)
cargo run --release                  ... release profile (see Performance)
cargo run -- --log target/frascii.log -vv               ... with logging
```

Every selected lane runs even after an earlier one fails. Output goes to `target/verify-logs/<lane>.log`; only a 40-line tail is printed. **The process exits with the first failing lane's reserved code** — **10** fmt, **11** clippy, **12** build, **13** test, **14** doc, and **2** for bad arguments — so you can tell what broke without opening a log. Unit tests assert the codes stay distinct and clear of the usage code.

`cargo verify`'s default is deliberately the *whole* set, matching CI's matrix: a default that skipped a lane would mean a green sweep did not imply a green PR, which is the only thing the tool is for. CI runs `cargo verify --only <lane>`, one job per lane, so both sides execute the same lane definition — and `the_ci_matrix_lists_exactly_these_lanes` fails if the workflow and `Lane::ALL` ever disagree.

**Adding a lane:** add a `Lane` variant in `xtask/src/lane.rs` and a matrix entry in `.github/workflows/ci.yml`. The tests fail until both are done.

`--log` takes a file because the TUI owns the alternate screen; tail it from a second terminal.

### Toolchain and platforms

**The project tracks current stable Rust.** There is no `rust-toolchain.toml` — whatever `stable` your rustup has is the toolchain, and CI runs `rustup update stable`. `rust-version` in `Cargo.toml` is therefore the only version statement in the repo, which is deliberate: too old fails as a clear cargo error ("not supported by the following packages") rather than a confusing compile error somewhere in a dependency.

It is currently **1.98**, and that number is doing two jobs. `resolver = "3"` is MSRV-aware, so the declared floor is a dependency-resolution input — raising it from 1.88 to 1.98 moved four transitive dependencies forward by itself, and an understated floor silently holds newer versions back. For reference the lowest version anything actually *needs* is 1.88 (ratatui 0.30 declares 1.88.0; the event loop uses a let-chain).

Edition is **2024**, which is the newest that exists. `rustc --edition` also accepts `future`, but that is a nightly-only testing edition with no stability guarantee — not a newer edition to adopt.

xtask inherits `$CARGO` when spawning a lane, so a sweep cannot run on a different toolchain than the one running it. One gotcha that has already bitten: a `RUSTUP_TOOLCHAIN` environment variable **silently overrides everything**, including a directory's own pin, so if a verification run seems to be on the wrong compiler, check the environment before the config.

**Linux and macOS only — Windows is not supported and is not planned.** That is a scoping decision, not an accident, and it changes which boundary claims matter: a receipt about Windows console behaviour is not a justification for code here. (`app.rs`'s `KeyEventKind` check is the worked example — see below.)

## Verification discipline

- Never pipe command output through `| tail` / `| head` when checking pass/fail — it masks exit codes. Run the command bare and read the real exit status.
- Do not claim tests, CI or builds are green without the actual exit code / CI conclusion in front of you. If verification was blocked, say so.
- After any rebase or merge, re-run a full build before declaring done.
- A render snapshot diff is evidence, not noise — see [Render snapshots](#render-snapshots).

## Workflow: TDD then review then merge

For every bug fix or behaviour change: (1) write a failing test that reproduces it, (2) implement the minimal fix, (3) run `cargo verify` in full, (4) request a review and address findings, (5) open the PR, watch CI, merge only when green.

## Architecture

```
frascii-core      frontend-agnostic fractal logic; zero dependencies
       ^
frascii-tui       the terminal frontend: rendering + ratatui + key dispatch
       ^
frascii           the binary: clap + wiring
```

Plus `xtask`, the task runner, which is not part of the product.

**`frascii-core` is the boundary that matters.** It is what a second frontend — an image exporter, a GPU surface — would consume *unchanged*, so nothing presentational and nothing host-bound may drift into it: no colour, no glyph ramp, no terminal, no `std::io`. Its `[dependencies]` block is empty and is the whole check. The test for new code is: *would a second frontend need this unchanged?*

Two senses of "grid", because conflating them is what pulls presentation downward: a **sample grid** of `Escape` values is core's output and frontend-agnostic; a **cell grid** of glyphs and colours is presentation. Likewise the plane↔sample geometry belongs in core with the pixel aspect ratio as an *input* (≈2.0 for a terminal cell, 1.0 for a square pixel) — that number is the frontend's, the geometry is not.

**Rendering deliberately sits with the terminal frontend** rather than in its own crate. That is a simplification for the design phase, and it is reversible by construction: `frascii-tui/src/render.rs` holds the cell grid's types (`Grid`, `Cell`, `Rgb`) and takes **no ratatui dependency**, while `widget.rs` holds the ratatui impl. Extracting `render.rs` later is a file move — and `this_module_stays_free_of_ratatui` in that module is what keeps it true, since the rule is otherwise unenforceable prose.

Two more rules with teeth:

- **`clap` lives in the binary and only in the binary.** A library embedding frascii should not inherit an argument parser, and a clap derive takes its program name and version from the package it compiles in — so a `Parser` in a library prints that library's name from `--version`. A correctness rule, not tidiness. (`xtask` has its own tree for the same reason: it is a binary.)
- **Reach through `ratatui::crossterm`, never a direct `crossterm` dependency.** ratatui re-exports the exact crossterm it was built against; a separate manifest entry resolves independently, so a semver-incompatible bump on either side compiles two crossterms into one binary and the key events you read come from a different type than the terminal you initialised.

### The crate seam

`frascii_core::Escape` is the contract between the kernels and whatever draws them — an iteration count plus a continuous `smooth` value, so a renderer can interpolate a palette rather than draw visible contour bands. It is **provisional**: shading that needs more than an escape time (distance estimation, orbit traps, derivative-based normals) would extend or replace it.

`Cell { glyph, colour }` is provisional too, and in a specific direction: the standard fix for a terminal's 2:1 cell is half-block or braille rendering, which needs a background colour or sub-cell samples. Expect this type to grow when that lands.

`Grid` is deliberately forgiving at its edges — a zero dimension is legal, out-of-bounds writes are dropped, and the `Widget` impl renders the overlap when grid and viewport disagree about size. Not defensiveness for its own sake: **a terminal resize is observed a frame before the grid is re-rendered to match**, and a panic there would turn a window drag into a crash.

`Widget` is implemented for `&Grid`, not `Grid`, because `Widget::render` takes `self` by value (`ratatui-core-0.1.2 src/widgets/widget.rs:73`) and a frame should not give up ownership of a grid it will redraw next tick.

### Not yet designed

Named so nobody has to guess whether it was forgotten.

In **`frascii-core`**:

- supersampling, if any
- more fractals beyond Mandelbrot and Julia (the `Fractal` trait is the seam)

In **`frascii-tui`** — presentation and dispatch:

- braille or octant modes, if ever. They reach 2×4 sub-samples but carry **one colour per cell**, which is useless for a colour-mapped fractal — 1×2 half-block is the ceiling for coloured sub-cell rendering, so this is a "probably never" rather than a "not yet"
- zoom about the cursor, which needs mouse capture. Not merely unwired: `EnableMouseCapture` turns on all-motion reporting that has to be coalesced, and it must be disabled in the teardown **and** the panic hook or a crash leaves the user's shell eating escape codes
- periodicity checking in the kernels, which is the next real optimisation if one is ever needed. It outranks SIMD for deep views, where the cardioid shortcut catches nothing. Needs a false-positive property test first: too loose an epsilon paints exterior as interior, which looks exactly like a kernel bug
- the camera and animation clock's *keybindings* (the camera's own maths is core's). The clock should be `Instant`-based: an earlier tick counter incremented only on idle polls, so any keypress skipped it, and it was deleted rather than left to be built on
- terminal colour-capability detection. It belongs *upstream* of `to_colour`, choosing which colour is produced — not in a per-cell conversion that runs tens of thousands of times a frame

The render path is built: `App::update` ticks the clock, advances whatever is moving, samples (only when its `SampleParams` changed) and shades; `App::draw` blits the finished grid and nothing else. Glyph and half-block rendering, nine palettes, both kernels, keyboard interaction and all three motion modes exist.

**The status bar takes a row rather than being overlaid**, so nothing shows through behind it and the sample lattice matches what is visible. Toggling it therefore re-samples, which a test asserts — that is the honest cost of the picture genuinely changing size, and asserting it stops the toggle being "optimised" back into an overlay that puts the fractal behind the text. It is dropped entirely in a terminal one row tall, where a bar would leave nowhere for the fractal.

One trap worth knowing: **`Precision::Exhausted` is unreachable by zooming.** The clamp refuses to go below `min_half_width`, which sits exactly at the `Marginal` boundary — so a status line testing for `Exhausted` says nothing at the one moment the user most needs told. Key "max zoom" off the clamp (`half_width <= min_half_width()`) instead.

**The defaults move.** Cycling *and* the dive are on at launch, because "realtime fractal renderer" is the premise and a static first frame does not deliver it — and cycling alone is not enough, which is the distinction worth keeping: it moves the colour while the geometry stands still, reading as a tinted photograph. Any navigation key calls `navigate`, which stops the drive first: panning against a descending dive is a fight the user cannot win, since the next frame moves the view back.

**Supersampling is invariant on the sample aspect, which is why it is its own setting** rather than more `CellMode` variants: it multiplies both lattice axes, so `cell_aspect × across/down` is unchanged. Colours are averaged **after** the palette and **in linear light** — averaging `smooth` and colouring once would give a boundary cell a colour belonging to neither side (the palette is cyclic, so the mean of two counts can land on a hue neither sample had), and averaging sRGB bytes makes a half-and-half edge 10–20% too dark, which shows as dark fringing on exactly the filigree supersampling was turned on to resolve. Its cap is measured, not guessed: 2× holds 30fps at depth and 3× does not.

**The dive must re-aim periodically, not once.** `boundary_target` picks a point that is on the boundary *at the current scale*; several decades down, the filament it sat on has resolved into structure that moved away from it. Diving on a single target renders solid interior within a few decades — measured at 898 blank frames out of 1200 before `RETARGET_EVERY` existed. Eight-fold is the factor the original validation used. This is the same lesson the first `--headless` benchmark taught, arriving a second time by a different route: blind descent lands in solid regions.

The dive also stops at `Precision::Marginal` rather than at the hard clamp. Reaching the clamp would show several visibly mushy frames before *every* cut, forever; the clamp stays as the thing that refuses a manual zoom.

Half-block landed in increment 4 and **the aspect-absorption claim held**: `switching_mode_keeps_the_same_view` zooms and pans, presses `m`, and asserts the centre, width and height of the plane region are unchanged while the sample lattice doubles and the aspect halves. Nothing in core needed touching — the mode is a `CellMode` variant, a subdivision pair, and a blit.

Two things that came out of building it. Half-block mode gets **no render snapshot**: `TestBackend` captures symbols only and every cell in that mode is `▀`, so a snapshot would be a uniform rectangle that passes whatever the picture does — coverage in appearance only. The packing is asserted directly instead. And a `Paragraph` overlay **recolours** a row without overwriting the glyphs under it, so the status line needed a `Clear` first or the fractal showed through to the right of the text.

`Kernel` is the reason the fractal can live in `SampleParams`: an enum is `Copy + PartialEq` where a `Box<dyn Fractal>` is neither, so a trait object there would have silently broken the comparison that decides whether to re-sample. It also moves dispatch out of the inner loop — `Kernel::sample_into` matches once per frame and hands the sampler a concrete type, where `&dyn Fractal` cost an indirect call per *sample*. That second benefit went unmeasured: on a machine at load average 7, eight repeats of the same binary spread 113–145µs, so the difference is well inside the noise. The enum is justified by what it is *required* for, not by a speedup anyone has demonstrated.

### Drawing into the terminal

Researched against the vendored sources before writing any of it, because these
are exactly the boundary claims this file says not to take on plausibility.

**Write a direct `Buffer` blit; do not use ratatui's `Canvas`.** The reason is
*not* that Canvas cannot do per-pixel colour — it can, `Painter::paint` takes a
`Color` per point (`ratatui-widgets-0.3.2 src/canvas.rs:478`) and `Shape` is a
one-method trait, so a custom shape can plot an arbitrary coloured grid. Only
the bundled shapes are per-shape-coloured. The actual reasons are that
`Context::new` runs inside `render` (`canvas.rs:880-886`), so every frame
rebuilds `vec![vec![None; width]; height*2]` (`canvas.rs:329`) — `2·height+1`
allocations — to fill a grid that is then re-packed into the buffer you could
have written directly; and that `CharGrid` holds **one fixed char for the whole
grid** (`canvas.rs:235`), so the glyph-density ramp cannot be expressed on it at
all.

**Half-block packing** (the logic worth copying, `canvas.rs:371-381`): upper
sample → `fg`, lower sample → `bg`, glyph `▀`. **No colour is lost** when the
two differ; both 24-bit values survive. Ratatui switches to `█` when they are
equal, but that is cosmetic (`canvas.rs:356-357`) — always emit `▀` instead, so
every cell stays single-width and the diff never enters its wide-character path.

**1×2 half-block is the ceiling for coloured sub-cell rendering.** Braille,
octant, quadrant and sextant all go through `PatternGrid`, which is explicitly
one colour per cell: "there's no way to individually set the background color of
each pseudo-pixel in a pattern character" (`canvas.rs:126-128`). Braille buys
eight samples of *shape* and one colour — useless for a colour-mapped fractal.

**Put a `BufWriter` under the backend.** `DefaultTerminal` writes through
`stdout()` (`ratatui-0.30.2 src/init.rs:401`), which is a `LineWriter`. A full
200×50 redraw where every cell changes both colours is ~30 bytes/cell ≈ 300 KB
per frame, and that write volume — not ratatui — is the ceiling. Crossterm
already emits fg and bg as a single SGR and its own comment measures ~20% more
fps from doing so on precisely this workload (`crossterm-0.29.0
src/style.rs:294-314`).

**Mouse capture is neither enabled nor cleaned up for you.** `try_init` sets
raw mode and the alternate screen only (`init.rs:397-403`), and `restore()` does
not disable capture (`init.rs:554-559`). Enable it explicitly, and disable it in
teardown *and* in the panic hook, or a crash leaves the user's shell receiving
escape codes. `EnableMouseCapture` turns on `?1003h`, i.e. every motion event
with no button held, so coalesce the queue or render frames behind the cursor.
Mouse coordinates are cell-granular (`event.rs:777-789`), so zoom-to-cursor in
half-block mode can only address the cell, not which half of it.

### The render path, and why it is shaped this way

Built in increment 2. Each of these was a decision the obvious implementation
gets wrong, so the reasoning is kept rather than just the result.

**Sampling and shading are two functions over two caller-owned buffers**, not
one loop. `sample_into(grid, viewport, fractal, limit)` in core;
`shade_into(shader, samples, cells)` in the frontend. This is not primarily
about frame rate — it is what makes every ramp, palette and sub-cell reduction
unit-testable against a synthetic grid of hand-written `Escape` values with no
kernel in the test, and it is the seam a background sampler or progressive
refinement would later plug into. Fused into one loop, the reductions get
written against that interleaving and three render modes depend on it before
anyone notices.

**Whether to re-sample is *derived* from input equality, never a `bool` someone
must remember to set.** Group every input that changes sample output into one
`Copy + PartialEq` struct, keep the last one used, and compare. The test for a
new field is one question: if this changes, must the kernel re-run? The
iteration limit yes; the palette phase no. Cache-skipping only benefits palette
cycling and the idle screen — Julia orbit and auto-zoom re-sample every frame by
nature — so build the equality check and nothing more elaborate.

**Neither the glyph ramp nor the palette may normalise against the iteration
limit.** A dive raises the limit from ~300 to ~5,000, so any `smooth / limit`
map re-shades the *entire frame* each time it steps: a whole-screen pulse,
repeating for every step of every dive, forever, reading as a rendering glitch
with its cause nowhere near the render code. Use a **fixed** cyclic period for
colour (32 iterations) and a **fixed** log reference for density (512). A rising
limit then only adds bands near the boundary and nothing already on screen
moves — which is the behaviour you want anyway.

**Density comes from escape time, not from perceived luminance** — the open
question this file used to pose. If density tracked the palette's luminance,
cycling the palette would change the *glyphs*, and the ASCII would shimmer;
that reads as noise, not motion. Escape-time density keeps the shape still and
moves only the colour, which is the point of having two channels. Log-compress
it, because a linear map puts almost every exterior sample into one or two ramp
steps and looks flat.

Two smaller ones with the same character. The palette is a **baked LUT** of 512
entries, not a cosine evaluated per sample: three `cos` calls per sample is
~0.5ms at 20,000 samples, which would make the palette *four times* the cost of
the kernel at the home view and the cheapest motion mode the most expensive
thing in the frame. And the cycling **phase must be wrapped** every frame
(`% period`), or after long uptime the increment goes small relative to the
accumulated value, cycling stutters and then stalls — a failure that only
appears after a day of the unattended mode running.

Four more, about structure rather than maths.

**Split `App::update(&mut self)` from a pure `App::draw(&self)`.** Sampling and
blitting go in `update`; `draw` only puts the finished grid into the frame. If
the cost moves into `draw` it needs `&mut self`, the snapshot helper changes
signature, and — the real loss — sample time, blit time and ratatui's buffer
diff collapse into one unmeasurable call. Getting the area before `update`
needs `terminal.autoresize()` then `terminal.get_frame().area()`, **not**
`Terminal::size()`, whose own docs say it reports the backend size without
updating ratatui's viewport bookkeeping — so it disagrees with `Frame::area()`
for one frame after a resize.

**`Cell` grows a `background` field, not a variant.** An enum would branch per
cell inside `Widget::render` and cost `Grid` its uniform `Copy` layout. Nothing
breaks, because every construction already goes through `Cell::new` or
`Cell::default` — there are no struct literals outside the module. One
consequence to make a decision rather than discover: covered cells then carry an
explicit black instead of leaving `Color::Reset`, so frascii paints over a
user's terminal theme. Right for a fractal, but say so in the code.

**The clock accumulates; it is not `now - start`.** Subtracting from a start
instant jumps the animation forward by however long the user was paused. And
**zoom is geometric in `dt`** — `factor.powf(dt.as_secs_f64())`, never
`factor * dt` — which is what makes a dive advance at the same plane-speed on a
slow machine, in fewer chunkier frames, rather than simply going slower. That
matters directly for the unattended mode.

**Half-block needs only `▀`.** `▄` with the colours swapped is pixel-identical,
so one glyph keeps the blit branchless and every cell single-width. Half-block
ignores the glyph ramp entirely — it is a 2× vertical pixel display, not ASCII
art, which is precisely why glyph mode is the default: that is the product's
identity, and half-block is the escape hatch for detail.

Two smaller notes for when the splash goes: **rename the snapshot tests and
`git mv` the `.snap` files together**, because insta derives filenames from test
names and a rename otherwise reads as a delete plus an add — exactly the shape
this file warns against under [Render snapshots](#render-snapshots). Keep all
three sizes; the tiny ones caught the border-overpaint bug and are worth more
against a fractal than a splash, since a symbols-only capture pins the glyph
ramp and the plane↔sample geometry, the two things most likely to shift
silently. And the palettes built from terminal themes must interpolate in
**Oklab or linear-light**, not sRGB — syntax themes put near-complementary
colours adjacent, and their sRGB midpoint is grey mud — with the loop closed
end-to-end, or cycling rotates a visible seam across the screen. Define them as
named colour lists plus an interpolation space; a hand-written 512-entry table
cannot be reviewed and a seam in one is invisible in a diff.

### Already designed

`frascii-core` now holds the kernels (`Fractal`, `Mandelbrot`, `Julia`), the plane↔sample mapping (`Viewport`, with `sample_aspect` as the frontend's only geometric input), the sampler (`sample_into`, rayon across rows), and the search for somewhere worth zooming (`boundary_target`, `interior_fraction`).

`sample_into` is a free function, and was briefly a `Sampler` trait with a `CpuSampler` impl "so a GPU backend could slot in later". **Do not re-add that trait on instinct.** The justification does not survive: a GPU backend is upload → dispatch → readback, with asynchronous completion and a buffer the host does not own in between, and that signature fits none of it — so the seam was shaped wrongly for the only thing it was a seam for, and would have been rewritten when the backend arrived. What actually keeps a second backend possible is that `Viewport`, `Escape` and `SampleGrid` are backend-neutral data naming no host; adding a trait then is a mechanical refactor of one function. Meanwhile the trait costs dynamic dispatch, or a generic parameter that goes viral through every type holding the frontend's `App`. The headless benchmark landed in `crates/frascii/src/headless.rs` — in the *binary*, because writing a file is host-bound I/O that core forbids.

**It is not compile-time proof of the boundary, and an earlier version of this file claimed it was.** Cargo dependencies are per-*package*, and that package depends on `frascii-tui`, so `frascii_tui` is in scope for `headless.rs` and the compiler would accept an import of it. The rule is enforced by a source scan (`the_headless_path_never_reaches_for_the_frontend`), which is the same grade of evidence the render boundary relies on — no more. Genuine compile-time proof is available if it is ever worth having: an example under `frascii-core` sees only that package's dependencies and so literally cannot name the frontend, at the cost of moving the feature out of the CLI. What *is* compile-time enforced is the other direction: core's manifest has no frontend dependency, so core cannot reach upward.

A tension to expect: a headless exporter that wants the *real* palette needs `frascii-tui`, at which point it stops being an independent consumer of core and becomes a second driver of the frontend. That is a reasonable thing to want, but it cannot carry the boundary claim — so keep the timing path greyscale and core-only, and mark any colour path as having given the claim up.

## Coding conventions

- Minimise duplication: extract shared logic into helpers rather than repeating it.
- Use idiomatic Rust: leverage the type system, enums, pattern matching, iterators and `?`; prefer `impl Into<T>` / `AsRef<T>` in signatures where it improves ergonomics.
- Errors use `thiserror` derives with a `Result<T>` alias beside them. **An error type is self-contained and carries no `#[from]` into another frascii crate's.** `TuiError` is the only one today; if `frascii-core` gains one, the absent blanket `#[from]` is what forces someone to decide which side of the boundary a new failure sits on.
- Use `tracing` macros for logging, never `println!`/`eprintln!` except in the binary's own output paths. **In a TUI this is correctness, not style:** anything written to stdout or stderr while the alternate screen is up is painted over the render and scrambles the frame. Hence `--log` taking a file, and logging being off without one.
- **Keep `main.rs` thin.** It wires arguments to library calls and prints errors. `main` cannot be tested without spawning the binary, so logic that lands there stops being covered. `cli_args.rs` is the pattern: the clap tree plus the pure functions it feeds, with their own tests.
- **Keep a clap doc comment to one line.** The exact rule (`clap_derive-4.6.4 src/utils/doc_comments.rs:123-135`): with a blank line in the comment, the first *paragraph* is the short help and the *whole* comment is the long help; with no blank line, every line is merged into one and there is no long help. So a multi-line doc comment either prints in full under `--help` or collapses into a run-on short help. Put rationale in `//` comments. `README.md`'s Usage and Keyboard shortcuts blocks are what these must agree with.
- **Check `KeyEventKind`** — and know that the check is **inert today**, which is the interesting part. A key-up must not re-trigger what its key-down ran. On Unix the non-Press kinds arrive only if the app pushes `REPORT_EVENT_TYPES` (`crossterm-0.29.0 src/event.rs:296-298`), which `ratatui::try_init` does not (`ratatui-0.30.2 src/init.rs:397-403`); the other source is a Windows console (`crossterm-0.29.0 src/event/sys/windows/parse.rs:226,289`), which is out of scope. It stays because it stops being inert the moment we push `REPORT_EVENT_TYPES` to get unambiguous modifier chords — which a pan/zoom UI will want — and `release_and_repeat_events_are_ignored` is what documents that intent. This receipt has been wrong twice — first blaming kitty, then Windows — so check it before citing it.
- **`CARGO_PKG_*` resolves per compiling package.** User-facing name and version therefore belong in the binary; a library reading the same macro reports itself. The splash currently prints the tui crate's version, which is correct only while the workspace shares one — anything user-facing that outlives the placeholder should take the binary's.
- **No re-export shims when moving code.** Rewrite the imports at every call site. A shim means nothing forces callers to acknowledge the new layout, so the old path lingers and the boundary never moves.
- **A comment claiming behaviour of code outside this repo must carry its receipt.** Claims about our own code are cheap to check and reviewers do check them; boundary claims get reasoned from plausibility and then read as fact forever. Cite it inline (as above), name the test that pins it, or phrase it as an assumption. In review a receipt-less boundary claim is **unverified by default** — ask where the receipt is.

### Lints

`[workspace.lints]` in the root manifest is the one place lint decisions are made; every crate inherits it. `unsafe_code` is **forbidden** (not denied — forbidden cannot be locally overridden) and `missing_docs` is warn-level, so with CI's `-D warnings` every public item needs a doc comment. In a binary crate nothing is externally reachable, so items are `pub(crate)`; `unreachable_pub` reports the difference.

**`clippy::pedantic` is deliberately not enabled, and adding it would be a regression.** CI denies warnings, so a group promotes every lint in it to a build failure — and a realtime renderer casts between `f64`, `u32` and `usize` on every cell of every frame, which `cast_precision_loss`, `cast_possible_truncation` and `cast_sign_loss` would flag hundreds of times in exactly the code where the cast is the point. Individual pedantic lints are listed explicitly; add to that list when one earns its keep.

## Performance

The interactive loop is the product, so its cost is a correctness concern rather than an afterthought.

- `[profile.dev]` sets `opt-level = 1` for the workspace and `3` for dependencies. Without it `cargo run` renders a slideshow and the first instinct is to blame the algorithm rather than the profile.
- **Even so, make frame-rate observations under `--release`.** The dev profile keeps debug assertions, and `opt-level = 1` is not what fat LTO produces.

Measured with `frascii --headless` on a Ryzen 9 7940HS (release, 20,000 samples/frame):

| view | limit | mean frame | fps |
|---|---|---|---|
| home | 300 | 110µs | 9,100 |
| Julia, home | 300 | 336µs | 3,000 |
| 1e6 magnification | 2,691 | 5.27ms | 190 |
| 1e10 magnification | 4,286 | 6.59ms | 152 |

So the 30fps budget is met with roughly 5× headroom in the worst case, on the CPU, with no SIMD and no GPU. That answers the question the staging was ordered to answer first, and it is why the `Sampler` trait has exactly one implementation.

**If that headroom is ever spent, the order is not the obvious one.** Two corrections worth keeping, both from review:

- **AVX-512 buys far less than its name suggests here.** The ~575 GFLOP/s figure above already assumes 256-bit AVX2 FMA (8 cores × 4.5 GHz × 2 units × 4 lanes × 2 flops = 576); Zen 4 executes 512-bit instructions on 256-bit datapaths over two cycles, so the wider ISA is not double the throughput. More importantly the escape loop is **latency-bound**, roughly ten cycles per iteration, so it reaches about 5% of peak. Vectorising helps by giving several *independent* dependency chains, not by widening arithmetic — budget 2–2.5× after lane divergence, and note SMT already harvests some of that. `unsafe_code` is forbidden workspace-wide, so a hand-written intrinsic path is off the table regardless; a safe crate such as `wide` would be the route.
- **Periodicity checking outranks SIMD** for the workload that actually matters. At depth the cost is interior points, and the cardioid test covers only the *main* cardioid, not the mini-brot being descended into — where the shortcut catches exactly nothing. Periodicity is plausibly 2–3× on that case. It carries a real risk (too loose an epsilon paints exterior as interior, which looks exactly like a kernel bug), so it needs a false-positive property test before it is trusted.

And one distinction to keep straight: **progressive refinement and dynamic resolution scaling are different tools.** Refinement — draw coarse, sharpen while idle — improves latency after a keypress. Auto-zoom has no idle frames to refine into, so it does nothing there; the tool for a sustained budget miss is dropping sample density.

**A benchmark must report what it rendered.** The first version of `--headless` zoomed straight in on the home centre, which sits *inside* the set, and produced a 100%-interior frame that the cardioid shortcut answers instantly — it claimed 16,198 fps where the real figure at that depth is 190. It now dives via `boundary_target` and prints the interior fraction, warning when a frame is not representative. Any future backend comparison has to keep that, or it will measure the shortcut and call it a speedup.

## Testing

Unit tests are co-located (`#[cfg(test)]`). Use red–green TDD: failing test first. Bug fixes need a regression test too — if the fix seems to live somewhere untestable, push the logic into a library crate rather than skipping the test.

`default-members` is `crates/*`, which is what makes a bare `cargo run` start the TUI — but it also means **a bare `cargo test` runs the product crates only and skips xtask's own tests**, the CI-matrix guard among them. Every verify lane passes `--workspace`, so `cargo verify` covers them; a bare `cargo test` is 29 tests where the lane is 41.

**Tests must not write anywhere outside a `tempfile::TempDir`** (add the dependency when one first needs it). *Reading* the checkout is fine and two tests do it, both via `include_str!` so the read happens at compile time: the CI-matrix guard and `this_module_stays_free_of_ratatui`. A path-shaped *value* that is never touched on disk is fine too — `cli_args.rs` and `runner.rs` each assert on one — but never open a hardcoded path.

### Render snapshots

`crates/frascii-tui/src/render_tests.rs` carries insta snapshots taken through ratatui's `TestBackend`. They catch what targeted assertions miss — a border that stops joining, a line that shifts a column, a frame that loses a row.

**Anything inside a bordered block measures against `block.inner(area)`, not the outer rect** — the splash originally centred in `frame.area()` and painted over the border columns in a narrow terminal, which a targeted text assertion passed straight over.

Snapshots capture **symbols only, not styles** — `TestBackend`'s limitation, not a choice — so colour and emphasis stay the job of targeted assertions (`widget.rs`'s `glyphs_and_colours_reach_the_buffer`).

`cargo insta review` to accept (`cargo install cargo-insta`). Never blind-accept: these fail only when rendering genuinely changed. **If a change invalidates a snapshot, re-point it at the new UI — never delete the file**, because a deleted snapshot leaves nothing to notice when the UI comes back. `*.snap.new` is gitignored; a committed one means a diff was never looked at.

### Covering a frontend

Terminal code is testable if the testable half is kept pure:

- **Key handling is a pure function.** `App::handle_key` returns a `Flow` and touches no terminal, so its bindings are unit-tested directly. A test that had to spawn a terminal would not get written. Every new binding gains a case.
- **Terminal lifecycle is separate from the loop.** `App::run` takes a terminal it does not own; `frascii_tui::run` does init and restore. A `run` that did both could not be driven by a test backend, and one that restored only on the happy path would leave a raw-mode terminal behind on the first error.

`Flow` is an enum rather than a `bool` so pause, reset and fractal-switch are additions rather than a second flag to keep consistent with the first.

For the terminal path no unit test reaches — raw mode, the alternate screen, the restore — the check is a scripted PTY run: spawn the binary on a pty with a real window size, confirm `?1049h`, that it paints, exit 0 on `q`, and `?1049l`. Worth doing after any change to `frascii_tui::run` or the event loop.

## Documentation

- **`README.md`** — what frascii is, how to run it, the keybinding table. Update the table in the same commit as a keybinding change.
- **`CONTRIBUTING.md`** — setup and the change workflow.
- **Crate module docs** — each crate's `lib.rs` states its charter and boundary rule. Those are the primary statement of the layering; this file summarises them.
- **`CLAUDE.md`** — update when a convention, lane or boundary rule changes. API details belong in rustdoc.

## Git conventions

**CRITICAL: Never force push under any circumstances. This includes `--force`, `--force-with-lease`, and amending commits that have been pushed. Always create new commits instead.**

- Branch names are lowercase with hyphens, no slashes, e.g. `add-julia-kernel`
- Never skip GPG commit signing
- Pre-commit hooks may autoformat while failing the commit; restage and reattempt
- Before committing, ensure `cargo verify` passes
- Run `pre-commit install` once per clone
