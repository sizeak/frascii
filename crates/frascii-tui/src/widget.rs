//! The ratatui half of rendering: blitting a [`Grid`] into a buffer.
//!
//! Kept apart from [`crate::render`], which holds the same pipeline's
//! ratatui-free data types. The split is a file boundary rather than a crate
//! one, so it costs nothing and keeps the extraction path open if a second
//! frontend ever needs the grid.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::widgets::Widget;

use crate::render::{Grid, Rgb};

/// Blits the grid into a ratatui [`Buffer`].
///
/// Implemented for `&Grid` rather than `Grid`: `Widget::render` takes `self` by
/// value, and a frame should not have to hand over ownership of a grid it will
/// redraw next tick.
impl Widget for &Grid {
    /// Cells outside `area` are dropped and cells outside the grid are left
    /// untouched, so a grid and a viewport that disagree about size render the
    /// overlap rather than panicking. They *do* disagree routinely: a terminal
    /// resize is observed a frame before the grid is re-rendered to match.
    fn render(self, area: Rect, buf: &mut Buffer) {
        // Saturating rather than wrapping: a grid wider than 65535 cells is not
        // a real terminal, but truncating the count would draw a narrow strip
        // and look like a render bug rather than an impossible size.
        let grid_cols = u16::try_from(self.width()).unwrap_or(u16::MAX);
        let grid_rows = u16::try_from(self.height()).unwrap_or(u16::MAX);
        let cols = area.width.min(grid_cols);
        let rows = area.height.min(grid_rows);

        for y in 0..rows {
            for x in 0..cols {
                let Some(cell) = self.get(usize::from(x), usize::from(y)) else {
                    continue;
                };
                if let Some(target) = buf.cell_mut((area.x + x, area.y + y)) {
                    target
                        .set_char(cell.glyph)
                        .set_fg(to_colour(cell.colour))
                        .set_bg(to_colour(cell.background));
                }
            }
        }
    }
}

/// Convert a render colour into a ratatui one.
///
/// Always truecolour: quantising to the 256-colour cube is a decision about the
/// *terminal's* capability, and this function has no way to know what the
/// terminal supports. When frascii grows capability detection it belongs
/// upstream of here, choosing which colour the renderer produces — not buried
/// in a per-cell conversion that runs tens of thousands of times a frame.
const fn to_colour(colour: Rgb) -> Color {
    Color::Rgb(colour.r, colour.g, colour.b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::Cell;

    fn buffer_text(buf: &Buffer) -> String {
        (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .filter_map(|x| buf.cell((x, y)).map(|c| c.symbol().to_owned()))
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn glyphs_and_colours_reach_the_buffer() {
        let mut grid = Grid::new(2, 1);
        grid.set(0, 0, Cell::new('a', Rgb::new(1, 2, 3)));
        grid.set(1, 0, Cell::new('b', Rgb::new(4, 5, 6)));

        let area = Rect::new(0, 0, 2, 1);
        let mut buf = Buffer::empty(area);
        (&grid).render(area, &mut buf);

        assert_eq!(buffer_text(&buf), "ab");
        assert_eq!(
            buf.cell((0, 0)).expect("cell in bounds").fg,
            Color::Rgb(1, 2, 3)
        );
    }

    #[test]
    fn a_grid_larger_than_its_area_is_clipped() {
        let mut grid = Grid::new(4, 4);
        for y in 0..4 {
            for x in 0..4 {
                grid.set(x, y, Cell::new('#', Rgb::BLACK));
            }
        }

        let area = Rect::new(0, 0, 2, 2);
        let mut buf = Buffer::empty(area);
        (&grid).render(area, &mut buf);

        assert_eq!(buffer_text(&buf), "##\n##");
    }

    #[test]
    fn a_grid_smaller_than_its_area_leaves_the_rest_untouched() {
        let mut grid = Grid::new(1, 1);
        grid.set(0, 0, Cell::new('x', Rgb::BLACK));

        let area = Rect::new(0, 0, 3, 1);
        let mut buf = Buffer::empty(area);
        (&grid).render(area, &mut buf);

        assert_eq!(buffer_text(&buf), "x  ");
    }

    #[test]
    fn rendering_is_offset_by_the_areas_origin() {
        let mut grid = Grid::new(1, 1);
        grid.set(0, 0, Cell::new('z', Rgb::BLACK));

        let mut buf = Buffer::empty(Rect::new(0, 0, 3, 2));
        (&grid).render(Rect::new(1, 1, 1, 1), &mut buf);

        assert_eq!(buffer_text(&buf), "   \n z ");
    }
}
