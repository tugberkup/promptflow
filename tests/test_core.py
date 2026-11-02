"""Tests for the parts of the engine that don't need a microphone, a GPU or Ollama. Run on Windows and macOS."""
import ast
import time
from pathlib import Path

import numpy as np
import pytest

import promptflow as pf

ROOT = Path(__file__).resolve().parent.parent


def test_strip_markdown():
    bad = ("Create this:\n\n*   **Setting:** a Turkish home.\n*   **Characters:**\n    *   Mom (enters).\n"
           "## Notes\nThis is *important*, 2 * 3 = 6, file: my_file_name.py")
    assert pf.strip_markdown(bad) == (
        "Create this:\n\n- Setting: a Turkish home.\n- Characters:\n- Mom (enters).\n"
        "Notes\nThis is important, 2 * 3 = 6, file: my_file_name.py")


def test_dictionary(tmp_path):
    path = tmp_path / "dictionary.txt"
    path.write_text("# comment\nclod -> Claude\nnext js -> Next.js\n", encoding="utf-8")
    d = pf.Dictionary(path)
    assert d.apply("ask clod about next js, not clodx") == "ask Claude about Next.js, not clodx"
    assert d.hint_words() == ["Claude", "Next.js"]


def test_history_roundtrip(tmp_path, monkeypatch):
    monkeypatch.setattr(pf, "HISTORY_FILE", tmp_path / "history.txt")
    pf.save_history("make it red no blue", "Make it blue.")
    pf.save_history("todo app", "Build a todo app:\n- Save tasks.\n- Dark mode.")
    items = pf.read_history()
    assert [text for _, text in items] == ["Make it blue.", "Build a todo app:\n- Save tasks.\n- Dark mode."]


def test_settings_roundtrip(tmp_path, monkeypatch):
    monkeypatch.setattr(pf, "SETTINGS_FILE", tmp_path / "settings.json")
    for key in pf.SAVED_SETTINGS:
        monkeypatch.setattr(pf, key, getattr(pf, key))
    pf.AI_MODE, pf.LANGUAGE = "prompt", "tr"
    pf.save_settings()
    pf.AI_MODE, pf.LANGUAGE = None, None
    pf.load_settings()
    assert (pf.AI_MODE, pf.LANGUAGE) == ("prompt", "tr")


@pytest.mark.parametrize("text, rest", [
    ("Promptflow, write a test", "write a test"),
    ("Hey Prompt Flow make it blue", "make it blue"),
    ("Promptly follow the guide", None),
    ("nothing to see here", None),
])
def test_strip_wake(text, rest):
    assert pf.strip_wake(text) == rest


def test_ai_skipped_when_ollama_is_off(monkeypatch):
    monkeypatch.setattr(pf, "OLLAMA_URL", "http://127.0.0.1:9")  # nothing listens there
    monkeypatch.setattr(pf, "AI_MODE", "clean")
    monkeypatch.setattr(pf, "ai_unreachable_until", 0.0)
    assert pf.ai_polish("um hello there", "en") == "um hello there"
    t = time.perf_counter()
    assert pf.ai_polish("second try", "en") == "second try"  # doesn't wait for the connection again
    assert time.perf_counter() - t < 0.1


def test_speech_only_drops_silence():
    assert pf.speech_only(np.zeros(pf.SAMPLE_RATE * 2, np.float32)).size == 0


def test_hotkeys():
    assert pf.plat.valid_hotkey(pf.plat.DEFAULT_HOTKEY)
    assert pf.plat.DEFAULT_HOTKEY in pf.plat.HOTKEYS.values()
    for name in pf.plat.HOTKEYS.values():
        assert pf.plat.valid_hotkey(name), name
    assert not pf.plat.valid_hotkey("no_such_key")


def public_names(path):
    tree = ast.parse(path.read_text(encoding="utf-8"))
    names = set()
    for node in tree.body:
        if isinstance(node, (ast.FunctionDef, ast.ClassDef)):
            names.add(node.name)
        elif isinstance(node, ast.Assign):
            names.update(t.id for t in node.targets if isinstance(t, ast.Name))
    return {n for n in names if not n.startswith("_") and n not in ("HERE",)}


def test_platforms_define_the_same_names():
    """Everything promptflow.py / app.pyw use must exist on both platforms."""
    win = public_names(ROOT / "platform_win.py")
    mac = public_names(ROOT / "platform_mac.py")
    used = set()
    for name in ("promptflow.py", "app.pyw"):
        src = (ROOT / name).read_text(encoding="utf-8")
        used.update(n.attr for n in ast.walk(ast.parse(src))
                    if isinstance(n, ast.Attribute) and isinstance(n.value, ast.Name) and n.value.id == "plat")
    assert used <= win, f"missing on Windows: {used - win}"
    assert used <= mac, f"missing on macOS: {used - mac}"
