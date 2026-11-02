//! Hold-to-talk hotkey. Uses handy-keys' raw keyboard listener (a low-level hook on Windows, an event tap
//! on macOS) so a single modifier like Right Ctrl / Right Command works, with separate down and up.

use std::sync::{Arc, Mutex};
use std::thread;

use handy_keys::{Key, KeyboardListener, Modifiers};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Hotkey {
    Modifier(Modifiers),
    Key(Key),
}

#[derive(Debug, PartialEq)]
pub enum HotkeyEvent {
    Down,
    Up,
    /// Another key went down (e.g. the hotkey is part of Ctrl+C): cancel the dictation
    Other,
}

/// (setting name, label) per OS. Setting names match PromptFlow 1.x's settings.json.
#[cfg(target_os = "macos")]
pub const HOTKEYS: &[(&str, &str)] = &[
    ("cmd_r", "Right Command"), ("alt_r", "Right Option"), ("ctrl_r", "Right Control"),
    ("shift_r", "Right Shift"), ("fn", "Fn (Globe)"), ("f8", "F8"), ("f9", "F9"), ("f10", "F10"), ("f13", "F13"),
];
#[cfg(not(target_os = "macos"))]
pub const HOTKEYS: &[(&str, &str)] = &[
    ("ctrl_r", "Right Ctrl"), ("shift_r", "Right Shift"), ("f8", "F8"), ("f9", "F9"), ("f10", "F10"),
    ("scroll_lock", "Scroll Lock"), ("pause", "Pause"),
];

#[cfg(target_os = "macos")]
pub const DEFAULT_HOTKEY: &str = "cmd_r";
#[cfg(not(target_os = "macos"))]
pub const DEFAULT_HOTKEY: &str = "ctrl_r";

pub fn parse(name: &str) -> Option<Hotkey> {
    Some(match name {
        "ctrl_r" => Hotkey::Modifier(Modifiers::CTRL_RIGHT),
        "shift_r" => Hotkey::Modifier(Modifiers::SHIFT_RIGHT),
        "alt_r" => Hotkey::Modifier(Modifiers::OPT_RIGHT),
        "cmd_r" => Hotkey::Modifier(Modifiers::CMD_RIGHT),
        "fn" => Hotkey::Modifier(Modifiers::FN),
        "f8" => Hotkey::Key(Key::F8),
        "f9" => Hotkey::Key(Key::F9),
        "f10" => Hotkey::Key(Key::F10),
        "f13" => Hotkey::Key(Key::F13),
        "scroll_lock" => Hotkey::Key(Key::ScrollLock),
        "pause" => Hotkey::Key(Key::Pause),
        _ => return None,
    })
}

pub fn is_valid(name: &str) -> bool {
    HOTKEYS.iter().any(|(n, _)| *n == name)
}

/// What a raw keyboard event means for the hotkey.
pub fn classify(hotkey: Hotkey, event: &handy_keys::KeyEvent) -> Option<HotkeyEvent> {
    match hotkey {
        Hotkey::Modifier(m) => {
            if event.changed_modifier == Some(m) {
                return Some(if event.is_key_down { HotkeyEvent::Down } else { HotkeyEvent::Up });
            }
        }
        Hotkey::Key(k) => {
            if event.key == Some(k) {
                return Some(if event.is_key_down { HotkeyEvent::Down } else { HotkeyEvent::Up });
            }
        }
    }
    event.is_key_down.then_some(HotkeyEvent::Other)
}

/// Starts listening; `on_event` runs on the listener's thread.
pub fn listen(hotkey: Arc<Mutex<Hotkey>>, on_event: impl Fn(HotkeyEvent) + Send + 'static) -> Result<(), String> {
    let listener = KeyboardListener::new().map_err(|e| e.to_string())?;
    thread::spawn(move || {
        while let Ok(event) = listener.recv() {
            let current = *hotkey.lock().unwrap();
            if let Some(e) = classify(current, &event) {
                on_event(e);
            }
        }
    });
    Ok(())
}
