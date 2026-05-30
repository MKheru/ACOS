use crate::konsole_handler::{Konsole, Color, Cell};

struct ThemeColors {
    bg: String,
    fg: String,
    accent: String,
    primary: String,
    secondary: String,
}

fn load_theme_colors() -> ThemeColors {
    let mut bg = "#0a0a0c".to_string();
    let mut fg = "#f8f8f2".to_string();
    let mut accent = "#ff79c6".to_string();
    let mut primary = "#bd93f9".to_string();
    let mut secondary = "#8be9fd".to_string();

    if let Ok(content) = std::fs::read_to_string("/etc/acos/theme.toml") {
        for line in content.lines() {
            let line = line.trim();
            if line.starts_with("bg = ") {
                if let Some(val) = line.split('"').nth(1) { bg = val.to_string(); }
            } else if line.starts_with("fg = ") {
                if let Some(val) = line.split('"').nth(1) { fg = val.to_string(); }
            } else if line.starts_with("accent = ") {
                if let Some(val) = line.split('"').nth(1) { accent = val.to_string(); }
            } else if line.starts_with("primary = ") {
                if let Some(val) = line.split('"').nth(1) { primary = val.to_string(); }
            } else if line.starts_with("secondary = ") {
                if let Some(val) = line.split('"').nth(1) { secondary = val.to_string(); }
            }
        }
    }

    ThemeColors { bg, fg, accent, primary, secondary }
}

fn hex_to_rgb(hex: &str) -> Option<(u8, u8, u8)> {
    let hex = hex.trim_start_matches('#');
    if hex.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some((r, g, b))
}

pub fn render_konsole_ansi(konsole: &Konsole) -> String {
    let mut out = String::new();
    let theme = load_theme_colors();

    // 1. Clear screen + cursor home
    out.push_str("\x1b[2J\x1b[H");

    // 2. Get theme-based border color
    let border_fg = if let Some((r, g, b)) = hex_to_rgb(&theme.accent) {
        format!("\x1b[1;38;2;{};{};{}m", r, g, b)
    } else {
        "\x1b[1;36m".to_string()
    };

    // 3. Draw top border with konsole info
    let title = format!(" Konsole {} [{}] — {} ", konsole.id,
        format!("{:?}", konsole.konsole_type), konsole.owner);
    out.push_str(&border_fg);
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

    // 4. Render each buffer line with ANSI colors
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
                push_fg_code(&mut out, cell.fg, &theme);
                push_bg_code(&mut out, cell.bg, &theme);
                out.push('m');
                current_fg = cell.fg;
                current_bg = cell.bg;
                current_bold = cell.bold;
            }
            out.push(cell.ch);
        }
        out.push_str("\x1b[0m│\r\n");
    }

    // 5. Bottom border
    out.push_str(&border_fg);
    out.push_str("└");
    out.push_str(&"─".repeat(konsole.cols as usize));
    out.push_str("┘\x1b[0m\r\n");

    // 6. Status bar
    out.push_str(&format!("\x1b[90m cursor: ({},{}) | scrollback: {} lines\x1b[0m\r\n",
        konsole.cursor_row, konsole.cursor_col, konsole.scrollback.len()));

    out
}

fn push_fg_code(out: &mut String, color: Color, theme: &ThemeColors) {
    let hex = match color {
        Color::Default => &theme.fg,
        Color::White | Color::BrightWhite => &theme.fg,
        Color::Black => &theme.bg,
        Color::Cyan | Color::BrightCyan => &theme.secondary,
        Color::Magenta | Color::BrightMagenta => &theme.accent,
        Color::Blue | Color::BrightBlue => &theme.primary,
        _ => {
            let code = match color {
                Color::Red => ";31", Color::Green => ";32", Color::Yellow => ";33",
                Color::BrightBlack => ";90", Color::BrightRed => ";91", Color::BrightGreen => ";92",
                Color::BrightYellow => ";93",
                _ => ";39",
            };
            out.push_str(code);
            return;
        }
    };

    if let Some((r, g, b)) = hex_to_rgb(hex) {
        out.push_str(&format!(";38;2;{};{};{}", r, g, b));
    } else {
        out.push_str(";39");
    }
}

fn push_bg_code(out: &mut String, color: Color, theme: &ThemeColors) {
    let hex = match color {
        Color::Default => &theme.bg,
        Color::Black => &theme.bg,
        Color::White | Color::BrightWhite => &theme.fg,
        Color::Cyan | Color::BrightCyan => &theme.secondary,
        Color::Magenta | Color::BrightMagenta => &theme.accent,
        Color::Blue | Color::BrightBlue => &theme.primary,
        _ => {
            let code = match color {
                Color::Red => ";41", Color::Green => ";42", Color::Yellow => ";43",
                Color::BrightBlack => ";100", Color::BrightRed => ";101", Color::BrightGreen => ";102",
                Color::BrightYellow => ";103",
                _ => ";49",
            };
            out.push_str(code);
            return;
        }
    };

    if let Some((r, g, b)) = hex_to_rgb(hex) {
        out.push_str(&format!(";48;2;{};{};{}", r, g, b));
    } else {
        out.push_str(";49");
    }
}
