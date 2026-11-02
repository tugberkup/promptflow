//! The dictation engine: hotkey → recording → Whisper → dictionary → AI cleanup → paste → history.
//! The window follows it through "engine" events: status, ready, recording, processing, done, idle, error.

use std::fs::{self, File};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::audio::Recorder;
use crate::hotkey::{self, Hotkey, HotkeyEvent};
use crate::settings::Settings;
use crate::stt::Stt;
use crate::text::{self, Dictionary};
use crate::{ai, paste};

pub const VAD_MODEL: &str = "ggml-silero-v5.1.2.bin";

#[derive(Serialize, Clone)]
pub struct EngineEvent {
    pub kind: &'static str,
    pub text: String,
}

#[derive(Default)]
struct Hold {
    held: bool,
    cancelled: bool,
    started: Option<Instant>,
}

pub struct Engine {
    app: AppHandle,
    pub data_dir: PathBuf,
    pub settings: Mutex<Settings>,
    pub recorder: Recorder,
    pub dictionary: Dictionary,
    pub paused: AtomicBool,
    hotkey: Arc<Mutex<Hotkey>>,
    hold: Mutex<Hold>,
    stt: Mutex<Option<Arc<Stt>>>,
    jobs: Mutex<Option<Sender<Vec<f32>>>>,
    log_file: Mutex<Option<File>>,
}

impl Engine {
    pub fn new(app: AppHandle, data_dir: PathBuf) -> Arc<Self> {
        let _ = fs::create_dir_all(data_dir.join("models"));
        let settings = Settings::load(&data_dir.join("settings.json"));
        let hotkey = hotkey::parse(&settings.hotkey).expect("valid hotkey");
        Arc::new(Self {
            app,
            dictionary: Dictionary::new(data_dir.join("dictionary.txt")),
            log_file: Mutex::new(File::create(data_dir.join("promptflow.log")).ok()),
            data_dir,
            settings: Mutex::new(settings),
            recorder: Recorder::new(),
            paused: AtomicBool::new(false),
            hotkey: Arc::new(Mutex::new(hotkey)),
            hold: Mutex::new(Hold::default()),
            stt: Mutex::new(None),
            jobs: Mutex::new(None),
        })
    }

    pub fn log(&self, msg: impl AsRef<str>) {
        let msg = msg.as_ref();
        eprintln!("{msg}");
        if let Some(f) = self.log_file.lock().unwrap().as_mut() {
            let _ = writeln!(f, "{msg}");
        }
    }

    pub fn emit(&self, kind: &'static str, text: impl Into<String>) {
        let _ = self.app.emit("engine", EngineEvent { kind, text: text.into() });
    }

    pub fn settings(&self) -> Settings {
        self.settings.lock().unwrap().clone()
    }

    pub fn save_settings(&self) {
        self.settings.lock().unwrap().save(&self.data_dir.join("settings.json"));
    }

    pub fn models_dir(&self) -> PathBuf {
        // PROMPTFLOW_MODELS lets a developer share one models folder between builds
        std::env::var_os("PROMPTFLOW_MODELS").map(PathBuf::from).unwrap_or_else(|| self.data_dir.join("models"))
    }

    pub fn history_file(&self) -> PathBuf {
        self.data_dir.join("history.txt")
    }

    pub fn is_ready(&self) -> bool {
        self.stt.lock().unwrap().is_some()
    }

    pub fn set_hotkey(&self, name: &str) -> bool {
        let Some(parsed) = hotkey::parse(name) else { return false };
        *self.hotkey.lock().unwrap() = parsed;
        self.settings.lock().unwrap().hotkey = name.into();
        self.save_settings();
        true
    }

    /// Starts everything; call once on a background thread (loading the model takes a few seconds).
    pub fn start(self: &Arc<Self>) {
        let me = self.clone();
        if let Err(e) = hotkey::listen(self.hotkey.clone(), move |event| me.on_hotkey(event)) {
            self.log(format!("[hotkey error] {e}"));
            self.emit("error", format!("Can't watch the hotkey: {e}"));
        }

        let (tx, rx) = channel::<Vec<f32>>();
        *self.jobs.lock().unwrap() = Some(tx);
        let me = self.clone();
        thread::spawn(move || {
            for audio in rx {
                me.process(audio);
            }
        });

        let me = self.clone();
        thread::spawn(move || me.keep_ai_warm());

        self.load_model();
    }

    /// Loads the selected speech model if it's on disk (again, if another one was picked).
    pub fn load_model(&self) {
        *self.stt.lock().unwrap() = None;
        let file = self.settings().speech_model;
        let path = self.models_dir().join(&file);
        if !path.exists() {
            self.log(format!("Speech model not downloaded yet: {file}"));
            self.emit("needs_model", file);
            return;
        }
        let vad = self.models_dir().join(VAD_MODEL);
        self.emit("status", "Loading model…");
        self.log(format!("Loading model: {file}"));
        let t = Instant::now();
        match Stt::load(&path, vad.exists().then_some(vad.as_path())) {
            Ok(stt) => {
                *self.stt.lock().unwrap() = Some(Arc::new(stt));
                self.log(format!("Ready in {:.1}s. Hold '{}', speak, release.", t.elapsed().as_secs_f32(), self.settings().hotkey));
                self.emit("ready", "");
            }
            Err(e) => {
                self.log(format!("[model error] {e}"));
                self.emit("error", format!("Couldn't load the speech model: {e}"));
            }
        }
    }

    fn keep_ai_warm(&self) {
        loop {
            let s = self.settings();
            if s.ai_mode.is_some() {
                match ai::warm(&s.ai_model) {
                    Ok(()) => self.log(format!("AI ready: {} ({})", s.ai_model, s.ai_mode.unwrap_or_default())),
                    Err(e) => self.log(format!("[warning] Can't reach Ollama, cleanup is skipped: {e}")),
                }
            }
            thread::sleep(Duration::from_secs(20 * 60)); // Ollama unloads it after 30 min
        }
    }

    fn on_hotkey(&self, event: HotkeyEvent) {
        let mut hold = self.hold.lock().unwrap();
        match event {
            HotkeyEvent::Down => {
                if hold.held || self.paused.load(Ordering::Relaxed) || !self.is_ready() {
                    return; // key repeat, paused, or still loading
                }
                match self.recorder.start(self.settings().mic) {
                    Ok(_) => {
                        *hold = Hold { held: true, cancelled: false, started: Some(Instant::now()) };
                        self.emit("recording", "");
                    }
                    Err(e) => {
                        self.log(format!("[microphone error] {e}"));
                        self.emit("error", format!("Couldn't open the microphone: {e}"));
                    }
                }
            }
            HotkeyEvent::Other => {
                if hold.held {
                    hold.cancelled = true; // the hotkey is part of a shortcut like Ctrl+C
                }
            }
            HotkeyEvent::Up => {
                if !hold.held {
                    return;
                }
                hold.held = false;
                let mut audio = self.recorder.stop();
                // Automated tests replace the microphone with a recording
                if let Some(wav) = std::env::var_os("PROMPTFLOW_TEST_WAV") {
                    audio = crate::audio::read_wav(std::path::Path::new(&wav)).unwrap_or(audio);
                }
                let seconds = hold.started.map(|t| t.elapsed().as_secs_f32()).unwrap_or(0.0);
                if hold.cancelled || seconds < self.settings().min_seconds {
                    self.emit("idle", "");
                    return;
                }
                self.emit("processing", "");
                if let Some(jobs) = self.jobs.lock().unwrap().as_ref() {
                    let _ = jobs.send(audio);
                }
            }
        }
    }

    fn process(&self, audio: Vec<f32>) {
        let Some(stt) = self.stt.lock().unwrap().clone() else { return };
        if audio.iter().all(|s| *s == 0.0) {
            // Exact zeros: the OS is blocking the microphone (on macOS, a missing permission)
            let hint = "The microphone only sent silence. Check the microphone and its permission.";
            self.log(format!("[microphone] {hint}"));
            self.emit("error", hint);
            return;
        }
        let s = self.settings();
        let t0 = Instant::now();
        let (raw, lang) = match stt.transcribe(&audio, s.language.as_deref(), &text::build_prompt(&self.dictionary)) {
            Ok(r) => r,
            Err(e) => {
                self.log(format!("[error] {e}"));
                self.emit("error", e.to_string());
                return;
            }
        };
        let raw = self.dictionary.apply(&raw);
        let t1 = Instant::now();
        if raw.is_empty() {
            self.log(format!("  (no speech detected, {:.2}s)", (t1 - t0).as_secs_f32()));
            self.emit("idle", "");
            return;
        }
        let lang = s.language.clone().or_else(|| (!lang.is_empty()).then_some(lang));
        let final_text = match &s.ai_mode {
            Some(mode) => ai::polish(&raw, mode, &s.ai_model, lang.as_deref(), &|m| self.log(m)),
            None => raw.clone(),
        };
        let t2 = Instant::now();
        let ai_time = if s.ai_mode.is_some() { format!(" + AI {:.2}s", (t2 - t1).as_secs_f32()) } else { String::new() };
        self.log(format!("  [{:.2}s{ai_time}] {final_text}", (t1 - t0).as_secs_f32()));
        if final_text != raw {
            self.log(format!("      raw: {raw}"));
        }
        if let Err(e) = paste::paste(&final_text, s.restore_clipboard) {
            self.log(format!("[paste error] {e}"));
            self.emit("error", format!("Couldn't paste: {e}"));
        }
        text::save_history(&self.history_file(), &raw, &final_text);
        self.emit("done", final_text);
    }
}
