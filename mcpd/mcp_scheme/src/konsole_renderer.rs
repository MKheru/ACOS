use crate::konsole_handler::{Konsole, Color, Cell};

pub fn render_konsole_ansi(konsole: &Konsole) -> String {
    let mut out = String::new();

    // 1. Clear screen + cursor home
    out.push_str("\x1b[2J\x1b[H");

    // 2. Draw top border with konsole info
    let title = format!(" Konsole {} [{}] — {} ", konsole.id,
        format!("{:?}", konsole.konsole_type), konsole.owner);
    out.push_str("\x1b[1;36m");  // bold cyan
    out.push_str("┌");
    out.push_str(&"─".repeat(konsole.cols as usize));
    out.push_str("┐\r\n");
    // Center title in border
    let pad = (konsole.cols as usize).saturating_sub(title.len()) / 2;
    out.push_str("│");
    out.push_str(&" ".repeat(pad));
    out.push_str(&title);
    out.push_str(&" ".repeat(konsole.cols as usize - pad - title.len()));
    out.push_str("│\r\n");
    out.push_str("├");
    out.push_str(&"─".repeat(konsole.cols as usize));
    out.push_str("┤\r\n");
    out.push_str("\x1b[0m");

    // 3. Render each buffer line with ANSI colors
    for row in &konsole.buffer {
        out.push_str("│");
        let mut current_fg = Color::Default;
        let mut current_bg = Color::Default;
        let mut current_bold = false;

        for cell in row.iter().take(konsole.cols as usize) {
            // Emit SGR if attributes changed
            if cell.fg != current_fg || cell.bg != current_bg || cell.bold != current_bold {
                out.push_str("\x1b[0");  // reset first
                if cell.bold { out.push_str(";1"); }
                push_fg_code(&mut out, cell.fg);
                push_bg_code(&mut out, cell.bg);
                out.push('m');
                current_fg = cell.fg;
                current_bg = cell.bg;
                current_bold = cell.bold;
            }
            out.push(cell.ch);
        }
        out.push_str("\x1b[0m│\r\n");
    }

    // 4. Bottom border
    out.push_str("\x1b[1;36m└");
    out.push_str(&"─".repeat(konsole.cols as usize));
    out.push_str("┘\x1b[0m\r\n");

    // 5. Status bar
    out.push_str(&format!("\x1b[90m cursor: ({},{}) | scrollback: {} lines\x1b[0m\r\n",
        konsole.cursor_row, konsole.cursor_col, konsole.scrollback.len()));

    out
}

fn push_fg_code(out: &mut String, color: Color) {
    let code = match color {
        Color::Black => ";30", Color::Red => ";31", Color::Green => ";32",
        Color::Yellow => ";33", Color::Blue => ";34", Color::Magenta => ";35",
        Color::Cyan => ";36", Color::White => ";37",
        Color::BrightBlack => ";90", Color::BrightRed => ";91", Color::BrightGreen => ";92",
        Color::BrightYellow => ";93", Color::BrightBlue => ";94", Color::BrightMagenta => ";95",
        Color::BrightCyan => ";96", Color::BrightWhite => ";97",
        Color::Default => ";39",
    };
    out.push_str(code);
}

fn push_bg_code(out: &mut String, color: Color) {
    let code = match color {
        Color::Black => ";40", Color::Red => ";41", Color::Green => ";42",
        Color::Yellow => ";43", Color::Blue => ";44", Color::Magenta => ";45",
        Color::Cyan => ";46", Color::White => ";47",
        Color::BrightBlack => ";100", Color::BrightRed => ";101", Color::BrightGreen => ";102",
        Color::BrightYellow => ";103", Color::BrightBlue => ";104", Color::BrightMagenta => ";105",
        Color::BrightCyan => ";106", Color::BrightWhite => ";107",
        Color::Default => ";49",
    };
    out.push_str(code);
}
