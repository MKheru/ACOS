//! Konsole service handler — virtual multi-console system with ANSI parsing

use std::cell::RefCell;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

use crate::handler::ServiceHandler;
use crate::protocol::{JsonRpcRequest, JsonRpcResponse, INVALID_PARAMS, METHOD_NOT_FOUND};
use crate::McpPath;

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Color {
    Black,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    White,
    BrightBlack,
    BrightRed,
    BrightGreen,
    BrightYellow,
    BrightBlue,
    BrightMagenta,
    BrightCyan,
    BrightWhite,
    Default,
}

impl Color {
    fn from_sgr(code: u32) -> Option<Color> {
        match code {
            30 => Some(Color::Black),
            31 => Some(Color::Red),
            32 => Some(Color::Green),
            33 => Some(Color::Yellow),
            34 => Some(Color::Blue),
            35 => Some(Color::Magenta),
            36 => Some(Color::Cyan),
            37 => Some(Color::White),
            39 => Some(Color::Default),
            40 => Some(Color::Black),
            41 => Some(Color::Red),
            42 => Some(Color::Green),
            43 => Some(Color::Yellow),
            44 => Some(Color::Blue),
            45 => Some(Color::Magenta),
            46 => Some(Color::Cyan),
            47 => Some(Color::White),
            49 => Some(Color::Default),
            90 => Some(Color::BrightBlack),
            91 => Some(Color::BrightRed),
            92 => Some(Color::BrightGreen),
            93 => Some(Color::BrightYellow),
            94 => Some(Color::BrightBlue),
            95 => Some(Color::BrightMagenta),
            96 => Some(Color::BrightCyan),
            97 => Some(Color::BrightWhite),
            100 => Some(Color::BrightBlack),
            101 => Some(Color::BrightRed),
            102 => Some(Color::BrightGreen),
            103 => Some(Color::BrightYellow),
            104 => Some(Color::BrightBlue),
            105 => Some(Color::BrightMagenta),
            106 => Some(Color::BrightCyan),
            107 => Some(Color::BrightWhite),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Cell {
    pub ch: char,
    pub fg: Color,
    pub bg: Color,
    pub bold: bool,
}

impl Default for Cell {
    fn default() -> Self {
        Cell {
            ch: ' ',
            fg: Color::Default,
            bg: Color::Default,
            bold: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum KonsoleType {
    RootAi,
    User,
    Agent,
    Service,
}

impl KonsoleType {
    fn as_str(&self) -> &str {
        match self {
            KonsoleType::RootAi => "root_ai",
            KonsoleType::User => "user",
            KonsoleType::Agent => "agent",
            KonsoleType::Service => "service",
        }
    }

    fn from_str(s: &str) -> Option<KonsoleType> {
        match s {
            "root_ai" => Some(KonsoleType::RootAi),
            "user" => Some(KonsoleType::User),
            "agent" => Some(KonsoleType::Agent),
            "service" => Some(KonsoleType::Service),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Limits
// ---------------------------------------------------------------------------

const MAX_KONSOLES: usize = 32;
const MAX_COLS: u32 = 512;
const MAX_ROWS: u32 = 256;
const MAX_ANSI_BUF: usize = 64;

// ---------------------------------------------------------------------------
// ANSI parser state
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
enum AnsiState {
    Normal,
    Escape,
    Csi,
    Osc,
}

// ---------------------------------------------------------------------------
// Fast byte-level substring search (avoids Pattern trait dispatch overhead).
// ---------------------------------------------------------------------------

#[inline(always)]
fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() {
        return true;
    }
    let n = needle.len();
    if n > haystack.len() {
        return false;
    }
    let first = needle[0];
    let mut i = 0;
    while i + n <= haystack.len() {
        if haystack[i] == first {
            if &haystack[i..i + n] == needle {
                return true;
            }
        }
        i += 1;
    }
    false
}

#[inline(always)]
fn cells_contain_pattern(line: &[Cell], pattern: &[char]) -> bool {
    let n = pattern.len();
    if n > line.len() {
        return false;
    }
    let first = pattern[0];
    let mut i = 0;
    while i + n <= line.len() {
        if line[i].ch == first {
            if pattern.iter().enumerate().all(|(j, &c)| line[i + j].ch == c) {
                return true;
            }
        }
        i += 1;
    }
    false
}

// ---------------------------------------------------------------------------
// Konsole
// ---------------------------------------------------------------------------

pub struct Konsole {
    pub id: u32,
    pub konsole_type: KonsoleType,
    pub owner: String,
    pub cols: u32,
    pub rows: u32,
    pub buffer: Vec<Vec<Cell>>,
    pub scrollback: VecDeque<Vec<Cell>>,
    pub scrollback_text: VecDeque<String>,
    /// Flat byte buffer: all scrollback line texts concatenated (no separator).
    /// Each line's position is tracked by scrollback_line_spans.
    pub scrollback_flat: Vec<u8>,
    /// (start, len) of each line in scrollback_flat, using absolute byte offsets.
    pub scrollback_line_spans: VecDeque<(u32, u32)>,
    /// How many bytes at the front of scrollback_flat have been evicted.
    pub scrollback_flat_base: usize,
    pub scrollback_limit: usize,
    /// Per-row string cache for the screen buffer. None means dirty (needs rebuild).
    pub buffer_cache: Vec<Option<String>>,
    pub cursor_row: u32,
    pub cursor_col: u32,
    pub dirty: bool,
    pub current_fg: Color,
    pub current_bg: Color,
    pub current_bold: bool,
    ansi_state: AnsiState,
    ansi_buf: String,
    saved_cursor: Option<(u32, u32)>,
    /// Lazily-built search cache: (combined_text, line_offsets). None means dirty.
    search_cache: RefCell<Option<(String, Vec<(usize, usize, bool)>)>>,
}

impl Konsole {
    pub fn new(id: u32, konsole_type: KonsoleType, owner: String, cols: u32, rows: u32) -> Self {
        let buffer = vec![vec![Cell::default(); cols as usize]; rows as usize];
        Konsole {
            id,
            konsole_type,
            owner,
            cols,
            rows,
            buffer,
            scrollback: VecDeque::new(),
            scrollback_text: VecDeque::new(),
            scrollback_flat: Vec::new(),
            scrollback_line_spans: VecDeque::new(),
            scrollback_flat_base: 0,
            scrollback_limit: 1000,
            buffer_cache: vec![None; rows as usize],
            cursor_row: 0,
            cursor_col: 0,
            dirty: false,
            current_fg: Color::Default,
            current_bg: Color::Default,
            current_bold: false,
            ansi_state: AnsiState::Normal,
            ansi_buf: String::new(),
            saved_cursor: None,
            search_cache: RefCell::new(None),
        }
    }

    /// Search scrollback buffer and current screen for a pattern.
    /// Returns matching lines with their source (scrollback vs screen) and line index.
    pub fn search_scrollback(&self, pattern: &str) -> Vec<(String, usize, bool)> {
        // Rebuild combined search buffer lazily via interior mutability.
        if self.search_cache.borrow().is_none() {
            let estimated = self.scrollback_flat.len() + self.buffer.len() * (self.cols as usize + 1);
            let mut search_text = String::with_capacity(estimated);
            let mut search_line_offsets: Vec<(usize, usize, bool)> = Vec::with_capacity(
                self.scrollback_line_spans.len() + self.buffer.len(),
            );
            let base = self.scrollback_flat_base;
            for (i, &(start, len)) in self.scrollback_line_spans.iter().enumerate() {
                let abs_start = start as usize - base;
                let line_bytes = &self.scrollback_flat[abs_start..abs_start + len as usize];
                // SAFETY: bytes were written from a valid UTF-8 String in scroll_up.
                let line_str = unsafe { std::str::from_utf8_unchecked(line_bytes) };
                search_line_offsets.push((search_text.len(), i, true));
                search_text.push_str(line_str.trim_end());
                search_text.push('\n');
            }
            for i in 0..self.buffer.len() {
                search_line_offsets.push((search_text.len(), i, false));
                let row_start = search_text.len();
                for c in &self.buffer[i] {
                    search_text.push(c.ch);
                }
                let trimmed_end = search_text[row_start..].trim_end().len();
                search_text.truncate(row_start + trimmed_end);
                search_text.push('\n');
            }
            *self.search_cache.borrow_mut() = Some((search_text, search_line_offsets));
        }

        let cache = self.search_cache.borrow();
        let (search_text, search_line_offsets) = cache.as_ref().unwrap();

        // Single-pass search over the combined string — one str::find() scan, no per-line alloc.
        let mut results = Vec::new();
        let mut search_from = 0usize;
        loop {
            let slice = &search_text[search_from..];
            let pos = if pattern.is_empty() {
                0
            } else {
                match slice.find(pattern) {
                    Some(p) => p,
                    None => break,
                }
            };
            let abs_pos = search_from + pos;
            // Binary search to find which line contains abs_pos.
            let idx = search_line_offsets.partition_point(|&(start, _, _)| start <= abs_pos).saturating_sub(1);
            let (byte_start, orig_idx, in_scrollback) = search_line_offsets[idx];
            let line_end = search_text[byte_start..].find('\n').map(|p| byte_start + p).unwrap_or(search_text.len());
            let line_text = search_text[byte_start..line_end].to_string();
            if !line_text.is_empty() {
                results.push((line_text, orig_idx, in_scrollback));
            }
            // Advance past this line to avoid duplicate results for the same line.
            search_from = line_end + 1;
            if search_from >= search_text.len() || pattern.is_empty() {
                break;
            }
        }
        results
    }

    fn scroll_up(&mut self) {
        if self.rows == 0 {
            return;
        }
        let top_line = self.buffer.remove(0);
        let line_text: String = top_line.iter().map(|c| c.ch).collect();
        let line_text = line_text.trim_end().to_string();

        // Maintain flat byte buffer for cache-friendly search.
        let start = self.scrollback_flat.len() + self.scrollback_flat_base;
        let len = line_text.len();
        self.scrollback_flat.extend_from_slice(line_text.as_bytes());
        self.scrollback_line_spans.push_back((start as u32, len as u32));

        self.scrollback_text.push_back(line_text);
        self.scrollback.push_back(top_line);

        if self.scrollback.len() > self.scrollback_limit {
            self.scrollback.pop_front();
            self.scrollback_text.pop_front();
            if let Some((evicted_start, evicted_len)) = self.scrollback_line_spans.pop_front() {
                let _ = (evicted_start, evicted_len); // tracked via flat_base
                // advance base by the evicted line's bytes
                self.scrollback_flat_base += evicted_len as usize;
            }
        }

        // Compact the flat buffer if wasted space exceeds 1MB to bound memory use.
        if self.scrollback_flat_base > 1024 * 1024 {
            let valid = self.scrollback_flat_base;
            self.scrollback_flat.drain(..valid);
            // Rebase all stored offsets.
            for span in self.scrollback_line_spans.iter_mut() {
                span.0 -= valid as u32;
            }
            self.scrollback_flat_base = 0;
        }

        self.buffer.push(vec![Cell::default(); self.cols as usize]);
        self.search_cache.get_mut().take();
    }

    fn put_char(&mut self, ch: char) {
        if self.rows == 0 || self.cols == 0 {
            return;
        }
        // Wrap if at end of line
        if self.cursor_col >= self.cols {
            self.cursor_col = 0;
            self.cursor_row += 1;
            if self.cursor_row >= self.rows {
                self.scroll_up();
                self.cursor_row = self.rows - 1;
            }
        }
        let r = self.cursor_row as usize;
        let c = self.cursor_col as usize;
        self.buffer[r][c] = Cell {
            ch,
            fg: self.current_fg,
            bg: self.current_bg,
            bold: self.current_bold,
        };
        self.cursor_col += 1;
    }

    fn process_sgr(&mut self) {
        if self.ansi_buf.is_empty() {
            // ESC[m is same as ESC[0m
            self.current_fg = Color::Default;
            self.current_bg = Color::Default;
            self.current_bold = false;
            return;
        }
        for part in self.ansi_buf.split(';') {
            let code: u32 = match part.parse() {
                Ok(n) => n,
                Err(_) => continue,
            };
            match code {
                0 => {
                    self.current_fg = Color::Default;
                    self.current_bg = Color::Default;
                    self.current_bold = false;
                }
                1 => {
                    self.current_bold = true;
                }
                30..=37 | 39 | 90..=97 => {
                    if let Some(c) = Color::from_sgr(code) {
                        self.current_fg = c;
                    }
                }
                40..=47 | 49 | 100..=107 => {
                    if let Some(c) = Color::from_sgr(code) {
                        self.current_bg = c;
                    }
                }
                _ => {}
            }
        }
    }

    fn process_csi_final(&mut self, ch: char) {
        match ch {
            'm' => self.process_sgr(),
            'H' | 'f' => {
                // Cursor position (1-indexed)
                let (row, col) = self.parse_two_params(1, 1);
                self.cursor_row = (row.saturating_sub(1)).min(self.rows.saturating_sub(1));
                self.cursor_col = (col.saturating_sub(1)).min(self.cols.saturating_sub(1));
            }
            'J' => {
                let param = self.parse_single_param(0);
                match param {
                    0 => {
                        // Erase from cursor to end of screen
                        let r = self.cursor_row as usize;
                        let c = self.cursor_col as usize;
                        if r < self.buffer.len() {
                            for i in c..self.buffer[r].len() {
                                self.buffer[r][i] = Cell::default();
                            }
                            for row in self.buffer.iter_mut().skip(r + 1) {
                                for cell in row.iter_mut() {
                                    *cell = Cell::default();
                                }
                            }
                        }
                    }
                    1 => {
                        // Erase from start of screen to cursor
                        let r = self.cursor_row as usize;
                        let c = self.cursor_col as usize;
                        for row in self.buffer.iter_mut().take(r) {
                            for cell in row.iter_mut() {
                                *cell = Cell::default();
                            }
                        }
                        if r < self.buffer.len() {
                            for i in 0..=c.min(self.buffer[r].len().saturating_sub(1)) {
                                self.buffer[r][i] = Cell::default();
                            }
                        }
                    }
                    2 => {
                        // Clear entire screen
                        for row in &mut self.buffer {
                            for cell in row.iter_mut() {
                                *cell = Cell::default();
                            }
                        }
                        self.cursor_row = 0;
                        self.cursor_col = 0;
                    }
                    _ => {}
                }
            }
            'K' => {
                // Erase from cursor to end of line
                let r = self.cursor_row as usize;
                let c = self.cursor_col as usize;
                if r < self.buffer.len() {
                    for i in c..self.buffer[r].len() {
                        self.buffer[r][i] = Cell::default();
                    }
                }
            }
            'A' => {
                let n = self.parse_single_param(1);
                self.cursor_row = self.cursor_row.saturating_sub(n);
            }
            'B' => {
                let n = self.parse_single_param(1);
                self.cursor_row = (self.cursor_row + n).min(self.rows.saturating_sub(1));
            }
            'C' => {
                let n = self.parse_single_param(1);
                self.cursor_col = (self.cursor_col + n).min(self.cols.saturating_sub(1));
            }
            'D' => {
                let n = self.parse_single_param(1);
                self.cursor_col = self.cursor_col.saturating_sub(n);
            }
            'G' => {
                // CHA: cursor horizontal absolute (1-indexed)
                let col = self.parse_single_param(1);
                self.cursor_col = (col.saturating_sub(1)).min(self.cols.saturating_sub(1));
            }
            's' => {
                self.saved_cursor = Some((self.cursor_row, self.cursor_col));
            }
            'u' => {
                if let Some((r, c)) = self.saved_cursor {
                    self.cursor_row = r.min(self.rows.saturating_sub(1));
                    self.cursor_col = c.min(self.cols.saturating_sub(1));
                }
            }
            _ => {} // Unrecognized CSI final — ignore
        }
    }

    fn parse_single_param(&self, default: u32) -> u32 {
        if self.ansi_buf.is_empty() {
            return default;
        }
        self.ansi_buf.parse().unwrap_or(default)
    }

    fn parse_two_params(&self, default1: u32, default2: u32) -> (u32, u32) {
        let parts: Vec<&str> = self.ansi_buf.split(';').collect();
        let a = parts
            .first()
            .and_then(|s| s.parse().ok())
            .unwrap_or(default1);
        let b = parts
            .get(1)
            .and_then(|s| s.parse().ok())
            .unwrap_or(default2);
        (a, b)
    }

    pub fn write_data(&mut self, data: &str) {
        // Invalidate the screen buffer string cache — will be rebuilt lazily on next search.
        self.buffer_cache.iter_mut().for_each(|e| *e = None);
        for ch in data.chars() {
            match self.ansi_state {
                AnsiState::Normal => match ch {
                    '\x1b' => self.ansi_state = AnsiState::Escape,
                    '\n' => {
                        self.cursor_col = 0;
                        self.cursor_row += 1;
                        if self.cursor_row >= self.rows {
                            self.scroll_up();
                            self.cursor_row = self.rows.saturating_sub(1);
                        }
                    }
                    '\r' => self.cursor_col = 0,
                    '\x08' => {
                        if self.cursor_col > 0 {
                            self.cursor_col -= 1;
                        }
                    }
                    '\t' => {
                        let next_stop = (self.cursor_col / 8 + 1) * 8;
                        self.cursor_col = next_stop.min(self.cols.saturating_sub(1));
                    }
                    c if !c.is_control() => self.put_char(c),
                    _ => {} // Ignore other control chars
                },
                AnsiState::Escape => match ch {
                    '[' => {
                        self.ansi_state = AnsiState::Csi;
                        self.ansi_buf.clear();
                    }
                    ']' => {
                        self.ansi_state = AnsiState::Osc;
                    }
                    '7' => {
                        // DECSC: save cursor position
                        self.saved_cursor = Some((self.cursor_row, self.cursor_col));
                        self.ansi_state = AnsiState::Normal;
                    }
                    '8' => {
                        // DECRC: restore cursor position
                        if let Some((r, c)) = self.saved_cursor {
                            self.cursor_row = r.min(self.rows.saturating_sub(1));
                            self.cursor_col = c.min(self.cols.saturating_sub(1));
                        }
                        self.ansi_state = AnsiState::Normal;
                    }
                    _ => self.ansi_state = AnsiState::Normal,
                },
                AnsiState::Csi => {
                    if ch.is_ascii_digit() || ch == ';' || ch == '?' {
                        if self.ansi_buf.len() >= MAX_ANSI_BUF {
                            // Abort malformed sequence
                            self.ansi_state = AnsiState::Normal;
                        } else {
                            self.ansi_buf.push(ch);
                        }
                    } else {
                        self.process_csi_final(ch);
                        self.ansi_state = AnsiState::Normal;
                    }
                }
                AnsiState::Osc => {
                    // Consume until BEL (\x07) or ST (\x1b\\)
                    if ch == '\x07' {
                        self.ansi_state = AnsiState::Normal;
                    }
                    // ST (\x1b\\ ) is handled by next ESC transitioning to Escape state naturally
                }
            }
        }
        self.dirty = true;
        self.search_cache.get_mut().take();
    }

    pub fn resize(&mut self, new_cols: u32, new_rows: u32) {
        let mut new_buffer = vec![vec![Cell::default(); new_cols as usize]; new_rows as usize];
        let copy_rows = (self.rows as usize).min(new_rows as usize);
        let copy_cols = (self.cols as usize).min(new_cols as usize);
        for r in 0..copy_rows {
            for c in 0..copy_cols {
                new_buffer[r][c] = self.buffer[r][c];
            }
        }
        self.buffer = new_buffer;
        self.cols = new_cols;
        self.rows = new_rows;
        self.cursor_row = self.cursor_row.min(new_rows.saturating_sub(1));
        self.cursor_col = self.cursor_col.min(new_cols.saturating_sub(1));
        self.buffer_cache.clear();
        self.search_cache.get_mut().take();
    }

    pub fn clear(&mut self) {
        for row in &mut self.buffer {
            for cell in row.iter_mut() {
                *cell = Cell::default();
            }
        }
        self.cursor_row = 0;
        self.cursor_col = 0;
        self.buffer_cache.iter_mut().for_each(|c| *c = None);
        self.search_cache.get_mut().take();
    }

    fn read_lines(&self, from_line: usize, count: usize) -> Vec<String> {
        let rows = self.rows as usize;
        let start = from_line.min(rows);
        let end = start.saturating_add(count).min(rows);
        self.buffer[start..end]
            .iter()
            .map(|row| row.iter().map(|cell| cell.ch).collect::<String>().trim_end().to_string())
            .collect()
    }
}

// ---------------------------------------------------------------------------
// KonsoleHandler
// ---------------------------------------------------------------------------

pub struct KonsoleHandler {
    pub konsoles: Arc<Mutex<Vec<Konsole>>>,
}

impl KonsoleHandler {
    pub fn new(konsoles: Arc<Mutex<Vec<Konsole>>>) -> Self {
        KonsoleHandler { konsoles }
    }

    fn lock_konsoles(&self) -> std::sync::MutexGuard<'_, Vec<Konsole>> {
        match self.konsoles.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    fn find_konsole_index(konsoles: &[Konsole], id: u32) -> Option<usize> {
        konsoles.iter().position(|k| k.id == id)
    }

    fn next_id(konsoles: &[Konsole]) -> u32 {
        konsoles.iter().map(|k| k.id).max().map(|m| m + 1).unwrap_or(0)
    }

    fn get_param_u64(params: &Value, key: &str) -> Option<u64> {
        params.get(key).and_then(|v| v.as_u64())
    }

    fn get_param_str<'a>(params: &'a Value, key: &str) -> Option<&'a str> {
        params.get(key).and_then(|v| v.as_str())
    }
}

// ---------------------------------------------------------------------------
// Per-method handlers (F6: extracted from monolithic handle())
// ---------------------------------------------------------------------------

impl KonsoleHandler {
    fn handle_list(&self, id: &Option<Value>) -> JsonRpcResponse {
        let konsoles = self.lock_konsoles();
        let list: Vec<Value> = konsoles
            .iter()
            .map(|k| {
                json!({
                    "id": k.id,
                    "type": k.konsole_type.as_str(),
                    "owner": k.owner,
                    "cols": k.cols,
                    "rows": k.rows,
                })
            })
            .collect();
        JsonRpcResponse::success(id.clone(), json!(list))
    }

    fn handle_create(&self, params: &Value, id: &Option<Value>) -> JsonRpcResponse {
        let type_str = match Self::get_param_str(params, "type") {
            Some(t) => t,
            None => return JsonRpcResponse::error(id.clone(), INVALID_PARAMS, "Missing required param: type"),
        };
        let konsole_type = match KonsoleType::from_str(type_str) {
            Some(t) => t,
            None => return JsonRpcResponse::error(id.clone(), INVALID_PARAMS, format!("Invalid konsole type: {}", type_str)),
        };
        let owner = match Self::get_param_str(params, "owner") {
            Some(o) => o.to_string(),
            None => return JsonRpcResponse::error(id.clone(), INVALID_PARAMS, "Missing required param: owner"),
        };
        let cols = Self::get_param_u64(params, "cols").unwrap_or(80) as u32;
        let rows = Self::get_param_u64(params, "rows").unwrap_or(24) as u32;

        if cols > MAX_COLS || rows > MAX_ROWS {
            return JsonRpcResponse::error(
                id.clone(),
                INVALID_PARAMS,
                format!("Dimensions exceed limits (max {}x{})", MAX_COLS, MAX_ROWS),
            );
        }

        let mut konsoles = self.lock_konsoles();

        if konsoles.len() >= MAX_KONSOLES {
            return JsonRpcResponse::error(
                id.clone(),
                INVALID_PARAMS,
                format!("Maximum konsole count ({}) reached", MAX_KONSOLES),
            );
        }

        let new_id = Self::next_id(&konsoles);
        let konsole = Konsole::new(new_id, konsole_type.clone(), owner, cols, rows);
        konsoles.push(konsole);

        JsonRpcResponse::success(
            id.clone(),
            json!({
                "id": new_id,
                "type": konsole_type.as_str(),
                "cols": cols,
                "rows": rows,
            }),
        )
    }

    fn handle_destroy(&self, params: &Value, id: &Option<Value>) -> JsonRpcResponse {
        let kid = match Self::get_param_u64(params, "id") {
            Some(i) => i as u32,
            None => return JsonRpcResponse::error(id.clone(), INVALID_PARAMS, "Missing required param: id"),
        };
        let mut konsoles = self.lock_konsoles();
        match Self::find_konsole_index(&konsoles, kid) {
            Some(idx) => {
                konsoles.remove(idx);
                JsonRpcResponse::success(id.clone(), json!({"ok": true}))
            }
            None => JsonRpcResponse::error(id.clone(), INVALID_PARAMS, format!("Konsole not found: {}", kid)),
        }
    }

    fn handle_read(&self, params: &Value, id: &Option<Value>) -> JsonRpcResponse {
        let kid = match Self::get_param_u64(params, "id") {
            Some(i) => i as u32,
            None => return JsonRpcResponse::error(id.clone(), INVALID_PARAMS, "Missing required param: id"),
        };
        let konsoles = self.lock_konsoles();
        match konsoles.iter().find(|k| k.id == kid) {
            Some(k) => {
                let from_line = Self::get_param_u64(params, "from_line").unwrap_or(0) as usize;
                let count = Self::get_param_u64(params, "count").unwrap_or(k.rows as u64) as usize;
                let lines = k.read_lines(from_line, count);
                JsonRpcResponse::success(
                    id.clone(),
                    json!({
                        "lines": lines,
                        "cursor": {"row": k.cursor_row, "col": k.cursor_col},
                    }),
                )
            }
            None => JsonRpcResponse::error(id.clone(), INVALID_PARAMS, format!("Konsole not found: {}", kid)),
        }
    }

    fn handle_write(&self, params: &Value, id: &Option<Value>) -> JsonRpcResponse {
        let kid = match Self::get_param_u64(params, "id") {
            Some(i) => i as u32,
            None => return JsonRpcResponse::error(id.clone(), INVALID_PARAMS, "Missing required param: id"),
        };
        let data = match Self::get_param_str(params, "data") {
            Some(d) => d.to_string(),
            None => return JsonRpcResponse::error(id.clone(), INVALID_PARAMS, "Missing required param: data"),
        };
        let bytes_written = data.len();
        let mut konsoles = self.lock_konsoles();
        match konsoles.iter_mut().find(|k| k.id == kid) {
            Some(k) => {
                k.write_data(&data);
                JsonRpcResponse::success(id.clone(), json!({"ok": true, "bytes_written": bytes_written}))
            }
            None => JsonRpcResponse::error(id.clone(), INVALID_PARAMS, format!("Konsole not found: {}", kid)),
        }
    }

    fn handle_resize(&self, params: &Value, id: &Option<Value>) -> JsonRpcResponse {
        let kid = match Self::get_param_u64(params, "id") {
            Some(i) => i as u32,
            None => return JsonRpcResponse::error(id.clone(), INVALID_PARAMS, "Missing required param: id"),
        };
        let cols = match Self::get_param_u64(params, "cols") {
            Some(c) => c as u32,
            None => return JsonRpcResponse::error(id.clone(), INVALID_PARAMS, "Missing required param: cols"),
        };
        let rows = match Self::get_param_u64(params, "rows") {
            Some(r) => r as u32,
            None => return JsonRpcResponse::error(id.clone(), INVALID_PARAMS, "Missing required param: rows"),
        };

        if cols > MAX_COLS || rows > MAX_ROWS {
            return JsonRpcResponse::error(
                id.clone(),
                INVALID_PARAMS,
                format!("Dimensions exceed limits (max {}x{})", MAX_COLS, MAX_ROWS),
            );
        }

        let mut konsoles = self.lock_konsoles();
        match konsoles.iter_mut().find(|k| k.id == kid) {
            Some(k) => {
                k.resize(cols, rows);
                JsonRpcResponse::success(id.clone(), json!({"ok": true}))
            }
            None => JsonRpcResponse::error(id.clone(), INVALID_PARAMS, format!("Konsole not found: {}", kid)),
        }
    }

    fn handle_clear(&self, params: &Value, id: &Option<Value>) -> JsonRpcResponse {
        let kid = match Self::get_param_u64(params, "id") {
            Some(i) => i as u32,
            None => return JsonRpcResponse::error(id.clone(), INVALID_PARAMS, "Missing required param: id"),
        };
        let mut konsoles = self.lock_konsoles();
        match konsoles.iter_mut().find(|k| k.id == kid) {
            Some(k) => {
                k.clear();
                JsonRpcResponse::success(id.clone(), json!({"ok": true}))
            }
            None => JsonRpcResponse::error(id.clone(), INVALID_PARAMS, format!("Konsole not found: {}", kid)),
        }
    }

    fn handle_info(&self, params: &Value, id: &Option<Value>) -> JsonRpcResponse {
        let kid = match Self::get_param_u64(params, "id") {
            Some(i) => i as u32,
            None => return JsonRpcResponse::error(id.clone(), INVALID_PARAMS, "Missing required param: id"),
        };
        let konsoles = self.lock_konsoles();
        match konsoles.iter().find(|k| k.id == kid) {
            Some(k) => JsonRpcResponse::success(
                id.clone(),
                json!({
                    "id": k.id,
                    "type": k.konsole_type.as_str(),
                    "owner": k.owner,
                    "cols": k.cols,
                    "rows": k.rows,
                    "cursor_row": k.cursor_row,
                    "cursor_col": k.cursor_col,
                    "scrollback_size": k.scrollback.len(),
                    "dirty": k.dirty,
                }),
            ),
            None => JsonRpcResponse::error(id.clone(), INVALID_PARAMS, format!("Konsole not found: {}", kid)),
        }
    }

    fn handle_scroll(&self, params: &Value, id: &Option<Value>) -> JsonRpcResponse {
        let kid = match Self::get_param_u64(params, "id") {
            Some(i) => i as u32,
            None => return JsonRpcResponse::error(id.clone(), INVALID_PARAMS, "Missing required param: id"),
        };
        let lines = match params.get("lines").and_then(|v| v.as_i64()) {
            Some(l) => l,
            None => return JsonRpcResponse::error(id.clone(), INVALID_PARAMS, "Missing required param: lines"),
        };
        let mut konsoles = self.lock_konsoles();
        match konsoles.iter_mut().find(|k| k.id == kid) {
            Some(k) => {
                if lines > 0 {
                    for _ in 0..lines.min(k.rows as i64) {
                        k.scroll_up();
                    }
                }
                JsonRpcResponse::success(id.clone(), json!({"ok": true}))
            }
            None => JsonRpcResponse::error(id.clone(), INVALID_PARAMS, format!("Konsole not found: {}", kid)),
        }
    }

    fn handle_cursor(&self, params: &Value, id: &Option<Value>) -> JsonRpcResponse {
        let kid = match Self::get_param_u64(params, "id") {
            Some(i) => i as u32,
            None => return JsonRpcResponse::error(id.clone(), INVALID_PARAMS, "Missing required param: id"),
        };
        let row = match Self::get_param_u64(params, "row") {
            Some(r) => r as u32,
            None => return JsonRpcResponse::error(id.clone(), INVALID_PARAMS, "Missing required param: row"),
        };
        let col = match Self::get_param_u64(params, "col") {
            Some(c) => c as u32,
            None => return JsonRpcResponse::error(id.clone(), INVALID_PARAMS, "Missing required param: col"),
        };
        let mut konsoles = self.lock_konsoles();
        match konsoles.iter_mut().find(|k| k.id == kid) {
            Some(k) => {
                k.cursor_row = row.min(k.rows.saturating_sub(1));
                k.cursor_col = col.min(k.cols.saturating_sub(1));
                JsonRpcResponse::success(id.clone(), json!({"ok": true}))
            }
            None => JsonRpcResponse::error(id.clone(), INVALID_PARAMS, format!("Konsole not found: {}", kid)),
        }
    }

    fn handle_search(&self, params: &Value, id: &Option<Value>) -> JsonRpcResponse {
        let kid = match Self::get_param_u64(params, "id") {
            Some(i) => i as u32,
            None => return JsonRpcResponse::error(id.clone(), INVALID_PARAMS, "Missing required param: id"),
        };
        let pattern = match Self::get_param_str(params, "pattern") {
            Some(p) => p.to_string(),
            None => return JsonRpcResponse::error(id.clone(), INVALID_PARAMS, "Missing required param: pattern"),
        };
        let mut konsoles = self.lock_konsoles();
        match konsoles.iter_mut().find(|k| k.id == kid) {
            Some(k) => {
                let matches = k.search_scrollback(&pattern);
                let results: Vec<Value> = matches
                    .iter()
                    .map(|(text, line, in_scrollback)| {
                        json!({
                            "text": text,
                            "line": line,
                            "in_scrollback": in_scrollback,
                        })
                    })
                    .collect();
                JsonRpcResponse::success(id.clone(), json!({"matches": results}))
            }
            None => JsonRpcResponse::error(id.clone(), INVALID_PARAMS, format!("Konsole not found: {}", kid)),
        }
    }
}

impl ServiceHandler for KonsoleHandler {
    fn handle(&self, _path: &McpPath, request: &JsonRpcRequest) -> JsonRpcResponse {
        match request.method.as_str() {
            "list" => self.handle_list(&request.id),
            "create" => self.handle_create(&request.params, &request.id),
            "destroy" => self.handle_destroy(&request.params, &request.id),
            "read" => self.handle_read(&request.params, &request.id),
            "write" => self.handle_write(&request.params, &request.id),
            "resize" => self.handle_resize(&request.params, &request.id),
            "clear" => self.handle_clear(&request.params, &request.id),
            "info" => self.handle_info(&request.params, &request.id),
            "scroll" => self.handle_scroll(&request.params, &request.id),
            "cursor" => self.handle_cursor(&request.params, &request.id),
            "search" => self.handle_search(&request.params, &request.id),
            _ => JsonRpcResponse::error(
                request.id.clone(),
                METHOD_NOT_FOUND,
                format!("Method '{}' not found in konsole service", request.method),
            ),
        }
    }

    fn list_methods(&self) -> Vec<&str> {
        vec![
            "list", "create", "destroy", "read", "write", "resize", "clear", "info", "scroll",
            "cursor", "search",
        ]
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn req(method: &str, params: Value) -> JsonRpcRequest {
        JsonRpcRequest {
            jsonrpc: "2.0".into(),
            method: method.into(),
            params,
            id: Some(json!(1)),
        }
    }

    fn dummy_path() -> McpPath {
        McpPath {
            service: "konsole".into(),
            resource: vec![],
        }
    }

    fn create_test_konsole(h: &KonsoleHandler) -> u32 {
        let path = dummy_path();
        let resp = h.handle(
            &path,
            &req(
                "create",
                json!({"type": "user", "owner": "test", "cols": 80, "rows": 24}),
            ),
        );
        resp.result.unwrap()["id"].as_u64().unwrap() as u32
    }

    #[test]
    fn create_and_list() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let path = dummy_path();
        let id = create_test_konsole(&h);

        let resp = h.handle(&path, &req("list", json!({})));
        let list = resp.result.unwrap();
        let arr = list.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["id"], json!(id));
        assert_eq!(arr[0]["type"], json!("user"));
    }

    #[test]
    fn write_and_read() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let path = dummy_path();
        let id = create_test_konsole(&h);

        h.handle(&path, &req("write", json!({"id": id, "data": "Hello"})));
        let resp = h.handle(&path, &req("read", json!({"id": id})));
        let result = resp.result.unwrap();
        let lines = result["lines"].as_array().unwrap();
        assert!(lines[0].as_str().unwrap().starts_with("Hello"));
        assert_eq!(result["cursor"]["col"], json!(5));
    }

    #[test]
    fn ansi_colors() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let path = dummy_path();
        let id = create_test_konsole(&h);

        // Write red text
        h.handle(
            &path,
            &req("write", json!({"id": id, "data": "\x1b[31mRed\x1b[0m"})),
        );
        let konsoles = h.lock_konsoles();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.buffer[0][0].fg, Color::Red);
        assert_eq!(k.buffer[0][0].ch, 'R');
        // After reset
        assert_eq!(k.current_fg, Color::Default);
    }

    #[test]
    fn ansi_bold() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let path = dummy_path();
        let id = create_test_konsole(&h);

        h.handle(
            &path,
            &req("write", json!({"id": id, "data": "\x1b[1mBold\x1b[0m"})),
        );
        let konsoles = h.lock_konsoles();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert!(k.buffer[0][0].bold);
        assert_eq!(k.buffer[0][0].ch, 'B');
    }

    #[test]
    fn ansi_cursor_position() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let path = dummy_path();
        let id = create_test_konsole(&h);

        // Move cursor to row 3, col 5 (1-indexed)
        h.handle(
            &path,
            &req("write", json!({"id": id, "data": "\x1b[3;5H*"})),
        );
        let konsoles = h.lock_konsoles();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.buffer[2][4].ch, '*');
    }

    #[test]
    fn ansi_erase_screen() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let path = dummy_path();
        let id = create_test_konsole(&h);

        h.handle(
            &path,
            &req("write", json!({"id": id, "data": "Hello\x1b[2J"})),
        );
        let konsoles = h.lock_konsoles();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        // Screen should be cleared
        assert_eq!(k.buffer[0][0].ch, ' ');
        assert_eq!(k.cursor_row, 0);
    }

    #[test]
    fn ansi_erase_line() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let path = dummy_path();
        let id = create_test_konsole(&h);

        h.handle(
            &path,
            &req(
                "write",
                json!({"id": id, "data": "Hello World\r\x1b[5C\x1b[K"}),
            ),
        );
        let konsoles = h.lock_konsoles();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.buffer[0][0].ch, 'H');
        assert_eq!(k.buffer[0][4].ch, 'o');
        assert_eq!(k.buffer[0][5].ch, ' '); // erased
    }

    #[test]
    fn line_wrapping() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let path = dummy_path();
        // Small konsole: 5 cols
        let resp = h.handle(
            &path,
            &req(
                "create",
                json!({"type": "user", "owner": "test", "cols": 5, "rows": 3}),
            ),
        );
        let id = resp.result.unwrap()["id"].as_u64().unwrap() as u32;

        h.handle(
            &path,
            &req("write", json!({"id": id, "data": "ABCDEFGH"})),
        );
        let konsoles = h.lock_konsoles();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        // First row: ABCDE, second row: FGH
        assert_eq!(k.buffer[0][4].ch, 'E');
        assert_eq!(k.buffer[1][0].ch, 'F');
        assert_eq!(k.buffer[1][2].ch, 'H');
    }

    #[test]
    fn newline_scrolling_and_scrollback() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let path = dummy_path();
        let resp = h.handle(
            &path,
            &req(
                "create",
                json!({"type": "user", "owner": "test", "cols": 10, "rows": 3}),
            ),
        );
        let id = resp.result.unwrap()["id"].as_u64().unwrap() as u32;

        // Fill 3 rows then push one more
        h.handle(
            &path,
            &req("write", json!({"id": id, "data": "AAA\nBBB\nCCC\nDDD"})),
        );
        let konsoles = h.lock_konsoles();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        // AAA should be in scrollback
        assert_eq!(k.scrollback.len(), 1);
        assert_eq!(k.scrollback[0][0].ch, 'A');
        // Current screen: BBB, CCC, DDD
        assert_eq!(k.buffer[0][0].ch, 'B');
        assert_eq!(k.buffer[1][0].ch, 'C');
        assert_eq!(k.buffer[2][0].ch, 'D');
    }

    #[test]
    fn destroy() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let path = dummy_path();
        let id = create_test_konsole(&h);

        let resp = h.handle(&path, &req("destroy", json!({"id": id})));
        assert_eq!(resp.result.unwrap()["ok"], json!(true));

        let resp = h.handle(&path, &req("list", json!({})));
        let arr = resp.result.unwrap().as_array().unwrap().clone();
        assert_eq!(arr.len(), 0);
    }

    #[test]
    fn resize() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let path = dummy_path();
        let id = create_test_konsole(&h);

        h.handle(&path, &req("write", json!({"id": id, "data": "Hi"})));
        h.handle(
            &path,
            &req("resize", json!({"id": id, "cols": 40, "rows": 10})),
        );

        let resp = h.handle(&path, &req("info", json!({"id": id})));
        let info = resp.result.unwrap();
        assert_eq!(info["cols"], json!(40));
        assert_eq!(info["rows"], json!(10));
    }

    #[test]
    fn clear_resets_cursor() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let path = dummy_path();
        let id = create_test_konsole(&h);

        h.handle(&path, &req("write", json!({"id": id, "data": "Hello"})));
        h.handle(&path, &req("clear", json!({"id": id})));

        let resp = h.handle(&path, &req("info", json!({"id": id})));
        let info = resp.result.unwrap();
        assert_eq!(info["cursor_row"], json!(0));
        assert_eq!(info["cursor_col"], json!(0));
    }

    #[test]
    fn cursor_set() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let path = dummy_path();
        let id = create_test_konsole(&h);

        h.handle(
            &path,
            &req("cursor", json!({"id": id, "row": 5, "col": 10})),
        );
        let resp = h.handle(&path, &req("info", json!({"id": id})));
        let info = resp.result.unwrap();
        assert_eq!(info["cursor_row"], json!(5));
        assert_eq!(info["cursor_col"], json!(10));
    }

    #[test]
    fn info_shows_metadata() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let path = dummy_path();
        let id = create_test_konsole(&h);

        let resp = h.handle(&path, &req("info", json!({"id": id})));
        let info = resp.result.unwrap();
        assert_eq!(info["type"], json!("user"));
        assert_eq!(info["owner"], json!("test"));
        assert_eq!(info["cols"], json!(80));
        assert_eq!(info["rows"], json!(24));
        assert_eq!(info["dirty"], json!(false));
    }

    #[test]
    fn scroll_up() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let path = dummy_path();
        let resp = h.handle(
            &path,
            &req(
                "create",
                json!({"type": "user", "owner": "test", "cols": 10, "rows": 3}),
            ),
        );
        let id = resp.result.unwrap()["id"].as_u64().unwrap() as u32;

        h.handle(
            &path,
            &req("write", json!({"id": id, "data": "AAA\nBBB\nCCC"})),
        );
        h.handle(&path, &req("scroll", json!({"id": id, "lines": 1})));

        let konsoles = h.lock_konsoles();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.scrollback.len(), 1);
        assert_eq!(k.scrollback[0][0].ch, 'A');
    }

    #[test]
    fn unknown_method_returns_error() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let path = dummy_path();
        let resp = h.handle(&path, &req("nonexistent", json!({})));
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, METHOD_NOT_FOUND);
    }

    #[test]
    fn missing_id_returns_error() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let path = dummy_path();
        let resp = h.handle(&path, &req("read", json!({})));
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, INVALID_PARAMS);
    }

    #[test]
    fn bright_colors() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let path = dummy_path();
        let id = create_test_konsole(&h);

        h.handle(
            &path,
            &req("write", json!({"id": id, "data": "\x1b[91mX"})),
        );
        let konsoles = h.lock_konsoles();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.buffer[0][0].fg, Color::BrightRed);
    }

    #[test]
    fn bg_colors() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let path = dummy_path();
        let id = create_test_konsole(&h);

        h.handle(
            &path,
            &req("write", json!({"id": id, "data": "\x1b[44mX"})),
        );
        let konsoles = h.lock_konsoles();
        let k = konsoles.iter().find(|k| k.id == id).unwrap();
        assert_eq!(k.buffer[0][0].bg, Color::Blue);
    }

    #[test]
    fn scrollback_search_basic() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let path = dummy_path();
        let resp = h.handle(
            &path,
            &req(
                "create",
                json!({"type": "user", "owner": "test", "cols": 20, "rows": 3}),
            ),
        );
        let id = resp.result.unwrap()["id"].as_u64().unwrap() as u32;

        // Write enough lines to push some into scrollback
        h.handle(
            &path,
            &req("write", json!({"id": id, "data": "alpha foo\nbeta bar\ngamma foo\ndelta baz\nepsilon foo"})),
        );

        // Search via handler
        let resp = h.handle(
            &path,
            &req("search", json!({"id": id, "pattern": "foo"})),
        );
        let result = resp.result.unwrap();
        let matches = result["matches"].as_array().unwrap();
        // "foo" appears in alpha, gamma, epsilon — some in scrollback, some on screen
        assert!(matches.len() >= 2, "Expected at least 2 matches for 'foo', got {}", matches.len());

        // All matches should contain "foo"
        for m in matches {
            assert!(m["text"].as_str().unwrap().contains("foo"));
        }
    }

    #[test]
    fn scrollback_search_no_match() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let path = dummy_path();
        let id = create_test_konsole(&h);

        h.handle(
            &path,
            &req("write", json!({"id": id, "data": "Hello world"})),
        );
        let resp = h.handle(
            &path,
            &req("search", json!({"id": id, "pattern": "zzz"})),
        );
        let result = resp.result.unwrap();
        let matches = result["matches"].as_array().unwrap();
        assert_eq!(matches.len(), 0);
    }

    #[test]
    fn scrollback_search_scrollback_only() {
        let h = KonsoleHandler::new(Arc::new(Mutex::new(Vec::new())));
        let path = dummy_path();
        let resp = h.handle(
            &path,
            &req(
                "create",
                json!({"type": "user", "owner": "test", "cols": 20, "rows": 2}),
            ),
        );
        let id = resp.result.unwrap()["id"].as_u64().unwrap() as u32;

        // Push "needle" into scrollback by filling past screen
        h.handle(
            &path,
            &req("write", json!({"id": id, "data": "needle here\nline2\nline3\nline4"})),
        );

        let resp = h.handle(
            &path,
            &req("search", json!({"id": id, "pattern": "needle"})),
        );
        let result = resp.result.unwrap();
        let matches = result["matches"].as_array().unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0]["in_scrollback"], json!(true));
    }
}
