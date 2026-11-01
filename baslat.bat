@echo off
rem PromptFlow'u konsolsuz baslatir; kurulum yapilmadiysa once kur.bat'i calistirir
set "VENV=%USERPROFILE%\.promptflow\venv"
if not exist "%VENV%\Scripts\pythonw.exe" (
    call "%~dp0kur.bat"
    exit /b
)
start "" "%VENV%\Scripts\pythonw.exe" "%~dp0app.pyw"
