#!/bin/bash
# PromptFlow setup for macOS. Run it from Terminal:  bash install.command
cd "$(dirname "$0")" || exit 1
HERE="$(pwd)"
VENV="$HOME/.promptflow/venv"
APP="$HOME/Applications/PromptFlow.app"

fail() {
    echo
    echo "[ERROR] Setup didn't finish. See the message above; if you can't solve it, open an issue on GitHub."
    exit 1
}

echo
echo "  PromptFlow setup (macOS)"
echo "  ========================"
echo

case "$HERE" in
    "$HOME/Desktop"*|"$HOME/Documents"*|"$HOME/Downloads"*)
        echo "  Note: macOS asks for extra permission for apps running from Desktop, Documents or Downloads."
        echo "        Moving this folder to your home folder (e.g. ~/promptflow) avoids that."
        echo ;;
esac

# ---- 1) Python with Tk ----
PY=""
for v in 3.12 3.11 3.13; do
    for c in "python$v" "/opt/homebrew/bin/python$v" "/usr/local/bin/python$v" \
             "/Library/Frameworks/Python.framework/Versions/$v/bin/python$v"; do
        if [ -z "$PY" ] && command -v "$c" >/dev/null 2>&1 && "$c" -c "import tkinter" >/dev/null 2>&1; then
            PY="$(command -v "$c")"
        fi
    done
done
if [ -z "$PY" ]; then
    echo "[ERROR] Python 3.11-3.13 with Tk not found."
    echo "        Install Python 3.12 from https://www.python.org/downloads/macos/"
    echo "        or with Homebrew:  brew install python@3.12 python-tk@3.12"
    echo "        then run this file again."
    fail
fi
echo "[1/4] Found Python: $PY"
if [ "$(uname -m)" = "arm64" ] && [ "$("$PY" -c 'import platform; print(platform.machine())')" != "arm64" ]; then
    echo "      Note: this Python runs under Rosetta, so the fast Apple Silicon (MLX) engine can't be used."
    echo "      For full speed install the arm64 / universal2 Python from python.org."
fi

# ---- 2) Virtual environment and packages ----
echo "[2/4] Installing packages (~1-2 GB the first time, takes a few minutes)..."
if [ ! -x "$VENV/bin/python" ]; then
    "$PY" -m venv "$VENV" || fail
fi
"$VENV/bin/python" -m pip install --disable-pip-version-check -q -r requirements.txt || fail

# ---- 3) Whisper model ----
echo "[3/4] Downloading the speech model (~1.6 GB, first time only)..."
"$VENV/bin/python" -c "import promptflow; promptflow.download_speech_model()" || fail

# ---- 4) Ollama model (optional) ----
if command -v ollama >/dev/null 2>&1; then
    GB=$(( $(sysctl -n hw.memsize) / 1073741824 ))
    if [ "$GB" -ge 24 ]; then REC=1; else REC=2; fi
    echo "[4/4] AI cleanup model (this Mac has $GB GB of memory):"
    echo "       1) gemma3:12b  best results, for 24 GB+ Macs   (8 GB download)"
    echo "       2) gemma3:4b   for 16 GB Macs, faster          (3 GB download)"
    echo "       3) Skip        (you can pick one in the app later)"
    read -r -p "       Your choice [1/2/3, Enter = $REC]: " CHOICE
    CHOICE="${CHOICE:-$REC}"
    AIM=""
    [ "$CHOICE" = "1" ] && AIM="gemma3:12b"
    [ "$CHOICE" = "2" ] && AIM="gemma3:4b"
    if [ -n "$AIM" ]; then
        ollama pull "$AIM" || fail
        "$VENV/bin/python" -c "import promptflow as pf; pf.load_settings(); pf.AI_MODEL = '$AIM'; pf.save_settings()" \
            >/dev/null 2>&1
    fi
else
    echo "[4/4] Ollama not found, skipping."
    echo "       For AI cleanup, install it from https://ollama.com/download (or: brew install ollama)"
    echo "       and run this file again. PromptFlow works without it too (\"Raw\" mode)."
fi

# ---- The app ----
"$VENV/bin/python" -c "import platform_mac; platform_mac.ensure_launcher()" || fail

echo
echo "  Done! Starting PromptFlow; look for the microphone icon in the menu bar."
echo "  macOS will ask for three permissions: Microphone, Input Monitoring and Accessibility."
echo "  Next time, open PromptFlow from Spotlight or Launchpad."
open "$APP"
