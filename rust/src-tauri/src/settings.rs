//! settings.json. The keys are the same as PromptFlow 1.x's, so an old settings file keeps working.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::hotkey;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE", default)]
pub struct Settings {
    pub hotkey: String,
    /// None = auto-detect
    pub language: Option<String>,
    /// Part of a microphone name; None = system default
    pub mic: Option<String>,
    /// "clean", "prompt", or None for raw
    pub ai_mode: Option<String>,
    pub ai_model: String,
    pub sounds: bool,
    /// File name of the Whisper model in the models folder
    pub speech_model: String,
    pub restore_clipboard: bool,
    /// Languages the person speaks, for model recommendations; None = the OS interface language
    pub speech_languages: Option<Vec<String>>,
    /// Language of PromptFlow's own window; None = follow the OS
    pub ui_language: Option<String>,
    /// The first-run setup guide has been completed
    pub onboarded: bool,
    /// Shorter presses are ignored
    pub min_seconds: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            hotkey: hotkey::DEFAULT_HOTKEY.into(),
            language: None,
            mic: None,
            ai_mode: Some("clean".into()),
            ai_model: "gemma3:12b".into(),
            sounds: false,
            speech_model: "ggml-large-v3-turbo-q5_0.bin".into(),
            restore_clipboard: true,
            speech_languages: None,
            ui_language: None,
            onboarded: false,
            min_seconds: 0.4,
        }
    }
}

impl Settings {
    pub fn load(path: &Path) -> Self {
        let mut settings: Settings = fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(s.trim_start_matches('\u{feff}')).ok())
            .unwrap_or_default();
        if !hotkey::is_valid(&settings.hotkey) {
            settings.hotkey = hotkey::DEFAULT_HOTKEY.into(); // e.g. a settings file from the other OS
        }
        settings
    }

    pub fn save(&self, path: &Path) {
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = fs::write(path, json);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_promptflow_1_settings_file() {
        let old = r#"{"HOTKEY": "ctrl_r", "LANGUAGE": "tr", "MIC": "QuadCast", "AI_MODE": "prompt",
                      "AI_MODEL": "gemma3:12b", "SOUNDS": false}"#;
        let s: Settings = serde_json::from_str(old).unwrap();
        assert_eq!(s.language.as_deref(), Some("tr"));
        assert_eq!(s.ai_mode.as_deref(), Some("prompt"));
        assert_eq!(s.mic.as_deref(), Some("QuadCast"));
        assert_eq!(s.speech_model, Settings::default().speech_model);
        let raw: Settings = serde_json::from_str(r#"{"AI_MODE": null}"#).unwrap();
        assert_eq!(raw.ai_mode, None);
    }
}
