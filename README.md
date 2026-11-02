# PromptFlow

**Stop typing your prompts. Say them.** A push-to-talk dictation app for Windows that runs entirely on your own machine.

Hold a key, say what you want, let go. Your speech is transcribed, cleaned of "um, uh, no wait, I meant…"
and pasted wherever your cursor is: Claude, ChatGPT, Cursor, VS Code, your browser, Discord, anything.
No audio ever leaves your computer, there's no subscription, and once it's set up it works offline.

<p align="center"><img src="docs/screenshot.png" width="420" alt="PromptFlow window"></p>

## Features

- **Push-to-talk dictation:** hold Right Ctrl, speak, release; the text is pasted into the active window.
- **AI cleanup:** a local LLM (via Ollama) removes filler words, fixes punctuation and understands self-corrections
  ("move it to Tuesday, sorry, Wednesday" → "Wednesday"). It never answers your prompt and never translates it.
- **Prompt mode:** turns a rambling spoken request into a clear, bullet-pointed prompt.
- **Many languages:** English, Turkish and everything else Whisper supports, including mixed-language speech.
- **Personal dictionary:** fixes words that always come out wrong (`clod -> Claude`).
- **History:** every dictation is saved and can be copied again with one click.
- **Stays out of the way:** tray icon, a small "Listening" pill at the bottom of the screen while recording,
  optional start with Windows.

## How it works

```
hold Right Ctrl ─► microphone ─► faster-whisper (large-v3-turbo, GPU) ─► dictionary ─► Ollama cleanup ─► Ctrl+V
                                        speech to text                                    (optional)
```

Everything runs locally. Audio isn't even written to disk; only the resulting text goes to `history.txt`.

## Requirements

| | Required | Recommended |
|---|---|---|
| OS | Windows 10 / 11 | |
| Python | 3.11 – 3.13 | 3.12 |
| GPU | None (runs on the CPU, takes a few seconds) | NVIDIA, 8 GB+ VRAM |
| AI cleanup | Optional | [Ollama](https://ollama.com) |
| Disk | ~4 GB | +3–8 GB for the AI model |

## Installation

1. **Install Python 3.12** from [python.org/downloads](https://www.python.org/downloads/).
   Keep the **"py launcher"** option checked during setup.
2. **(Recommended) Install Ollama** from [ollama.com/download](https://ollama.com/download). It powers the AI cleanup;
   without it PromptFlow still works in "Raw" mode.
3. **Download PromptFlow:** click the green **Code → Download ZIP** button on this page and extract it to a folder
   (or `git clone https://github.com/tugberkup/promptflow.git`).
4. **Double-click `install.bat`.** It will:
   - install the packages into `%USERPROFILE%\.promptflow\venv` (outside synced folders like OneDrive),
   - download the speech model (~1.6 GB, once),
   - if Ollama is installed, ask which AI model to download,
   - put a **PromptFlow** shortcut in the folder and start the app.

After the first launch PromptFlow is also in the Start menu; just search for "PromptFlow".

> Windows SmartScreen or your antivirus may warn about an app that listens to a keyboard shortcut. The code is open:
> `Engine._on_press` in `promptflow.py` shows what it listens to (only the hotkey you picked).

## Usage

1. Wait for the purple microphone icon next to the clock (the first start takes a few seconds).
2. Click into the text box you want to write in.
3. **Hold Right Ctrl → speak → release.**

While recording, a **"● Listening"** pill shows up at the bottom of the screen and the tray icon turns red.
On release it shows "Transcribing", then "✓ Pasted". If you use the hotkey together with another key
(e.g. Ctrl+C), the dictation is cancelled and the normal shortcut works as usual.

Closing the window doesn't quit the app; it goes to the tray. Right-click the tray icon for **Show / Pause / Quit**.

### Modes

| Mode | What it does | Example |
|---|---|---|
| **Clean** | Removes fillers, repetitions and self-corrections | "make the button red no wait make it blue" → "Make the button blue." |
| **Prompt** | Rewrites what you said as a structured prompt | "I want a landing page with next js uh tailwind and a pricing table" → a bulleted request |
| **Raw** | No AI; exactly what Whisper heard | Fastest, doesn't need Ollama |

### The window

| Setting | |
|---|---|
| **AI model** | Pick one of the models installed in Ollama (see the table below) |
| **Hotkey** | Right Ctrl, Right Shift, F8, F9, F10, Scroll Lock, Pause. (No Right Alt: it's AltGr on many keyboard layouts.) |
| **Microphone** | A specific microphone instead of the Windows default |
| **Language** | Auto-detect, or pin a language if short sentences get detected wrong |
| **Start with Windows** | Starts quietly in the tray when you log in |
| **Recent dictations** | Copy a dictation again if the paste went to the wrong place |
| **Edit dictionary** | `dictionary.txt`: one `wrong -> right` per line, applied as soon as you save |

Your choices are saved to `settings.json`.

## Which AI model?

The cleanup model shares the GPU with Whisper (which uses ~2–3 GB).

| GPU | Model | Install | Speed (per sentence) |
|---|---|---|---|
| 16 GB+ VRAM | `gemma3:12b` (default) | `ollama pull gemma3:12b` | ~1–1.5 s, handles self-corrections well |
| 8–12 GB VRAM | `gemma3:4b` | `ollama pull gemma3:4b` | faster, sometimes misses corrections |
| No / small GPU | — | | **Raw** mode |

After downloading a model, select it in the **AI model** menu. Timings measured on an RTX 4060 Ti 16 GB.

## Advanced settings

At the top of `promptflow.py`:

| Setting | Default | |
|---|---|---|
| `MODEL` | `large-v3-turbo` | Whisper model. Use `small` or `base` on weak machines. |
| `INITIAL_PROMPT` | Claude, React, Next.js… | Terms you use often, given to Whisper as a hint |
| `DEVICE` | `auto` | `cuda` / `cpu` |
| `MIN_SECONDS` | `0.4` | Shorter presses are ignored |
| `RESTORE_CLIPBOARD` | `True` | Puts your previous clipboard back after pasting |
| `WAKE_ENABLED` | `False` | **Experimental:** say "Promptflow" to dictate without a hotkey. Keeps the microphone open. |

To run it in a console (for debugging): `%USERPROFILE%\.promptflow\venv\Scripts\python promptflow.py`

## Troubleshooting

| Problem | Fix |
|---|---|
| Nothing was pasted | Copy it from **Recent dictations**. If the target window runs as administrator, run PromptFlow as administrator too. |
| No speech detected | Pick the right microphone. In Windows Settings → Privacy → Microphone, "Let desktop apps access your microphone" must be on. |
| No AI cleanup | Is Ollama running and is the selected model installed? (`ollama list`) |
| Too slow | Check the GPU with `nvidia-smi`; pick a smaller AI model or switch to **Raw** mode. |
| `PortAudio library not found` / `error 0xce` | The folder path exceeds Windows' 260-character limit; move the project to a shorter path. |
| Anything else | Check `promptflow.log` in the folder and [open an issue](https://github.com/tugberkup/promptflow/issues). |

## Uninstall

1. Turn off **Start with Windows** in the window, then **Quit** from the tray.
2. Delete the project folder and `%USERPROFILE%\.promptflow`.
3. Start menu shortcut: `%APPDATA%\Microsoft\Windows\Start Menu\Programs\PromptFlow.lnk`
4. Speech model: `%USERPROFILE%\.cache\huggingface\hub\models--mobiuslabsgmbh--faster-whisper-large-v3-turbo`
5. AI model: `ollama rm gemma3:12b`

## Project structure

```
app.pyw           desktop app: window, tray icon, recording pill
promptflow.py     engine: recording, Whisper, dictionary, Ollama, pasting (also runs in a console)
install.bat       one-click setup
start.bat         starts without a console (installs first if needed)
requirements.txt  Python packages
assets/           icon
```

`history.txt`, `dictionary.txt`, `settings.json` and `promptflow.log` are created on your machine and are not tracked by git.

## License

[MIT](LICENSE). Use it, change it, share it.
