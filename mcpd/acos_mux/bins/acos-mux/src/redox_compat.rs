//! Crossterm compatibility shims for the Redox target.
//!
//! Redox does not support mio/epoll, so crossterm cannot be used there.
//! This module provides stub types that match the crossterm API surface
//! used by the acos-mux binary, allowing the crate to compile for
//! `x86_64-unknown-redox`. The actual terminal I/O uses termion.
//!
//! Only compiled when `target_os = "redox"`.

use std::io::{self, Write};
use std::time::Duration;

// ---------------------------------------------------------------------------
// event module — mirrors crossterm::event
// ---------------------------------------------------------------------------

pub mod event {
    use super::*;

    // ---- KeyModifiers (bitflags-style) ------------------------------------

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    pub struct KeyModifiers(u8);

    impl KeyModifiers {
        pub const NONE: KeyModifiers = KeyModifiers(0);
        pub const SHIFT: KeyModifiers = KeyModifiers(0x01);
        pub const CONTROL: KeyModifiers = KeyModifiers(0x02);
        pub const ALT: KeyModifiers = KeyModifiers(0x04);

        pub fn empty() -> Self { Self(0) }
        pub fn is_empty(self) -> bool { self.0 == 0 }
        pub fn contains(self, other: KeyModifiers) -> bool { (self.0 & other.0) == other.0 }
    }

    impl std::ops::BitOr for KeyModifiers {
        type Output = Self;
        fn bitor(self, rhs: Self) -> Self { Self(self.0 | rhs.0) }
    }

    impl std::ops::BitOrAssign for KeyModifiers {
        fn bitor_assign(&mut self, rhs: Self) { self.0 |= rhs.0; }
    }

    impl std::ops::BitAnd for KeyModifiers {
        type Output = Self;
        fn bitand(self, rhs: Self) -> Self { Self(self.0 & rhs.0) }
    }

    // ---- KeyCode ----------------------------------------------------------

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub enum KeyCode {
        Backspace,
        Enter,
        Left,
        Right,
        Up,
        Down,
        Home,
        End,
        PageUp,
        PageDown,
        Tab,
        BackTab,
        Delete,
        Insert,
        F(u8),
        Char(char),
        Null,
        Esc,
        CapsLock,
        ScrollLock,
        NumLock,
        PrintScreen,
        Pause,
        Menu,
        KeypadBegin,
    }

    // ---- KeyEvent ---------------------------------------------------------

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct KeyEvent {
        pub code: KeyCode,
        pub modifiers: KeyModifiers,
    }

    impl KeyEvent {
        pub fn new(code: KeyCode, modifiers: KeyModifiers) -> Self {
            Self { code, modifiers }
        }
    }

    // ---- Mouse types ------------------------------------------------------

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum MouseButton { Left, Right, Middle }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum MouseEventKind {
        Down(MouseButton),
        Up(MouseButton),
        Drag(MouseButton),
        Moved,
        ScrollDown,
        ScrollUp,
        ScrollLeft,
        ScrollRight,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct MouseEvent {
        pub kind: MouseEventKind,
        pub column: u16,
        pub row: u16,
        pub modifiers: KeyModifiers,
    }

    // ---- Event ------------------------------------------------------------

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum Event {
        Key(KeyEvent),
        Mouse(MouseEvent),
        Resize(u16, u16),
        Paste(String),
        FocusGained,
        FocusLost,
    }

    // ---- No-op terminal commands -----------------------------------------

    pub struct DisableBracketedPaste;
    pub struct EnableBracketedPaste;
    pub struct DisableMouseCapture;
    pub struct EnableMouseCapture;

    impl super::terminal::CrosstermCommand for DisableBracketedPaste {
        fn write_ansi(&self, _f: &mut dyn Write) -> io::Result<()> { Ok(()) }
    }
    impl super::terminal::CrosstermCommand for EnableBracketedPaste {
        fn write_ansi(&self, _f: &mut dyn Write) -> io::Result<()> { Ok(()) }
    }
    impl super::terminal::CrosstermCommand for DisableMouseCapture {
        fn write_ansi(&self, _f: &mut dyn Write) -> io::Result<()> { Ok(()) }
    }
    impl super::terminal::CrosstermCommand for EnableMouseCapture {
        fn write_ansi(&self, _f: &mut dyn Write) -> io::Result<()> { Ok(()) }
    }

    // ---- Blocking event reader (termion-based) ---------------------------
    //
    // Redox has no epoll. We run a background stdin-reader thread that sends
    // events through a std::sync::mpsc channel.  `poll()` does a timed
    // recv_timeout and `read()` does a blocking recv.
    //
    // The reader properly parses multi-byte ANSI escape sequences so that
    // arrow keys and function keys are not split into individual bytes.

    use std::sync::{Mutex, OnceLock};
    use std::sync::mpsc::{self, Receiver, Sender};

    struct EventChannel {
        tx: Sender<Event>,
        rx: Mutex<Receiver<Event>>,
    }

    static CHANNEL: OnceLock<EventChannel> = OnceLock::new();

    fn channel() -> &'static EventChannel {
        CHANNEL.get_or_init(|| {
            let (tx, rx) = mpsc::channel::<Event>();
            let chan = EventChannel {
                tx: tx.clone(),
                rx: Mutex::new(rx),
            };
            // Spawn background reader thread.
            // Reads one byte at a time. When ESC (0x1b) is seen, waits 5ms and
            // then tries to read the rest of the escape sequence. This correctly
            // handles arrow keys, function keys, and other multi-byte sequences
            // sent atomically by the terminal.
            let tx2 = tx;
            std::thread::spawn(move || {
                use std::io::{BufReader, Read};
                let stdin = std::io::stdin();
                let mut reader = BufReader::with_capacity(1024, stdin.lock());
                let mut buf = [0u8; 1];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) => break,
                        Ok(_) => {
                            let byte = buf[0];
                            if byte == 0x1b {
                                // Potential escape sequence — wait briefly for
                                // more bytes to arrive (they are sent atomically
                                // by the terminal driver).
                                std::thread::sleep(std::time::Duration::from_millis(5));
                                let mut seq = vec![0x1b_u8];
                                let mut extra = [0u8; 16];
                                match reader.read(&mut extra) {
                                    Ok(n) if n > 0 => {
                                        seq.extend_from_slice(&extra[..n]);
                                        let ev = parse_escape_sequence(&seq);
                                        if tx2.send(ev).is_err() { return; }
                                    }
                                    _ => {
                                        // Standalone ESC key — no sequence followed.
                                        let ev = Event::Key(KeyEvent {
                                            code: KeyCode::Esc,
                                            modifiers: KeyModifiers::empty(),
                                        });
                                        if tx2.send(ev).is_err() { return; }
                                    }
                                }
                            } else {
                                let ev = byte_to_event(byte);
                                if tx2.send(ev).is_err() { return; }
                            }
                        }
                        Err(_) => break,
                    }
                }
            });
            chan
        })
    }

    /// Parse a multi-byte ANSI escape sequence starting with 0x1b.
    /// Returns the corresponding Event, falling back to Esc if unrecognised.
    fn parse_escape_sequence(seq: &[u8]) -> Event {
        let key = |code: KeyCode| Event::Key(KeyEvent { code, modifiers: KeyModifiers::empty() });

        match seq {
            // CSI sequences: ESC [ ...
            [0x1b, b'[', b'A'] => key(KeyCode::Up),
            [0x1b, b'[', b'B'] => key(KeyCode::Down),
            [0x1b, b'[', b'C'] => key(KeyCode::Right),
            [0x1b, b'[', b'D'] => key(KeyCode::Left),
            [0x1b, b'[', b'H'] => key(KeyCode::Home),
            [0x1b, b'[', b'F'] => key(KeyCode::End),
            // Tilde sequences
            [0x1b, b'[', b'2', b'~'] => key(KeyCode::Insert),
            [0x1b, b'[', b'3', b'~'] => key(KeyCode::Delete),
            [0x1b, b'[', b'5', b'~'] => key(KeyCode::PageUp),
            [0x1b, b'[', b'6', b'~'] => key(KeyCode::PageDown),
            // F1-F4 via SS3
            [0x1b, b'O', b'P'] => key(KeyCode::F(1)),
            [0x1b, b'O', b'Q'] => key(KeyCode::F(2)),
            [0x1b, b'O', b'R'] => key(KeyCode::F(3)),
            [0x1b, b'O', b'S'] => key(KeyCode::F(4)),
            // F1-F4 via CSI (some terminals)
            [0x1b, b'[', b'1', b'1', b'~'] => key(KeyCode::F(1)),
            [0x1b, b'[', b'1', b'2', b'~'] => key(KeyCode::F(2)),
            [0x1b, b'[', b'1', b'3', b'~'] => key(KeyCode::F(3)),
            [0x1b, b'[', b'1', b'4', b'~'] => key(KeyCode::F(4)),
            // F5-F12
            [0x1b, b'[', b'1', b'5', b'~'] => key(KeyCode::F(5)),
            [0x1b, b'[', b'1', b'7', b'~'] => key(KeyCode::F(6)),
            [0x1b, b'[', b'1', b'8', b'~'] => key(KeyCode::F(7)),
            [0x1b, b'[', b'1', b'9', b'~'] => key(KeyCode::F(8)),
            [0x1b, b'[', b'2', b'0', b'~'] => key(KeyCode::F(9)),
            [0x1b, b'[', b'2', b'1', b'~'] => key(KeyCode::F(10)),
            [0x1b, b'[', b'2', b'3', b'~'] => key(KeyCode::F(11)),
            [0x1b, b'[', b'2', b'4', b'~'] => key(KeyCode::F(12)),
            // Home/End alternative encodings
            [0x1b, b'[', b'1', b'~'] => key(KeyCode::Home),
            [0x1b, b'[', b'4', b'~'] => key(KeyCode::End),
            // Alt+char: ESC followed by a printable byte
            [0x1b, b] if *b >= 0x20 && *b < 0x7f => Event::Key(KeyEvent {
                code: KeyCode::Char(*b as char),
                modifiers: KeyModifiers::ALT,
            }),
            // Unrecognised — emit standalone Esc and reprocess remaining bytes
            // by falling back to just Esc (extra bytes are lost; acceptable).
            _ => key(KeyCode::Esc),
        }
    }

    /// Non-blocking poll: check if an event is ready within `timeout`.
    pub fn poll(timeout: Duration) -> io::Result<bool> {
        let chan = channel();
        let rx = chan.rx.lock().unwrap();
        match rx.recv_timeout(if timeout == Duration::ZERO {
            Duration::from_millis(1) // 1ms instead of 1ns — avoids busy-spin on Redox
        } else {
            timeout
        }) {
            Ok(ev) => {
                // Put it back by dropping rx and re-sending via the stored tx.
                // We can't un-recv, so we push to a thread-local buffer instead.
                PENDING.with(|cell| {
                    cell.borrow_mut().push_front(ev);
                });
                Ok(true)
            }
            Err(mpsc::RecvTimeoutError::Timeout) => Ok(false),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                Err(io::Error::new(io::ErrorKind::UnexpectedEof, "stdin reader exited"))
            }
        }
    }

    use std::collections::VecDeque;
    use std::cell::RefCell;

    thread_local! {
        static PENDING: RefCell<VecDeque<Event>> = RefCell::new(VecDeque::new());
    }

    /// Blocking read of the next event.
    pub fn read() -> io::Result<Event> {
        // Drain pending buffer first (items placed there by poll()).
        let pending = PENDING.with(|cell| cell.borrow_mut().pop_front());
        if let Some(ev) = pending {
            return Ok(ev);
        }

        let chan = channel();
        let rx = chan.rx.lock().unwrap();
        rx.recv().map_err(|_| {
            io::Error::new(io::ErrorKind::UnexpectedEof, "stdin reader exited")
        })
    }

    /// Convert a raw byte to an Event.
    /// Handles ASCII control codes, printable chars, and basic escape sequences.
    fn byte_to_event(byte: u8) -> Event {
        let (code, modifiers) = match byte {
            0x01..=0x1a => {
                // Ctrl+A..Ctrl+Z
                let c = (byte - 1 + b'a') as char;
                (KeyCode::Char(c), KeyModifiers::CONTROL)
            }
            0x1b => (KeyCode::Esc, KeyModifiers::empty()),
            0x7f => (KeyCode::Backspace, KeyModifiers::empty()),
            b'\r' | b'\n' => (KeyCode::Enter, KeyModifiers::empty()),
            b'\t' => (KeyCode::Tab, KeyModifiers::empty()),
            b if b >= 0x20 && b < 0x7f => (KeyCode::Char(b as char), KeyModifiers::empty()),
            _ => (KeyCode::Null, KeyModifiers::empty()),
        };
        Event::Key(KeyEvent { code, modifiers })
    }

    /// Convert a termion key event to our Event type (kept for reference).
    #[allow(dead_code)]
    fn termion_key_to_event(key: termion::event::Key) -> Event {
        use termion::event::Key as TKey;
        let (code, modifiers) = match key {
            TKey::Char('\n') | TKey::Char('\r') => (KeyCode::Enter, KeyModifiers::empty()),
            TKey::Char('\t') => (KeyCode::Tab, KeyModifiers::empty()),
            TKey::Char(c) => (KeyCode::Char(c), KeyModifiers::empty()),
            TKey::Alt(c) => (KeyCode::Char(c), KeyModifiers::ALT),
            TKey::Ctrl(c) => (KeyCode::Char(c), KeyModifiers::CONTROL),
            TKey::Backspace => (KeyCode::Backspace, KeyModifiers::empty()),
            TKey::Delete => (KeyCode::Delete, KeyModifiers::empty()),
            TKey::Up => (KeyCode::Up, KeyModifiers::empty()),
            TKey::Down => (KeyCode::Down, KeyModifiers::empty()),
            TKey::Left => (KeyCode::Left, KeyModifiers::empty()),
            TKey::Right => (KeyCode::Right, KeyModifiers::empty()),
            TKey::Home => (KeyCode::Home, KeyModifiers::empty()),
            TKey::End => (KeyCode::End, KeyModifiers::empty()),
            TKey::PageUp => (KeyCode::PageUp, KeyModifiers::empty()),
            TKey::PageDown => (KeyCode::PageDown, KeyModifiers::empty()),
            TKey::Insert => (KeyCode::Insert, KeyModifiers::empty()),
            TKey::Esc => (KeyCode::Esc, KeyModifiers::empty()),
            TKey::F(n) => (KeyCode::F(n), KeyModifiers::empty()),
            TKey::Null => (KeyCode::Null, KeyModifiers::empty()),
            _ => (KeyCode::Null, KeyModifiers::empty()),
        };
        Event::Key(KeyEvent { code, modifiers })
    }
}

// ---------------------------------------------------------------------------
// terminal module — mirrors crossterm::terminal
// ---------------------------------------------------------------------------

pub mod terminal {
    use super::*;

    /// Minimal crossterm Command trait shim.
    /// Uses `dyn Write` to avoid object-safety issues with `impl Trait` in traits.
    pub trait CrosstermCommand {
        fn write_ansi(&self, f: &mut dyn Write) -> io::Result<()>;
    }

    pub struct EnterAlternateScreen;
    pub struct LeaveAlternateScreen;

    impl CrosstermCommand for EnterAlternateScreen {
        fn write_ansi(&self, f: &mut dyn Write) -> io::Result<()> {
            f.write_all(b"\x1b[?1049h")
        }
    }
    impl CrosstermCommand for LeaveAlternateScreen {
        fn write_ansi(&self, f: &mut dyn Write) -> io::Result<()> {
            f.write_all(b"\x1b[?1049l")
        }
    }

    pub enum ClearType {
        All,
        AfterCursor,
        BeforeCursor,
        CurrentLine,
        UntilNewLine,
        Purge,
    }

    pub struct Clear(pub ClearType);

    impl CrosstermCommand for Clear {
        fn write_ansi(&self, f: &mut dyn Write) -> io::Result<()> {
            let seq: &[u8] = match self.0 {
                ClearType::All          => b"\x1b[2J",
                ClearType::AfterCursor  => b"\x1b[0J",
                ClearType::BeforeCursor => b"\x1b[1J",
                ClearType::CurrentLine  => b"\x1b[2K",
                ClearType::UntilNewLine => b"\x1b[0K",
                ClearType::Purge        => b"\x1b[3J",
            };
            f.write_all(seq)
        }
    }

    /// Return terminal size via termion.
    pub fn size() -> io::Result<(u16, u16)> {
        termion::terminal_size()
    }

    /// Enable raw mode via termion. Stored in a thread-local RAII guard.
    pub fn enable_raw_mode() -> io::Result<()> {
        RAW_MODE.with(|cell| {
            let mut guard = cell.borrow_mut();
            if guard.is_none() {
                use termion::raw::IntoRawMode;
                let raw = std::io::stdout()
                    .into_raw_mode()
                    .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;
                *guard = Some(raw);
            }
            Ok(())
        })
    }

    /// Disable raw mode by dropping the RAII guard.
    pub fn disable_raw_mode() -> io::Result<()> {
        RAW_MODE.with(|cell| {
            *cell.borrow_mut() = None;
            Ok(())
        })
    }

    use std::cell::RefCell;
    use termion::raw::RawTerminal;

    thread_local! {
        static RAW_MODE: RefCell<Option<RawTerminal<std::io::Stdout>>> = RefCell::new(None);
    }
}

// ---------------------------------------------------------------------------
// cursor module — mirrors crossterm::cursor
// ---------------------------------------------------------------------------

pub mod cursor {
    use super::*;
    use super::terminal::CrosstermCommand;

    pub struct Hide;
    pub struct Show;
    pub struct MoveTo(pub u16, pub u16);

    impl CrosstermCommand for Hide {
        fn write_ansi(&self, f: &mut dyn Write) -> io::Result<()> {
            f.write_all(b"\x1b[?25l")
        }
    }
    impl CrosstermCommand for Show {
        fn write_ansi(&self, f: &mut dyn Write) -> io::Result<()> {
            f.write_all(b"\x1b[?25h")
        }
    }
    impl CrosstermCommand for MoveTo {
        fn write_ansi(&self, f: &mut dyn Write) -> io::Result<()> {
            // ANSI is 1-based; crossterm MoveTo is 0-based.
            write!(f, "\x1b[{};{}H", self.1 + 1, self.0 + 1)
        }
    }
}

// ---------------------------------------------------------------------------
// style module — mirrors crossterm::style
// ---------------------------------------------------------------------------

pub mod style {
    use super::*;
    use super::terminal::CrosstermCommand;

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub enum Color {
        #[default]
        Reset,
        Black,
        DarkGrey,
        Red,
        DarkRed,
        Green,
        DarkGreen,
        Yellow,
        DarkYellow,
        Blue,
        DarkBlue,
        Magenta,
        DarkMagenta,
        Cyan,
        DarkCyan,
        White,
        Grey,
        Rgb { r: u8, g: u8, b: u8 },
        AnsiValue(u8),
    }

    impl Color {
        /// Map any Color variant to a basic 16-color ANSI index (0–15).
        ///
        /// The Redox VGA console only understands the original 16 ANSI colors.
        /// 256-color (`\x1b[38;5;Nm`) and true-color (`\x1b[38;2;R;G;Bm`)
        /// sequences are silently ignored, which causes the status bar and
        /// colored output to appear invisible. We therefore downgrade all
        /// colors to the nearest 16-color equivalent before emitting ANSI.
        fn to_ansi16(self) -> u8 {
            match self {
                // Named 16-color variants map directly.
                Color::Black       => 0,
                Color::DarkRed     => 1,
                Color::DarkGreen   => 2,
                Color::DarkYellow  => 3,
                Color::DarkBlue    => 4,
                Color::DarkMagenta => 5,
                Color::DarkCyan    => 6,
                Color::Grey        => 7,
                Color::DarkGrey    => 8,
                Color::Red         => 9,
                Color::Green       => 10,
                Color::Yellow      => 11,
                Color::Blue        => 12,
                Color::Magenta     => 13,
                Color::Cyan        => 14,
                Color::White       => 15,
                Color::Reset       => 9, // treated as bright-red sentinel; callers handle Reset specially.

                // 256-color palette: indices 0–15 are the standard 16 colors.
                // Indices 16–231 are the 6×6×6 color cube; map to nearest 16-color.
                // Indices 232–255 are a greyscale ramp.
                Color::AnsiValue(n) => match n {
                    0..=15 => n,
                    16..=231 => {
                        // Convert cube index to r,g,b (each 0–5) then pick nearest.
                        let idx = n - 16;
                        let b6 = idx % 6;
                        let g6 = (idx / 6) % 6;
                        let r6 = idx / 36;
                        // Scale to 0–255
                        let scale = |v: u8| if v == 0 { 0u8 } else { 55 + v * 40 };
                        rgb_to_ansi16(scale(r6), scale(g6), scale(b6))
                    }
                    232..=255 => {
                        // Greyscale: 232=dark, 255=light
                        let level = (n - 232) as u16 * 255 / 23;
                        let v = level as u8;
                        rgb_to_ansi16(v, v, v)
                    }
                    _ => 7,
                },

                // True color: find nearest of the 16 ANSI colors.
                Color::Rgb { r, g, b } => rgb_to_ansi16(r, g, b),
            }
        }

        fn to_fg_ansi(self, f: &mut dyn Write) -> io::Result<()> {
            if matches!(self, Color::Reset) {
                return write!(f, "\x1b[39m");
            }
            let n = self.to_ansi16();
            if n < 8 {
                write!(f, "\x1b[{}m", 30 + n)   // \x1b[30m..\x1b[37m
            } else {
                write!(f, "\x1b[{}m", 90 + n - 8) // \x1b[90m..\x1b[97m
            }
        }

        fn to_bg_ansi(self, f: &mut dyn Write) -> io::Result<()> {
            if matches!(self, Color::Reset) {
                return write!(f, "\x1b[49m");
            }
            let n = self.to_ansi16();
            if n < 8 {
                write!(f, "\x1b[{}m", 40 + n)    // \x1b[40m..\x1b[47m
            } else {
                write!(f, "\x1b[{}m", 100 + n - 8) // \x1b[100m..\x1b[107m
            }
        }
    }

    /// Map an RGB triplet to the nearest of the 16 standard ANSI colors.
    ///
    /// Uses squared Euclidean distance in RGB space against the canonical
    /// 16-color palette.
    fn rgb_to_ansi16(r: u8, g: u8, b: u8) -> u8 {
        // The 16 standard ANSI colors in (R, G, B).
        const PALETTE: [(u8, u8, u8); 16] = [
            (0,   0,   0  ), // 0  Black
            (170, 0,   0  ), // 1  DarkRed
            (0,   170, 0  ), // 2  DarkGreen
            (170, 170, 0  ), // 3  DarkYellow / Olive
            (0,   0,   170), // 4  DarkBlue
            (170, 0,   170), // 5  DarkMagenta
            (0,   170, 170), // 6  DarkCyan
            (170, 170, 170), // 7  Grey / LightGrey
            (85,  85,  85 ), // 8  DarkGrey
            (255, 85,  85 ), // 9  Red (bright)
            (85,  255, 85 ), // 10 Green (bright)
            (255, 255, 85 ), // 11 Yellow (bright)
            (85,  85,  255), // 12 Blue (bright)
            (255, 85,  255), // 13 Magenta (bright)
            (85,  255, 255), // 14 Cyan (bright)
            (255, 255, 255), // 15 White (bright)
        ];
        let mut best = 0u8;
        let mut best_dist = u32::MAX;
        for (i, &(pr, pg, pb)) in PALETTE.iter().enumerate() {
            let dr = (r as i32) - (pr as i32);
            let dg = (g as i32) - (pg as i32);
            let db = (b as i32) - (pb as i32);
            let dist = (dr * dr + dg * dg + db * db) as u32;
            if dist < best_dist {
                best_dist = dist;
                best = i as u8;
            }
        }
        best
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Attribute {
        Reset,
        Bold,
        Italic,
        Underlined,
        SlowBlink,
        Reverse,
        Hidden,
        CrossedOut,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct Attributes(u16);

    impl Attributes {
        pub fn set(&mut self, attr: Attribute) { self.0 |= attr_bit(attr); }
        pub fn has(self, attr: Attribute) -> bool { self.0 & attr_bit(attr) != 0 }
    }

    fn attr_bit(a: Attribute) -> u16 {
        match a {
            Attribute::Reset        => 0,
            Attribute::Bold         => 1,
            Attribute::Italic       => 2,
            Attribute::Underlined   => 4,
            Attribute::SlowBlink    => 8,
            Attribute::Reverse      => 16,
            Attribute::Hidden       => 32,
            Attribute::CrossedOut   => 64,
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq, Default)]
    pub struct ContentStyle {
        pub foreground_color: Option<Color>,
        pub background_color: Option<Color>,
        pub underline_color: Option<Color>,
        pub attributes: Attributes,
    }

    pub struct SetStyle(pub ContentStyle);
    pub struct SetForegroundColor(pub Color);
    pub struct SetBackgroundColor(pub Color);
    pub struct SetAttribute(pub Attribute);
    pub struct ResetColor;
    pub struct Print<T: std::fmt::Display>(pub T);

    impl CrosstermCommand for ResetColor {
        fn write_ansi(&self, f: &mut dyn Write) -> io::Result<()> {
            f.write_all(b"\x1b[0m")
        }
    }

    impl<T: std::fmt::Display> CrosstermCommand for Print<T> {
        fn write_ansi(&self, f: &mut dyn Write) -> io::Result<()> {
            write!(f, "{}", self.0)
        }
    }

    impl CrosstermCommand for SetForegroundColor {
        fn write_ansi(&self, f: &mut dyn Write) -> io::Result<()> {
            self.0.to_fg_ansi(f)
        }
    }

    impl CrosstermCommand for SetBackgroundColor {
        fn write_ansi(&self, f: &mut dyn Write) -> io::Result<()> {
            self.0.to_bg_ansi(f)
        }
    }

    impl CrosstermCommand for SetAttribute {
        fn write_ansi(&self, f: &mut dyn Write) -> io::Result<()> {
            match self.0 {
                Attribute::Reset      => write!(f, "\x1b[0m"),
                Attribute::Bold       => write!(f, "\x1b[1m"),
                Attribute::Italic     => write!(f, "\x1b[3m"),
                Attribute::Underlined => write!(f, "\x1b[4m"),
                Attribute::SlowBlink  => write!(f, "\x1b[5m"),
                Attribute::Reverse    => write!(f, "\x1b[7m"),
                Attribute::Hidden     => write!(f, "\x1b[8m"),
                Attribute::CrossedOut => write!(f, "\x1b[9m"),
            }
        }
    }

    impl CrosstermCommand for SetStyle {
        fn write_ansi(&self, f: &mut dyn Write) -> io::Result<()> {
            f.write_all(b"\x1b[0m")?;
            if let Some(fg) = self.0.foreground_color { fg.to_fg_ansi(f)?; }
            if let Some(bg) = self.0.background_color { bg.to_bg_ansi(f)?; }
            if self.0.attributes.has(Attribute::Bold)        { write!(f, "\x1b[1m")?; }
            if self.0.attributes.has(Attribute::Italic)      { write!(f, "\x1b[3m")?; }
            if self.0.attributes.has(Attribute::Underlined)  { write!(f, "\x1b[4m")?; }
            if self.0.attributes.has(Attribute::SlowBlink)   { write!(f, "\x1b[5m")?; }
            if self.0.attributes.has(Attribute::Reverse)     { write!(f, "\x1b[7m")?; }
            if self.0.attributes.has(Attribute::Hidden)      { write!(f, "\x1b[8m")?; }
            if self.0.attributes.has(Attribute::CrossedOut)  { write!(f, "\x1b[9m")?; }
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// ExecutableCommand + QueueableCommand trait shims
// ---------------------------------------------------------------------------

use terminal::CrosstermCommand;

/// Mirror crossterm's `ExecutableCommand` — flush after writing the command.
pub trait ExecutableCommand {
    fn execute<C: CrosstermCommand>(&mut self, cmd: C) -> io::Result<&mut Self>;
}

/// Mirror crossterm's `QueueableCommand` — write without flushing.
pub trait QueueableCommand {
    fn queue<C: CrosstermCommand>(&mut self, cmd: C) -> io::Result<&mut Self>;
}

impl<W: Write> ExecutableCommand for W {
    fn execute<C: CrosstermCommand>(&mut self, cmd: C) -> io::Result<&mut Self> {
        cmd.write_ansi(self as &mut dyn Write)?;
        self.flush()?;
        Ok(self)
    }
}

impl<W: Write> QueueableCommand for W {
    fn queue<C: CrosstermCommand>(&mut self, cmd: C) -> io::Result<&mut Self> {
        cmd.write_ansi(self as &mut dyn Write)?;
        Ok(self)
    }
}
