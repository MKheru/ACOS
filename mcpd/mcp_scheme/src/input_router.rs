//! Input router — keyboard event routing, hotkeys, shell integration

use std::sync::{Arc, Mutex};

use crate::konsole_handler::Konsole;

// ---------------------------------------------------------------------------
// Key types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum Key {
    Char(char),
    Enter,
    Backspace,
    Tab,
    Escape,
    Up,
    Down,
    Left,
    Right,
    F(u8),
    Delete,
    Home,
    End,
    PageUp,
    PageDown,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Modifiers {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

impl Modifiers {
    pub fn none() -> Self {
        Modifiers { ctrl: false, alt: false, shift: false }
    }

    pub fn ctrl_alt() -> Self {
        Modifiers { ctrl: true, alt: true, shift: false }
    }
}

#[derive(Debug, Clone)]
pub struct KeyEvent {
    pub key: Key,
    pub modifiers: Modifiers,
}

// ---------------------------------------------------------------------------
// InputAction — result of processing a key event
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq)]
pub enum InputAction {
    /// Hotkey was consumed by the input router
    Consumed,
    /// Key was forwarded to the focused konsole (by id)
    Forwarded(u32),
    /// No konsole is currently focused
    NoFocusedKonsole,
    /// Request to create a new user konsole
    CreateKonsole,
}

// ---------------------------------------------------------------------------
// InputRouter
// ---------------------------------------------------------------------------

pub struct InputRouter {
    konsole_state: Arc<Mutex<Vec<Konsole>>>,
    focused_konsole: Arc<Mutex<u32>>,
}

impl InputRouter {
    pub fn new(
        konsole_state: Arc<Mutex<Vec<Konsole>>>,
        focused_konsole: Arc<Mutex<u32>>,
    ) -> Self {
        InputRouter { konsole_state, focused_konsole }
    }

    /// Process a key event: check hotkeys first, then forward to focused konsole.
    pub fn process_key(&self, event: KeyEvent) -> InputAction {
        // 1. Check hotkeys (Ctrl+Alt combos)
        if event.modifiers.ctrl && event.modifiers.alt {
            return self.handle_hotkey(&event.key);
        }

        // 2. Forward to focused konsole
        self.forward_to_focused(&event)
    }

    fn handle_hotkey(&self, key: &Key) -> InputAction {
        match key {
            // Ctrl+Alt+0..9 → switch to konsole N
            Key::Char(c @ '0'..='9') => {
                let target_id = (*c as u32) - ('0' as u32);
                self.switch_focus(target_id);
                InputAction::Consumed
            }
            // Ctrl+Alt+Right → focus next konsole
            Key::Right => {
                self.cycle_focus(1);
                InputAction::Consumed
            }
            // Ctrl+Alt+Left → focus previous konsole
            Key::Left => {
                self.cycle_focus(-1);
                InputAction::Consumed
            }
            // Ctrl+Alt+C → create new user konsole
            Key::Char('c') | Key::Char('C') => InputAction::CreateKonsole,
            _ => InputAction::Consumed,
        }
    }

    fn switch_focus(&self, target_id: u32) {
        let konsoles = self.konsole_state.lock().unwrap_or_else(|e| e.into_inner());
        // Only switch if target konsole exists
        if konsoles.iter().any(|k| k.id == target_id) {
            let mut focused = self.focused_konsole.lock().unwrap_or_else(|e| e.into_inner());
            *focused = target_id;
        }
    }

    fn cycle_focus(&self, direction: i32) {
        let konsoles = self.konsole_state.lock().unwrap_or_else(|e| e.into_inner());
        if konsoles.is_empty() {
            return;
        }

        let focused = *self.focused_konsole.lock().unwrap_or_else(|e| e.into_inner());

        // Find current index
        let current_idx = konsoles.iter().position(|k| k.id == focused).unwrap_or(0);
        let len = konsoles.len() as i32;
        let next_idx = ((current_idx as i32 + direction) % len + len) % len;
        let next_id = konsoles[next_idx as usize].id;
        drop(konsoles);

        let mut f = self.focused_konsole.lock().unwrap_or_else(|e| e.into_inner());
        *f = next_id;
    }

    fn forward_to_focused(&self, event: &KeyEvent) -> InputAction {
        let focused_id = *self.focused_konsole.lock().unwrap_or_else(|e| e.into_inner());

        let mut konsoles = self.konsole_state.lock().unwrap_or_else(|e| e.into_inner());
        let konsole = konsoles.iter_mut().find(|k| k.id == focused_id);

        match konsole {
            Some(k) => {
                let text = key_to_string(event);
                if !text.is_empty() {
                    k.write_data(&text);
                }
                InputAction::Forwarded(focused_id)
            }
            None => InputAction::NoFocusedKonsole,
        }
    }
}

/// Convert a KeyEvent into the string/escape sequence to write to a konsole.
fn key_to_string(event: &KeyEvent) -> String {
    match &event.key {
        Key::Char(c) => {
            if event.modifiers.ctrl && !event.modifiers.alt {
                // Ctrl+A..Z → 0x01..0x1A
                if c.is_ascii_alphabetic() {
                    let ctrl_code = (*c as u8 & 0x1f) as char;
                    return ctrl_code.to_string();
                }
            }
            c.to_string()
        }
        Key::Enter => "\r".into(),
        Key::Backspace => "\x7f".into(),
        Key::Tab => "\t".into(),
        Key::Escape => "\x1b".into(),
        Key::Up => "\x1b[A".into(),
        Key::Down => "\x1b[B".into(),
        Key::Right => "\x1b[C".into(),
        Key::Left => "\x1b[D".into(),
        Key::Delete => "\x1b[3~".into(),
        Key::Home => "\x1b[H".into(),
        Key::End => "\x1b[F".into(),
        Key::PageUp => "\x1b[5~".into(),
        Key::PageDown => "\x1b[6~".into(),
        Key::F(n) => match n {
            1 => "\x1b[11~".into(),
            2 => "\x1b[12~".into(),
            3 => "\x1b[13~".into(),
            4 => "\x1b[14~".into(),
            5 => "\x1b[15~".into(),
            6 => "\x1b[17~".into(),
            7 => "\x1b[18~".into(),
            8 => "\x1b[19~".into(),
            9 => "\x1b[20~".into(),
            10 => "\x1b[21~".into(),
            11 => "\x1b[23~".into(),
            12 => "\x1b[24~".into(),
            _ => String::new(),
        },
    }
}

// ---------------------------------------------------------------------------
// Redox keyboard reading (dual-mode)
// ---------------------------------------------------------------------------

#[cfg(target_os = "redox")]
pub fn read_keyboard_event() -> Option<KeyEvent> {
    // TODO: Open display:input scheme, read raw keyboard events,
    // parse into KeyEvent. Redox keyboard events come as orbital::KeyEvent
    // or raw scancodes.
    None
}

#[cfg(not(target_os = "redox"))]
pub fn read_keyboard_event() -> Option<KeyEvent> {
    // Mock: no real keyboard in host-test
    None
}

// ---------------------------------------------------------------------------
// Shell pipe integration (ptyd stub)
// ---------------------------------------------------------------------------

/// Shell pipe integration for user konsoles.
/// On Redox, this would create a ptyd pair and connect ion's stdin/stdout.
/// For now, provide the interface — full ptyd integration comes with QEMU testing.
pub struct ShellPipe {
    pub konsole_id: u32,
    // #[cfg(target_os = "redox")]
    // pub pty_master: File,
}

impl ShellPipe {
    pub fn new(konsole_id: u32) -> Self {
        ShellPipe { konsole_id }
    }

    /// Write a key event to the shell's stdin (via pty master)
    pub fn write_key(&self, _event: &KeyEvent) -> Result<(), &'static str> {
        // TODO: Implement with ptyd on Redox
        Ok(())
    }

    /// Read output from the shell's stdout (via pty master)
    pub fn read_output(&self) -> Result<String, &'static str> {
        // TODO: Implement with ptyd on Redox
        Ok(String::new())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::konsole_handler::KonsoleType;

    fn make_router_with_konsoles(ids: &[u32]) -> InputRouter {
        let mut konsoles = Vec::new();
        for &id in ids {
            konsoles.push(Konsole::new(id, KonsoleType::User, "test".into(), 80, 24));
        }
        let state = Arc::new(Mutex::new(konsoles));
        let focused = Arc::new(Mutex::new(ids.first().copied().unwrap_or(0)));
        InputRouter::new(state, focused)
    }

    #[test]
    fn test_hotkey_switch_konsole() {
        let router = make_router_with_konsoles(&[0, 1, 2]);
        let event = KeyEvent {
            key: Key::Char('2'),
            modifiers: Modifiers::ctrl_alt(),
        };
        let action = router.process_key(event);
        assert_eq!(action, InputAction::Consumed);
        assert_eq!(*router.focused_konsole.lock().unwrap(), 2);
    }

    #[test]
    fn test_hotkey_switch_nonexistent() {
        let router = make_router_with_konsoles(&[0, 1]);
        let event = KeyEvent {
            key: Key::Char('5'),
            modifiers: Modifiers::ctrl_alt(),
        };
        router.process_key(event);
        // Focus should remain on 0 (konsole 5 doesn't exist)
        assert_eq!(*router.focused_konsole.lock().unwrap(), 0);
    }

    #[test]
    fn test_hotkey_cycle_right() {
        let router = make_router_with_konsoles(&[0, 1, 2]);
        let event = KeyEvent {
            key: Key::Right,
            modifiers: Modifiers::ctrl_alt(),
        };
        router.process_key(event);
        assert_eq!(*router.focused_konsole.lock().unwrap(), 1);
        router.process_key(KeyEvent { key: Key::Right, modifiers: Modifiers::ctrl_alt() });
        assert_eq!(*router.focused_konsole.lock().unwrap(), 2);
        // Wrap around
        router.process_key(KeyEvent { key: Key::Right, modifiers: Modifiers::ctrl_alt() });
        assert_eq!(*router.focused_konsole.lock().unwrap(), 0);
    }

    #[test]
    fn test_hotkey_cycle_left() {
        let router = make_router_with_konsoles(&[0, 1, 2]);
        let event = KeyEvent {
            key: Key::Left,
            modifiers: Modifiers::ctrl_alt(),
        };
        router.process_key(event);
        // Wraps from 0 to 2
        assert_eq!(*router.focused_konsole.lock().unwrap(), 2);
    }

    #[test]
    fn test_hotkey_create_konsole() {
        let router = make_router_with_konsoles(&[0]);
        let event = KeyEvent {
            key: Key::Char('c'),
            modifiers: Modifiers::ctrl_alt(),
        };
        assert_eq!(router.process_key(event), InputAction::CreateKonsole);
    }

    #[test]
    fn test_forward_to_focused() {
        let router = make_router_with_konsoles(&[0, 1]);
        let event = KeyEvent {
            key: Key::Char('a'),
            modifiers: Modifiers::none(),
        };
        let action = router.process_key(event);
        assert_eq!(action, InputAction::Forwarded(0));

        // Verify the character was written to konsole 0
        let konsoles = router.konsole_state.lock().unwrap();
        let k = &konsoles[0];
        assert!(k.dirty);
    }

    #[test]
    fn test_no_focused_konsole() {
        let state = Arc::new(Mutex::new(Vec::<Konsole>::new()));
        let focused = Arc::new(Mutex::new(99)); // non-existent
        let router = InputRouter::new(state, focused);

        let event = KeyEvent {
            key: Key::Char('x'),
            modifiers: Modifiers::none(),
        };
        assert_eq!(router.process_key(event), InputAction::NoFocusedKonsole);
    }

    #[test]
    fn test_ctrl_key_forward() {
        let router = make_router_with_konsoles(&[0]);
        let event = KeyEvent {
            key: Key::Char('c'),
            modifiers: Modifiers { ctrl: true, alt: false, shift: false },
        };
        let action = router.process_key(event);
        assert_eq!(action, InputAction::Forwarded(0));
    }

    #[test]
    fn test_key_to_string_special_keys() {
        assert_eq!(key_to_string(&KeyEvent { key: Key::Enter, modifiers: Modifiers::none() }), "\r");
        assert_eq!(key_to_string(&KeyEvent { key: Key::Up, modifiers: Modifiers::none() }), "\x1b[A");
        assert_eq!(key_to_string(&KeyEvent { key: Key::F(1), modifiers: Modifiers::none() }), "\x1b[11~");
    }

    #[test]
    fn test_shell_pipe_stub() {
        let pipe = ShellPipe::new(0);
        let event = KeyEvent { key: Key::Char('a'), modifiers: Modifiers::none() };
        assert!(pipe.write_key(&event).is_ok());
        assert_eq!(pipe.read_output().unwrap(), "");
    }

    #[test]
    fn test_read_keyboard_mock() {
        assert!(read_keyboard_event().is_none());
    }
}
