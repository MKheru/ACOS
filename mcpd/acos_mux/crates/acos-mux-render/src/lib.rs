//! Rendering primitives: text output, cursor drawing, and damage tracking.

pub mod cursor;
pub mod damage;
pub mod statusbar;
pub mod text;

use std::io::{self, Write};

use acos_mux_term::{Screen, Color};

use crate::cursor::write_cursor_style;
use crate::damage::DamageTracker;
use crate::text::{render_row, CellStyle};

/// Terminal renderer with damage tracking for efficient redraws.
pub struct Renderer {
    damage: DamageTracker,
    last_cols: usize,
    last_rows: usize,
}

impl Renderer {
    /// Create a new renderer for a terminal of the given size.
    pub fn new(cols: usize, rows: usize) -> Self {
        Self {
            damage: DamageTracker::new(rows),
            last_cols: cols,
            last_rows: rows,
        }
    }

    /// Render the screen to the given writer, only updating dirty rows.
    pub fn render<W: Write>(&mut self, writer: &mut W, screen: &Screen) -> io::Result<()> {
        let cols = screen.cols();
        let rows = screen.rows();

        // Detect size changes
        if cols != self.last_cols || rows != self.last_rows {
            self.resize(cols, rows);
        }

        if !self.damage.needs_redraw() {
            return Ok(());
        }

        // Hide cursor during rendering
        write!(writer, "\x1b[?25l")?;

        let dirty = self.damage.dirty_rows();
        for row in dirty {
            if row >= rows {
                continue;
            }

            // Move to start of this row (1-based in ANSI)
            write!(writer, "\x1b[{};{}H", row as u16 + 1, 1)?;

            // Get the row cells from the grid
            let grid_row = screen.grid.row(row);
            let spans = render_row(&grid_row.cells, cols);

            for (cell_style, text) in spans {
                Self::write_cell_style(writer, &cell_style)?;
                write!(writer, "{}", text)?;
            }
        }

        // Reset all attributes
        write!(writer, "\x1b[0m")?;

        // Position cursor (1-based in ANSI)
        let cursor_obj = &screen.cursor;
        write!(writer, "\x1b[{};{}H", cursor_obj.row as u16 + 1, cursor_obj.col as u16 + 1)?;

        // Show/hide cursor and set shape
        if cursor_obj.visible {
            write!(writer, "\x1b[?25h")?;
            write_cursor_style(writer, cursor_obj.shape)?;
        } else {
            write!(writer, "\x1b[?25l")?;
        }

        writer.flush()?;
        self.damage.clear();

        Ok(())
    }

    /// Write cell style to output.
    fn write_cell_style<W: Write>(w: &mut W, style: &CellStyle) -> io::Result<()> {
        // Reset all attributes first
        write!(w, "\x1b[0m")?;

        // Foreground color
        if let Some(fg) = &style.fg {
            match fg {
                Color::Default => {
                    write!(w, "\x1b[39m")?;
                }
                Color::Indexed(idx) => {
                    write!(w, "\x1b[38;5;{}m", idx)?;
                }
                Color::Rgb(r, g, b) => {
                    write!(w, "\x1b[38;2;{};{};{}m", r, g, b)?;
                }
            }
        }

        // Background color
        if let Some(bg) = &style.bg {
            match bg {
                Color::Default => {
                    write!(w, "\x1b[49m")?;
                }
                Color::Indexed(idx) => {
                    write!(w, "\x1b[48;5;{}m", idx)?;
                }
                Color::Rgb(r, g, b) => {
                    write!(w, "\x1b[48;2;{};{};{}m", r, g, b)?;
                }
            }
        }

        // Text attributes
        if style.bold {
            write!(w, "\x1b[1m")?;
        }
        if style.italic {
            write!(w, "\x1b[3m")?;
        }
        match style.underline {
            1 => write!(w, "\x1b[4m")?,   // Single underline
            2 => write!(w, "\x1b[21m")?,  // Double underline
            3 => write!(w, "\x1b[4:3m")?, // Curly underline
            _ => {}
        }
        if style.blink {
            write!(w, "\x1b[5m")?;
        }
        if style.reverse {
            write!(w, "\x1b[7m")?;
        }
        if style.invisible {
            write!(w, "\x1b[8m")?;
        }
        if style.strikethrough {
            write!(w, "\x1b[9m")?;
        }

        Ok(())
    }

    /// Force a full redraw on the next render call.
    pub fn force_redraw(&mut self) {
        self.damage.mark_all();
    }

    /// Resize the renderer to match new terminal dimensions.
    pub fn resize(&mut self, cols: usize, rows: usize) {
        self.last_cols = cols;
        self.last_rows = rows;
        self.damage.resize(rows);
    }
}
