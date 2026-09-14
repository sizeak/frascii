//! The rendered output: a grid of coloured glyphs.
//!
//! Presentation lives in this crate rather than a shared one, which is a
//! deliberate simplification and not a claim that rendering is inherently
//! terminal-specific. What it buys is one fewer crate boundary to maintain
//! while the pipeline is still being designed.
//!
//! The cost is known and bounded: if a second frontend arrives, *this module*
//! plus the ramps and palettes that will join it are what move out into a
//! shared render crate, and the module keeps no ratatui dependency precisely so
//! that stays a file move. The ratatui half is next door in
//! [`crate::widget`].

/// A 24-bit colour.
///
/// Truecolour is the target rather than a 256-colour cube: the whole point of a
/// continuous palette is that adjacent bands differ by less than a step of the
/// cube, and terminals that cannot do 24-bit are better served by quantising a
/// true colour at the last moment than by computing a coarse one here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rgb {
    /// Red channel.
    pub r: u8,
    /// Green channel.
    pub g: u8,
    /// Blue channel.
    pub b: u8,
}

impl Rgb {
    /// Black.
    pub const BLACK: Self = Self::new(0, 0, 0);

    /// A colour from its three channels.
    #[must_use]
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }
}

/// One character cell of rendered output: a glyph, its colour, and what sits
/// behind it.
///
/// A struct with three fields rather than an enum over render modes. An enum
/// would mean a branch per cell inside the ratatui blit and would cost [`Grid`]
/// its uniform `Copy` layout, for a distinction the blit can make once per
/// frame instead of once per cell.
///
/// Note that a rendered frame sets `background` explicitly on every cell, so
/// frascii paints over the user's terminal theme rather than letting it show
/// through. That is deliberate — a fractal with the user's background bleeding
/// into the interior would be a different picture on every terminal — but it is
/// a choice, not an accident.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    /// The glyph to draw.
    pub glyph: char,
    /// The glyph's foreground colour.
    pub colour: Rgb,
    /// What sits behind the glyph.
    pub background: Rgb,
}

impl Cell {
    /// A cell with the given glyph and colour, on black.
    #[must_use]
    pub const fn new(glyph: char, colour: Rgb) -> Self {
        Self {
            glyph,
            colour,
            background: Rgb::BLACK,
        }
    }

    /// A cell with an explicit background.
    ///
    /// What half-block mode will use: one glyph, two independently coloured
    /// halves.
    #[must_use]
    pub const fn with_background(glyph: char, colour: Rgb, background: Rgb) -> Self {
        Self {
            glyph,
            colour,
            background,
        }
    }
}

impl Default for Cell {
    /// A blank cell — a space, black on black.
    fn default() -> Self {
        Self::new(' ', Rgb::BLACK)
    }
}

/// A rectangular grid of rendered cells: the renderer's whole output.
///
/// Row-major and densely stored, so a frontend can blit a row at a time and a
/// resize is a single reallocation rather than a per-row one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grid {
    width: usize,
    height: usize,
    cells: Vec<Cell>,
}

impl Grid {
    /// A grid of blank cells.
    ///
    /// A zero width or height is legal and yields an empty grid: a terminal can
    /// genuinely report one mid-resize, and a renderer that panicked on it
    /// would turn a window drag into a crash.
    #[must_use]
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            cells: vec![Cell::default(); width * height],
        }
    }

    /// Cells across.
    #[must_use]
    pub const fn width(&self) -> usize {
        self.width
    }

    /// Cells down.
    #[must_use]
    pub const fn height(&self) -> usize {
        self.height
    }

    /// The cell at `(x, y)`, or `None` when the coordinate is outside the grid.
    #[must_use]
    pub fn get(&self, x: usize, y: usize) -> Option<Cell> {
        self.index(x, y).map(|i| self.cells[i])
    }

    /// Overwrite the cell at `(x, y)`.
    ///
    /// Out-of-bounds writes are dropped rather than panicking, for the same
    /// reason [`Grid::new`] accepts a zero dimension: a frontend mid-resize is
    /// briefly wrong about its own size, and losing a cell is preferable to
    /// taking down the render loop.
    pub fn set(&mut self, x: usize, y: usize, cell: Cell) {
        if let Some(i) = self.index(x, y) {
            self.cells[i] = cell;
        }
    }

    /// Every cell in row-major order.
    pub fn cells(&self) -> impl Iterator<Item = Cell> + '_ {
        self.cells.iter().copied()
    }

    /// One row of cells, or `None` when `y` is outside the grid.
    #[must_use]
    pub fn row(&self, y: usize) -> Option<&[Cell]> {
        if y >= self.height {
            return None;
        }
        let start = y * self.width;
        Some(&self.cells[start..start + self.width])
    }

    /// Resize in place, blanking every cell.
    ///
    /// Contents are not preserved: the caller re-renders after a resize anyway,
    /// because the viewport it maps from has changed shape.
    pub fn resize(&mut self, width: usize, height: usize) {
        self.width = width;
        self.height = height;
        self.cells.clear();
        self.cells.resize(width * height, Cell::default());
    }

    fn index(&self, x: usize, y: usize) -> Option<usize> {
        (x < self.width && y < self.height).then(|| y * self.width + x)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// This module's own source, read at compile time.
    const SOURCE: &str = include_str!("render.rs");

    #[test]
    fn this_module_stays_free_of_ratatui() {
        // The rule that makes extracting this module for a second frontend a
        // file move rather than a rewrite, and the only thing enforcing it.
        // Stated in the crate docs, the manifest and CLAUDE.md; asserted here.
        //
        // Code lines only, up to the test module: the prose above legitimately
        // discusses ratatui, and this test necessarily names it.
        let offenders: Vec<String> = SOURCE
            .lines()
            .enumerate()
            .take_while(|(_, line)| !line.trim_start().starts_with("#[cfg(test)]"))
            .filter(|(_, line)| !line.trim_start().starts_with("//"))
            // `crate::widget` too: importing it would be an *indirect*
            // ratatui dependency, which a search for the literal string
            // would not catch.
            .filter(|(_, line)| line.contains("ratatui") || line.contains("crate::widget"))
            .map(|(n, line)| format!("line {}: {}", n + 1, line.trim()))
            .collect();

        assert!(
            offenders.is_empty(),
            "render.rs must not depend on ratatui, found:\n{}",
            offenders.join("\n")
        );
    }

    #[test]
    fn new_grid_is_blank_at_the_requested_size() {
        let grid = Grid::new(3, 2);
        assert_eq!((grid.width(), grid.height()), (3, 2));
        assert_eq!(grid.cells().count(), 6);
        assert!(grid.cells().all(|c| c == Cell::default()));
    }

    #[test]
    fn a_zero_dimension_yields_an_empty_grid_rather_than_panicking() {
        let grid = Grid::new(0, 40);
        assert_eq!(grid.cells().count(), 0);
        assert_eq!(grid.get(0, 0), None);
        // A zero-width grid still has `height` rows; each is empty. `row`
        // bounds on `y` alone, so an empty slice here and `None` past the last
        // row are different answers to different questions.
        assert_eq!(grid.row(0), Some(&[][..]));
        assert_eq!(grid.row(40), None);
    }

    #[test]
    fn set_and_get_round_trip_within_bounds() {
        let mut grid = Grid::new(2, 2);
        let cell = Cell::new('*', Rgb::new(1, 2, 3));
        grid.set(1, 1, cell);
        assert_eq!(grid.get(1, 1), Some(cell));
        assert_eq!(grid.get(0, 0), Some(Cell::default()));
    }

    #[test]
    fn out_of_bounds_access_is_dropped_not_panicked() {
        let mut grid = Grid::new(2, 2);
        grid.set(9, 9, Cell::new('!', Rgb::BLACK));
        assert_eq!(grid.get(9, 9), None);
        assert_eq!(grid.get(2, 0), None);
    }

    #[test]
    fn rows_are_row_major_slices() {
        let mut grid = Grid::new(2, 2);
        grid.set(0, 1, Cell::new('a', Rgb::BLACK));
        grid.set(1, 1, Cell::new('b', Rgb::BLACK));
        let row = grid.row(1).expect("row 1 is in bounds");
        assert_eq!(row.iter().map(|c| c.glyph).collect::<String>(), "ab");
    }

    #[test]
    fn resize_blanks_every_cell() {
        let mut grid = Grid::new(2, 2);
        grid.set(0, 0, Cell::new('x', Rgb::new(9, 9, 9)));
        grid.resize(4, 1);
        assert_eq!((grid.width(), grid.height()), (4, 1));
        assert!(grid.cells().all(|c| c == Cell::default()));
    }
}
