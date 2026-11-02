"""macOS only: the hotkey listener and the app bundle."""
import plistlib
import sys
import time

import pytest

if sys.platform != "darwin":
    pytest.skip("macOS only", allow_module_level=True)

import Quartz  # noqa: E402

import platform_mac as plat  # noqa: E402


def key_event(code, down, flags=0):
    event = Quartz.CGEventCreateKeyboardEvent(None, code, down)
    Quartz.CGEventSetFlags(event, flags)
    return event


def collect(listener):
    out = []
    while not listener.events.empty():
        out.append(listener.events.get())
    return out


def test_modifier_hotkey_logic():
    """Feeds events straight into the callback; needs no permission."""
    listener = plat.HotkeyListener("cmd_r", None, None, None)
    cb = lambda event, kind=Quartz.kCGEventFlagsChanged: listener._callback(None, kind, event, None)  # noqa: E731
    cmd = Quartz.kCGEventFlagMaskCommand
    cb(key_event(54, True, cmd | 0x10))                         # Right Command down
    cb(key_event(9, True, cmd | 0x10), Quartz.kCGEventKeyDown)  # V while holding it -> a shortcut
    cb(key_event(54, False, 0))                                 # Right Command up
    cb(key_event(55, True, cmd | 0x08))                         # Left Command: not the hotkey
    cb(key_event(55, False, 0))
    assert collect(listener) == ["down", "other", "up", "other"]


def test_function_key_hotkey_logic():
    listener = plat.HotkeyListener("f9", None, None, None)
    listener._callback(None, Quartz.kCGEventKeyDown, key_event(101, True), None)
    listener._callback(None, Quartz.kCGEventKeyDown, key_event(0, True), None)
    listener._callback(None, Quartz.kCGEventKeyUp, key_event(101, False), None)
    assert collect(listener) == ["down", "other", "up"]


def test_real_event_tap():
    """Posts a synthetic Right Command press; needs Input Monitoring (CI machines may not have it)."""
    calls = []
    listener = plat.HotkeyListener("cmd_r", lambda: calls.append("down"), lambda: calls.append("up"),
                                   lambda: calls.append("other"))
    try:
        listener.start()
    except PermissionError:
        pytest.skip("no Input Monitoring permission")
    time.sleep(0.3)
    Quartz.CGEventPost(Quartz.kCGHIDEventTap, key_event(54, True, Quartz.kCGEventFlagMaskCommand | 0x10))
    Quartz.CGEventPost(Quartz.kCGHIDEventTap, key_event(54, False, 0))
    for _ in range(20):
        if len(calls) >= 2:
            break
        time.sleep(0.1)
    listener.stop()
    if not calls:
        pytest.skip("synthetic events were not delivered on this machine")
    assert calls[:2] == ["down", "up"]


def test_app_bundle(tmp_path, monkeypatch):
    monkeypatch.setattr(plat, "APP_DIR", tmp_path / "PromptFlow.app")
    plat.ensure_launcher()
    info = plistlib.loads((tmp_path / "PromptFlow.app" / "Contents" / "Info.plist").read_bytes())
    assert info["CFBundleExecutable"] == "PromptFlow" and "NSMicrophoneUsageDescription" in info
    exe = tmp_path / "PromptFlow.app" / "Contents" / "MacOS" / "PromptFlow"
    assert exe.stat().st_mode & 0o111
    spec = (tmp_path / "PromptFlow.app" / "Contents" / "Resources" / "launcher.txt").read_text()
    assert sys.executable in spec
    if plat._has_compiler():
        assert exe.read_bytes()[:4] == b"\xcf\xfa\xed\xfe", "the launcher should be a compiled program"
    built = exe.stat().st_mtime_ns
    plat.ensure_launcher()  # already up to date: not rebuilt
    assert exe.stat().st_mtime_ns == built
    assert (tmp_path / "PromptFlow.app" / "Contents" / "Resources" / "PromptFlow.icns").exists()


def test_permissions_check_runs():
    assert set(plat.missing_permissions()) <= {"Input Monitoring", "Accessibility"}
