//! Which languages this person uses: the interface language of the OS, then the other languages in their
//! preferred-languages list and keyboard layouts. Read locally; never leaves the machine.

use serde::Serialize;

#[derive(Serialize, Clone, Debug)]
pub struct Languages {
    /// Two-letter code of the OS interface language, e.g. "en", "pt"
    pub ui: String,
    /// Every language found, UI language first, without duplicates
    pub detected: Vec<String>,
}

/// "pt-BR" → "pt", "zh-Hans-CN" → "zh"
pub fn primary(tag: &str) -> String {
    tag.split(['-', '_']).next().unwrap_or("").to_lowercase()
}

#[cfg(target_os = "windows")]
fn keyboard_languages() -> Vec<String> {
    use windows::Win32::Globalization::{LCIDToLocaleName, LOCALE_ALLOW_NEUTRAL_NAMES};
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyboardLayoutList, HKL};

    let mut layouts = [HKL::default(); 32];
    let n = unsafe { GetKeyboardLayoutList(Some(&mut layouts)) } as usize;
    let mut out = Vec::new();
    for hkl in &layouts[..n.min(layouts.len())] {
        let langid = (hkl.0 as usize & 0xFFFF) as u32; // low word: the layout's language
        let mut buf = [0u16; 85];
        let len = unsafe { LCIDToLocaleName(langid, Some(&mut buf), LOCALE_ALLOW_NEUTRAL_NAMES) } as usize;
        if len > 1 {
            out.push(primary(&String::from_utf16_lossy(&buf[..len - 1])));
        }
    }
    out
}

#[cfg(not(target_os = "windows"))]
fn keyboard_languages() -> Vec<String> {
    Vec::new() // macOS: the preferred-languages list already covers it
}

pub fn detect() -> Languages {
    let mut detected: Vec<String> = Vec::new();
    for tag in sys_locale::get_locales().chain(keyboard_languages()) {
        let code = primary(&tag);
        if code.len() >= 2 && !detected.contains(&code) {
            detected.push(code);
        }
    }
    if detected.is_empty() {
        detected.push("en".into());
    }
    Languages { ui: detected[0].clone(), detected }
}

#[cfg(test)]
mod tests {
    #[test]
    fn primary_codes() {
        assert_eq!(super::primary("pt-BR"), "pt");
        assert_eq!(super::primary("zh-Hans-CN"), "zh");
        assert_eq!(super::primary("en_US"), "en");
        let langs = super::detect();
        println!("{langs:?}");
        assert!(!langs.ui.is_empty());
    }
}
