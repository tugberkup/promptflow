"""
macOS side of PromptFlow: everything that talks to macOS itself.

Defines the same names as platform_win.py. Needs pyobjc (Quartz, AppKit, ApplicationServices).

macOS asks for three permissions, all granted to "PromptFlow" (the app bundle in ~/Applications):
- Microphone: to record while the hotkey is held
- Input Monitoring: to notice the hotkey in other apps
- Accessibility: to press Cmd+V for you
"""
import fcntl
import hashlib
import io
import platform
import plistlib
import queue
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

import Quartz
from AppKit import NSApplication, NSImage, NSMenu, NSMenuItem, NSObject, NSStatusBar
from Foundation import NSBundle, NSData

HERE = Path(__file__).resolve().parent

NAME = "macOS"
DEFAULT_HOTKEY = "cmd_r"
HOTKEYS = {
    "Right Command": "cmd_r", "Right Option": "alt_r", "Right Control": "ctrl_r", "Right Shift": "shift_r",
    "Fn (Globe)": "fn", "F8": "f8", "F9": "f9", "F10": "f10", "F13": "f13",
}
TRAY_NAME = "menu bar"
TRAY_WHERE = "the microphone icon in the menu bar"
AUTOSTART_LABEL = "Start at login"
UI_FONT = ("Helvetica Neue", 13)
OVERLAY_MARGIN = 110    # above the Dock
MIC_SILENT_HINT = ("The microphone only sent silence. Allow PromptFlow in System Settings → "
                   "Privacy & Security → Microphone, then restart it.")

BUNDLE_ID = "io.github.tugberkup.promptflow"
APP_DIR = Path.home() / "Applications" / "PromptFlow.app"
LAUNCH_AGENT = Path.home() / "Library" / "LaunchAgents" / f"{BUNDLE_ID}.plist"

SETTINGS_PANES = {
    "Input Monitoring": "Privacy_ListenEvent",
    "Accessibility": "Privacy_Accessibility",
    "Microphone": "Privacy_Microphone",
}


# ---------------- engine ----------------
def setup():
    pass


def speech_backends():
    """Whisper backends to try, fastest first. MLX runs on the GPU of Apple Silicon Macs."""
    return ["mlx", "cpu"] if platform.machine() == "arm64" else ["cpu"]


def beep(freq, ms):
    import numpy as np
    import sounddevice as sd
    t = np.arange(int(44100 * ms / 1000)) / 44100
    sd.play((0.2 * np.sin(2 * np.pi * freq * t)).astype(np.float32), 44100)


PROBE_MIC = True  # open the microphone once at start so macOS asks for access before the first dictation


def _ax_trusted(prompt=False):
    from ApplicationServices import AXIsProcessTrustedWithOptions, kAXTrustedCheckOptionPrompt
    return bool(AXIsProcessTrustedWithOptions({kAXTrustedCheckOptionPrompt: prompt}))


def missing_permissions():
    """Names of the permissions PromptFlow still needs (the microphone can't be checked without asking)."""
    missing = []
    if hasattr(Quartz, "CGPreflightListenEventAccess") and not Quartz.CGPreflightListenEventAccess():
        missing.append("Input Monitoring")
    if not _ax_trusted():
        missing.append("Accessibility")
    return missing


def request_permissions():
    """Shows macOS' own permission prompts for whatever is still missing."""
    missing = missing_permissions()
    if "Input Monitoring" in missing:
        Quartz.CGRequestListenEventAccess()
    if "Accessibility" in missing:
        _ax_trusted(prompt=True)


def open_permission_settings(name):
    pane = SETTINGS_PANES.get(name, "")
    subprocess.Popen(["open", f"x-apple.systempreferences:com.apple.preference.security?{pane}"])


# Hotkeys. Modifier keys only send "flags changed" events; the device flag tells left and right apart.
MODIFIERS = {  # name: (key code, flag that is set while the key is down)
    "cmd_r": (54, 0x10), "shift_r": (60, 0x04), "alt_r": (61, 0x40), "ctrl_r": (62, 0x2000),
    "fn": (63, 0x800000),
}
MODIFIER_FLAGS = {54: 0x10, 55: 0x08, 56: 0x02, 58: 0x20, 59: 0x01, 60: 0x04, 61: 0x40, 62: 0x2000, 63: 0x800000}
KEYS = {"f8": 100, "f9": 101, "f10": 109, "f13": 105}
KEY_V = 9  # the V key in ANSI / ISO layouts


def valid_hotkey(name):
    return name in MODIFIERS or name in KEYS


class HotkeyListener:
    """Calls on_down / on_up when the hotkey goes down or up, and on_other when any other key is pressed.

    Uses a listen-only Quartz event tap: it only sees key codes, never needs the keyboard layout
    (asking for the layout off the main thread crashes on recent macOS), and can't block typing.
    The callbacks run on a separate thread so a slow one never makes macOS disable the tap.
    """

    def __init__(self, hotkey, on_down, on_up, on_other):
        self.set_hotkey(hotkey)
        self.on_down, self.on_up, self.on_other = on_down, on_up, on_other
        self.events = queue.Queue()
        self.tap = None
        self.loop = None
        self.thread = None

    def set_hotkey(self, name):
        if not valid_hotkey(name):
            raise ValueError(f"Unknown hotkey on macOS: {name}")
        self.hotkey = name

    def _callback(self, proxy, kind, event, refcon):
        if kind in (Quartz.kCGEventTapDisabledByTimeout, Quartz.kCGEventTapDisabledByUserInput):
            Quartz.CGEventTapEnable(self.tap, True)
            return event
        code = Quartz.CGEventGetIntegerValueField(event, Quartz.kCGKeyboardEventKeycode)
        if kind == Quartz.kCGEventFlagsChanged:
            flag = MODIFIER_FLAGS.get(code)
            if flag is not None:
                down = bool(Quartz.CGEventGetFlags(event) & flag)
                if self.hotkey in MODIFIERS and MODIFIERS[self.hotkey][0] == code:
                    self.events.put("down" if down else "up")
                elif down:
                    self.events.put("other")
        elif kind == Quartz.kCGEventKeyDown:
            self.events.put("down" if KEYS.get(self.hotkey) == code else "other")
        elif kind == Quartz.kCGEventKeyUp and KEYS.get(self.hotkey) == code:
            self.events.put("up")
        return event

    def _dispatch(self):
        handlers = {"down": self.on_down, "up": self.on_up, "other": self.on_other}
        while True:
            handlers[self.events.get()]()

    def _run(self):
        self.loop = Quartz.CFRunLoopGetCurrent()
        source = Quartz.CFMachPortCreateRunLoopSource(None, self.tap, 0)
        Quartz.CFRunLoopAddSource(self.loop, source, Quartz.kCFRunLoopCommonModes)
        Quartz.CGEventTapEnable(self.tap, True)
        Quartz.CFRunLoopRun()

    def start(self):
        mask = (Quartz.CGEventMaskBit(Quartz.kCGEventKeyDown) | Quartz.CGEventMaskBit(Quartz.kCGEventKeyUp)
                | Quartz.CGEventMaskBit(Quartz.kCGEventFlagsChanged))
        self.tap = Quartz.CGEventTapCreate(
            Quartz.kCGSessionEventTap, Quartz.kCGHeadInsertEventTap, Quartz.kCGEventTapOptionListenOnly,
            mask, self._callback, None)
        if self.tap is None:
            raise PermissionError("PromptFlow can't see the hotkey yet. Allow it in System Settings → "
                                  "Privacy & Security → Input Monitoring.")
        threading.Thread(target=self._dispatch, daemon=True).start()
        self.thread = threading.Thread(target=self._run, daemon=True)
        self.thread.start()

    def stop(self):
        if self.loop is not None:
            Quartz.CFRunLoopStop(self.loop)

    def join(self):
        while self.thread.is_alive():
            self.thread.join(0.5)  # a plain join() can't be interrupted with Ctrl+C


def paste_keys():
    source = Quartz.CGEventSourceCreate(Quartz.kCGEventSourceStateHIDSystemState)
    for down in (True, False):
        event = Quartz.CGEventCreateKeyboardEvent(source, KEY_V, down)
        Quartz.CGEventSetFlags(event, Quartz.kCGEventFlagMaskCommand)
        Quartz.CGEventPost(Quartz.kCGHIDEventTap, event)
        time.sleep(0.01)


# ---------------- app ----------------
_lock = None


def single_instance():
    """False if PromptFlow is already running from this folder (every dictation would be pasted twice)."""
    global _lock
    name = "promptflow-" + hashlib.md5(str(HERE).lower().encode()).hexdigest()[:8] + ".lock"
    _lock = open(Path(tempfile.gettempdir()) / name, "w")
    try:
        fcntl.flock(_lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        return True
    except OSError:
        return False


def _applescript_string(text):
    return '"' + text.replace("\\", "\\\\").replace('"', '\\"') + '"'


def message(title, text):
    subprocess.run(["osascript", "-e", f"display alert {_applescript_string(title)} "
                                       f"message {_applescript_string(text)}"])


def app_init():
    """Runs before the window is created: name the app "PromptFlow" instead of "Python" in the menu bar."""
    bundle = NSBundle.mainBundle()
    info = bundle.localizedInfoDictionary() or bundle.infoDictionary()
    if info is not None:
        info["CFBundleName"] = "PromptFlow"


def _nsimage(img, size=None, template=False):
    buf = io.BytesIO()
    img.save(buf, "PNG")
    data = buf.getvalue()
    image = NSImage.alloc().initWithData_(NSData.dataWithBytes_length_(data, len(data)))
    if size:
        image.setSize_(size)
    image.setTemplate_(template)
    return image


def setup_window(root, icon, on_quit, on_reopen):
    NSApplication.sharedApplication().setApplicationIconImage_(_nsimage(icon))
    root.createcommand("tk::mac::Quit", on_quit)                 # Cmd+Q and Quit in the Dock menu
    root.createcommand("tk::mac::ReopenApplication", on_reopen)  # click on the Dock icon


def ui_scale(root):
    return 1.0  # Tk on macOS already works in points


LAUNCHER = """#!/bin/bash
# Created by PromptFlow. macOS grants the microphone, Input Monitoring and Accessibility
# permissions to this app, and Python runs as its child so it inherits them.
cd {here}
exec >{log} 2>&1
export LANG="${{LANG:-en_US.UTF-8}}"
{python} {app} "$@"
"""


def _quote(path):
    return "'" + str(path).replace("'", "'\\''") + "'"


def ensure_launcher(log=print):
    """Creates ~/Applications/PromptFlow.app, which starts PromptFlow from this folder.

    It shows up in Spotlight and Launchpad, and macOS shows "PromptFlow" in its permission lists.
    """
    exe = APP_DIR / "Contents" / "MacOS" / "PromptFlow"
    script = LAUNCHER.format(here=_quote(HERE), log=_quote(HERE / "promptflow.log"),
                             python=_quote(sys.executable), app=_quote(HERE / "app.pyw"))
    if exe.exists() and exe.read_text() == script:
        return
    try:
        import promptflow
        version = promptflow.VERSION
    except Exception:
        version = "1.0"
    try:
        resources = APP_DIR / "Contents" / "Resources"
        exe.parent.mkdir(parents=True, exist_ok=True)
        resources.mkdir(parents=True, exist_ok=True)
        info = {
            "CFBundleName": "PromptFlow",
            "CFBundleDisplayName": "PromptFlow",
            "CFBundleIdentifier": BUNDLE_ID,
            "CFBundleExecutable": "PromptFlow",
            "CFBundlePackageType": "APPL",
            "CFBundleIconFile": "PromptFlow",
            "CFBundleShortVersionString": version,
            "CFBundleVersion": version,
            "LSMinimumSystemVersion": "12.0",
            "LSUIElement": True,  # the launcher itself has no Dock icon; the Python process shows one
            "NSHighResolutionCapable": True,
            "NSMicrophoneUsageDescription": "PromptFlow records while you hold the hotkey and turns your speech "
                                            "into text, entirely on this Mac.",
        }
        with open(APP_DIR / "Contents" / "Info.plist", "wb") as f:
            plistlib.dump(info, f)
        exe.write_text(script)
        exe.chmod(0o755)
        png = HERE / "assets" / "promptflow.png"
        if png.exists():
            from PIL import Image
            Image.open(png).save(resources / "PromptFlow.icns")
        # An ad-hoc signature lets macOS remember the permissions across updates of this file
        subprocess.run(["codesign", "--force", "--deep", "-s", "-", str(APP_DIR)], capture_output=True, timeout=30)
        log(f"Created {APP_DIR}")
    except Exception as e:
        log(f"[couldn't create {APP_DIR}] {e}")


def autostart_enabled():
    return LAUNCH_AGENT.exists()


def set_autostart(on):
    if on:
        ensure_launcher()
        LAUNCH_AGENT.parent.mkdir(parents=True, exist_ok=True)
        with open(LAUNCH_AGENT, "wb") as f:
            plistlib.dump({
                "Label": BUNDLE_ID,
                # Through "open" so macOS starts it as the PromptFlow app (and its permissions apply)
                "ProgramArguments": ["/usr/bin/open", "-g", "-a", str(APP_DIR), "--args", "--hidden"],
                "RunAtLoad": True,
            }, f)
    else:
        LAUNCH_AGENT.unlink(missing_ok=True)


def open_file(path):
    subprocess.Popen(["open", "-t", str(path)])  # the default text editor


def overlay_background(win):
    """Prepares the overlay window for a transparent background and returns the color to paint it with."""
    win.attributes("-transparent", True)
    return "systemTransparent"


def _nswindow(title):
    for window in NSApplication.sharedApplication().windows():
        if window.title() == title:
            return window
    return None


def make_overlay_passive(win):
    """The overlay never takes focus, lets clicks through and shows on every Space, even over full-screen apps."""
    window = _nswindow(win.title())
    if window is None:
        return
    window.setIgnoresMouseEvents_(True)
    window.setHidesOnDeactivate_(False)
    window.setHasShadow_(False)
    window.setLevel_(Quartz.kCGStatusWindowLevel)
    # canJoinAllSpaces | stationary | ignoresCycle | fullScreenAuxiliary
    window.setCollectionBehavior_((1 << 0) | (1 << 4) | (1 << 6) | (1 << 8))


def show_window(root):
    app = NSApplication.sharedApplication()
    app.setActivationPolicy_(0)  # regular: Dock icon and menus while the window is open
    root.deiconify()
    root.lift()
    app.activateIgnoringOtherApps_(True)


def hide_window(root):
    root.withdraw()
    NSApplication.sharedApplication().setActivationPolicy_(1)  # accessory: only the menu bar icon stays


class _MenuTarget(NSObject):
    def show_(self, sender):
        self.callbacks["show"]()

    def pause_(self, sender):
        self.callbacks["pause"]()

    def quit_(self, sender):
        self.callbacks["quit"]()


class Tray:
    """Microphone icon in the menu bar. Must be created on the main thread; menu callbacks run there too."""

    def __init__(self, make_icon, colors, on_show, on_pause, is_paused, on_quit):
        self.is_paused = is_paused
        # Black "template" images follow the menu bar's light/dark look; the active states stay colored
        self.icons = {}
        for name, color in colors.items():
            template = name in ("idle", "paused")
            glyph = make_icon("black" if template else color, 44, badge=False)
            if name == "paused":
                glyph.putalpha(glyph.getchannel("A").point(lambda a: a * 45 // 100))
            self.icons[name] = _nsimage(glyph, (18, 18), template)

        self.target = _MenuTarget.alloc().init()
        self.target.callbacks = {"show": on_show, "pause": on_pause, "quit": on_quit}
        menu = NSMenu.alloc().init()
        menu.setAutoenablesItems_(False)
        for title, action in (("Show PromptFlow", "show:"), ("Pause", "pause:"), (None, None), ("Quit", "quit:")):
            if title is None:
                menu.addItem_(NSMenuItem.separatorItem())
                continue
            item = NSMenuItem.alloc().initWithTitle_action_keyEquivalent_(title, action, "")
            item.setTarget_(self.target)
            menu.addItem_(item)
        self.pause_item = menu.itemAtIndex_(1)
        self.item = NSStatusBar.systemStatusBar().statusItemWithLength_(-1)  # variable length
        self.item.setMenu_(menu)
        self.item.button().setToolTip_("PromptFlow")
        self.set_icon("idle")

    def set_icon(self, key):
        self.item.button().setImage_(self.icons[key])

    def update_menu(self):
        self.pause_item.setTitle_("Resume" if self.is_paused() else "Pause")

    def notify(self, title, message):
        subprocess.Popen(["osascript", "-e", f"display notification {_applescript_string(message)} "
                                             f"with title {_applescript_string(title)}"])

    def stop(self):
        NSStatusBar.systemStatusBar().removeStatusItem_(self.item)
