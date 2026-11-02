//! Plain-text helpers: personal dictionary, history file, Markdown cleanup, Whisper hallucinations.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use regex::Regex;

pub const DICTIONARY_TEMPLATE: &str = "# Personal dictionary: one \"wrong -> right\" per line.
# Changes apply as soon as you save; no restart needed.
# The right-hand words are also given to Whisper as hints, so it tries to spell them right in the first place.
clod -> Claude
next js -> Next.js
";

/// Words you use often, given to Whisper as a hint so it spells them right.
pub const INITIAL_PROMPT: &str =
    "Claude, prompt, React, Next.js, TypeScript, Python, API, commit, deploy, frontend, backend.";

/// Classic phrases Whisper hallucinates on silence.
const HALLUCINATIONS: &[&str] = &[
    "altyazı m.k.", "altyazı m.k", "izlediğiniz için teşekkürler.", "teşekkürler.",
    "thank you.", "thanks for watching!", "thank you for watching.", "you",
];

pub fn is_hallucination(text: &str) -> bool {
    HALLUCINATIONS.contains(&text.trim().to_lowercase().as_str())
}

type Rules = Vec<(Regex, String)>;

/// dictionary.txt, reloaded whenever the file changes.
pub struct Dictionary {
    path: PathBuf,
    cache: Mutex<(Option<SystemTime>, Rules)>,
}

impl Dictionary {
    pub fn new(path: PathBuf) -> Self {
        if !path.exists() {
            let _ = fs::write(&path, DICTIONARY_TEMPLATE);
        }
        Self { path, cache: Mutex::new((None, Vec::new())) }
    }

    fn rules(&self) -> Rules {
        let mut cache = self.cache.lock().unwrap();
        let mtime = fs::metadata(&self.path).and_then(|m| m.modified()).ok();
        if mtime != cache.0 || mtime.is_none() {
            cache.1 = fs::read_to_string(&self.path).map(|s| parse_dictionary(&s)).unwrap_or_default();
            cache.0 = mtime;
        }
        cache.1.clone()
    }

    pub fn apply(&self, text: &str) -> String {
        let mut text = text.to_string();
        for (pattern, right) in self.rules() {
            text = pattern.replace_all(&text, regex::NoExpand(&right)).into_owned();
        }
        text
    }

    pub fn hint_words(&self) -> Vec<String> {
        let mut words: Vec<String> = Vec::new();
        for (_, right) in self.rules() {
            if !right.is_empty() && !words.contains(&right) {
                words.push(right);
            }
        }
        words
    }
}

pub fn parse_dictionary(source: &str) -> Rules {
    let mut rules = Vec::new();
    for line in source.trim_start_matches('\u{feff}').lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((wrong, right)) = line.split_once("->") else { continue };
        let (wrong, right) = (wrong.trim(), right.trim());
        if wrong.is_empty() {
            continue;
        }
        // Whole words only: "clod" must not change "clodx". (regex has no look-behind, so \b where it applies)
        let edge = |c: Option<char>| if c.is_some_and(|c| c.is_alphanumeric() || c == '_') { r"\b" } else { "" };
        let pattern = format!(
            "(?i){}{}{}",
            edge(wrong.chars().next()),
            regex::escape(wrong),
            edge(wrong.chars().last())
        );
        if let Ok(re) = Regex::new(&pattern) {
            rules.push((re, right.to_string()));
        }
    }
    rules
}

/// The Whisper hint: the built-in terms plus the dictionary's right-hand words.
pub fn build_prompt(dictionary: &Dictionary) -> String {
    let extra: Vec<String> =
        dictionary.hint_words().into_iter().filter(|w| !INITIAL_PROMPT.contains(w.as_str())).collect();
    format!("{} {}", INITIAL_PROMPT, extra.join(", ")).trim().to_string()
}

/// Turns any Markdown the model still produces into plain text that pastes cleanly into any text box.
pub fn strip_markdown(text: &str) -> String {
    let heading = Regex::new(r"^\s*#{1,6}\s+").unwrap();
    let bullet = Regex::new(r"^\s*(?:[*+•]|-)\s+").unwrap();
    let bold = Regex::new(r"\*\*(.+?)\*\*|__(.+?)__").unwrap();
    // Italics: *word* but not 2 * 3
    let italic = Regex::new(r"(^|[^\w*])\*([^\s*](?:[^*]*[^\s*])?)\*($|[^\w*])").unwrap();
    let mut lines = Vec::new();
    for line in text.lines() {
        let line = heading.replace(line, "");
        let line = bullet.replace(&line, "- ");
        let line = bold.replace_all(&line, |c: &regex::Captures| {
            c.get(1).or_else(|| c.get(2)).map_or("", |m| m.as_str()).to_string()
        });
        let line = italic.replace_all(&line, "$1$2$3");
        lines.push(line.trim_end().to_string());
    }
    let joined = lines.join("\n");
    Regex::new(r"\n{3,}").unwrap().replace_all(&joined, "\n\n").trim().to_string()
}

/// history.txt: "[time] text", continuation lines indented by 4 spaces, "    (raw: ...)" when cleaned.
pub fn save_history(path: &Path, raw: &str, text: &str) {
    let mut entry = format!("[{}] {}\n", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"), text.replace('\n', "\n    "));
    if raw != text {
        entry.push_str(&format!("    (raw: {raw})\n"));
    }
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = f.write_all(entry.as_bytes());
    }
}

/// The latest dictations, oldest first: [(time, text)].
pub fn read_history(path: &Path, limit: usize) -> Vec<(String, String)> {
    let Ok(source) = fs::read_to_string(path) else { return Vec::new() };
    let mut items: Vec<(String, String)> = Vec::new();
    for line in source.trim_start_matches('\u{feff}').lines() {
        if let Some(rest) = line.strip_prefix('[') {
            if let Some((time, text)) = rest.split_once("] ") {
                items.push((time.to_string(), text.to_string()));
                continue;
            }
        }
        if let Some(cont) = line.strip_prefix("    ") {
            if !cont.starts_with("(raw: ") {
                if let Some(last) = items.last_mut() {
                    last.1.push('\n');
                    last.1.push_str(cont);
                }
            }
        }
    }
    let start = items.len().saturating_sub(limit);
    items.split_off(start)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_becomes_plain_text() {
        let bad = "Create this:\n\n*   **Setting:** a Turkish home.\n*   **Characters:**\n    *   Mom (enters).\n\
                   ## Notes\nThis is *important*, 2 * 3 = 6, file: my_file_name.py";
        assert_eq!(
            strip_markdown(bad),
            "Create this:\n\n- Setting: a Turkish home.\n- Characters:\n- Mom (enters).\n\
             Notes\nThis is important, 2 * 3 = 6, file: my_file_name.py"
        );
    }

    #[test]
    fn dictionary_replaces_whole_words() {
        let rules = parse_dictionary("# c\nclod -> Claude\nnext js -> Next.js\n");
        let mut text = "ask clod about next js, not clodx".to_string();
        for (re, right) in rules {
            text = re.replace_all(&text, regex::NoExpand(&right)).into_owned();
        }
        assert_eq!(text, "ask Claude about Next.js, not clodx");
    }

    #[test]
    fn history_roundtrip() {
        let dir = std::env::temp_dir().join(format!("pf-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("history.txt");
        let _ = fs::remove_file(&path);
        save_history(&path, "make it red no blue", "Make it blue.");
        save_history(&path, "todo app", "Build a todo app:\n- Save tasks.\n- Dark mode.");
        let items: Vec<String> = read_history(&path, 30).into_iter().map(|(_, t)| t).collect();
        assert_eq!(items, vec!["Make it blue.", "Build a todo app:\n- Save tasks.\n- Dark mode."]);
    }

    #[test]
    fn hallucinations() {
        assert!(is_hallucination(" Thank you. "));
        assert!(!is_hallucination("Thank you for the code"));
    }
}
