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
  const models = state.speech_models.includes(s.SPEECH_MODEL) ? state.speech_models : [...state.speech_models, s.SPEECH_MODEL];
  fillSelect($("speech-model"), models.map((m) => [m, m.replace(/^ggml-|\.bin$/g, "")]), s.SPEECH_MODEL);
  $("sounds").checked = s.SOUNDS;
  $("autostart-label").textContent = state.autostart_label;
  $("hide").textContent = `Hide to ${state.tray_name}`;
  renderHistory(state.history);
  backToReady();
  loadAiModels();
}

const set = (key, value) => invoke("set_setting", { key, value }).catch((e) => console.error(e));

function wire() {
  $("mode").addEventListener("click", (e) => {
    const mode = e.target.dataset.value;
    if (!mode) return;
    setMode(mode);
    state.settings.AI_MODE = mode === "raw" ? null : mode;
    set("mode", mode);
  });
  $("ai-model").onchange = (e) => set("ai_model", e.target.value);
  $("speech-model").onchange = (e) => {
    state.ready = false;
    backToReady();
    set("speech_model", e.target.value);
  };
  $("hotkey").onchange = (e) => {
    state.settings.HOTKEY = e.target.value;
    set("hotkey", e.target.value);
    updateHint();
  };
  $("mic").onchange = (e) => set("mic", e.target.value);
  $("language").onchange = (e) => set("language", e.target.value);
  $("sounds").onchange = (e) => set("sounds", e.target.checked);
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
        $("hint").textContent = `Download a speech model first (${text}).`;
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
refresh();
