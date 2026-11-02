# PromptFlow 2 (Rust)

The Rust rewrite of PromptFlow: a [Tauri 2](https://tauri.app) app with a plain HTML/CSS/JS window (`ui/`) and the
dictation engine in Rust (`src-tauri/`). It replaces the Python app at the repository root once it's released.

## How it fits together

| File | What it does |
|---|---|
| `src-tauri/src/lib.rs` | App setup, tray / menu bar, the "Listening" pill window, commands for the settings window |
| `src-tauri/src/engine.rs` | Hotkey → recording → Whisper → dictionary → AI cleanup → paste → history |
| `src-tauri/src/hotkey.rs` | Hold-to-talk on a single key (Right Ctrl / Right Command / F-keys) via [handy-keys](https://github.com/handy-computer/handy-keys) |
| `src-tauri/src/audio.rs` | Microphone recording with cpal, resampled to 16 kHz mono |
| `src-tauri/src/stt.rs` | whisper.cpp through [whisper-rs](https://codeberg.org/tazz4843/whisper-rs): Vulkan on Windows, Metal on Apple Silicon, CPU on Intel Macs; Silero VAD |
| `src-tauri/src/catalog.rs` | Model catalog, recommendations for this machine, resumable downloads with SHA256 checks |
| `src-tauri/src/hardware.rs` | GPU name and video memory (DXGI on Windows), RAM, Apple Silicon |
| `src-tauri/src/ai.rs` | Optional cleanup through a local [Ollama](https://ollama.com) model (same prompts as 1.x) |
| `src-tauri/src/paste.rs` | Clipboard + Ctrl+V / Cmd+V, clipboard restore, beeps |
| `src-tauri/src/text.rs` | Dictionary, history file, Markdown cleanup |
| `src-tauri/src/migrate.rs` | Imports history, dictionary and settings from a 1.x install on first start |
| `ui/index.html`, `ui/main.js` | Settings window and model catalog |
| `ui/overlay.html` | The recording pill |

User data lives in the app data folder (`%APPDATA%\io.github.tugberkup.promptflow` on Windows,
`~/Library/Application Support/io.github.tugberkup.promptflow` on macOS): `settings.json`, `dictionary.txt`,
`history.txt`, `promptflow.log` and `models/`.

## Building from source

**Windows**
- [Rust](https://rustup.rs), Visual Studio 2022 Build Tools with the C++ workload, [CMake](https://cmake.org),
  the [Vulkan SDK](https://vulkan.lunarg.com) and LLVM (for libclang):
  ```powershell
  winget install Rustlang.Rustup Kitware.CMake KhronosGroup.VulkanSDK LLVM.LLVM
  winget install Microsoft.VisualStudio.2022.BuildTools --override "--passive --wait --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
  ```
- Set `LIBCLANG_PATH=C:\Program Files\LLVM\bin`.
- whisper.cpp's build fails on long paths ("No CMAKE_C_COMPILER could be found"), so point the build output at a
  short folder, e.g. `CARGO_TARGET_DIR=C:\pf-target`. This also keeps gigabytes of build files out of synced
  folders such as OneDrive.

**macOS**
- Xcode Command Line Tools (`xcode-select --install`), [Rust](https://rustup.rs) and CMake (`brew install cmake`).

**Both**
```bash
cargo install tauri-cli --version "^2" --locked
cd rust/src-tauri
cargo tauri dev            # run it
cargo test                 # unit tests
cargo tauri build          # installers in <target>/release/bundle
```

## Development helpers

| Variable | Effect |
|---|---|
| `PROMPTFLOW_MODELS` | Use this folder for models instead of the app data folder (share downloads between builds) |
| `PROMPTFLOW_TEST_WAV` | Use this 16-bit WAV file instead of the microphone, for automated end-to-end tests |
| `PROMPTFLOW_IMPORT_FROM` | Import 1.x data from this folder on first start |
