//! Puts the text on the clipboard, presses Ctrl+V / Cmd+V and restores the previous clipboard.

use std::thread::sleep;
use std::time::Duration;

use anyhow::Result;
use arboard::Clipboard;
use enigo::{Direction, Enigo, Key, Keyboard, Settings};

/// The V key by position, so the shortcut works whatever the keyboard layout.
#[cfg(target_os = "windows")]
const KEY_V: Key = Key::Other(0x56); // VK_V
#[cfg(target_os = "macos")]
const KEY_V: Key = Key::Other(9); // kVK_ANSI_V
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
const KEY_V: Key = Key::Unicode('v');

#[cfg(target_os = "macos")]
const MODIFIER: Key = Key::Meta;
#[cfg(not(target_os = "macos"))]
const MODIFIER: Key = Key::Control;

pub fn paste(text: &str, restore: bool) -> Result<()> {
    let mut clipboard = Clipboard::new()?;
    let old = if restore { clipboard.get_text().ok() } else { None };
    clipboard.set_text(text)?;

    let mut enigo = Enigo::new(&Settings::default())?;
    enigo.key(MODIFIER, Direction::Press)?;
    let pressed = enigo.key(KEY_V, Direction::Click);
    sleep(Duration::from_millis(30));
    enigo.key(MODIFIER, Direction::Release)?;
    pressed?;

    if let Some(old) = old {
        sleep(Duration::from_millis(300)); // wait until the target app has read the clipboard
        clipboard.set_text(old)?;
    }
    Ok(())
}
