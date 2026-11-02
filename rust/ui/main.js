// Settings window. Talks to the Rust side through Tauri commands and events.
const { invoke } = window.__TAURI__.core;

const MODE_HINTS = {
  clean: "Removes fillers and repetitions, fixes punctuation.",
  prompt: "Rewrites what you said as a clear, structured prompt.",
  raw: "Exactly what Whisper heard. No AI, fastest.",
};

const $ = (id) => document.getElementById(id);

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

async function init() {
  setMode("clean");
  renderHistory([]);
  $("mode").addEventListener("click", (e) => e.target.dataset.value && setMode(e.target.dataset.value));
  const version = await invoke("app_version");
  document.title = `PromptFlow ${version}`;
}

init();
