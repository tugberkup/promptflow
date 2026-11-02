#!/bin/bash
# Starts PromptFlow on macOS; runs install.command first if it isn't installed yet
cd "$(dirname "$0")" || exit 1
VENV="$HOME/.promptflow/venv"
if [ ! -x "$VENV/bin/python" ]; then
    exec bash ./install.command
fi
"$VENV/bin/python" -c "import platform_mac; platform_mac.ensure_launcher()" && open "$HOME/Applications/PromptFlow.app"
