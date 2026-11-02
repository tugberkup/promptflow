"""
Windows side of PromptFlow: everything that talks to Windows itself.

promptflow.py and app.pyw only use the names defined here; platform_mac.py defines the same ones for macOS.
"""
import ctypes
import hashlib
import os
import site
import subprocess
import sys
import threading
import winreg
import winsound
from pathlib import Path

HERE = Path(__file__).resolve().parent

NAME = "Windows"
DEFAULT_HOTKEY = "ctrl_r"
# No Right Alt: it's AltGr on many keyboard layouts
HOTKEYS = {
    "Right Ctrl": "ctrl_r", "Right Shift": "shift_r", "F8": "f8", "F9": "f9",
    "F10": "f10", "Scroll Lock": "scroll_lock", "Pause": "pause",
}
TRAY_NAME = "tray"
TRAY_WHERE = "the icon next to the clock"
AUTOSTART_LABEL = "Start with Windows"
UI_FONT = ("Segoe UI", 10)
OVERLAY_MARGIN = 80     # distance of the recording pill from the bottom of the screen
MIC_SILENT_HINT = "The microphone only sent silence. Check the microphone picked in PromptFlow."

RUN_KEY = r"Software\Microsoft\Windows\CurrentVersion\Run"


# ---------------- engine ----------------
def setup():
    """Make the DLLs from the pip packages nvidia-cublas / nvidia-cudnn visible."""
    for sp in site.getsitepackages():
        for sub in ("cublas", "cudnn", "cuda_nvrtc"):
            d = Path(sp) / "nvidia" / sub / "bin"
            if d.is_dir():
                os.add_dll_directory(str(d))
                os.environ["PATH"] = str(d) + os.pathsep + os.environ["PATH"]


def speech_backends():
    """Whisper backends to try, fastest first."""
    return ["cuda", "cpu"]


def beep(freq, ms):
    threading.Thread(target=winsound.Beep, args=(freq, ms), daemon=True).start()


def missing_permissions():
    return []


def request_permissions():
    pass


PROBE_MIC = False  # Windows doesn't ask for microphone access per app


def _parse_hotkey(name):
    from pynput import keyboard
    if len(name) == 1:
        return keyboard.KeyCode.from_char(name)
    return getattr(keyboard.Key, name)


def valid_hotkey(name):
    try:
        _parse_hotkey(name)
        return True
    except AttributeError:
        return False


class HotkeyListener:
    """Calls on_down / on_up when the hotkey goes down or up, and on_other when any other key is pressed."""

    def __init__(self, hotkey, on_down, on_up, on_other):
        self.key = _parse_hotkey(hotkey)
        self.on_down, self.on_up, self.on_other = on_down, on_up, on_other
        self._listener = None

    def set_hotkey(self, name):
        self.key = _parse_hotkey(name)

    def _press(self, key):
        if key == self.key:
            self.on_down()
        else:
            self.on_other()

    def _release(self, key):
        if key == self.key:
            self.on_up()

    def start(self):
        from pynput import keyboard
        self._listener = keyboard.Listener(on_press=self._press, on_release=self._release)
        self._listener.start()

    def stop(self):
        if self._listener:
            self._listener.stop()

    def join(self):
        self._listener.join()


def paste_keys():
    from pynput import keyboard
    kb = keyboard.Controller()
    with kb.pressed(keyboard.Key.ctrl):
        kb.press("v")
        kb.release("v")


# ---------------- app ----------------
_mutex = None


def single_instance():
    """False if PromptFlow is already running from this folder (every dictation would be pasted twice)."""
    global _mutex
    _mutex = ctypes.windll.kernel32.CreateMutexW(
        None, False, "PromptFlow-" + hashlib.md5(str(HERE).lower().encode()).hexdigest()[:8])
    return ctypes.windll.kernel32.GetLastError() != 183  # ERROR_ALREADY_EXISTS


def message(title, text):
    ctypes.windll.user32.MessageBoxW(None, text, title, 0x40)


def app_init():
    """Runs before the window is created."""
    # Show our own icon in the taskbar instead of Python's
    ctypes.windll.shell32.SetCurrentProcessExplicitAppUserModelID("PromptFlow")


def _ensure_ico(img):
    path = HERE / "assets" / "promptflow.ico"
    if not path.exists():
        path.parent.mkdir(exist_ok=True)
        img.save(path, sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)])
    return path


def setup_window(root, icon, on_quit, on_reopen):
    ico = _ensure_ico(icon)
    root.after(300, lambda: root.iconbitmap(str(ico)))  # customtkinter sets its own icon at 200 ms


def ui_scale(root):
    return root.winfo_fpixels("1i") / 96


def ensure_launcher(log=print):
    """Adds a Start menu shortcut so PromptFlow shows up in Windows search."""
    link = Path(os.environ["APPDATA"]) / "Microsoft" / "Windows" / "Start Menu" / "Programs" / "PromptFlow.lnk"
    if link.exists():
        return
    pythonw = Path(sys.executable).with_name("pythonw.exe")
    script = (
        "$s = (New-Object -ComObject WScript.Shell).CreateShortcut($env:PF_LINK); "
        "$s.TargetPath = $env:PF_EXE; $s.Arguments = '\"' + $env:PF_APP + '\"'; "
        "$s.WorkingDirectory = $env:PF_DIR; $s.IconLocation = $env:PF_ICO + ',0'; $s.Save()"
    )
    env = dict(os.environ, PF_LINK=str(link), PF_EXE=str(pythonw), PF_APP=str(HERE / "app.pyw"),
               PF_DIR=str(HERE), PF_ICO=str(HERE / "assets" / "promptflow.ico"))
    try:
        subprocess.run(["powershell", "-NoProfile", "-Command", script], env=env,
                       creationflags=subprocess.CREATE_NO_WINDOW, timeout=20)
    except Exception as e:
        log(f"[couldn't create the Start menu shortcut] {e}")


def autostart_enabled():
    try:
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER, RUN_KEY) as key:
            winreg.QueryValueEx(key, "PromptFlow")
        return True
    except OSError:
        return False


def set_autostart(on):
    with winreg.OpenKey(winreg.HKEY_CURRENT_USER, RUN_KEY, 0, winreg.KEY_SET_VALUE) as key:
        if on:
            pythonw = Path(sys.executable).with_name("pythonw.exe")
            winreg.SetValueEx(key, "PromptFlow", 0, winreg.REG_SZ, f'"{pythonw}" "{HERE / "app.pyw"}" --hidden')
        else:
            try:
                winreg.DeleteValue(key, "PromptFlow")
            except FileNotFoundError:
                pass


def open_file(path):
    os.startfile(path)


def open_permission_settings(name):
    pass


OVERLAY_KEY = "#010203"  # background color treated as transparent


def overlay_background(win):
    """Prepares the overlay window for a transparent background and returns the color to paint it with."""
    win.attributes("-transparentcolor", OVERLAY_KEY)
    return OVERLAY_KEY


def make_overlay_passive(win):
    """The overlay never takes focus and lets clicks through."""
    user32 = ctypes.windll.user32
    hwnd = user32.GetParent(win.winfo_id())
    GWL_EXSTYLE = -20
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT = 0x08000000, 0x80, 0x20
    style = user32.GetWindowLongW(hwnd, GWL_EXSTYLE)
    user32.SetWindowLongW(hwnd, GWL_EXSTYLE, style | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TRANSPARENT)


def show_window(root):
    root.deiconify()
    root.lift()
    root.focus_force()


def hide_window(root):
    root.withdraw()


class Tray:
    """Icon next to the clock. Menu callbacks run on the tray's own thread."""

    def __init__(self, make_icon, colors, on_show, on_pause, is_paused, on_quit):
        import pystray
        self.icons = {name: make_icon(color) for name, color in colors.items()}
        menu = pystray.Menu(
            pystray.MenuItem("Show", lambda: on_show(), default=True),
            pystray.MenuItem(lambda item: "Resume" if is_paused() else "Pause", lambda: on_pause()),
            pystray.Menu.SEPARATOR,
            pystray.MenuItem("Quit", lambda: on_quit()),
        )
        self.icon = pystray.Icon("PromptFlow", self.icons["idle"], "PromptFlow", menu)
        threading.Thread(target=self.icon.run, daemon=True).start()

    def set_icon(self, key):
        self.icon.icon = self.icons[key]

    def update_menu(self):
        self.icon.update_menu()

    def notify(self, title, message):
        try:
            self.icon.notify(message, title)
        except Exception:
            pass

    def stop(self):
        self.icon.stop()
