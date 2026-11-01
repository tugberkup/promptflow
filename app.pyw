"""
PromptFlow arayüzü: tepsi ikonu, ayar penceresi ve ekranın altındaki kayıt balonu.

Motor promptflow.py'dedir; bu dosya pythonw ile konsol penceresi açmadan çalışır.
"--gizli" ile başlatılırsa (Windows açılışında) pencere açılmaz, sadece tepsiye yerleşir.
"""
import ctypes
import hashlib
import math
import os
import queue
import subprocess
import sys
import threading
import winreg
from datetime import datetime
from pathlib import Path

HERE = Path(__file__).resolve().parent

# pythonw'da konsol yok: çıktıyı log dosyasına yaz, yoksa bazı kütüphaneler yazamayınca çöker
if sys.stdout is None or sys.stderr is None:
    sys.stdout = sys.stderr = open(HERE / "promptflow.log", "w", encoding="utf-8", buffering=1)

# Tek kopya: iki PromptFlow aynı anda çalışırsa her dikte iki kez yapıştırılır
_mutex = ctypes.windll.kernel32.CreateMutexW(
    None, False, "PromptFlow-" + hashlib.md5(str(HERE).lower().encode()).hexdigest()[:8])
if ctypes.windll.kernel32.GetLastError() == 183:  # ERROR_ALREADY_EXISTS
    ctypes.windll.user32.MessageBoxW(
        None, "PromptFlow zaten çalışıyor. Saatin yanındaki ikona tıkla.", "PromptFlow", 0x40
    )
    sys.exit(0)

# Görev çubuğunda Python'un değil kendi ikonumuz görünsün
ctypes.windll.shell32.SetCurrentProcessExplicitAppUserModelID("PromptFlow")

import tkinter as tk  # noqa: E402

import customtkinter as ctk  # noqa: E402
import pyperclip  # noqa: E402
import pystray  # noqa: E402
from PIL import Image, ImageDraw  # noqa: E402

import promptflow as pf  # noqa: E402

MODES = {"Temizle": "temizle", "Prompt": "prompt", "Ham": None}
MODE_HINTS = {
    "Temizle": "\"ııı, şey\"leri ve tekrarları siler, noktalamayı düzeltir.",
    "Prompt": "Anlattığını madde madde, düzgün bir prompt'a çevirir.",
    "Ham": "Whisper ne duyduysa o. Yapay zeka kullanılmaz, en hızlısı.",
}
HOTKEYS = {
    "Sağ Ctrl": "ctrl_r", "Sağ Shift": "shift_r", "F8": "f8", "F9": "f9",
    "F10": "f10", "Scroll Lock": "scroll_lock", "Pause": "pause",
}
LANGS = {"Otomatik": None, "Türkçe": "tr", "English": "en"}

STATUS_STYLE = {
    "loading": ("Yükleniyor…", "#4B5563"),
    "ready": ("Hazır", "#1F8F4E"),
    "recording": ("Dinliyor", "#D93F45"),
    "processing": ("Yazılıyor…", "#C77418"),
    "paused": ("Duraklatıldı", "#4B5563"),
    "error": ("Hata", "#D93F45"),
}
ICON_COLORS = {"idle": "#5B5BD6", "recording": "#E5484D", "processing": "#E08A1E", "paused": "#6B7280"}

RUN_KEY = r"Software\Microsoft\Windows\CurrentVersion\Run"


def label_for(mapping, value):
    return next((label for label, v in mapping.items() if v == value), value or next(iter(mapping)))


def make_icon(color, size=256):
    """Renkli daire içinde beyaz mikrofon."""
    img = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    s = size / 64
    w = max(1, round(3.5 * s))
    d.ellipse((0, 0, size - 1, size - 1), fill=color)
    d.rounded_rectangle((24 * s, 11 * s, 40 * s, 37 * s), radius=8 * s, fill="white")
    d.arc((17 * s, 20 * s, 47 * s, 45 * s), start=0, end=180, fill="white", width=w)
    d.line((32 * s, 45 * s, 32 * s, 52 * s), fill="white", width=w)
    d.line((25 * s, 52 * s, 39 * s, 52 * s), fill="white", width=w)
    return img


def ensure_ico(img):
    path = HERE / "assets" / "promptflow.ico"
    if not path.exists():
        path.parent.mkdir(exist_ok=True)
        img.save(path, sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)])
    return path


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
            winreg.SetValueEx(key, "PromptFlow", 0, winreg.REG_SZ, f'"{pythonw}" "{HERE / "app.pyw"}" --gizli')
        else:
            try:
                winreg.DeleteValue(key, "PromptFlow")
            except FileNotFoundError:
                pass


def ensure_start_menu_shortcut(ico):
    """Windows aramasında "PromptFlow" yazınca çıksın diye Başlat menüsüne kısayol koyar."""
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
               PF_DIR=str(HERE), PF_ICO=str(ico))
    try:
        subprocess.run(["powershell", "-NoProfile", "-Command", script], env=env,
                       creationflags=subprocess.CREATE_NO_WINDOW, timeout=20)
    except Exception as e:
        pf.log(f"[kısayol oluşturulamadı] {e}")


def open_file(path, template=""):
    if not path.exists():
        path.write_text(template, encoding="utf-8")
    os.startfile(path)


class Overlay:
    """Ekranın altında kayıt durumunu gösteren küçük balon.

    Hep açık durur ama görünmezdir (alpha 0); odak çalmaz ve tıklamaları alttaki pencereye geçirir,
    böylece yazının yapıştırılacağı pencere hiç değişmez.
    """

    KEY = "#010203"  # şeffaf sayılan arka plan rengi

    def __init__(self, root, engine):
        self.engine = engine
        self.mode = None
        self.phase = 0
        self.smooth = 0.0
        self.hide_token = 0
        self.scale = root.winfo_fpixels("1i") / 96
        self.w, self.h = int(210 * self.scale), int(44 * self.scale)

        self.win = tk.Toplevel(root)
        self.win.title("PromptFlow balon")
        self.win.overrideredirect(True)
        self.win.attributes("-topmost", True)
        self.win.attributes("-transparentcolor", self.KEY)
        self.win.attributes("-alpha", 0.0)
        self.win.configure(bg=self.KEY)
        sw, sh = self.win.winfo_screenwidth(), self.win.winfo_screenheight()
        self.win.geometry(f"{self.w}x{self.h}+{(sw - self.w) // 2}+{sh - self.h - int(80 * self.scale)}")
        self.canvas = tk.Canvas(self.win, width=self.w, height=self.h, bg=self.KEY, highlightthickness=0)
        self.canvas.pack()
        self.win.update_idletasks()
        self._make_passive()
        self._tick()

    def _make_passive(self):
        user32 = ctypes.windll.user32
        hwnd = user32.GetParent(self.win.winfo_id())
        GWL_EXSTYLE = -20
        WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT = 0x08000000, 0x80, 0x20
        style = user32.GetWindowLongW(hwnd, GWL_EXSTYLE)
        user32.SetWindowLongW(hwnd, GWL_EXSTYLE, style | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TRANSPARENT)

    def show(self, mode):
        """mode: recording, processing, done, error ya da None (gizle)."""
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
        font = ("Segoe UI", 10, "bold")
        cy = h // 2
        if self.mode == "recording":
            level = self.engine.rec.level if self.engine.rec else 0.0
            self.smooth = 0.6 * self.smooth + 0.4 * min(1.0, (level / 0.08) ** 0.7)
            pulse = 4 + 1.5 * math.sin(self.phase * 0.25)
            x = 22 * s
            c.create_oval(x - pulse * s, cy - pulse * s, x + pulse * s, cy + pulse * s, fill="#E5484D", outline="")
            c.create_text(36 * s, cy, text="Dinliyorum", fill="white", font=font, anchor="w")
            for i in range(5):
                bar = (3 + 15 * self.smooth * (0.55 + 0.45 * math.sin(self.phase * 0.5 + i * 1.3))) * s
                bx = (140 + i * 10) * s
                c.create_rectangle(bx, cy - bar, bx + 5 * s, cy + bar, fill="#E5484D", outline="")
        elif self.mode == "processing":
            c.create_text(22 * s, cy, text="Yazıya çevriliyor", fill="white", font=font, anchor="w")
            for i in range(3):
                r = (2.5 + 1.5 * max(0.0, math.sin(self.phase * 0.3 - i * 0.9))) * s
                dx = (160 + i * 12) * s
                c.create_oval(dx - r, cy - r, dx + r, cy + r, fill="#E08A1E", outline="")
        elif self.mode == "done":
            c.create_text(self.w // 2, cy, text="✓  Yapıştırıldı", fill="#4ADE80", font=font)
        elif self.mode == "error":
            c.create_text(self.w // 2, cy, text="!  Bir hata oldu", fill="#F87171", font=font)


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
        ico = ensure_ico(self.icons["idle"])
        self.root.after(300, lambda: self.root.iconbitmap(str(ico)))  # customtkinter kendi ikonunu 200 ms'de koyuyor
        threading.Thread(target=ensure_start_menu_shortcut, args=(ico,), daemon=True).start()

        self._build()
        self._set_status("loading")
        self.overlay = Overlay(self.root, self.engine)
        self._start_tray()
        threading.Thread(target=self._start_engine, daemon=True).start()
        self.root.after(50, self._poll)
        if start_hidden:
            self.root.withdraw()
            self.hidden_once = True

    # ---------- arayüz ----------
    def _build(self):
        r = self.root
        r.grid_columnconfigure(0, weight=1)
        r.grid_rowconfigure(2, weight=1)
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

        card = ctk.CTkFrame(r, corner_radius=12)
        card.grid(row=1, column=0, sticky="ew", padx=20, pady=8)
        card.grid_columnconfigure(1, weight=1)

        def row(i, text, widget, pady=7):
            ctk.CTkLabel(card, text=text, anchor="w").grid(row=i, column=0, sticky="w", padx=(16, 14), pady=pady)
            widget.grid(row=i, column=1, sticky="ew", padx=(0, 16), pady=pady)

        self.mode_var = ctk.StringVar(value=label_for(MODES, pf.AI_MODE))
        row(0, "Mod", ctk.CTkSegmentedButton(card, values=list(MODES), variable=self.mode_var,
                                              command=self.on_mode), pady=(16, 2))
        self.mode_hint = ctk.CTkLabel(card, text="", text_color="gray60", font=small, anchor="w")
        self.mode_hint.grid(row=1, column=1, sticky="ew", padx=(0, 16), pady=(0, 6))
        self._update_mode_hint()

        # Kurulu modeller Ollama'dan arka planda alınır (Ollama kapalıysa pencere beklemesin)
        self.model_var = ctk.StringVar(value=pf.AI_MODEL)
        self.model_menu = ctk.CTkOptionMenu(card, values=[pf.AI_MODEL], variable=self.model_var,
                                            command=self.on_model, dynamic_resizing=False)
        row(2, "YZ modeli", self.model_menu)
        threading.Thread(target=lambda: self.events.put(("models", {"names": pf.ai_list_models()})),
                         daemon=True).start()

        self.hotkey_var = ctk.StringVar(value=label_for(HOTKEYS, pf.HOTKEY))
        row(3, "Kısayol", ctk.CTkOptionMenu(card, values=list(HOTKEYS), variable=self.hotkey_var,
                                             command=self.on_hotkey))

        mics = pf.list_mics() or ["Varsayılan"]
        current = next((m for m in mics if pf.MIC and pf.MIC.lower() in m.lower()), mics[0])
        self.mic_var = ctk.StringVar(value=current)
        row(4, "Mikrofon", ctk.CTkOptionMenu(card, values=mics, variable=self.mic_var, command=self.on_mic,
                                              dynamic_resizing=False))

        self.lang_var = ctk.StringVar(value=label_for(LANGS, pf.LANGUAGE))
        row(5, "Dil", ctk.CTkOptionMenu(card, values=list(LANGS), variable=self.lang_var, command=self.on_lang))

        switches = ctk.CTkFrame(card, fg_color="transparent")
        switches.grid(row=6, column=0, columnspan=2, sticky="ew", padx=16, pady=(8, 16))
        self.auto_var = ctk.BooleanVar(value=autostart_enabled())
        ctk.CTkSwitch(switches, text="Windows ile başlat", variable=self.auto_var,
                      command=self.on_autostart).pack(side="left")
        self.sound_var = ctk.BooleanVar(value=pf.SOUNDS)
        ctk.CTkSwitch(switches, text="Bip sesleri", variable=self.sound_var,
                      command=self.on_sounds).pack(side="left", padx=(24, 0))

        hist = ctk.CTkFrame(r, corner_radius=12)
        hist.grid(row=2, column=0, sticky="nsew", padx=20, pady=8)
        hist.grid_columnconfigure(0, weight=1)
        hist.grid_rowconfigure(1, weight=1)
        top = ctk.CTkFrame(hist, fg_color="transparent")
        top.grid(row=0, column=0, sticky="ew", padx=16, pady=(12, 2))
        ctk.CTkLabel(top, text="Son dikteler", font=bold).pack(side="left")
        ctk.CTkButton(top, text="Tüm geçmiş", width=96, height=26, fg_color="transparent", border_width=1,
                      border_color="gray35", hover_color="gray25",
                      command=lambda: open_file(pf.HISTORY_FILE)).pack(side="right")
        self.hist_list = ctk.CTkScrollableFrame(hist, fg_color="transparent")
        self.hist_list.grid(row=1, column=0, sticky="nsew", padx=6, pady=(0, 8))
        self.hist_list.grid_columnconfigure(0, weight=1)
        self.history = list(reversed(pf.read_history(20)))
        self._render_history()

        foot = ctk.CTkFrame(r, fg_color="transparent")
        foot.grid(row=3, column=0, sticky="ew", padx=20, pady=(4, 18))
        ctk.CTkButton(foot, text="Sözlüğü düzenle", width=140, fg_color="gray25", hover_color="gray30",
                      command=lambda: open_file(pf.DICTIONARY_FILE, pf.SOZLUK_TEMPLATE)).pack(side="left")
        ctk.CTkButton(foot, text="Arka plana al", width=140, command=self.hide).pack(side="right")

    def _update_mode_hint(self):
        self.mode_hint.configure(text=MODE_HINTS.get(self.mode_var.get(), ""))

    def _update_hint(self):
        if self.engine.paused:
            text = "Duraklatıldı. Tepsi ikonundan devam ettirebilirsin."
        elif self.ready:
            text = f"{self.hotkey_var.get()} tuşunu basılı tut, konuş, bırak."
        else:
            text = "Konuşma modeli yükleniyor, birkaç saniye sürer."
        self.hint.configure(text=text)

    def _set_status(self, key, text=None):
        label, color = STATUS_STYLE[key]
        self.status.configure(text=text or label, fg_color=color)

    def _render_history(self):
        for child in self.hist_list.winfo_children():
            child.destroy()
        if not self.history:
            ctk.CTkLabel(self.hist_list, text="Henüz dikte yok.\nKısayolu basılı tut ve konuşmaya başla.",
                         text_color="gray55").grid(row=0, column=0, pady=40)
            return
        today = datetime.now().strftime("%Y-%m-%d")
        for i, (stamp, text) in enumerate(self.history[:20]):
            day, _, clock = stamp.partition(" ")
            when = clock[:5] if day == today else f"{day[8:10]}.{day[5:7]} {clock[:5]}"
            preview = text if len(text) <= 240 else text[:240] + "…"

            item = ctk.CTkFrame(self.hist_list, corner_radius=8, fg_color="gray17")
            item.grid(row=i, column=0, sticky="ew", padx=4, pady=4)
            item.grid_columnconfigure(0, weight=1)
            ctk.CTkLabel(item, text=when, text_color="gray55", font=ctk.CTkFont(size=11), height=16).grid(
                row=0, column=0, sticky="w", padx=12, pady=(8, 0))
            ctk.CTkLabel(item, text=preview, justify="left", anchor="w", wraplength=300).grid(
                row=1, column=0, sticky="ew", padx=12, pady=(0, 10))
            btn = ctk.CTkButton(item, text="Kopyala", width=78, height=28, fg_color="gray28", hover_color="gray33")
            btn.configure(command=lambda t=text, b=btn: self._copy(t, b))
            btn.grid(row=0, column=1, rowspan=2, padx=10)

    def _copy(self, text, button):
        pyperclip.copy(text)
        button.configure(text="Kopyalandı")
        button.after(1200, lambda: button.winfo_exists() and button.configure(text="Kopyala"))

    # ---------- ayarlar ----------
    def on_mode(self, value):
        pf.AI_MODE = MODES[value]
        pf.save_settings()
        self._update_mode_hint()
        threading.Thread(target=pf.ai_warm, daemon=True).start()  # modeli şimdiden belleğe al

    def on_model(self, value):
        if value not in self.models:
            self.model_var.set(pf.AI_MODEL)  # "Ollama bulunamadı" gibi bir bilgi satırı seçildi
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
            set_autostart(self.auto_var.get())
        except OSError as e:
            pf.log(f"[otomatik başlatma hatası] {e}")
            self.auto_var.set(autostart_enabled())

    # ---------- motor olayları ----------
    def _start_engine(self):
        try:
            self.engine.start()
        except Exception as e:
            pf.log(f"[başlatma hatası] {e}")
            self.events.put(("error", {"text": f"Başlatılamadı: {e}"}))

    def _poll(self):
        try:
            while True:
                kind, data = self.events.get_nowait()
                self._handle(kind, data)
        except queue.Empty:
            pass
        self.root.after(50, self._poll)

    def _set_tray(self, key):
        self.tray.icon = self.icons[key]

    def _handle(self, kind, data):
        if kind == "status":
            self._set_status("loading", data.get("text"))
        elif kind == "ready":
            self.ready = True
            self._set_status("ready")
            self._update_hint()
            if self.hidden_once:
                self._notify("PromptFlow hazır", f"{self.hotkey_var.get()} tuşunu basılı tutup konuşabilirsin.")
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
                values = ["Ollama bulunamadı"]
            elif pf.AI_MODEL in self.models:
                values = self.models
            else:
                values = self.models + [f"{pf.AI_MODEL} (kurulu değil)"]
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

    # ---------- tepsi ve pencere ----------
    def _start_tray(self):
        menu = pystray.Menu(
            pystray.MenuItem("Göster", lambda: self.events.put(("ui_show", {})), default=True),
            pystray.MenuItem(lambda item: "Devam et" if self.engine.paused else "Duraklat",
                             lambda: self.events.put(("ui_pause", {}))),
            pystray.Menu.SEPARATOR,
            pystray.MenuItem("Çıkış", lambda: self.events.put(("ui_quit", {}))),
        )
        self.tray = pystray.Icon("PromptFlow", self.icons["idle"], "PromptFlow", menu)
        threading.Thread(target=self.tray.run, daemon=True).start()

    def _notify(self, title, message):
        try:
            self.tray.notify(message, title)
        except Exception:
            pass

    def toggle_pause(self):
        self.engine.paused = not self.engine.paused
        self._back_to_ready()
        self.tray.update_menu()

    def show(self):
        self.root.deiconify()
        self.root.lift()
        self.root.focus_force()

    def hide(self):
        self.root.withdraw()
        if not self.hidden_once:
            self.hidden_once = True
            self._notify("PromptFlow arka planda çalışıyor", "Saatin yanındaki ikondan tekrar açabilirsin.")

    def quit(self):
        try:
            self.tray.stop()
            if self.engine.listener:
                self.engine.listener.stop()
        finally:
            self.root.destroy()
            os._exit(0)  # CUDA ve ses thread'lerini beklemeden kapan


if __name__ == "__main__":
    App(start_hidden="--gizli" in sys.argv).root.mainloop()
