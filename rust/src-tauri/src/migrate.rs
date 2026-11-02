//! First start of 2.x: bring over history, dictionary and settings from a PromptFlow 1.x (Python) folder.

use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;

const FILES: &[&str] = &["history.txt", "dictionary.txt", "settings.json"];

/// Where 1.x lives: its Start menu shortcut / startup entry on Windows, its app bundle on macOS.
fn find_old_folder() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("PROMPTFLOW_IMPORT_FROM") {
        return Some(PathBuf::from(dir));
    }
    candidates().into_iter().find(|dir| dir.join("app.pyw").exists() && dir.join("promptflow.py").exists())
}

#[cfg(target_os = "windows")]
fn candidates() -> Vec<PathBuf> {
    let mut found = Vec::new();
    // The shortcut's arguments hold the path of app.pyw, stored as UTF-16
    let app_pyw = Regex::new(r#"([A-Za-z]:\\[^"\x00]*?)\\app\.pyw"#).unwrap();
    let mut sources = Vec::new();
    if let Some(appdata) = std::env::var_os("APPDATA") {
        let link = Path::new(&appdata).join(r"Microsoft\Windows\Start Menu\Programs\PromptFlow.lnk");
        if let Ok(bytes) = fs::read(link) {
            // The string may start at an odd offset, so read it with both alignments
            for start in [0, 1] {
                let wide: Vec<u16> = bytes[start..].as_chunks::<2>().0.iter().map(|b| u16::from_le_bytes(*b)).collect();
                sources.push(String::from_utf16_lossy(&wide));
            }
        }
    }
    // 1.x's "Start with Windows" entry: "pythonw.exe" "...\app.pyw" --hidden
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000; // no console flashing up
    if let Ok(out) = std::process::Command::new("reg")
        .args(["query", r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run", "/v", "PromptFlow"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
    {
        sources.push(String::from_utf8_lossy(&out.stdout).to_string());
    }
    for source in sources {
        for cap in app_pyw.captures_iter(&source) {
            found.push(PathBuf::from(&cap[1]));
        }
    }
    found
}

#[cfg(target_os = "macos")]
fn candidates() -> Vec<PathBuf> {
    // 1.x wrote "<python>\n<folder>\n<arch>" next to its launcher
    let spec = dirs_home().join("Applications/PromptFlow.app/Contents/Resources/launcher.txt");
    fs::read_to_string(spec)
        .ok()
        .and_then(|s| s.lines().nth(1).map(PathBuf::from))
        .into_iter()
        .collect()
}

#[cfg(target_os = "macos")]
fn dirs_home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default()
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn candidates() -> Vec<PathBuf> {
    Vec::new()
}

/// Copies 1.x's files into `data_dir` once, before anything else creates them. Returns the old folder.
pub fn import_once(data_dir: &Path) -> Option<PathBuf> {
    let marker = data_dir.join(".imported");
    if marker.exists() || FILES.iter().any(|f| data_dir.join(f).exists()) {
        return None;
    }
    let _ = fs::create_dir_all(data_dir);
    let _ = fs::write(&marker, "");
    let old = find_old_folder()?;
    let mut copied = false;
    for file in FILES {
        if fs::copy(old.join(file), data_dir.join(file)).is_ok() {
            copied = true;
        }
    }
    copied.then_some(old)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run with --ignored on a machine that has 1.x installed
    #[test]
    #[ignore]
    fn finds_this_machines_install() {
        println!("1.x candidates: {:?}", candidates());
        assert!(find_old_folder().is_some());
    }

    #[test]
    fn copies_the_files_once() {
        let root = std::env::temp_dir().join(format!("pf-migrate-{}", std::process::id()));
        let (old, new) = (root.join("old"), root.join("new"));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&old).unwrap();
        for f in ["app.pyw", "promptflow.py"] {
            fs::write(old.join(f), "").unwrap();
        }
        fs::write(old.join("history.txt"), "[2026-11-01 10:00:00] hello\n").unwrap();
        fs::write(old.join("settings.json"), r#"{"LANGUAGE": "tr"}"#).unwrap();
        std::env::set_var("PROMPTFLOW_IMPORT_FROM", &old);
        assert_eq!(import_once(&new), Some(old.clone()));
        assert!(new.join("history.txt").exists() && new.join("settings.json").exists());
        assert_eq!(import_once(&new), None); // only the first time
        std::env::remove_var("PROMPTFLOW_IMPORT_FROM");
    }
}
