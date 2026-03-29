//! Text rendering: cell-to-terminal-output conversion.

use acos_mux_term::Color;
use acos_mux_term::grid::{Cell, UnderlineStyle};

/// Style information for a cell (collected output without actual write).
#[derive(Clone, PartialEq, Eq, Default)]
pub struct CellStyle {
    pub fg: Option<Color>,
    pub bg: Option<Color>,
    pub bold: bool,
    pub italic: bool,
    pub underline: u8,  // 0=none, 1=single, 2=double, 3=curly
    pub blink: bool,
    pub reverse: bool,
    pub invisible: bool,
    pub strikethrough: bool,
}

/// Convert an `acos_mux_term::Color` with None/Some mapping.
pub fn color_to_termion(color: &Color) -> Option<Color> {
    match color {
        Color::Default => None,
        c => Some(c.clone()),
    }
}


/// Build a `CellStyle` from a cell's attributes and colors.
pub fn cell_style(cell: &Cell) -> CellStyle {
    let mut style = CellStyle::default();
    style.fg = color_to_termion(&cell.fg);
    style.bg = color_to_termion(&cell.bg);

    if cell.attrs.bold {
        style.bold = true;
    }
    if cell.attrs.italic {
        style.italic = true;
    }
    style.underline = match cell.attrs.underline {
        UnderlineStyle::None => 0,
        UnderlineStyle::Single => 1,
        UnderlineStyle::Double => 2,
        UnderlineStyle::Curly => 3,
    };
    if cell.attrs.blink {
        style.blink = true;
    }
    if cell.attrs.reverse {
        style.reverse = true;
    }
    if cell.attrs.invisible {
        style.invisible = true;
    }
    if cell.attrs.strikethrough {
        style.strikethrough = true;
    }

    style
}

/// Convert a row of cells into a sequence of styled text spans.
///
/// Adjacent cells with the same style are coalesced into a single span.
/// Wide-char continuation cells (width == 0) are skipped.  The output
/// is padded with spaces to exactly `width` columns.
pub fn render_row(cells: &[Cell], width: usize) -> Vec<(CellStyle, String)> {
    let mut spans: Vec<(CellStyle, String)> = Vec::new();
    let mut col = 0;

    for cell in cells.iter().take(width) {
        // Skip continuation cells for wide characters
        if cell.width == 0 {
            col += 1;
            continue;
        }

        let style = cell_style(cell);
        let ch = if cell.c < ' ' { ' ' } else { cell.c };

        if let Some(last) = spans.last_mut() {
            if last.0 == style {
                last.1.push(ch);
            } else {
                spans.push((style, ch.to_string()));
            }
        } else {
            spans.push((style, ch.to_string()));
        }

        col += cell.width as usize;
    }

    // Pad to the full width if needed
    while col < width {
        let style = CellStyle::default();
        if let Some(last) = spans.last_mut() {
            if last.0 == style {
                last.1.push(' ');
            } else {
                spans.push((style, " ".to_string()));
            }
        } else {
            spans.push((style, " ".to_string()));
        }
        col += 1;
    }

    spans
}
