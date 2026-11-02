@echo off
setlocal
cd /d "%~dp0"
title PromptFlow setup
set "HF_HUB_DISABLE_SYMLINKS_WARNING=1"
echo.
echo   PromptFlow setup
echo   ================
echo.

rem ---- 1) Python ----
set "PY="
for %%v in (3.12 3.11 3.13) do (
    if not defined PY (
        py -%%v -c "import sys" >nul 2>&1 && set "PY=py -%%v"
    )
)
if not defined PY (
    echo [ERROR] Python 3.11-3.13 not found.
    echo         Install Python 3.12 from https://www.python.org/downloads/
    echo         ^(keep the "py launcher" option checked^), then run this file again.
    goto :fail
)
echo [1/4] Found Python: %PY%
nvidia-smi >nul 2>&1 || echo       Note: no NVIDIA GPU found, PromptFlow will run on the CPU ^(slower^).

rem ---- 2) Virtual environment and packages ----
rem Installed under the user folder so it stays out of synced folders such as OneDrive
set "VENV=%USERPROFILE%\.promptflow\venv"
echo [2/4] Installing packages ^(~2 GB the first time, takes a few minutes^)...
if not exist "%VENV%\Scripts\python.exe" (
    %PY% -m venv "%VENV%" || goto :fail
)
"%VENV%\Scripts\python" -m pip install --disable-pip-version-check -q -r requirements.txt || goto :fail

rem ---- 3) Whisper model ----
echo [3/4] Downloading the speech model ^(~1.6 GB, first time only^)...
"%VENV%\Scripts\python" -c "from faster_whisper import download_model; download_model('large-v3-turbo')" || goto :fail

rem ---- 4) Ollama model (optional) ----
where ollama >nul 2>&1 || goto :no_ollama
echo [4/4] AI cleanup model:
echo        1^) gemma3:12b  best results, for 16 GB GPUs        ^(8 GB download^)
echo        2^) gemma3:4b   for 8 GB GPUs, faster               ^(3 GB download^)
echo        3^) Skip        ^(you can pick one in the app later^)
choice /c 123 /n /m "       Your choice [1/2/3]: "
if errorlevel 3 goto :shortcut
if errorlevel 2 (set "AIM=gemma3:4b") else (set "AIM=gemma3:12b")
ollama pull %AIM% || goto :fail
"%VENV%\Scripts\python" -c "import promptflow as pf; pf.load_settings(); pf.AI_MODEL = '%AIM%'; pf.save_settings()" >nul 2>&1
goto :shortcut

:no_ollama
echo [4/4] Ollama not found, skipping.
echo        For AI cleanup, install it from https://ollama.com and run this file again.
echo        PromptFlow works without it too ^("Raw" mode^).

:shortcut
powershell -NoProfile -Command "$s = (New-Object -ComObject WScript.Shell).CreateShortcut('%~dp0PromptFlow.lnk'); $s.TargetPath = '%VENV%\Scripts\pythonw.exe'; $s.Arguments = '\"%~dp0app.pyw\"'; $s.WorkingDirectory = '%~dp0'; $s.IconLocation = '%~dp0assets\promptflow.ico,0'; $s.Save()"

echo.
echo   Done! Starting PromptFlow; look for the microphone icon next to the clock.
echo   Next time, use the PromptFlow shortcut or search "PromptFlow" in the Start menu.
start "" "%VENV%\Scripts\pythonw.exe" "%~dp0app.pyw"
timeout /t 5 >nul
exit /b 0

:fail
echo.
echo [ERROR] Setup didn't finish. See the message above; if you can't solve it, open an issue on GitHub.
pause
exit /b 1
