//! Cursor rendering (block, beam, underline styles).

use std::io::{self, Write};
use acos_mux_term::CursorShape;

/// Cursor style enum for mapping CursorShape to ANSI escape sequences.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetCursorStyle {
    SteadyBlock,
    SteadyBar,
    SteadyUnderScore,
}

/// Get cursor style for comparison.
pub fn cursor_style(shape: CursorShape) -> SetCursorStyle {
    match shape {
        CursorShape::Block => SetCursorStyle::SteadyBlock,
        CursorShape::Bar => SetCursorStyle::SteadyBar,
        CursorShape::Underline => SetCursorStyle::SteadyUnderScore,
    }
}

/// Write cursor style escape sequence to the given writer.
pub fn write_cursor_style<W: Write>(w: &mut W, shape: CursorShape) -> io::Result<()> {
    let seq = match shape {
        CursorShape::Block => "\x1b[2 q",      // Steady block
        CursorShape::Underline => "\x1b[4 q",  // Steady underline
        CursorShape::Bar => "\x1b[6 q",        // Steady bar
    };
    write!(w, "{}", seq)
}
