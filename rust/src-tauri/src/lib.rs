//! PromptFlow desktop app: tray / menu bar icon, settings window and the dictation engine.

mod ai;
mod audio;
mod engine;
mod hotkey;
mod paste;
mod settings;
mod stt;
mod text;

use std::fs;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread;

use serde::Serialize;
use serde_json::Value;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, State, Wry};

use engine::{Engine, VAD_MODEL};
use settings::Settings;

#[cfg(target_os = "macos")]
const TRAY_NAME: &str = "menu bar";
#[cfg(not(target_os = "macos"))]
const TRAY_NAME: &str = "tray";

#[cfg(target_os = "macos")]
const AUTOSTART_LABEL: &str = "Start at login";
#[cfg(not(target_os = "macos"))]
const AUTOSTART_LABEL: &str = "Start with Windows";

struct PauseItem(MenuItem<Wry>);

#[derive(Serialize)]
struct HistoryItem {
    time: String,
    text: String,
}

#[derive(Serialize)]
struct WindowState {
    version: &'static str,
    settings: Settings,
    hotkeys: Vec<(&'static str, &'static str)>,
    languages: Vec<(&'static str, &'static str)>,
    mics: Vec<String>,
    speech_models: Vec<String>,
    history: Vec<HistoryItem>,
    ready: bool,
    paused: bool,
    tray_name: &'static str,
    autostart_label: &'static str,
}

/// Whisper models in the models folder (the catalog with downloads comes next).
fn speech_models(engine: &Engine) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(engine.models_dir())
        .map(|dir| {
            dir.filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().to_string())
                .filter(|n| n.starts_with("ggml-") && n.ends_with(".bin") && n != VAD_MODEL)
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

fn history(engine: &Engine) -> Vec<HistoryItem> {
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    text::read_history(&engine.history_file(), 20)
        .into_iter()
        .rev()
        .map(|(stamp, text)| {
            let (day, clock) = stamp.split_once(' ').unwrap_or((&stamp, ""));
            let clock = clock.get(..5).unwrap_or(clock);
            let time = if day == today {
                clock.to_string()
            } else {
                format!("{}/{} {clock}", day.get(5..7).unwrap_or(""), day.get(8..10).unwrap_or(""))
            };
            HistoryItem { time, text }
        })
        .collect()
}

#[tauri::command]
fn get_state(engine: State<'_, Arc<Engine>>) -> WindowState {
    WindowState {
        version: env!("CARGO_PKG_VERSION"),
        settings: engine.settings(),
        hotkeys: hotkey::HOTKEYS.to_vec(),
        languages: ai::LANGUAGE_NAMES.to_vec(),
        mics: audio::list_mics(),
        speech_models: speech_models(&engine),
        history: history(&engine),
        ready: engine.is_ready(),
        paused: engine.paused.load(Ordering::Relaxed),
        tray_name: TRAY_NAME,
        autostart_label: AUTOSTART_LABEL,
    }
}

/// Installed Ollama models (asked off the main thread: Ollama may take a moment or be off).
#[tauri::command]
async fn ai_models() -> Vec<String> {
    tauri::async_runtime::spawn_blocking(ai::list_models).await.unwrap_or_default()
}

#[tauri::command]
fn set_setting(engine: State<'_, Arc<Engine>>, key: String, value: Value) -> Result<(), String> {
    let text = value.as_str().map(String::from);
    let warm_ai = || {
        let s = engine.settings();
        if s.ai_mode.is_some() {
            thread::spawn(move || ai::warm(&s.ai_model));
        }
    };
    match key.as_str() {
        "hotkey" => {
            if !engine.set_hotkey(text.as_deref().unwrap_or_default()) {
                return Err("unknown hotkey".into());
            }
            return Ok(());
        }
        "mode" => {
            engine.settings.lock().unwrap().ai_mode = text.filter(|m| m != "raw");
            warm_ai();
        }
        "ai_model" => {
            engine.settings.lock().unwrap().ai_model = text.unwrap_or_default();
            warm_ai();
        }
        "mic" => engine.settings.lock().unwrap().mic = text.filter(|m| !m.is_empty()),
        "language" => engine.settings.lock().unwrap().language = text.filter(|l| !l.is_empty()),
        "sounds" => engine.settings.lock().unwrap().sounds = value.as_bool().unwrap_or(false),
        "speech_model" => {
            engine.settings.lock().unwrap().speech_model = text.unwrap_or_default();
            engine.save_settings();
            let engine = engine.inner().clone();
            thread::spawn(move || engine.load_model());
            return Ok(());
        }
        _ => return Err(format!("unknown setting {key}")),
    }
    engine.save_settings();
    Ok(())
}

#[tauri::command]
fn open_data_file(engine: State<'_, Arc<Engine>>, name: String) -> Result<(), String> {
    let path = match name.as_str() {
        "dictionary" => engine.data_dir.join("dictionary.txt"),
        "history" => engine.history_file(),
        _ => return Err("unknown file".into()),
    };
    if !path.exists() {
        let _ = fs::write(&path, "");
    }
    opener::open(&path).map_err(|e| e.to_string())
}

fn show_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn toggle_pause(app: &AppHandle) {
    let engine = app.state::<Arc<Engine>>();
    let paused = !engine.paused.load(Ordering::Relaxed);
    engine.paused.store(paused, Ordering::Relaxed);
    let _ = app.state::<PauseItem>().0.set_text(if paused { "Resume" } else { "Pause" });
    let _ = app.emit("engine", engine::EngineEvent { kind: if paused { "paused" } else { "resumed" }, text: String::new() });
}

pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![get_state, ai_models, set_setting, open_data_file])
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            let engine = Engine::new(app.handle().clone(), data_dir);
            app.manage(engine.clone());

            let show = MenuItem::with_id(app, "show", "Show", true, None::<&str>)?;
            let pause = MenuItem::with_id(app, "pause", "Pause", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let separator = PredefinedMenuItem::separator(app)?;
            let menu = Menu::with_items(app, &[&show, &pause, &separator, &quit])?;
            app.manage(PauseItem(pause));

            TrayIconBuilder::with_id("main")
                .icon(app.default_window_icon().cloned().expect("app icon"))
                .tooltip("PromptFlow")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => show_main(app),
                    "pause" => toggle_pause(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        show_main(tray.app_handle());
                    }
                })
                .build(app)?;

            thread::spawn(move || engine.start());
            Ok(())
        })
        // Closing the window keeps PromptFlow running in the tray / menu bar
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running PromptFlow");
}
