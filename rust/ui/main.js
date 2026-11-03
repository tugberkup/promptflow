// Settings window: sidebar pages and the first-run wizard. Talks to Rust through commands and events.
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const $ = (id) => document.getElementById(id);
const el = (tag, cls, text) => {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text !== undefined) e.textContent = text;
  return e;
};
const fmtSize = (b) => (b >= 1e9 ? `${(b / 1e9).toFixed(1)} GB` : `${Math.round(b / 1e6)} MB`);
const fmtNum = (n, digits = 1) => n.toLocaleString(pfLang, { maximumFractionDigits: digits, minimumFractionDigits: digits });
const set = (key, value) => invoke("set_setting", { key, value }).catch((e) => console.error(e));

let state = null;
let catalog = null;
let status = "loading"; // loading, ready, recording, processing, paused, error, model, permission

// ---------------- common ----------------
function fillSelect(select, options, value) {
  select.replaceChildren(...options.map(([v, label]) => {
    const o = document.createElement("option");
    o.value = v;
    o.textContent = label;
    return o;
  }));
  select.value = value;
}

const trayName = () => t(state.os === "macos" ? "tray.menu_bar" : "tray.tray");
/** A key's name in the interface language ("Right Ctrl", "Sağ Ctrl"); F-keys stay as they are. */
function keyName(name, fallback) {
  const mac = state.os === "macos" && PF_LOCALES.en[`key.${name}.macos`];
  const key = mac ? `key.${name}.macos` : `key.${name}`;
  return PF_LOCALES.en[key] ? t(key) : fallback || name.toUpperCase();
}

function hotkeyLabel() {
  const hotkey = state.hotkeys.find(([name]) => name === state.settings.HOTKEY);
  return keyName(state.settings.HOTKEY, hotkey && hotkey[1]);
}

function showPage(name) {
  for (const b of document.querySelectorAll(".nav")) b.classList.toggle("active", b.dataset.page === name);
  for (const p of document.querySelectorAll(".page")) p.classList.toggle("active", p.id === `page-${name}`);
  if (name === "models") renderCatalog();
  if (name === "ai") renderAi();
  if (name === "dictionary") loadDictionary();
  if (name === "history") renderHistory();
}

// ---------------- status ----------------
const DOT = { ready: "ready", recording: "recording", processing: "processing", model: "attention", permission: "attention", error: "recording" };
const STATUS_KEY = {
  loading: "status.loading", ready: "status.ready", recording: "status.recording", processing: "status.processing",
  paused: "status.paused", error: "status.error", model: "status.no_model", permission: "status.permission",
};

function setStatus(next, hint) {
  status = next;
  $("status").textContent = t(STATUS_KEY[next]);
  $("status-dot").className = `dot ${DOT[next] || ""}`;
  $("foot-dot").className = `dot ${DOT[next] || ""}`;
  if (hint !== undefined) $("hint").textContent = hint;
  else if (next === "paused") $("hint").textContent = t("hint.paused", { tray: trayName() });
  else if (next === "model") $("hint").textContent = t("hint.pick_model");
  else if (state.ready) $("hint").textContent = t("hint.ready", { key: hotkeyLabel() });
  else $("hint").textContent = t("hint.loading");
  $("status-key").textContent = hotkeyLabel();
}

function backToReady() {
  setStatus(state.paused ? "paused" : state.ready ? "ready" : status === "model" ? "model" : "loading");
}

// ---------------- General ----------------
function setMode(mode) {
  for (const b of $("mode").querySelectorAll(".mode")) b.classList.toggle("active", b.dataset.value === mode);
}

function sendTrayLabels() {
  invoke("set_tray_labels", {
    labels: { show: t("menu.show"), pause: t("menu.pause"), resume: t("menu.resume"), quit: t("menu.quit") },
  }).catch(() => {});
}

function applyLanguage() {
  setLanguage(state.ui_language);
  translatePage();
  $("autostart-label").textContent = t(state.os === "macos" ? "settings.autostart.macos" : "settings.autostart.windows");
  $("history-search").placeholder = t("history.search");
  $("about-version").textContent = t("about.version", { version: state.version });
  $("foot-version").textContent = `v${state.version}`;
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
  setMode(s.AI_MODE || "raw");
  fillSelect($("hotkey"), state.hotkeys.map(([name, label]) => [name, keyName(name, label)]), s.HOTKEY);
  fillSelect($("mic"), [["", t("settings.system_default")], ...state.mics.map((m) => [m, m])],
    state.mics.find((m) => s.MIC && m.toLowerCase().includes(s.MIC.toLowerCase())) || "");
  $("sounds").checked = s.SOUNDS;
  $("autostart").checked = state.autostart;
  backToReady();
  updateFooter();
}

function updateFooter() {
  if (!catalog) return;
  const m = catalog.models.find((m) => m.file === catalog.current && m.installed);
  $("foot-model").textContent = m ? m.name : t("footer.no_model");
}

// ---------------- permissions (macOS) ----------------
let permissionTimer = null;

async function checkPermissions() {
  const missing = await invoke("missing_permissions");
  $("permissions").hidden = !missing.length;
  clearTimeout(permissionTimer);
  if (!missing.length) return missing;
  const permission = t(`permissions.${missing[0].toLowerCase()}`);
  $("permissions-text").textContent = t("permissions.text", { permission });
  $("permissions-open").textContent = t("permissions.open", { permission });
  $("permissions-open").onclick = () => invoke("open_permission_settings", { name: missing[0] });
  permissionTimer = setTimeout(checkPermissions, 2000);
  return missing;
}

// ---------------- History ----------------
async function renderHistory() {
  const items = (await invoke("get_state")).history;
  const query = $("history-search").value.trim().toLowerCase();
  const shown = query ? items.filter((i) => i.text.toLowerCase().includes(query)) : items;
  const list = $("history-list");
  list.replaceChildren();
  if (!shown.length) {
    list.append(el("p", "empty", t(items.length ? "history.no_results" : "history.empty")));
    return;
  }
  for (const { time, text } of shown) {
    const item = el("div", "item");
    const body = el("div", "text");
    body.append(el("time", "", time), text);
    const copy = el("button", "btn", t("history.copy"));
    copy.onclick = async () => {
      await navigator.clipboard.writeText(text);
      copy.textContent = t("history.copied");
      setTimeout(() => (copy.textContent = t("history.copy")), 1200);
    };
    item.append(body, copy);
    list.append(item);
  }
}

// ---------------- Models ----------------
const cards = {}; // model id -> { bar, text }
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
  const meta = [fmtSize(m.size), m.english_only ? t("catalog.english_only") : t("catalog.languages_count", { count: m.language_count })];
  card.append(el("div", "meta", meta.join(" · ")));

  const actions = el("div", "actions");
  if (m.downloading) {
    const bar = el("div", "bar");
    bar.append(el("span"));
    const text = el("span", "progress", t("catalog.starting"));
    const cancel = el("button", "btn", t("catalog.cancel"));
    cancel.onclick = () => invoke("cancel_download", { id: m.id });
    actions.append(bar, text, cancel);
    cards[m.id] = { bar: bar.firstChild, text };
  } else if (m.installed && !inUse) {
    const use = el("button", "btn", t("catalog.use"));
    use.onclick = async () => {
      await invoke("use_model", { id: m.id });
      state.ready = false;
      backToReady();
      renderCatalog();
    };
    actions.append(use);
  } else if (!m.installed) {
    const dl = el("button", `btn${tag === "recommended" ? " primary" : ""}`, t("catalog.download", { size: fmtSize(m.size) }));
    dl.onclick = async () => {
      await invoke("download_model", { id: m.id });
      renderCatalog();
    };
    actions.append(dl);
  }
  if (actions.childElementCount) card.append(actions);
  return card;
}

function languageChips(container, big) {
  container.replaceChildren();
  container.classList.toggle("big-chips", !!big);
  const selected = catalog.languages;
  const offered = [...new Set([...selected, ...catalog.detected])];
  for (const code of offered) {
    const chip = el("button", `chip${selected.includes(code) ? " on" : ""}`, languageName(code));
    chip.onclick = () => {
      const next = selected.includes(code) ? selected.filter((c) => c !== code) : [...selected, code];
      if (next.length) setLanguages(next);
    };
    container.append(chip);
  }
  const add = el("select");
  const others = state.languages.filter(([c]) => !offered.includes(c));
  fillSelect(add, [["", t("catalog.add_language")], ...others.map(([c]) => [c, languageName(c)])], "");
  add.onchange = () => add.value && setLanguages([...selected, add.value]);
  container.append(add);
}

async function setLanguages(languages) {
  await invoke("set_speech_languages", { languages });
  await loadCatalog();
  if (!$("wizard").hidden) renderWizard();
  else renderCatalog(false);
}

function hardwareLine(hw) {
  const parts = [];
  if (hw.gpu_kind === "discrete" || hw.gpu_kind === "integrated") parts.push(hw.gpu);
  if (hw.gpu_kind === "discrete") parts.push(t("catalog.hw.vram", { size: Math.round(hw.vram_gb) }));
  if (hw.gpu_kind === "apple") parts.push(hw.gpu);
  else parts.push(hw.cpu || "CPU", t("catalog.hw.cores", { count: hw.cpu_cores }));
  parts.push(t("catalog.hw.ram", { size: Math.round(hw.ram_gb) }));
  if (hw.gpu_kind === "none") parts.push(t("catalog.hw.cpu_only"));
  return parts.join(" · ");
}

function reasonText() {
  const { hardware: hw, recommendation: rec } = catalog;
  const languages = languageList(catalog.languages);
  if (!rec.recommended) return t("catalog.reason_none", { languages });
  const est = estimateOf(rec.recommended);
  const device = hw.gpu_kind === "none" ? hw.cpu || "CPU" : hw.gpu;
  return t("catalog.reason", { languages, device, wait: fmtNum(est.wait), accuracy: fmtNum(accuracyOf(est.error)) });
}

/** The recommended trio, in order, with their tags. */
function suggestions() {
  const rec = catalog.recommendation;
  return [[rec.recommended, "recommended"], [rec.faster, "faster"], [rec.more_accurate, "accurate"]]
    .filter(([id]) => id)
    .map(([id, tag]) => [catalog.models.find((m) => m.id === id), tag]);
}

async function loadCatalog() {
  catalog = await invoke("get_catalog");
  updateFooter();
}

async function renderCatalog(reload = true) {
  if (reload) await loadCatalog();
  $("catalog-hw").textContent = hardwareLine(catalog.hardware);
  languageChips($("language-chips"));
  $("catalog-reason").textContent = reasonText();
  const list = $("catalog-list");
  list.replaceChildren();
  const picked = suggestions();
  if (picked.length) {
    list.append(el("div", "section-title", t("catalog.for_you")));
    const group = el("div", "group");
    for (const [m, tag] of picked) group.append(modelCard(m, tag));
    list.append(group);
  }
  list.append(el("div", "section-title", t("catalog.other")));
  const others = el("div", "group");
  const ids = picked.map(([m]) => m.id);
  for (const m of catalog.models) if (!ids.includes(m.id)) others.append(modelCard(m));
  list.append(others);
}

// ---------------- AI cleanup ----------------
async function renderAi() {
  if (!catalog) await loadCatalog();
  const installed = await invoke("ai_models");
  const current = state.settings.AI_MODEL;
  let options = installed.map((m) => [m, m]);
  if (!installed.length) options = [[current, t("settings.ollama_missing")]];
  else if (!installed.includes(current)) options.push([current, t("settings.not_installed", { model: current })]);
  fillSelect($("ai-model"), options, current);
  $("ai-status").textContent = t(installed.length ? "ai.status_ok" : "ai.status_missing");

  const box = $("ai-box");
  const rec = catalog.recommendation;
  box.replaceChildren();
  if (!rec.ai_model) {
    box.append(el("p", "muted", t("ai.raw_better")));
    return;
  }
  box.append(el("p", "", t("ai.recommended", { model: rec.ai_model })));
  const actions = el("div", "actions");
  if (installed.includes(rec.ai_model)) {
    actions.append(el("span", "progress", t("ai.installed")));
  } else {
    const pull = el("button", "btn primary", t("ai.download", { model: rec.ai_model }));
    const progress = el("span", "progress", installed.length ? "" : t("ai.needs_ollama"));
    pull.onclick = () => {
      pull.disabled = true;
      progress.textContent = t("catalog.starting");
      invoke("pull_ai_model", { model: rec.ai_model });
    };
    const get = el("button", "btn", t("ai.get_ollama"));
    get.onclick = () => invoke("open_link", { url: "https://ollama.com/download" });
    actions.append(pull, get, progress);
    cards.ai = { text: progress, button: pull };
  }
  box.append(actions);
}

// ---------------- Dictionary ----------------
let dictTimer = null;

function dictRow(wrong = "", right = "") {
  const row = el("div", "dict-row");
  const a = el("input");
  a.type = "text";
  a.value = wrong;
  a.placeholder = "clod";
  const b = el("input");
  b.type = "text";
  b.value = right;
  b.placeholder = "Claude";
  const del = el("button", "icon-btn", "✕");
  del.title = t("dictionary.delete");
  del.onclick = () => {
    row.remove();
    saveDictionarySoon();
  };
  a.oninput = b.oninput = saveDictionarySoon;
  row.append(a, el("span", "arrow", "→"), b, del);
  return row;
}

async function loadDictionary() {
  const entries = await invoke("get_dictionary");
  const list = $("dict-list");
  list.replaceChildren(...entries.map(([w, r]) => dictRow(w, r)));
  if (!entries.length) list.append(el("p", "empty", t("dictionary.empty")));
  $("dict-saved").textContent = "";
}

function saveDictionarySoon() {
  clearTimeout(dictTimer);
  dictTimer = setTimeout(async () => {
    const entries = [...$("dict-list").querySelectorAll(".dict-row")]
      .map((r) => [...r.querySelectorAll("input")].map((i) => i.value.trim()))
      .filter(([w]) => w);
    await invoke("save_dictionary", { entries });
    $("dict-saved").textContent = t("dictionary.saved");
  }, 600);
}

// ---------------- first-run wizard ----------------
const wizard = { step: 0, steps: [], tried: false };

async function startWizard() {
  await loadCatalog();
  const missing = state.os === "macos" ? await invoke("missing_permissions") : [];
  wizard.steps = ["welcome", "model", ...(state.os === "macos" ? ["permissions"] : []), "try", "done"];
  wizard.step = 0;
  wizard.tried = false;
  wizard.missing = missing;
  $("wizard").hidden = false;
  renderWizard();
}

function modelReady() {
  return catalog.models.some((m) => m.installed && m.file === catalog.current) && state.ready;
}

function renderWizard() {
  const name = wizard.steps[wizard.step];
  const box = $("wizard-step");
  box.replaceChildren();
  const title = (key, vars) => box.append(el("h1", "", t(key, vars)));
  const sub = (key, vars) => box.append(el("p", "muted", t(key, vars)));
  let canContinue = true;

  if (name === "welcome") {
    const logo = el("img", "hero-logo");
    logo.src = "icon.png";
    box.append(logo);
    title("onboarding.welcome.title");
    sub("onboarding.welcome.subtitle");
    const chips = el("div", "chips");
    box.append(chips);
    languageChips(chips, true);
    box.append(el("p", "muted small", t("onboarding.welcome.hint")));
  } else if (name === "model") {
    title("onboarding.model.title");
    sub("onboarding.model.subtitle");
    box.append(el("p", "muted small", hardwareLine(catalog.hardware)));
    const group = el("div", "group");
    group.style.marginTop = "14px";
    for (const [m, tag] of suggestions()) group.append(modelCard(m, tag));
    box.append(group);
    // a download keeps going in the background, so it is enough that one has started
    const installed = catalog.models.some((m) => m.installed);
    const downloading = catalog.models.some((m) => m.downloading);
    canContinue = installed || downloading;
    if (!installed) box.append(el("p", "muted small reason", t(downloading ? "onboarding.model.background" : "onboarding.model.wait")));
  } else if (name === "permissions") {
    title("onboarding.permissions.title");
    sub("onboarding.permissions.subtitle");
    const banner = el("div", "banner");
    if (wizard.missing.length) {
      const permission = t(`permissions.${wizard.missing[0].toLowerCase()}`);
      banner.append(el("p", "", t("permissions.text", { permission })));
      const open = el("button", "btn primary", t("permissions.open", { permission }));
      open.onclick = () => invoke("open_permission_settings", { name: wizard.missing[0] });
      banner.append(open);
      setTimeout(async () => {
        wizard.missing = await invoke("missing_permissions");
        if (wizard.steps[wizard.step] === "permissions") renderWizard();
      }, 2000);
    } else {
      banner.append(el("p", "", t("onboarding.permissions.ok")));
    }
    box.append(banner);
  } else if (name === "try") {
    title("onboarding.try.title");
    sub("onboarding.try.subtitle", { key: hotkeyLabel() });
    const area = el("textarea", "try-box");
    area.id = "try-box";
    area.placeholder = t(modelReady() ? "onboarding.try.placeholder" : "onboarding.try.loading");
    box.append(area);
    if (wizard.tried) box.append(el("p", "reason", t("onboarding.try.nice")));
    setTimeout(() => area.focus(), 50);
  } else if (name === "done") {
    title("onboarding.done.title");
    sub("onboarding.done.subtitle", { tray: trayName(), key: hotkeyLabel() });
  }

  $("wizard-back").style.visibility = wizard.step === 0 ? "hidden" : "visible";
  const last = wizard.step === wizard.steps.length - 1;
  $("wizard-next").textContent = t(last ? "onboarding.finish" : "onboarding.continue");
  $("wizard-next").disabled = !canContinue;
  $("wizard-skip").hidden = last;
  $("wizard-dots").replaceChildren(...wizard.steps.map((_, i) => el("span", i === wizard.step ? "on" : "")));
}

async function finishWizard() {
  await invoke("finish_onboarding");
  state.settings.ONBOARDED = true;
  $("wizard").hidden = true;
  await refresh();
  if (!catalog.models.some((m) => m.installed)) showPage("models");
}

async function wizardNext() {
  if (wizard.step === wizard.steps.length - 1) return finishWizard();
  wizard.step++;
  renderWizard();
}

// ---------------- events ----------------
function errorText(code, detail) {
  if (code === "mic_silent" && state.os === "macos") return t("error.mic_silent.macos");
  return code ? t(`error.${code}`, { detail }) : detail;
}

function wire() {
  for (const b of document.querySelectorAll(".nav")) b.onclick = () => showPage(b.dataset.page);
  $("mode").addEventListener("click", (e) => {
    const mode = e.target.closest(".mode")?.dataset.value;
    if (!mode) return;
    setMode(mode);
    state.settings.AI_MODE = mode === "raw" ? null : mode;
    set("mode", mode);
  });
  $("ai-model").onchange = (e) => {
    state.settings.AI_MODEL = e.target.value;
    set("ai_model", e.target.value);
  };
  $("hotkey").onchange = (e) => {
    state.settings.HOTKEY = e.target.value;
    set("hotkey", e.target.value);
    backToReady();
  };
  $("mic").onchange = (e) => set("mic", e.target.value);
  $("language").onchange = (e) => set("language", e.target.value);
  $("ui-language").onchange = async (e) => {
    await set("ui_language", e.target.value);
    await refresh();
  };
  $("sounds").onchange = (e) => set("sounds", e.target.checked);
  $("autostart").onchange = (e) => set("autostart", e.target.checked);
  $("history-search").oninput = renderHistory;
  $("open-history").onclick = () => invoke("open_data_file", { name: "history" });
  $("dict-add").onclick = () => {
    $("dict-list").querySelector(".empty")?.remove();
    const row = dictRow();
    $("dict-list").append(row);
    row.querySelector("input").focus();
  };
  $("about-github").onclick = () => invoke("open_link", { url: "https://github.com/tugberkup/promptflow" });
  $("about-issue").onclick = () => invoke("open_link", { url: "https://github.com/tugberkup/promptflow/issues" });
  $("about-data").onclick = () => invoke("open_data_file", { name: "folder" });
  $("about-wizard").onclick = startWizard;
  $("wizard-next").onclick = wizardNext;
  $("wizard-skip").onclick = finishWizard;
  $("wizard-back").onclick = () => {
    wizard.step = Math.max(0, wizard.step - 1);
    renderWizard();
  };

  listen("engine", ({ payload: { kind, text, code } }) => {
    switch (kind) {
      case "status": setStatus("loading"); break;
      case "ready":
        state.ready = true;
        backToReady();
        if (!$("wizard").hidden) renderWizard();
        break;
      case "needs_model":
        state.ready = false;
        setStatus("model");
        if ($("wizard").hidden && state.settings.ONBOARDED) showPage("models");
        break;
      case "permissions": setStatus("permission"); checkPermissions(); break;
      case "permissions_ok": checkPermissions(); backToReady(); break;
      case "recording": setStatus("recording"); break;
      case "processing": setStatus("processing"); break;
      case "done":
        backToReady();
        if (!$("wizard").hidden && wizard.steps[wizard.step] === "try" && !wizard.tried) {
          wizard.tried = true;
          const area = $("try-box");
          const typed = area ? area.value : "";
          renderWizard();
          if ($("try-box")) $("try-box").value = typed;
        }
        if ($("page-history").classList.contains("active")) renderHistory();
        break;
      case "idle": backToReady(); break;
      case "paused":
      case "resumed":
        state.paused = kind === "paused";
        backToReady();
        break;
      case "error":
        setStatus("error", errorText(code, text).slice(0, 160));
        setTimeout(backToReady, 5000);
        break;
    }
  });
  listen("download", async ({ payload: { id, state: st, done, total, error } }) => {
    const card = cards[id];
    if (st === "downloading" && card && card.bar.isConnected) {
      card.bar.style.width = `${((done / total) * 100).toFixed(1)}%`;
      card.text.textContent = `${fmtSize(done)} / ${fmtSize(total)}`;
      return;
    }
    if (st === "downloading") return;
    await loadCatalog();
    if (!$("wizard").hidden) renderWizard();
    else renderCatalog(false);
    if (st === "error") alert(t("catalog.download_failed", { error }));
  });
  listen("ai_pull", ({ payload: { state: st, done, total, status: s } }) => {
    const card = cards.ai;
    if (!card || !card.text.isConnected) return;
    if (st === "pulling") card.text.textContent = total ? `${s} ${Math.round((done / total) * 100)}%` : s;
    else if (st === "done") renderAi();
    else {
      card.text.textContent = s;
      card.button.disabled = false;
    }
  });
}

wire();
checkPermissions();
refresh().then(async () => {
  await loadCatalog();
  if (!state.settings.ONBOARDED) startWizard();
  else if (!catalog.models.some((m) => m.installed)) showPage("models");
});
