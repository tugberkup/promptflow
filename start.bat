@echo off
rem Starts PromptFlow without a console; runs install.bat first if it isn't installed yet
set "VENV=%USERPROFILE%\.promptflow\venv"
if not exist "%VENV%\Scripts\pythonw.exe" (
    call "%~dp0install.bat"
    exit /b
)
start "" "%VENV%\Scripts\pythonw.exe" "%~dp0app.pyw"
