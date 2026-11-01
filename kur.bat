@echo off
setlocal
cd /d "%~dp0"
title PromptFlow kurulumu
set "HF_HUB_DISABLE_SYMLINKS_WARNING=1"
echo.
echo   PromptFlow kurulumu
echo   ===================
echo.

rem ---- 1) Python ----
set "PY="
for %%v in (3.12 3.11 3.13) do (
    if not defined PY (
        py -%%v -c "import sys" >nul 2>&1 && set "PY=py -%%v"
    )
)
if not defined PY (
    echo [HATA] Python 3.11-3.13 bulunamadi.
    echo        https://www.python.org/downloads/ adresinden Python 3.12 kur
    echo        ^(kurarken "py launcher" secenegi isaretli kalsin^), sonra bu dosyayi tekrar calistir.
    goto :fail
)
echo [1/4] Python bulundu: %PY%
nvidia-smi >nul 2>&1 || echo       Not: NVIDIA ekran karti bulunamadi, PromptFlow islemciyle ^(daha yavas^) calisacak.

rem ---- 2) Sanal ortam ve paketler ----
rem OneDrive gibi senkron klasorlerde durmasin diye kullanici klasorune kurulur
set "VENV=%USERPROFILE%\.promptflow\venv"
echo [2/4] Paketler kuruluyor ^(ilk seferde ~2 GB, birkac dakika surer^)...
if not exist "%VENV%\Scripts\python.exe" (
    %PY% -m venv "%VENV%" || goto :fail
)
"%VENV%\Scripts\python" -m pip install --disable-pip-version-check -q -r requirements.txt || goto :fail

rem ---- 3) Whisper modeli ----
echo [3/4] Konusma modeli indiriliyor ^(~1,6 GB, sadece ilk seferde^)...
"%VENV%\Scripts\python" -c "from faster_whisper import download_model; download_model('large-v3-turbo')" || goto :fail

rem ---- 4) Ollama modeli (istege bagli) ----
where ollama >nul 2>&1 || goto :no_ollama
echo [4/4] Yapay zeka ile toparlama modeli:
echo        1^) gemma3:12b  en iyi sonuc, 16 GB ekran karti icin   ^(8 GB indirme^)
echo        2^) gemma3:4b   8 GB ekran karti icin, daha hizli      ^(3 GB indirme^)
echo        3^) Atla        ^(sonra uygulamadan da secebilirsin^)
choice /c 123 /n /m "       Secimin [1/2/3]: "
if errorlevel 3 goto :shortcut
if errorlevel 2 (set "AIM=gemma3:4b") else (set "AIM=gemma3:12b")
ollama pull %AIM% || goto :fail
"%VENV%\Scripts\python" -c "import promptflow as pf; pf.load_settings(); pf.AI_MODEL = '%AIM%'; pf.save_settings()" >nul 2>&1
goto :shortcut

:no_ollama
echo [4/4] Ollama bulunamadi, atlaniyor.
echo        Yapay zeka ile toparlama icin https://ollama.com adresinden kurup bu dosyayi tekrar calistir.
echo        Ollama olmadan da PromptFlow calisir ^("Ham" mod^).

:shortcut
powershell -NoProfile -Command "$s = (New-Object -ComObject WScript.Shell).CreateShortcut('%~dp0PromptFlow.lnk'); $s.TargetPath = '%VENV%\Scripts\pythonw.exe'; $s.Arguments = '\"%~dp0app.pyw\"'; $s.WorkingDirectory = '%~dp0'; $s.IconLocation = '%~dp0assets\promptflow.ico,0'; $s.Save()"

echo.
echo   Kurulum tamam! PromptFlow aciliyor; saatin yanindaki mikrofon ikonuna bak.
echo   Sonraki seferlerde PromptFlow kisayoluna ya da Baslat menusunden "PromptFlow"a tikla.
start "" "%VENV%\Scripts\pythonw.exe" "%~dp0app.pyw"
timeout /t 5 >nul
exit /b 0

:fail
echo.
echo [HATA] Kurulum yarida kaldi. Yukaridaki mesaja bak; cozemezsen GitHub'da issue ac.
pause
exit /b 1
