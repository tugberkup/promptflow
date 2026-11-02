// Settings window. Talks to the Rust side through Tauri commands and the "engine" event.
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const { getCurrentWindow } = window.__TAURI__.window;

const MODE_HINTS = {
  clean: "Removes fillers and repetitions, fixes punctuation.",
  prompt: "Rewrites what you said as a clear, structured prompt.",
  raw: "Exactly what Whisper heard. No AI, fastest.",
};

const STATUS = {
  loading: ["Loading…", "loading"],
  ready: ["Ready", "ready"],
  recording: ["Listening", "recording"],
  processing: ["Writing…", "processing"],
  paused: ["Paused", "loading"],
  error: ["Error", "error"],
  model: ["No model", "permission"],
};

const $ = (id) => document.getElementById(id);
let state = null;

function setStatus(key, text) {
  const [label, cls] = STATUS[key];
  const el = $("status");
  el.textContent = text || label;
  el.className = `status ${cls}`;
}

function updateHint() {
  const hotkey = state.hotkeys.find(([name]) => name === state.settings.HOTKEY);
  if (state.paused) $("hint").textContent = `Paused. Resume it from the ${state.tray_name} icon.`;
  else if (state.ready) $("hint").textContent = `Hold ${hotkey ? hotkey[1] : "the hotkey"}, speak, release.`;
  else $("hint").textContent = "Loading the speech model, takes a few seconds.";
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
  for (const b of $("mode").querySelectorAll("button")) {
    b.classList.toggle("active", b.dataset.value === mode);
  }
  $("mode-hint").textContent = MODE_HINTS[mode] || "";
}

function renderHistory(items) {
  const list = $("history-list");
  list.replaceChildren();
  if (!items.length) {
    const p = document.createElement("p");
    p.className = "empty";
    p.textContent = "No dictations yet.\nHold the hotkey and start talking.";
    p.style.whiteSpace = "pre-line";
    list.append(p);
    return;
  }
  for (const { time, text } of items) {
    const item = document.createElement("div");
    item.className = "item";
    const body = document.createElement("div");
    body.className = "text";
    const when = document.createElement("time");
    when.textContent = time;
    body.append(when, text.length > 240 ? text.slice(0, 240) + "…" : text);
    const copy = document.createElement("button");
    copy.className = "copy";
    copy.textContent = "Copy";
    copy.onclick = async () => {
      await navigator.clipboard.writeText(text);
      copy.textContent = "Copied";
      setTimeout(() => (copy.textContent = "Copy"), 1200);
    };
    item.append(body, copy);
    list.append(item);
  }
}

async function loadAiModels() {
  const models = await invoke("ai_models");
  const current = state.settings.AI_MODEL;
  let options = models.map((m) => [m, m]);
  if (!models.length) options = [[current, "Ollama not found"]];
  else if (!models.includes(current)) options.push([current, `${current} (not installed)`]);
  fillSelect($("ai-model"), options, current);
}

async function refresh() {
  state = await invoke("get_state");
  const s = state.settings;
  document.title = "PromptFlow";
  setMode(s.AI_MODE || "raw");
  fillSelect($("hotkey"), state.hotkeys, s.HOTKEY);
  fillSelect($("mic"), [["", "System default"], ...state.mics.map((m) => [m, m])],
    state.mics.find((m) => s.MIC && m.toLowerCase().includes(s.MIC.toLowerCase())) || "");
  fillSelect($("language"), [["", "Auto-detect"], ...state.languages], s.LANGUAGE || "");
  $("speech-model").textContent = modelName(s.SPEECH_MODEL);
  $("sounds").checked = s.SOUNDS;
  $("autostart").checked = state.autostart;
  $("autostart-label").textContent = state.autostart_label;
  $("hide").textContent = `Hide to ${state.tray_name}`;
  renderHistory(state.history);
  backToReady();
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
  $("permissions-text").textContent =
    `PromptFlow needs the ${missing.join(" and ")} permission to notice the hotkey and paste for you. ` +
    "Turn PromptFlow on in the list.";
  $("permissions-open").textContent = `Open ${missing[0]} settings`;
  $("permissions-open").onclick = () => invoke("open_permission_settings", { name: missing[0] });
  permissionTimer = setTimeout(checkPermissions, 2000);
}

// ---------------- model catalog ----------------
let catalog = null;
const cards = {}; // model id -> { bar, text }

const fmtSize = (b) => (b >= 1e9 ? `${(b / 1e9).toFixed(1)} GB` : `${Math.round(b / 1e6)} MB`);
const el = (tag, cls, text) => {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text !== undefined) e.textContent = text;
  return e;
};

function modelName(file) {
  const m = catalog && catalog.models.find((m) => m.file === file);
  return m ? m.name : file.replace(/^ggml-|\.bin$/g, "");
}

function modelCard(m, tag) {
  const card = el("div", `model${tag === "Recommended" ? " recommended" : ""}`);
  const top = el("div", "model-top");
  top.append(el("strong", "", m.name));
  if (tag) top.append(el("span", `badge${tag === "Recommended" ? " rec" : ""}`, tag));
  const inUse = m.installed && m.file === catalog.current;
  if (inUse) top.append(el("span", "badge use", "In use"));
  card.append(top, el("p", "", m.description));

  const lang = state.settings.LANGUAGE;
  const meta = [fmtSize(m.size), m.english_only ? "English only" : "99 languages"];
  if (m.turkish_error && (!lang || lang === "tr")) meta.push(`Turkish error rate ${m.turkish_error.toFixed(1)}%`);
  card.append(el("div", "meta", meta.join(" · ")));

  const actions = el("div", "model-actions");
  if (m.downloading) {
    const bar = el("div", "bar");
    bar.append(el("span"));
    const text = el("span", "progress-text", "Starting…");
    const cancel = el("button", "secondary", "Cancel");
    cancel.onclick = () => invoke("cancel_download", { id: m.id });
    actions.append(bar, text, cancel);
    cards[m.id] = { bar: bar.firstChild, text };
  } else if (m.installed) {
    if (!inUse) {
      const use = el("button", "", "Use");
      use.onclick = async () => {
        await invoke("use_model", { id: m.id });
        state.ready = false;
        backToReady();
        renderCatalog();
      };
      actions.append(use);
    } else {
      actions.append(el("span", "progress-text", "Installed"));
    }
  } else {
    const dl = el("button", "", `Download ${fmtSize(m.size)}`);
    dl.onclick = async () => {
      await invoke("download_model", { id: m.id });
      renderCatalog();
    };
    actions.append(dl);
  }
  card.append(actions);
  return card;
}

async function renderAiBox(rec) {
  const box = $("ai-box");
  box.replaceChildren();
  box.append(el("strong", "", "AI cleanup (Clean and Prompt modes)"));
  if (!rec.ai_model) {
    box.append(el("p", "sub", "This computer is better off in Raw mode: an AI model would make every dictation slow."));
    return;
  }
  const installed = await invoke("ai_models");
  const p = el("p", "");
  p.append(`Recommended for your computer: `, el("strong", "", rec.ai_model), ` (through Ollama).`);
  box.append(p);
  if (installed.includes(rec.ai_model)) {
    box.append(el("span", "progress-text", "Installed ✓"));
    return;
  }
  const actions = el("div", "model-actions");
  const pull = el("button", "", `Download ${rec.ai_model}`);
  const status = el("span", "progress-text", installed.length ? "" : "Needs Ollama running.");
  pull.onclick = () => {
    pull.disabled = true;
    status.textContent = "Starting…";
    invoke("pull_ai_model", { model: rec.ai_model });
  };
  const get = el("button", "secondary", "Get Ollama");
  get.onclick = () => invoke("open_link", { url: "https://ollama.com/download" });
  actions.append(pull, get, status);
  box.append(actions);
  cards.ai = { text: status, button: pull };
}

async function renderCatalog() {
  catalog = await invoke("get_catalog");
  const { hardware: hw, recommendation: rec } = catalog;
  const parts = [hw.gpu];
  if (!hw.apple_silicon && hw.vram_gb >= 1) parts.push(`${Math.round(hw.vram_gb)} GB video memory`);
  parts.push(`${Math.round(hw.ram_gb)} GB RAM`);
  $("catalog-hw").textContent = parts.join(" · ");
  $("catalog-reason").textContent = rec.reason;

  const tags = {};
  if (rec.more_accurate) tags[rec.more_accurate] = "More accurate";
  if (rec.faster) tags[rec.faster] = "Faster";
  tags[rec.recommended] = "Recommended";
  const order = [rec.recommended, rec.faster, rec.more_accurate].filter(Boolean);
  const list = $("catalog-list");
  list.replaceChildren(el("div", "group-title", "For your computer"));
  for (const id of order) list.append(modelCard(catalog.models.find((m) => m.id === id), tags[id]));
  list.append(el("div", "group-title", "Other models"));
  for (const m of catalog.models) if (!tags[m.id]) list.append(modelCard(m));
  $("speech-model").textContent = modelName(catalog.current);
  renderAiBox(rec);
}

function openCatalog() {
  $("catalog").hidden = false;
  renderCatalog();
}

function wireCatalog() {
  $("speech-model").onclick = openCatalog;
  $("catalog-close").onclick = () => ($("catalog").hidden = true);
  listen("download", ({ payload: { id, state: st, done, total, error } }) => {
    const card = cards[id];
    if (st === "downloading" && card) {
      card.bar.style.width = `${((done / total) * 100).toFixed(1)}%`;
      card.text.textContent = `${fmtSize(done)} / ${fmtSize(total)}`;
    } else if (st === "error") {
      renderCatalog().then(() => alert(`Download failed: ${error}`));
    } else if (st !== "downloading") {
      renderCatalog();
    }
  });
  listen("ai_pull", ({ payload: { state: st, done, total, status } }) => {
    const card = cards.ai;
    if (!card) return;
    if (st === "pulling") card.text.textContent = total ? `${status} ${Math.round((done / total) * 100)}%` : status;
    else if (st === "done") {
      card.text.textContent = "Installed ✓";
      card.button.remove();
      loadAiModels();
    } else {
      card.text.textContent = status;
      card.button.disabled = false;
    }
  });
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
  $("sounds").onchange = (e) => set("sounds", e.target.checked);
  $("autostart").onchange = (e) => set("autostart", e.target.checked);
  $("open-history").onclick = () => invoke("open_data_file", { name: "history" });
  $("edit-dictionary").onclick = () => invoke("open_data_file", { name: "dictionary" });
  $("hide").onclick = () => getCurrentWindow().hide();

  listen("engine", ({ payload: { kind, text } }) => {
    switch (kind) {
      case "status":
        setStatus("loading", text);
        break;
      case "ready":
        state.ready = true;
        backToReady();
        break;
      case "needs_model":
        state.ready = false;
        setStatus("model");
        $("hint").textContent = "Pick a speech model to get started.";
        openCatalog();
        break;
      case "permissions":
        setStatus("model", "Permission");
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
        $("hint").textContent = text.slice(0, 120);
        setTimeout(backToReady, 5000);
        break;
    }
  });
}

wire();
wireCatalog();
checkPermissions();
refresh().then(async () => {
  catalog = await invoke("get_catalog");
  $("speech-model").textContent = modelName(catalog.current);
  // First start: nothing downloaded yet
  if (!catalog.models.some((m) => m.installed)) openCatalog();
});
