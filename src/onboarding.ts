import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

type Info = { model_ready: boolean; folders: string[] };
type Detected = { claude: boolean; codex: boolean; gemini: boolean; ollama: boolean };

const LAST = 5;
const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
const steps = [...document.querySelectorAll<HTMLElement>("[data-step]")];
const back = $<HTMLButtonElement>("back");
const next = $<HTMLButtonElement>("next");
let step = 1;
let folders: string[] = [];
const chosen = new Set<string>();
let micTimer: number | undefined;

function el<K extends keyof HTMLElementTagNameMap>(tag: K, cls = "", text?: string) {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text !== undefined) e.textContent = text;
  return e;
}

function show(n: number) {
  step = n;
  steps.forEach((s) => (s.hidden = Number(s.dataset.step) !== n));
  $("count").textContent = `${n} / ${LAST}`;
  back.hidden = n === 1;
  $("model").hidden = n !== 1;
  next.textContent = n === 1 ? "Get started" : n === LAST ? "Start using Otto" : "Continue";
  $("error").hidden = true;
  if (n === LAST) {
    $("ready-folders").textContent = chosen.size
      ? `I can read ${[...chosen].join(", ")}.`
      : "I won't read any of your folders until you ask.";
  }
  // Live mic status only matters while that screen is open.
  clearInterval(micTimer);
  if (n === 2) {
    refreshMic();
    micTimer = window.setInterval(refreshMic, 1500);
  }
  steps[n - 1].querySelector<HTMLElement>("h1")?.focus();
}

async function refreshMic() {
  const status = await invoke<string>("mic_status");
  $("mic-ok").hidden = status !== "granted";
  const btn = $<HTMLButtonElement>("mic-btn");
  btn.hidden = status === "granted";
  btn.textContent = status === "denied" ? "Open settings" : "Allow";
}

function renderFolders() {
  const box = $("folders");
  box.replaceChildren();
  if (!folders.length) box.append(el("span", "muted small", "No standard folders found."));
  for (const f of folders) {
    const label = el("label");
    const cb = el("input");
    cb.type = "checkbox";
    cb.checked = chosen.has(f);
    cb.addEventListener("change", () => (cb.checked ? chosen.add(f) : chosen.delete(f)));
    label.append(cb, f);
    box.append(label);
  }
}

function renderAis(d: Detected) {
  const rows: [string, boolean, boolean][] = [
    ["Claude Code", d.claude, true],
    ["Codex", d.codex, false],
    ["Gemini CLI", d.gemini, false],
    ["Ollama", d.ollama, false],
  ];
  const box = $("ais");
  box.replaceChildren();
  rows.forEach(([name, found, supported], i) => {
    const row = el("div", "ai");
    if (!supported) row.dataset.off = "";
    const id = `ai-${i}`;
    const radio = el("input");
    radio.type = "radio";
    radio.name = "ai";
    radio.id = id;
    radio.disabled = !supported;
    radio.checked = supported && found;
    const label = el("label", "", name);
    label.htmlFor = id;
    const tag = el("span", "tag", !supported ? (found ? "Found, coming soon" : "Coming soon") : found ? "Found" : "Not found");
    row.append(radio, label, tag);
    box.append(row);
  });
}

back.addEventListener("click", () => step > 1 && show(step - 1));
next.addEventListener("click", async () => {
  if (step < LAST) return show(step + 1);
  next.disabled = true;
  try {
    await invoke("finish_onboarding", { readableDirs: [...chosen] });
  } catch (e) {
    $("error").textContent = String(e);
    $("error").hidden = false;
  } finally {
    next.disabled = false;
  }
});
$("mic-btn").addEventListener("click", () => invoke("request_mic"));
$("notif-btn").addEventListener("click", () => invoke("open_settings", { pane: "notifications" }));

listen<{ state: string; text: string | null }>("otto://pill", (e) => {
  if (e.payload.state !== "heard" || !e.payload.text) return;
  const heard = $("heard");
  heard.textContent = `Heard you: ${e.payload.text}`;
  heard.hidden = false;
});

(async () => {
  show(1);
  const info = await invoke<Info>("onboarding_info");
  // TODO: download the speech model in the background; today "ready" only means the file exists.
  $("model").textContent = info.model_ready ? "Speech model ready" : "Speech model not found yet";
  folders = info.folders;
  folders.forEach((f) => chosen.add(f));
  renderFolders();
  renderAis(await invoke<Detected>("onboarding_detect"));
})();
