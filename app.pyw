"""
PromptFlow desktop app: tray / menu bar icon, settings window and the recording pill at the bottom of the screen.

The engine lives in promptflow.py; on Windows this file runs under pythonw, without a console window.
Started with "--hidden" (as at login) it skips the window and sits in the tray / menu bar.
"""
import math
import os
import queue
import sys
import threading
from datetime import datetime
from pathlib import Path

HERE = Path(__file__).resolve().parent

# pythonw has no console: send output to a log file, otherwise some libraries crash when they can't write
if sys.stdout is None or sys.stderr is None:
    sys.stdout = sys.stderr = open(HERE / "promptflow.log", "w", encoding="utf-8", buffering=1)

if sys.platform == "darwin":
    import platform_mac as plat
else:
    import platform_win as plat

# Single instance: with two PromptFlows running, every dictation would be pasted twice
if not plat.single_instance():
    plat.message("PromptFlow", f"PromptFlow is already running. Click {plat.TRAY_WHERE}.")
    sys.exit(0)

plat.app_init()

import tkinter as tk  # noqa: E402

import customtkinter as ctk  # noqa: E402
import pyperclip  # noqa: E402
from PIL import Image, ImageDraw  # noqa: E402

import promptflow as pf  # noqa: E402

MODES = {"Clean": "clean", "Prompt": "prompt", "Raw": None}
MODE_HINTS = {
    "Clean": "Removes fillers and repetitions, fixes punctuation.",
    "Prompt": "Rewrites what you said as a clear, structured prompt.",
    "Raw": "Exactly what Whisper heard. No AI, fastest.",
}
HOTKEYS = plat.HOTKEYS
LANGS = {"Auto-detect": None, **{name: code for code, name in pf.LANGUAGE_NAMES.items()}}

STATUS_STYLE = {
    "loading": ("Loading…", "#4B5563"),
    "ready": ("Ready", "#1F8F4E"),
    "recording": ("Listening", "#D93F45"),
    "processing": ("Writing…", "#C77418"),
    "paused": ("Paused", "#4B5563"),
    "error": ("Error", "#D93F45"),
}
ICON_COLORS = {"idle": "#5B5BD6", "recording": "#E5484D", "processing": "#E08A1E", "paused": "#6B7280"}


def label_for(mapping, value):
    return next((label for label, v in mapping.items() if v == value), value or next(iter(mapping)))


def make_icon(color, size=256, badge=True):
    """A white microphone on a colored circle; with badge=False just the microphone, in that color."""
    img = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    s = size / 64
    w = max(1, round(3.5 * s))
    ink = "white" if badge else color
    if badge:
        d.ellipse((0, 0, size - 1, size - 1), fill=color)
    else:
        s, w = size / 44, max(1, round(4 * size / 44))  # the microphone alone fills the image
        d.rounded_rectangle((15 * s, 2 * s, 29 * s, 25 * s), radius=7 * s, fill=ink)
        d.arc((8 * s, 10 * s, 36 * s, 34 * s), start=0, end=180, fill=ink, width=w)
        d.line((22 * s, 34 * s, 22 * s, 41 * s), fill=ink, width=w)
        d.line((14 * s, 41 * s, 30 * s, 41 * s), fill=ink, width=w)
        return img
    d.rounded_rectangle((24 * s, 11 * s, 40 * s, 37 * s), radius=8 * s, fill=ink)
    d.arc((17 * s, 20 * s, 47 * s, 45 * s), start=0, end=180, fill=ink, width=w)
    d.line((32 * s, 45 * s, 32 * s, 52 * s), fill=ink, width=w)
    d.line((25 * s, 52 * s, 39 * s, 52 * s), fill=ink, width=w)
    return img


def open_file(path, template=""):
    if not path.exists():
        path.write_text(template, encoding="utf-8")
    plat.open_file(path)


class Overlay:
    """A small pill at the bottom of the screen showing the recording state.

    It is always open but invisible (alpha 0); it never takes focus and lets clicks through,
    so the window the text will be pasted into never changes.
    """

    def __init__(self, root, engine):
        self.engine = engine
        self.mode = None
        self.phase = 0
        self.smooth = 0.0
        self.hide_token = 0
        self.scale = plat.ui_scale(root)
        self.w, self.h = int(210 * self.scale), int(44 * self.scale)

        self.win = tk.Toplevel(root)
        self.win.title("PromptFlow overlay")
        self.win.overrideredirect(True)
        self.win.attributes("-topmost", True)
        bg = plat.overlay_background(self.win)  # a color the OS treats as transparent
        self.win.attributes("-alpha", 0.0)
        self.win.configure(bg=bg)
        sw, sh = self.win.winfo_screenwidth(), self.win.winfo_screenheight()
        y = sh - self.h - int(plat.OVERLAY_MARGIN * self.scale)
        self.win.geometry(f"{self.w}x{self.h}+{(sw - self.w) // 2}+{y}")
        self.canvas = tk.Canvas(self.win, width=self.w, height=self.h, bg=bg, highlightthickness=0)
        self.canvas.pack()
        self.win.update_idletasks()
        plat.make_overlay_passive(self.win)
        self._tick()

    def show(self, mode):
        """mode: recording, processing, done, error, or None (hide)."""
        self.mode = mode
        self.hide_token += 1
        self.win.attributes("-alpha", 0.0 if mode is None else 0.96)
        if mode in ("done", "error"):
            token = self.hide_token
            self.win.after(1000, lambda: token == self.hide_token and self.show(None))

    def _tick(self):
        self._draw()
        self.phase += 1
        self.win.after(40, self._tick)

    def _pill(self, fill):
        c, w, h = self.canvas, self.w, self.h
        r = h // 2
        c.create_oval(0, 0, 2 * r, h - 1, fill=fill, outline="")
        c.create_oval(w - 2 * r - 1, 0, w - 1, h - 1, fill=fill, outline="")
        c.create_rectangle(r, 0, w - r, h - 1, fill=fill, outline="")

    def _draw(self):
        c, s, h = self.canvas, self.scale, self.h
        c.delete("all")
        if self.mode is None:
            return
        self._pill("#1C1C21")
        font = (*plat.UI_FONT, "bold")
        cy = h // 2
        if self.mode == "recording":
            level = self.engine.rec.level if self.engine.rec else 0.0
            self.smooth = 0.6 * self.smooth + 0.4 * min(1.0, (level / 0.08) ** 0.7)
            pulse = 4 + 1.5 * math.sin(self.phase * 0.25)
            x = 22 * s
            c.create_oval(x - pulse * s, cy - pulse * s, x + pulse * s, cy + pulse * s, fill="#E5484D", outline="")
            c.create_text(36 * s, cy, text="Listening", fill="white", font=font, anchor="w")
            for i in range(5):
                bar = (3 + 15 * self.smooth * (0.55 + 0.45 * math.sin(self.phase * 0.5 + i * 1.3))) * s
                bx = (140 + i * 10) * s
                c.create_rectangle(bx, cy - bar, bx + 5 * s, cy + bar, fill="#E5484D", outline="")
        elif self.mode == "processing":
            c.create_text(22 * s, cy, text="Transcribing", fill="white", font=font, anchor="w")
            for i in range(3):
                r = (2.5 + 1.5 * max(0.0, math.sin(self.phase * 0.3 - i * 0.9))) * s
                dx = (160 + i * 12) * s
                c.create_oval(dx - r, cy - r, dx + r, cy + r, fill="#E08A1E", outline="")
        elif self.mode == "done":
            c.create_text(self.w // 2, cy, text="✓  Pasted", fill="#4ADE80", font=font)
        elif self.mode == "error":
            c.create_text(self.w // 2, cy, text="!  Something went wrong", fill="#F87171", font=font)


class App:
    def __init__(self, start_hidden):
        pf.load_settings()
        ctk.set_appearance_mode("dark")
        ctk.set_default_color_theme("blue")

        self.root = ctk.CTk()
        self.root.title("PromptFlow")
        self.root.geometry("480x730")
        self.root.minsize(440, 600)
        self.root.protocol("WM_DELETE_WINDOW", self.hide)

        self.events = queue.Queue()
        self.engine = pf.Engine(on_event=lambda kind, data: self.events.put((kind, data)))
        self.ready = False
        self.hidden_once = False
        self.models = []
        self.icons = {name: make_icon(color) for name, color in ICON_COLORS.items()}
        plat.setup_window(self.root, self.icons["idle"], on_quit=self.quit, on_reopen=self.show)
        # Start menu shortcut on Windows, ~/Applications/PromptFlow.app on macOS
        threading.Thread(target=plat.ensure_launcher, args=(pf.log,), daemon=True).start()
        if plat.autostart_enabled():
            plat.set_autostart(True)  # keep the startup entry pointing at this folder

        self._build()
        self._set_status("loading")
        self.overlay = Overlay(self.root, self.engine)
        self._start_tray()
        threading.Thread(target=self._start_engine, daemon=True).start()
        self.root.after(50, self._poll)
        if start_hidden:
            plat.hide_window(self.root)
            self.hidden_once = True

    # ---------- window ----------
    def _build(self):
        r = self.root
        r.grid_columnconfigure(0, weight=1)
        r.grid_rowconfigure(3, weight=1)
        bold = ctk.CTkFont(size=14, weight="bold")
        small = ctk.CTkFont(size=12)

        header = ctk.CTkFrame(r, fg_color="transparent")
        header.grid(row=0, column=0, sticky="ew", padx=20, pady=(18, 6))
        header.grid_columnconfigure(1, weight=1)
        self.logo = ctk.CTkImage(self.icons["idle"], size=(38, 38))
        ctk.CTkLabel(header, image=self.logo, text="").grid(row=0, column=0, rowspan=2, padx=(0, 12))
        ctk.CTkLabel(header, text="PromptFlow", font=ctk.CTkFont(size=20, weight="bold")).grid(
            row=0, column=1, sticky="sw")
        self.hint = ctk.CTkLabel(header, text="", text_color="gray60", font=small, anchor="w", justify="left")
        self.hint.grid(row=1, column=1, sticky="nw")
        self.status = ctk.CTkLabel(header, text="", width=104, height=26, corner_radius=13,
                                   text_color="white", font=ctk.CTkFont(size=12, weight="bold"))
        self.status.grid(row=0, column=2, rowspan=2, sticky="e")

        # macOS only: shown while a permission is missing
        self.perm = ctk.CTkFrame(r, corner_radius=12, fg_color="#3A2A12", border_width=1, border_color="#C77418")
        self.perm.grid_columnconfigure(0, weight=1)
        self.perm_text = ctk.CTkLabel(self.perm, text="", justify="left", anchor="w", wraplength=390)
        self.perm_text.grid(row=0, column=0, sticky="ew", padx=14, pady=(10, 4))
        self.perm_button = ctk.CTkButton(self.perm, text="Open Settings", fg_color="#C77418",
                                         hover_color="#A86212", command=self.open_permission)
        self.perm_button.grid(row=1, column=0, sticky="w", padx=14, pady=(0, 12))
        self.missing = []
        self._check_permissions()

        card = ctk.CTkFrame(r, corner_radius=12)
        card.grid(row=2, column=0, sticky="ew", padx=20, pady=8)
        card.grid_columnconfigure(1, weight=1)

        def row(i, text, widget, pady=7):
            ctk.CTkLabel(card, text=text, anchor="w").grid(row=i, column=0, sticky="w", padx=(16, 14), pady=pady)
            widget.grid(row=i, column=1, sticky="ew", padx=(0, 16), pady=pady)

        self.mode_var = ctk.StringVar(value=label_for(MODES, pf.AI_MODE))
        row(0, "Mode", ctk.CTkSegmentedButton(card, values=list(MODES), variable=self.mode_var,
                                               command=self.on_mode), pady=(16, 2))
        self.mode_hint = ctk.CTkLabel(card, text="", text_color="gray60", font=small, anchor="w")
        self.mode_hint.grid(row=1, column=1, sticky="ew", padx=(0, 16), pady=(0, 6))
        self._update_mode_hint()

        # Installed models are fetched from Ollama in the background (so the window doesn't wait if it's off)
        self.model_var = ctk.StringVar(value=pf.AI_MODEL)
        self.model_menu = ctk.CTkOptionMenu(card, values=[pf.AI_MODEL], variable=self.model_var,
                                            command=self.on_model, dynamic_resizing=False)
        row(2, "AI model", self.model_menu)
        threading.Thread(target=lambda: self.events.put(("models", {"names": pf.ai_list_models()})),
                         daemon=True).start()

        self.hotkey_var = ctk.StringVar(value=label_for(HOTKEYS, pf.HOTKEY))
        row(3, "Hotkey", ctk.CTkOptionMenu(card, values=list(HOTKEYS), variable=self.hotkey_var,
                                            command=self.on_hotkey))

        mics = pf.list_mics() or ["Default"]
        current = next((m for m in mics if pf.MIC and pf.MIC.lower() in m.lower()), mics[0])
        self.mic_var = ctk.StringVar(value=current)
        row(4, "Microphone", ctk.CTkOptionMenu(card, values=mics, variable=self.mic_var, command=self.on_mic,
                                                dynamic_resizing=False))

        self.lang_var = ctk.StringVar(value=label_for(LANGS, pf.LANGUAGE))
        row(5, "Language", ctk.CTkOptionMenu(card, values=list(LANGS), variable=self.lang_var,
                                              command=self.on_lang))

        switches = ctk.CTkFrame(card, fg_color="transparent")
        switches.grid(row=6, column=0, columnspan=2, sticky="ew", padx=16, pady=(8, 16))
        self.auto_var = ctk.BooleanVar(value=plat.autostart_enabled())
        ctk.CTkSwitch(switches, text=plat.AUTOSTART_LABEL, variable=self.auto_var,
                      command=self.on_autostart).pack(side="left")
        self.sound_var = ctk.BooleanVar(value=pf.SOUNDS)
        ctk.CTkSwitch(switches, text="Beeps", variable=self.sound_var,
                      command=self.on_sounds).pack(side="left", padx=(24, 0))

        hist = ctk.CTkFrame(r, corner_radius=12)
        hist.grid(row=3, column=0, sticky="nsew", padx=20, pady=8)
        hist.grid_columnconfigure(0, weight=1)
        hist.grid_rowconfigure(1, weight=1)
        top = ctk.CTkFrame(hist, fg_color="transparent")
        top.grid(row=0, column=0, sticky="ew", padx=16, pady=(12, 2))
        ctk.CTkLabel(top, text="Recent dictations", font=bold).pack(side="left")
        ctk.CTkButton(top, text="Full history", width=96, height=26, fg_color="transparent", border_width=1,
                      border_color="gray35", hover_color="gray25",
                      command=lambda: open_file(pf.HISTORY_FILE)).pack(side="right")
        self.hist_list = ctk.CTkScrollableFrame(hist, fg_color="transparent")
        self.hist_list.grid(row=1, column=0, sticky="nsew", padx=6, pady=(0, 8))
        self.hist_list.grid_columnconfigure(0, weight=1)
        self.history = list(reversed(pf.read_history(20)))
        self._render_history()

        foot = ctk.CTkFrame(r, fg_color="transparent")
        foot.grid(row=4, column=0, sticky="ew", padx=20, pady=(4, 18))
        ctk.CTkButton(foot, text="Edit dictionary", width=140, fg_color="gray25", hover_color="gray30",
                      command=lambda: open_file(pf.DICTIONARY_FILE, pf.DICTIONARY_TEMPLATE)).pack(side="left")
        ctk.CTkButton(foot, text=f"Hide to {plat.TRAY_NAME}", width=140, command=self.hide).pack(side="right")

    def _check_permissions(self):
        """Shows the permission banner while something is missing; checks again every 2 s until all is allowed."""
        try:
            missing = plat.missing_permissions()
        except Exception as e:
            pf.log(f"[permission check failed] {e}")
            missing = []
        if missing != self.missing:
            self.missing = missing
            if missing:
                self.perm_text.configure(text=f"PromptFlow needs the {' and '.join(missing)} permission"
                                              f"{'s' if len(missing) > 1 else ''}. Turn PromptFlow on in the list.")
                self.perm_button.configure(text=f"Open {missing[0]}")
                self.perm.grid(row=1, column=0, sticky="ew", padx=20, pady=(4, 0))
            else:
                self.perm.grid_remove()
        if missing:
            self.root.after(2000, self._check_permissions)

    def open_permission(self):
        if self.missing:
            plat.open_permission_settings(self.missing[0])

    def _update_mode_hint(self):
        self.mode_hint.configure(text=MODE_HINTS.get(self.mode_var.get(), ""))

    def _update_hint(self):
        if self.engine.paused:
            text = f"Paused. Resume it from the {plat.TRAY_NAME} icon."
        elif self.ready:
            text = f"Hold {self.hotkey_var.get()}, speak, release."
        else:
            text = "Loading the speech model, takes a few seconds."
        self.hint.configure(text=text)

    def _set_status(self, key, text=None):
        label, color = STATUS_STYLE[key]
        self.status.configure(text=text or label, fg_color=color)

    def _render_history(self):
        for child in self.hist_list.winfo_children():
            child.destroy()
        if not self.history:
            ctk.CTkLabel(self.hist_list, text="No dictations yet.\nHold the hotkey and start talking.",
                         text_color="gray55").grid(row=0, column=0, pady=40)
            return
        today = datetime.now().strftime("%Y-%m-%d")
        for i, (stamp, text) in enumerate(self.history[:20]):
            day, _, clock = stamp.partition(" ")
            when = clock[:5] if day == today else f"{day[5:7]}/{day[8:10]} {clock[:5]}"
            preview = text if len(text) <= 240 else text[:240] + "…"

            item = ctk.CTkFrame(self.hist_list, corner_radius=8, fg_color="gray17")
            item.grid(row=i, column=0, sticky="ew", padx=4, pady=4)
            item.grid_columnconfigure(0, weight=1)
            ctk.CTkLabel(item, text=when, text_color="gray55", font=ctk.CTkFont(size=11), height=16).grid(
                row=0, column=0, sticky="w", padx=12, pady=(8, 0))
            ctk.CTkLabel(item, text=preview, justify="left", anchor="w", wraplength=300).grid(
                row=1, column=0, sticky="ew", padx=12, pady=(0, 10))
            btn = ctk.CTkButton(item, text="Copy", width=78, height=28, fg_color="gray28", hover_color="gray33")
            btn.configure(command=lambda t=text, b=btn: self._copy(t, b))
            btn.grid(row=0, column=1, rowspan=2, padx=10)

    def _copy(self, text, button):
        pyperclip.copy(text)
        button.configure(text="Copied")
        button.after(1200, lambda: button.winfo_exists() and button.configure(text="Copy"))

    # ---------- settings ----------
    def on_mode(self, value):
        pf.AI_MODE = MODES[value]
        pf.save_settings()
        self._update_mode_hint()
        threading.Thread(target=pf.ai_warm, daemon=True).start()  # load the model now

    def on_model(self, value):
        if value not in self.models:
            self.model_var.set(pf.AI_MODEL)  # an info line such as "Ollama not found" was picked
            return
        pf.AI_MODEL = value
        pf.save_settings()
        threading.Thread(target=pf.ai_warm, daemon=True).start()

    def on_hotkey(self, value):
        self.engine.set_hotkey(HOTKEYS[value])
        pf.save_settings()
        self._update_hint()

    def on_mic(self, value):
        self.engine.set_mic(value)
        pf.save_settings()

    def on_lang(self, value):
        pf.LANGUAGE = LANGS[value]
        pf.save_settings()

    def on_sounds(self):
        pf.SOUNDS = self.sound_var.get()
        pf.save_settings()

    def on_autostart(self):
        try:
            plat.set_autostart(self.auto_var.get())
        except OSError as e:
            pf.log(f"[autostart error] {e}")
            self.auto_var.set(plat.autostart_enabled())

    # ---------- engine events ----------
    def _start_engine(self):
        try:
            self.engine.start()
        except Exception as e:
            pf.log(f"[startup error] {e}")
            self.events.put(("error", {"text": f"Couldn't start: {e}"}))

    def _poll(self):
        try:
            while True:
                kind, data = self.events.get_nowait()
                self._handle(kind, data)
        except queue.Empty:
            pass
        self.root.after(50, self._poll)

    def _set_tray(self, key):
        self.tray.set_icon(key)

    def _handle(self, kind, data):
        if kind == "status":
            self._set_status("loading", data.get("text"))
        elif kind == "permissions":
            self._set_status("error", "Permission")
            self.hint.configure(text="Allow PromptFlow in System Settings (see below).")
            self._check_permissions()
            if self.hidden_once:
                self.show()
        elif kind == "ready":
            self.ready = True
            self._set_status("ready")
            self._update_hint()
            if self.hidden_once:
                self._notify("PromptFlow is ready", f"Hold {self.hotkey_var.get()} and speak.")
        elif kind == "recording":
            self._set_status("recording")
            self._set_tray("recording")
            self.overlay.show("recording")
        elif kind == "processing":
            self._set_status("processing")
            self._set_tray("processing")
            self.overlay.show("processing")
        elif kind == "done":
            self._back_to_ready()
            self.overlay.show("done")
            self.history.insert(0, (datetime.now().strftime("%Y-%m-%d %H:%M:%S"), data["text"]))
            self._render_history()
        elif kind == "idle":
            self._back_to_ready()
            self.overlay.show(None)
        elif kind == "error":
            self._set_status("error")
            self._set_tray("idle")
            self.overlay.show("error")
            self.hint.configure(text=data.get("text", "")[:120])
            self.root.after(5000, lambda: self.ready and self._back_to_ready())
        elif kind == "models":
            self.models = data["names"]
            if not self.models:
                values = ["Ollama not found"]
            elif pf.AI_MODEL in self.models:
                values = self.models
            else:
                values = self.models + [f"{pf.AI_MODEL} (not installed)"]
                self.model_var.set(values[-1])
            self.model_menu.configure(values=values)
        elif kind == "ui_show":
            self.show()
        elif kind == "ui_pause":
            self.toggle_pause()
        elif kind == "ui_quit":
            self.quit()

    def _back_to_ready(self):
        paused = self.engine.paused
        self._set_status("paused" if paused else "ready")
        self._set_tray("paused" if paused else "idle")
        self._update_hint()

    # ---------- tray / menu bar and window ----------
    def _start_tray(self):
        self.tray = plat.Tray(
            make_icon, ICON_COLORS,
            on_show=lambda: self.events.put(("ui_show", {})),
            on_pause=lambda: self.events.put(("ui_pause", {})),
            is_paused=lambda: self.engine.paused,
            on_quit=lambda: self.events.put(("ui_quit", {})),
        )

    def _notify(self, title, message):
        self.tray.notify(title, message)

    def toggle_pause(self):
        self.engine.paused = not self.engine.paused
        self._back_to_ready()
        self.tray.update_menu()

    def show(self):
        plat.show_window(self.root)

    def hide(self):
        plat.hide_window(self.root)
        if not self.hidden_once:
            self.hidden_once = True
            self._notify("PromptFlow keeps running in the background", f"Open it again from {plat.TRAY_WHERE}.")

    def quit(self):
        try:
            self.tray.stop()
            if self.engine.listener:
                self.engine.listener.stop()
        finally:
            self.root.destroy()
            os._exit(0)  # exit without waiting for CUDA and audio threads


if __name__ == "__main__":
    App(start_hidden="--hidden" in sys.argv).root.mainloop()
