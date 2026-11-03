//! PromptFlow desktop app: tray / menu bar icon, settings window and the dictation engine.

mod ai;
mod audio;
mod catalog;
mod cloud;
mod engine;
mod hardware;
mod hotkey;
mod locale;
mod migrate;
mod paste;
mod settings;
mod stt;
mod text;

use std::collections::HashMap;
use std::fs;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;

use serde::Serialize;
use serde_json::Value;
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Listener, Manager, PhysicalPosition, State, WebviewUrl, WebviewWindowBuilder, Wry};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};

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

/// The tray menu's items, so the window can put them in its language.
struct TrayMenu {
    show: MenuItem<Wry>,
    pause: MenuItem<Wry>,
    quit: MenuItem<Wry>,
    labels: Mutex<TrayLabels>,
}

#[derive(serde::Deserialize, Clone)]
struct TrayLabels {
    show: String,
    pause: String,
    resume: String,
    quit: String,
}

#[tauri::command]
fn set_tray_labels(app: AppHandle, labels: TrayLabels) {
    let menu = app.state::<TrayMenu>();
    let paused = app.state::<Arc<Engine>>().paused.load(Ordering::Relaxed);
    let _ = menu.show.set_text(&labels.show);
    let _ = menu.pause.set_text(if paused { &labels.resume } else { &labels.pause });
    let _ = menu.quit.set_text(&labels.quit);
    *menu.labels.lock().unwrap() = labels;
}

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
    /// Every language the speech models know (codes; the window names them)
    languages: Vec<(String, String)>,
    mics: Vec<String>,
    speech_models: Vec<String>,
    history: Vec<HistoryItem>,
    ready: bool,
    paused: bool,
    autostart: bool,
    tray_name: &'static str,
    autostart_label: &'static str,
    /// Language of the window: the setting, or the OS interface language
    ui_language: String,
    os: &'static str,
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
fn get_state(app: AppHandle, engine: State<'_, Arc<Engine>>) -> WindowState {
    WindowState {
        version: env!("CARGO_PKG_VERSION"),
        settings: engine.settings(),
        hotkeys: hotkey::HOTKEYS.to_vec(),
        languages: catalog::languages().into_iter().map(|c| (c.clone(), c)).collect(),
        mics: audio::list_mics(),
        speech_models: speech_models(&engine),
        history: history(&engine),
        ready: engine.is_ready(),
        paused: engine.paused.load(Ordering::Relaxed),
        autostart: app.autolaunch().is_enabled().unwrap_or(false),
        tray_name: TRAY_NAME,
        autostart_label: AUTOSTART_LABEL,
        ui_language: engine.settings().ui_language.unwrap_or_else(|| system_languages().ui.clone()),
        os: std::env::consts::OS,
    }
}

#[derive(Serialize)]
struct CloudState {
    has_key: bool,
    /// Text models this key can use, best default first; empty without a key or when the provider can't be reached
    models: Vec<String>,
    model: Option<String>,
    error: Option<String>,
}

/// What the AI page shows for a cloud provider.
#[tauri::command]
async fn cloud_state(engine: State<'_, Arc<Engine>>, provider: String) -> Result<CloudState, String> {
    let provider = cloud::Provider::from_id(&provider).ok_or("unknown provider")?;
    let model = engine.settings().cloud_model;
    let Some(key) = cloud::key(provider) else {
        return Ok(CloudState { has_key: false, models: Vec::new(), model, error: None });
    };
    let listed = tauri::async_runtime::spawn_blocking(move || cloud::models(provider, &key)).await.map_err(|e| e.to_string())?;
    Ok(match listed {
        Ok(models) => CloudState { has_key: true, models, model, error: None },
        Err(e) => CloudState { has_key: true, models: Vec::new(), model, error: Some(e.to_string()) },
    })
}

/// Checks the key with the provider, then keeps it in the OS keychain and picks the fast default model.
#[tauri::command]
async fn save_api_key(engine: State<'_, Arc<Engine>>, provider: String, key: String) -> Result<(), String> {
    let provider = cloud::Provider::from_id(&provider).ok_or("unknown provider")?;
    let key = key.trim().to_string();
    let check = key.clone();
    let models = tauri::async_runtime::spawn_blocking(move || cloud::models(provider, &check))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    cloud::set_key(provider, &key).map_err(|e| e.to_string())?;
    let mut s = engine.settings.lock().unwrap();
    s.ai_provider = provider.id().into();
    s.cloud_model = models.first().cloned();
    drop(s);
    engine.save_settings();
    engine.log(format!("API key saved for {}", provider.id()));
    Ok(())
}

#[tauri::command]
fn forget_api_key(engine: State<'_, Arc<Engine>>, provider: String) -> Result<(), String> {
    let provider = cloud::Provider::from_id(&provider).ok_or("unknown provider")?;
    cloud::delete_key(provider);
    let mut s = engine.settings.lock().unwrap();
    if s.ai_provider == provider.id() {
        s.ai_provider = "ollama".into();
        s.cloud_model = None;
    }
    drop(s);
    engine.save_settings();
    Ok(())
}

/// Installed Ollama models (asked off the main thread: Ollama may take a moment or be off).
#[tauri::command]
async fn ai_models() -> Vec<String> {
    tauri::async_runtime::spawn_blocking(ai::list_models).await.unwrap_or_default()
}

#[tauri::command]
fn set_setting(app: AppHandle, engine: State<'_, Arc<Engine>>, key: String, value: Value) -> Result<(), String> {
    let text = value.as_str().map(String::from);
    let warm_ai = || {
        let s = engine.settings();
        if s.ai_mode.is_some() && s.ai_provider == "ollama" {
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
        "ai_provider" => {
            let provider = text.filter(|p| p == "ollama" || cloud::Provider::from_id(p).is_some()).ok_or("unknown provider")?;
            let mut s = engine.settings.lock().unwrap();
            if s.ai_provider != provider {
                s.ai_provider = provider;
                s.cloud_model = None; // each provider has its own models
            }
            drop(s);
            warm_ai();
        }
        "cloud_model" => engine.settings.lock().unwrap().cloud_model = text.filter(|m| !m.is_empty()),
        "mic" => engine.settings.lock().unwrap().mic = text.filter(|m| !m.is_empty()),
        "language" => engine.settings.lock().unwrap().language = text.filter(|l| !l.is_empty()),
        "sounds" => engine.settings.lock().unwrap().sounds = value.as_bool().unwrap_or(false),
        "ui_language" => {
            let chosen = text.filter(|l| !l.is_empty() && l != "auto");
            engine.settings.lock().unwrap().ui_language = chosen.clone();
            // The pill window follows along
            let _ = app.emit("ui_language", chosen.unwrap_or_else(|| system_languages().ui.clone()));
        }
        "autostart" => {
            let launcher = app.autolaunch();
            let result = if value.as_bool().unwrap_or(false) { launcher.enable() } else { launcher.disable() };
            return result.map_err(|e| e.to_string());
        }
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
fn get_dictionary(engine: State<'_, Arc<Engine>>) -> Vec<(String, String)> {
    text::read_dictionary(&engine.data_dir.join("dictionary.txt"))
}

#[tauri::command]
fn save_dictionary(engine: State<'_, Arc<Engine>>, entries: Vec<(String, String)>) -> Result<(), String> {
    text::write_dictionary(&engine.data_dir.join("dictionary.txt"), &entries).map_err(|e| e.to_string())
}

#[tauri::command]
fn finish_onboarding(engine: State<'_, Arc<Engine>>) {
    engine.settings.lock().unwrap().onboarded = true;
    engine.save_settings();
}

#[tauri::command]
fn open_data_file(engine: State<'_, Arc<Engine>>, name: String) -> Result<(), String> {
    let path = match name.as_str() {
        "dictionary" => engine.data_dir.join("dictionary.txt"),
        "history" => engine.history_file(),
        "folder" => return opener::open(&engine.data_dir).map_err(|e| e.to_string()),
        _ => return Err("unknown file".into()),
    };
    if !path.exists() {
        let _ = fs::write(&path, "");
    }
    opener::open(&path).map_err(|e| e.to_string())
}

// ---------------- model catalog ----------------

static HARDWARE: OnceLock<hardware::Hardware> = OnceLock::new();

fn hardware() -> &'static hardware::Hardware {
    HARDWARE.get_or_init(hardware::detect)
}

/// Downloads in progress, by model id, with their cancel flags.
#[derive(Default)]
struct Downloads(Mutex<HashMap<String, Arc<AtomicBool>>>);

#[derive(Serialize)]
struct ModelInfo {
    #[serde(flatten)]
    model: catalog::Model,
    installed: bool,
    downloading: bool,
}

#[derive(Serialize)]
struct CatalogState {
    models: Vec<ModelInfo>,
    hardware: hardware::Hardware,
    recommendation: catalog::Recommendation,
    current: String,
    /// Languages the recommendation is for
    languages: Vec<String>,
    /// Languages found on this computer (interface language first), offered as quick picks
    detected: Vec<String>,
}

static LANGUAGES: OnceLock<locale::Languages> = OnceLock::new();

fn system_languages() -> &'static locale::Languages {
    LANGUAGES.get_or_init(locale::detect)
}

fn speech_languages(settings: &Settings) -> Vec<String> {
    settings.speech_languages.clone().filter(|l| !l.is_empty()).unwrap_or_else(|| vec![system_languages().ui.clone()])
}

#[tauri::command]
fn set_speech_languages(engine: State<'_, Arc<Engine>>, languages: Vec<String>) {
    engine.settings.lock().unwrap().speech_languages = Some(languages).filter(|l| !l.is_empty());
    engine.save_settings();
}

#[derive(Serialize, Clone)]
struct DownloadEvent {
    id: String,
    state: &'static str, // downloading, done, error, cancelled
    done: u64,
    total: u64,
    error: String,
}

#[tauri::command]
async fn get_catalog(engine: State<'_, Arc<Engine>>, downloads: State<'_, Downloads>) -> Result<CatalogState, ()> {
    let hw = tauri::async_runtime::spawn_blocking(|| hardware().clone()).await.map_err(|_| ())?;
    let dir = engine.models_dir();
    let active = downloads.0.lock().unwrap();
    let settings = engine.settings();
    Ok(CatalogState {
        models: catalog::MODELS
            .iter()
            .map(|m| ModelInfo { model: *m, installed: dir.join(m.file).exists(), downloading: active.contains_key(m.id) })
            .collect(),
        recommendation: catalog::recommend(&hw, &speech_languages(&settings)),
        hardware: hw,
        languages: speech_languages(&settings),
        detected: system_languages().detected.clone(),
        current: settings.speech_model,
    })
}

#[tauri::command]
fn download_model(app: AppHandle, id: String) -> Result<(), String> {
    let model = catalog::find(&id).ok_or("unknown model")?;
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let downloads = app.state::<Downloads>();
        let mut active = downloads.0.lock().unwrap();
        if active.contains_key(model.id) {
            return Ok(());
        }
        active.insert(model.id.into(), cancel.clone());
    }
    thread::spawn(move || {
        let engine = app.state::<Arc<Engine>>().inner().clone();
        let dir = engine.models_dir();
        let send = |state, done, total, error: String| {
            let _ = app.emit("download", DownloadEvent { id: model.id.into(), state, done, total, error });
        };
        // The voice detector comes with the first model
        let result = catalog::download(&catalog::VAD, &dir, &cancel, &|_, _| {})
            .and_then(|_| catalog::download(&model, &dir, &cancel, &|done, total| send("downloading", done, total, String::new())));
        app.state::<Downloads>().0.lock().unwrap().remove(model.id);
        match result {
            Ok(()) => {
                engine.log(format!("Downloaded {}", model.file));
                send("done", model.size, model.size, String::new());
                // Use it right away if nothing usable is loaded yet
                if !engine.is_ready() || engine.settings().speech_model == model.file {
                    engine.settings.lock().unwrap().speech_model = model.file.into();
                    engine.save_settings();
                    engine.load_model();
                }
            }
            Err(e) if cancel.load(Ordering::Relaxed) => {
                let _ = e;
                send("cancelled", 0, model.size, String::new());
            }
            Err(e) => {
                engine.log(format!("[download error] {}: {e}", model.file));
                send("error", 0, model.size, e.to_string());
            }
        }
    });
    Ok(())
}

#[tauri::command]
fn cancel_download(downloads: State<'_, Downloads>, id: String) {
    if let Some(flag) = downloads.0.lock().unwrap().get(&id) {
        flag.store(true, Ordering::Relaxed);
    }
}

#[tauri::command]
fn use_model(engine: State<'_, Arc<Engine>>, id: String) -> Result<(), String> {
    let model = catalog::find(&id).ok_or("unknown model")?;
    if !engine.models_dir().join(model.file).exists() {
        return Err("not downloaded yet".into());
    }
    engine.settings.lock().unwrap().speech_model = model.file.into();
    engine.save_settings();
    let engine = engine.inner().clone();
    thread::spawn(move || engine.load_model());
    Ok(())
}

#[derive(Serialize, Clone)]
struct PullEvent {
    model: String,
    state: &'static str,
    done: u64,
    total: u64,
    status: String,
}

/// Downloads an Ollama model for the AI cleanup and selects it.
#[tauri::command]
fn pull_ai_model(app: AppHandle, model: String) {
    thread::spawn(move || {
        let send = |state, done, total, status: &str| {
            let _ = app.emit("ai_pull", PullEvent { model: model.clone(), state, done, total, status: status.into() });
        };
        match ai::pull(&model, &|done, total, status| send("pulling", done, total, status)) {
            Ok(()) => {
                let engine = app.state::<Arc<Engine>>();
                engine.settings.lock().unwrap().ai_model = model.clone();
                engine.save_settings();
                send("done", 0, 0, "");
                let _ = ai::warm(&model);
            }
            Err(e) => send("error", 0, 0, &e),
        }
    });
}

/// Permissions PromptFlow still needs (macOS: Accessibility for the hotkey and for pasting).
#[tauri::command]
fn missing_permissions() -> Vec<&'static str> {
    #[cfg(target_os = "macos")]
    {
        if !handy_keys::check_accessibility() {
            return vec!["Accessibility"];
        }
    }
    Vec::new()
}

#[tauri::command]
fn open_permission_settings(name: String) -> Result<(), String> {
    let pane = match name.as_str() {
        "Accessibility" => "Privacy_Accessibility",
        "Microphone" => "Privacy_Microphone",
        _ => return Err("unknown permission".into()),
    };
    opener::open(format!("x-apple.systempreferences:com.apple.preference.security?{pane}")).map_err(|e| e.to_string())
}

/// Opens a link in the browser (only the few the window uses).
#[tauri::command]
fn open_link(url: String) -> Result<(), String> {
    const ALLOWED: &[&str] = &[
        "https://ollama.com/",
        "https://github.com/tugberkup/promptflow",
        // where each provider hands out API keys
        "https://console.anthropic.com/settings/keys",
        "https://aistudio.google.com/apikey",
        "https://platform.openai.com/api-keys",
    ];
    if !ALLOWED.iter().any(|a| url.starts_with(a)) {
        return Err("not allowed".into());
    }
    opener::open_browser(&url).map_err(|e| e.to_string())
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
    let menu = app.state::<TrayMenu>();
    let labels = menu.labels.lock().unwrap().clone();
    let _ = menu.pause.set_text(if paused { labels.resume } else { labels.pause });
    let kind = if paused { "paused" } else { "resumed" };
    let _ = app.emit("engine", engine::EngineEvent { kind, text: String::new(), code: "" });
}

#[tauri::command]
fn get_level(engine: State<'_, Arc<Engine>>) -> f32 {
    engine.recorder.level()
}

// ---------------- tray icon colors ----------------

#[cfg(target_os = "macos")]
fn tray_image(state: &str) -> Image<'static> {
    let bytes: &[u8] = match state {
        "recording" => include_bytes!("../icons/tray/mac-recording.png"),
        "processing" => include_bytes!("../icons/tray/mac-processing.png"),
        "paused" => include_bytes!("../icons/tray/mac-paused.png"),
        _ => include_bytes!("../icons/tray/mac-idle.png"),
    };
    Image::from_bytes(bytes).expect("tray icon")
}

#[cfg(not(target_os = "macos"))]
fn tray_image(state: &str) -> Image<'static> {
    let bytes: &[u8] = match state {
        "recording" => include_bytes!("../icons/tray/recording.png"),
        "processing" => include_bytes!("../icons/tray/processing.png"),
        "paused" => include_bytes!("../icons/tray/paused.png"),
        _ => include_bytes!("../icons/tray/idle.png"),
    };
    Image::from_bytes(bytes).expect("tray icon")
}

/// Follows the engine's events with the tray icon's color.
fn set_tray_state(app: &AppHandle, kind: &str) {
    let paused = app.state::<Arc<Engine>>().paused.load(Ordering::Relaxed);
    let state = match kind {
        "recording" => "recording",
        "processing" => "processing",
        "status" | "needs_model" => return,
        _ if paused => "paused",
        _ => "idle",
    };
    if let Some(tray) = app.tray_by_id("main") {
        let _ = tray.set_icon(Some(tray_image(state)));
        // A black template image follows the light/dark menu bar on macOS
        let _ = tray.set_icon_as_template(cfg!(target_os = "macos") && matches!(state, "idle" | "paused"));
    }
}

/// The "Listening" pill: a transparent, click-through window that never takes focus.
fn create_overlay(app: &AppHandle) -> tauri::Result<()> {
    let (w, h) = (210.0, 44.0);
    let overlay = WebviewWindowBuilder::new(app, "overlay", WebviewUrl::App("overlay.html".into()))
        .title("PromptFlow overlay")
        .inner_size(w, h)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .resizable(false)
        .always_on_top(true)
        .visible_on_all_workspaces(true)
        .skip_taskbar(true)
        .focused(false)
        .focusable(false)
        .visible(false)
        .build()?;
    overlay.set_ignore_cursor_events(true)?;
    if let Some(monitor) = overlay.primary_monitor()? {
        let scale = monitor.scale_factor();
        let (size, origin) = (monitor.size(), monitor.position());
        let margin = if cfg!(target_os = "macos") { 110.0 } else { 80.0 }; // macOS: above the Dock
        let x = origin.x as f64 + (size.width as f64 - w * scale) / 2.0;
        let y = origin.y as f64 + size.height as f64 - (h + margin) * scale;
        overlay.set_position(PhysicalPosition::new(x, y))?;
    }
    // Shown once and kept: it's transparent until there's something to draw, so it never has to
    // be shown again in the middle of someone's typing
    overlay.show()?;
    Ok(())
}

pub fn run() {
    tauri::Builder::default()
        // A second launch just opens the window of the running one
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show_main(app)))
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, Some(vec!["--hidden"])))
        .invoke_handler(tauri::generate_handler![
            get_state, ai_models, set_setting, open_data_file, cloud_state, save_api_key, forget_api_key,
            get_catalog, download_model, cancel_download, use_model, pull_ai_model, open_link, get_level, missing_permissions, open_permission_settings,
            set_speech_languages, set_tray_labels, get_dictionary, save_dictionary, finish_onboarding
        ])
        .manage(Downloads::default())
        .setup(|app| {
            thread::spawn(hardware); // detect the GPU early; the catalog needs it
            // A menu bar app on macOS: no Dock icon
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            let data_dir = app.path().app_data_dir()?;
            let imported = migrate::import_once(&data_dir);
            let engine = Engine::new(app.handle().clone(), data_dir);
            if let Some(old) = imported {
                engine.log(format!("Imported history, dictionary and settings from PromptFlow 1.x in {}", old.display()));
            }
            app.manage(engine.clone());

            // Started at login with --hidden: stay in the tray / menu bar
            if !std::env::args().any(|a| a == "--hidden") {
                show_main(app.handle());
            }
            if let Err(e) = create_overlay(app.handle()) {
                engine.log(format!("[overlay error] {e}"));
            }
            let handle = app.handle().clone();
            app.listen_any("engine", move |event| {
                if let Ok(e) = serde_json::from_str::<Value>(event.payload()) {
                    set_tray_state(&handle, e["kind"].as_str().unwrap_or(""));
                }
            });

            let show = MenuItem::with_id(app, "show", "Show", true, None::<&str>)?;
            let pause = MenuItem::with_id(app, "pause", "Pause", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let separator = PredefinedMenuItem::separator(app)?;
            let menu = Menu::with_items(app, &[&show, &pause, &separator, &quit])?;
            let labels = TrayLabels { show: "Show".into(), pause: "Pause".into(), resume: "Resume".into(), quit: "Quit".into() };
            app.manage(TrayMenu { show, pause, quit, labels: Mutex::new(labels) });

            TrayIconBuilder::with_id("main")
                .icon(tray_image("idle"))
                .icon_as_template(cfg!(target_os = "macos"))
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
                if window.label() != "main" {
                    return;
                }
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running PromptFlow");
}
