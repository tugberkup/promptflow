// Settings window. Talks to the Rust side through Tauri commands and the "engine" event.
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const { getCurrentWindow } = window.__TAURI__.window;

const STATUS = {
  loading: ["status.loading", "loading"],
  ready: ["status.ready", "ready"],
  recording: ["status.recording", "recording"],
  processing: ["status.processing", "processing"],
  paused: ["status.paused", "loading"],
  error: ["status.error", "error"],
  model: ["status.no_model", "permission"],
  permission: ["status.permission", "permission"],
};

const $ = (id) => document.getElementById(id);
const el = (tag, cls, text) => {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text !== undefined) e.textContent = text;
  return e;
};
const fmtSize = (b) => (b >= 1e9 ? `${(b / 1e9).toFixed(1)} GB` : `${Math.round(b / 1e6)} MB`);
const fmtNum = (n, digits = 1) => n.toLocaleString(pfLang, { maximumFractionDigits: digits, minimumFractionDigits: digits });

let state = null;
let catalog = null;
let statusKey = "loading";

function trayName() {
  return t(state.os === "macos" ? "tray.menu_bar" : "tray.tray");
}

function setStatus(key, text) {
  statusKey = key;
  const [label, cls] = STATUS[key];
  const s = $("status");
  s.textContent = text || t(label);
  s.className = `status ${cls}`;
}

function updateHint() {
  const hotkey = state.hotkeys.find(([name]) => name === state.settings.HOTKEY);
  if (state.paused) $("hint").textContent = t("hint.paused", { tray: trayName() });
  else if (state.ready) $("hint").textContent = t("hint.ready", { key: hotkey ? hotkey[1] : state.settings.HOTKEY });
  else $("hint").textContent = t("hint.loading");
}

function backToReady() {
  setStatus(state.paused ? "paused" : state.ready ? "ready" : "loading");
  updateHint();
}

function fillSelect(select, options, value) {
  select.replaceChildren(
    ...options.map(([v, label]) => {
      const o = document.createElement("option");
      o.value = v;
      o.textContent = label;
      return o;
    }),
  );
  select.value = value;
}

function setMode(mode) {
  for (const b of $("mode").querySelectorAll("button")) b.classList.toggle("active", b.dataset.value === mode);
  $("mode-hint").textContent = t(`mode.${mode}.hint`);
}

function renderHistory(items) {
  const list = $("history-list");
  list.replaceChildren();
  if (!items.length) {
    const p = el("p", "empty", t("history.empty"));
    p.style.whiteSpace = "pre-line";
    list.append(p);
    return;
  }
  for (const { time, text } of items) {
    const item = el("div", "item");
    const body = el("div", "text");
    body.append(el("time", "", time), text.length > 240 ? text.slice(0, 240) + "…" : text);
    const copy = el("button", "copy", t("history.copy"));
    copy.onclick = async () => {
      await navigator.clipboard.writeText(text);
      copy.textContent = t("history.copied");
      setTimeout(() => (copy.textContent = t("history.copy")), 1200);
    };
    item.append(body, copy);
    list.append(item);
  }
}

async function loadAiModels() {
  const models = await invoke("ai_models");
  const current = state.settings.AI_MODEL;
  let options = models.map((m) => [m, m]);
  if (!models.length) options = [[current, t("settings.ollama_missing")]];
  else if (!models.includes(current)) options.push([current, t("settings.not_installed", { model: current })]);
  fillSelect($("ai-model"), options, current);
}

/** Texts that come from Rust: the tray menu follows the interface language too. */
function sendTrayLabels() {
  invoke("set_tray_labels", {
    labels: { show: t("menu.show"), pause: t("menu.pause"), resume: t("menu.resume"), quit: t("menu.quit") },
  }).catch(() => {});
}

function applyLanguage() {
  setLanguage(state.ui_language);
  translatePage();
  $("autostart-label").textContent = t(state.os === "macos" ? "settings.autostart.macos" : "settings.autostart.windows");
  $("hide").textContent = t("footer.hide", { tray: trayName() });
  setMode(state.settings.AI_MODE || "raw");
  const ui = PF_UI_LANGUAGES.map((c) => [c, languageName(c, true)]);
  fillSelect($("ui-language"), [["auto", `${t("settings.auto_detect")} (${languageName(state.ui_language, true)})`], ...ui],
    state.settings.UI_LANGUAGE || "auto");
  fillSelect($("language"), [["", t("settings.auto_detect")], ...state.languages.map(([c]) => [c, languageName(c)])],
    state.settings.LANGUAGE || "");
  sendTrayLabels();
}

async function refresh() {
  state = await invoke("get_state");
  const s = state.settings;
  applyLanguage();
  fillSelect($("hotkey"), state.hotkeys, s.HOTKEY);
  fillSelect($("mic"), [["", t("settings.system_default")], ...state.mics.map((m) => [m, m])],
    state.mics.find((m) => s.MIC && m.toLowerCase().includes(s.MIC.toLowerCase())) || "");
  $("speech-model-name").textContent = modelName(s.SPEECH_MODEL);
  $("sounds").checked = s.SOUNDS;
  $("autostart").checked = state.autostart;
  renderHistory(state.history);
  if (statusKey !== "model" && statusKey !== "permission") backToReady();
  else updateHint();
  loadAiModels();
}

const set = (key, value) => invoke("set_setting", { key, value }).catch((e) => console.error(e));

// ---------------- permissions (macOS) ----------------
let permissionTimer = null;

async function checkPermissions() {
  const missing = await invoke("missing_permissions");
  $("permissions").hidden = !missing.length;
  clearTimeout(permissionTimer);
  if (!missing.length) return;
  const permission = t(`permissions.${missing[0].toLowerCase()}`);
  $("permissions-text").textContent = t("permissions.text", { permission });
  $("permissions-open").textContent = t("permissions.open", { permission });
  $("permissions-open").onclick = () => invoke("open_permission_settings", { name: missing[0] });
  permissionTimer = setTimeout(checkPermissions, 2000);
}

// ---------------- model catalog ----------------
const cards = {}; // model id -> { bar, text }

function modelName(file) {
  const m = catalog && catalog.models.find((m) => m.file === file);
  return m ? m.name : file.replace(/^ggml-|\.bin$/g, "");
}

const estimateOf = (id) => catalog.recommendation.estimates.find((e) => e.id === id);
const accuracyOf = (error) => Math.max(0, 100 - error);

function modelCard(m, tag) {
  const est = estimateOf(m.id);
  const usable = est.fits && est.error !== null;
  const card = el("div", `model${tag === "recommended" ? " recommended" : ""}${usable ? "" : " dim"}`);
  const top = el("div", "model-top");
  top.append(el("strong", "", m.name));
  if (tag) top.append(el("span", `badge${tag === "recommended" ? " rec" : ""}`, t(`catalog.badge.${tag}`)));
  const inUse = m.installed && m.file === catalog.current;
  if (inUse) top.append(el("span", "badge use", t("catalog.badge.in_use")));
  card.append(top, el("p", "", t(`model.${m.id}`)));

  const stats = el("div", "stats");
  if (est.error === null) stats.append(el("span", "warn", t("catalog.unsupported", { languages: languageList(catalog.languages) })));
  else stats.append(el("span", "good", t("catalog.accuracy", { accuracy: fmtNum(accuracyOf(est.error)) })));
  if (!est.fits) stats.append(el("span", "warn", t("catalog.too_big")));
  else stats.append(el("span", est.wait > 3 ? "warn" : "", t("catalog.wait", { wait: fmtNum(est.wait) })));
  card.append(stats);
  const meta = [fmtSize(m.size), m.english_only ? t("catalog.english_only") : t("catalog.languages_count", { count: 99 })];
  card.append(el("div", "meta", meta.join(" · ")));

  const actions = el("div", "model-actions");
  if (m.downloading) {
    const bar = el("div", "bar");
    bar.append(el("span"));
    const text = el("span", "progress-text", t("catalog.starting"));
    const cancel = el("button", "secondary", t("catalog.cancel"));
    cancel.onclick = () => invoke("cancel_download", { id: m.id });
    actions.append(bar, text, cancel);
    cards[m.id] = { bar: bar.firstChild, text };
  } else if (m.installed) {
    if (!inUse) {
      const use = el("button", "", t("catalog.use"));
      use.onclick = async () => {
        await invoke("use_model", { id: m.id });
        state.ready = false;
        backToReady();
        renderCatalog();
      };
      actions.append(use);
    } else {
      actions.append(el("span", "progress-text", t("catalog.installed")));
    }
  } else {
    const dl = el("button", "", t("catalog.download", { size: fmtSize(m.size) }));
    dl.onclick = async () => {
      await invoke("download_model", { id: m.id });
      renderCatalog();
    };
    actions.append(dl);
  }
  card.append(actions);
  return card;
}

function renderLanguageChips() {
  const chips = $("language-chips");
  chips.replaceChildren();
  const selected = catalog.languages;
  const offered = [...new Set([...selected, ...catalog.detected])];
  for (const code of offered) {
    const chip = el("button", `chip${selected.includes(code) ? " on" : ""}`, languageName(code));
    chip.onclick = () => {
      const next = selected.includes(code) ? selected.filter((c) => c !== code) : [...selected, code];
      if (next.length) setLanguages(next);
    };
    chips.append(chip);
  }
  // Any other language Whisper knows
  const add = el("select", "chip add");
  const others = state.languages.filter(([c]) => !offered.includes(c));
  fillSelect(add, [["", t("catalog.add_language")], ...others.map(([c]) => [c, languageName(c)])], "");
  add.onchange = () => add.value && setLanguages([...selected, add.value]);
  chips.append(add);
}

async function setLanguages(languages) {
  await invoke("set_speech_languages", { languages });
  renderCatalog();
}

function hardwareLine(hw) {
  const parts = [];
  if (hw.gpu_kind === "discrete" || hw.gpu_kind === "integrated") parts.push(hw.gpu);
  if (hw.gpu_kind === "discrete") parts.push(t("catalog.hw.vram", { size: Math.round(hw.vram_gb) }));
  if (hw.gpu_kind !== "apple") parts.push(hw.cpu || "CPU", t("catalog.hw.cores", { count: hw.cpu_cores }));
  else parts.push(hw.gpu);
  parts.push(t("catalog.hw.ram", { size: Math.round(hw.ram_gb) }));
  if (hw.gpu_kind === "none") parts.push(t("catalog.hw.cpu_only"));
  return parts.join(" · ");
}

async function renderAiBox(rec) {
  const box = $("ai-box");
  box.replaceChildren(el("strong", "", t("ai.title")));
  if (!rec.ai_model) {
    box.append(el("p", "sub", t("ai.raw_better")));
    return;
  }
  const installed = await invoke("ai_models");
  box.append(el("p", "", t("ai.recommended", { model: rec.ai_model })));
  if (installed.includes(rec.ai_model)) {
    box.append(el("span", "progress-text", t("ai.installed")));
    return;
  }
  const actions = el("div", "model-actions");
  const pull = el("button", "", t("ai.download", { model: rec.ai_model }));
  const status = el("span", "progress-text", installed.length ? "" : t("ai.needs_ollama"));
  pull.onclick = () => {
    pull.disabled = true;
    status.textContent = t("catalog.starting");
    invoke("pull_ai_model", { model: rec.ai_model });
  };
  const get = el("button", "secondary", t("ai.get_ollama"));
  get.onclick = () => invoke("open_link", { url: "https://ollama.com/download" });
  actions.append(pull, get, status);
  box.append(actions);
  cards.ai = { text: status, button: pull };
}

async function renderCatalog() {
  catalog = await invoke("get_catalog");
  const { hardware: hw, recommendation: rec } = catalog;
  $("catalog-hw").textContent = hardwareLine(hw);
  renderLanguageChips();

  const languages = languageList(catalog.languages);
  if (rec.recommended) {
    const est = estimateOf(rec.recommended);
    const device = hw.gpu_kind === "none" ? hw.cpu || "CPU" : hw.gpu;
    $("catalog-reason").textContent = t("catalog.reason", {
      languages, device, wait: fmtNum(est.wait), accuracy: fmtNum(accuracyOf(est.error)),
    });
  } else {
    $("catalog-reason").textContent = t("catalog.reason_none", { languages });
  }

  const tags = {};
  if (rec.more_accurate) tags[rec.more_accurate] = "accurate";
  if (rec.faster) tags[rec.faster] = "faster";
  if (rec.recommended) tags[rec.recommended] = "recommended";
  const order = [rec.recommended, rec.faster, rec.more_accurate].filter(Boolean);
  const list = $("catalog-list");
  list.replaceChildren();
  if (order.length) list.append(el("div", "group-title", t("catalog.for_you")));
  for (const id of order) list.append(modelCard(catalog.models.find((m) => m.id === id), tags[id]));
  list.append(el("div", "group-title", t("catalog.other")));
  for (const m of catalog.models) if (!tags[m.id]) list.append(modelCard(m));
  $("speech-model-name").textContent = modelName(catalog.current);
  renderAiBox(rec);
}

function openCatalog() {
  $("catalog").hidden = false;
  renderCatalog();
}

// ---------------- events ----------------
function errorText(code, detail) {
  if (code === "mic_silent" && state.os === "macos") return t("error.mic_silent.macos");
  return code ? t(`error.${code}`, { detail }) : detail;
}

function wire() {
  $("mode").addEventListener("click", (e) => {
    const mode = e.target.dataset.value;
    if (!mode) return;
    setMode(mode);
    state.settings.AI_MODE = mode === "raw" ? null : mode;
    set("mode", mode);
  });
  $("ai-model").onchange = (e) => set("ai_model", e.target.value);
  $("hotkey").onchange = (e) => {
    state.settings.HOTKEY = e.target.value;
    set("hotkey", e.target.value);
    updateHint();
  };
  $("mic").onchange = (e) => set("mic", e.target.value);
  $("language").onchange = (e) => set("language", e.target.value);
  $("ui-language").onchange = async (e) => {
    await set("ui_language", e.target.value);
    await refresh();
    if (!$("catalog").hidden) renderCatalog();
  };
  $("sounds").onchange = (e) => set("sounds", e.target.checked);
  $("autostart").onchange = (e) => set("autostart", e.target.checked);
  $("open-history").onclick = () => invoke("open_data_file", { name: "history" });
  $("edit-dictionary").onclick = () => invoke("open_data_file", { name: "dictionary" });
  $("hide").onclick = () => getCurrentWindow().hide();
  $("speech-model").onclick = openCatalog;
  $("catalog-close").onclick = () => ($("catalog").hidden = true);

  listen("engine", ({ payload: { kind, text, code } }) => {
    switch (kind) {
      case "status":
        setStatus("loading");
        break;
      case "ready":
        state.ready = true;
        backToReady();
        break;
      case "needs_model":
        state.ready = false;
        setStatus("model");
        $("hint").textContent = t("hint.pick_model");
        openCatalog();
        break;
      case "permissions":
        setStatus("permission");
        checkPermissions();
        break;
      case "permissions_ok":
        checkPermissions();
        backToReady();
        break;
      case "recording":
        setStatus("recording");
        break;
      case "processing":
        setStatus("processing");
        break;
      case "done":
        backToReady();
        refresh();
        break;
      case "idle":
        backToReady();
        break;
      case "paused":
      case "resumed":
        state.paused = kind === "paused";
        backToReady();
        break;
      case "error":
        setStatus("error");
        $("hint").textContent = errorText(code, text).slice(0, 160);
        setTimeout(backToReady, 5000);
        break;
    }
  });
  listen("download", ({ payload: { id, state: st, done, total, error } }) => {
    const card = cards[id];
    if (st === "downloading" && card) {
      card.bar.style.width = `${((done / total) * 100).toFixed(1)}%`;
      card.text.textContent = `${fmtSize(done)} / ${fmtSize(total)}`;
    } else if (st === "error") {
      renderCatalog().then(() => alert(t("catalog.download_failed", { error })));
    } else if (st !== "downloading") {
      renderCatalog();
    }
  });
  listen("ai_pull", ({ payload: { state: st, done, total, status } }) => {
    const card = cards.ai;
    if (!card) return;
    if (st === "pulling") card.text.textContent = total ? `${status} ${Math.round((done / total) * 100)}%` : status;
    else if (st === "done") {
      card.text.textContent = t("ai.installed");
      card.button.remove();
      loadAiModels();
    } else {
      card.text.textContent = status;
      card.button.disabled = false;
    }
  });
}

wire();
checkPermissions();
refresh().then(async () => {
  catalog = await invoke("get_catalog");
  $("speech-model-name").textContent = modelName(catalog.current);
  // First start: nothing downloaded yet
  if (!catalog.models.some((m) => m.installed)) openCatalog();
});
