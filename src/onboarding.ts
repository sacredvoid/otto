import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

type Info = { model_ready: boolean; folders: string[]; disabled: string[]; plugins: string[] };
type Server = { key: string; name: string; tools: string[] };
/** One switch. `off` is what goes in config "disabled" when it is switched off. */
type Cap = { name: string; what: string; asks: string; off: string[] | null };
type Detected = { claude: boolean; codex: boolean; gemini: boolean; ollama: boolean };
type Account = { installed: boolean; logged_in: boolean; email: string | null; method: string | null };

const LAST = 6;
const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
const steps = [...document.querySelectorAll<HTMLElement>("[data-step]")];
const back = $<HTMLButtonElement>("back");
const next = $<HTMLButtonElement>("next");
let step = 1;
let folders: string[] = [];
const chosen = new Set<string>();
const disabled = new Set<string>();
let plugins: string[] = [];
let servers: Server[] | string | null = null; // null = still checking, string = error
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

const BUILT_IN: Cap[] = [
  { name: "Read and write files", what: "Reads the folders you picked. Writes only inside each task's own folder.", asks: "Always on", off: null },
  { name: "Run commands and control Mac apps", what: "Shell commands, AppleScript, Shortcuts and screenshots.", asks: "Asks every time", off: ["Bash"] },
  { name: "Web search and web pages", what: "Looks things up online.", asks: "Asks once a task has read your files", off: ["WebSearch", "WebFetch"] },
  { name: "Spotlight search", what: "Finds files fast, only inside your picked folders.", asks: "Automatic", off: ["mcp__otto__find_files"] },
  { name: "Memory", what: "Saves short notes about you to ~/Otto/memory.md.", asks: "Automatic", off: ["mcp__otto__remember"] },
];

function capRow(c: Cap) {
  const row = el("div", "cap");
  const text = el("div", "grow");
  text.append(el("span", "name", c.name), el("span", "muted small", c.what));
  const tag = el("span", "tag", c.asks);
  row.append(text, tag);
  if (!c.off) return row;
  const off = c.off;
  const sw = el("input", "switch");
  sw.type = "checkbox";
  sw.setAttribute("role", "switch");
  sw.setAttribute("aria-label", c.name);
  sw.checked = !off.every((k) => disabled.has(k));
  sw.addEventListener("change", () => off.forEach((k) => (sw.checked ? disabled.delete(k) : disabled.add(k))));
  row.append(sw);
  return row;
}

function renderCaps() {
  const box = $("caps");
  const group = (title: string, ...rows: HTMLElement[]) => {
    const g = el("div", "cap-group");
    g.append(el("h2", "", title), ...rows);
    return g;
  };
  const connectors =
    servers === null
      ? [el("p", "muted small", "Checking your Claude account\u2026")]
      : typeof servers === "string"
        ? [el("p", "muted small", servers)]
        : servers.length
          ? servers.map((s) =>
              capRow({ name: s.name, what: `${s.tools.length} tools: ${s.tools.slice(0, 4).join(", ")}${s.tools.length > 4 ? "\u2026" : ""}`, asks: "Reading is automatic; changes ask", off: [s.key] }),
            )
          : [el("p", "muted small", "No connectors on this Claude account. Add them at claude.ai, Settings, Connectors.")];
  const groups = [group("Built in", ...BUILT_IN.map(capRow)), group("From your Claude account", ...(account ? [el("p", "muted small", `Signed in as ${account}`)] : []), ...connectors)];
  if (plugins.length) {
    groups.push(group("Plugins", ...plugins.map((p) => capRow({ name: p.split("@")[0], what: `Claude Code plugin ${p}`, asks: "Asks unless allowed in config", off: [`plugin:${p}`] }))));
  }
  box.replaceChildren(...groups);
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

function button(text: string, cls: string, onClick: () => void) {
  const b = el("button", cls, text);
  b.type = "button";
  b.addEventListener("click", onClick);
  return b;
}

/** What this sign-in's account can reach; reruns whenever the account changes. */
let account = "";
function loadConnectors() {
  servers = null;
  renderCaps();
  invoke<Server[]>("detect_connectors")
    .then((s) => (servers = s))
    .catch((e) => (servers = `Couldn't check connectors: ${e}`))
    .finally(renderCaps);
}

const WHERE =
  "Otto has its own sign-in, separate from Claude Code in your Terminal. The token is saved in your Mac's Keychain " +
  '(as "Claude Code-credentials" plus a short code), is used only for Otto\'s tasks, and only goes to Anthropic. ' +
  "Signing in or out here doesn't touch your Terminal login.";

/** "Sign in" runs the CLI's own login. The token stays in the Keychain, owned by Claude Code. */
async function renderAccount(waiting = false, error = "") {
  const box = $("account");
  if (waiting) {
    const code = el("input");
    code.placeholder = "Browser showing a code? Paste it here";
    code.setAttribute("aria-label", "Sign-in code");
    const send = button("Send", "ghost", () =>
      invoke("claude_login_code", { code: code.value }).catch((e) => renderAccount(true, String(e))),
    );
    code.addEventListener("keydown", (e) => e.key === "Enter" && send.click());
    const msg = el("p", "muted", "Finish signing in in your browser. This updates automatically.");
    box.replaceChildren(msg, code, send, ...(error ? [el("p", "error", error)] : []));
    return;
  }
  const a = await invoke<Account>("claude_account");
  const who = a.logged_in ? a.email ?? "?" : "";
  if (who !== account || servers === null) {
    account = who;
    if (who) loadConnectors();
    else {
      servers = "Sign in to Claude on the previous step to see what your account connects to.";
      renderCaps();
    }
  }
  const where = el("p", "muted small", WHERE);
  const signIn = (label: string, cls: string) =>
    button(label, cls, () =>
      invoke("claude_login")
        .then(() => renderAccount(true))
        .catch((e) => renderAccount(false, String(e))),
    );
  const err = error ? [el("p", "error", error)] : [];
  if (!a.installed) {
    box.replaceChildren(
      el("p", "", "Claude Code isn't installed. In Terminal, run: curl -fsSL https://claude.ai/install.sh | bash"),
      button("Check again", "ghost", async () => {
        renderAis(await invoke<Detected>("onboarding_detect"));
        renderAccount();
      }),
      ...err,
    );
  } else if (!a.logged_in) {
    box.replaceChildren(
      el("p", "", "Sign in with your Claude account. Otto uses your plan, no API key needed."),
      signIn("Sign in to Claude", "ghost"),
      where,
      ...err,
    );
  } else {
    const signOut = button("Sign out", "quiet", () =>
      invoke("claude_logout")
        .then(() => renderAccount())
        .catch((e) => renderAccount(false, String(e))),
    );
    box.replaceChildren(el("p", "", `Signed in as ${a.email ?? "your Claude account"}.`), signIn("Use a different account", "ghost"), signOut, where, ...err);
  }
}

listen<string | null>("otto://claude-login", (e) => renderAccount(false, e.payload ?? ""));

back.addEventListener("click", () => step > 1 && show(step - 1));
next.addEventListener("click", async () => {
  if (step < LAST) return show(step + 1);
  next.disabled = true;
  try {
    await invoke("finish_onboarding", { readableDirs: [...chosen], disabled: [...disabled] });
  } catch (e) {
    $("error").textContent = String(e);
    $("error").hidden = false;
  } finally {
    next.disabled = false;
  }
});
$("mic-btn").addEventListener("click", () => invoke("request_mic"));

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
  info.disabled.forEach((d) => disabled.add(d));
  plugins = info.plugins;
  renderCaps();
  renderAis(await invoke<Detected>("onboarding_detect"));
  renderAccount();
})();
