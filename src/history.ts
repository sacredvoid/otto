import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { renderMarkdown } from "./markdown";

type Status = "done" | "failed" | "incomplete";
type Entry = { folder: string; created_unix: number; transcript: string; status: Status; preview: string };
type Step = { label: string; tool: string };
type Approval = { ts: number; tool: string; decision: string; by: string; input: string };
type Detail = {
  transcript: string;
  created_unix: number;
  status: Status;
  result: string;
  steps: Step[];
  approvals: Approval[];
  files: string[];
};

const STATUS_LABEL: Record<Status, string> = { done: "Done", failed: "Failed", incomplete: "Didn't finish" };

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
const list = $<HTMLUListElement>("list");
const search = $<HTMLInputElement>("search");
let entries: Entry[] = [];
let shown: Entry[] = [];
let selected: string | null = null;

function el<K extends keyof HTMLElementTagNameMap>(tag: K, cls = "", text?: string) {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text !== undefined) e.textContent = text;
  return e;
}

function relative(unix: number): string {
  const secs = Date.now() / 1000 - unix;
  if (secs < 60) return "just now";
  if (secs < 3600) return `${Math.floor(secs / 60)} min ago`;
  if (secs < 86400) return `${Math.floor(secs / 3600)} h ago`;
  if (secs < 7 * 86400) return `${Math.floor(secs / 86400)} d ago`;
  return new Date(unix * 1000).toLocaleDateString();
}

const firstLine = (t: string) => t.trim().split("\n")[0];

function renderList() {
  const q = search.value.trim().toLowerCase();
  shown = q ? entries.filter((e) => e.transcript.toLowerCase().includes(q)) : entries;
  $("no-match").hidden = !(entries.length && !shown.length);
  list.replaceChildren(
    ...shown.map((e) => {
      const li = el("li", "item");
      li.id = `t-${e.folder}`;
      li.setAttribute("role", "option");
      li.setAttribute("aria-selected", String(e.folder === selected));
      li.dataset.folder = e.folder;
      const dot = el("span", `dot ${e.status}`);
      dot.title = STATUS_LABEL[e.status];
      dot.setAttribute("role", "img");
      dot.setAttribute("aria-label", STATUS_LABEL[e.status]);
      const text = el("div", "text");
      text.append(el("span", "title", firstLine(e.transcript)), el("span", "when", relative(e.created_unix)));
      li.append(dot, text);
      li.addEventListener("click", () => select(e.folder));
      return li;
    }),
  );
  if (selected) list.setAttribute("aria-activedescendant", `t-${selected}`);
}

async function select(folder: string) {
  selected = folder;
  for (const li of list.querySelectorAll<HTMLElement>("li")) {
    const on = li.dataset.folder === folder;
    li.setAttribute("aria-selected", String(on));
    if (on) li.scrollIntoView({ block: "nearest" });
  }
  list.setAttribute("aria-activedescendant", `t-${folder}`);
  let d: Detail;
  try {
    d = await invoke<Detail>("history_detail", { folder });
  } catch (e) {
    if (selected !== folder) return;
    $("task").hidden = false;
    $("transcript").textContent = "";
    $("meta").textContent = "";
    $("result").replaceChildren(el("p", "error", `Couldn't load this task: ${e}`));
    return;
  }
  if (selected !== folder) return; // a newer click won
  renderDetail(folder, d);
}

function renderDetail(folder: string, d: Detail) {
  $("empty").hidden = true;
  $("task").hidden = false;
  $("transcript").textContent = `“${d.transcript.trim()}”`;
  $("meta").textContent = `${new Date(d.created_unix * 1000).toLocaleString()} · ${STATUS_LABEL[d.status]}`;

  const result = $("result");
  result.dataset.status = d.status;
  if (d.status === "done") result.replaceChildren(d.result.trim() ? renderMarkdown(d.result) : el("p", "muted", "No summary."));
  else if (d.status === "failed") result.replaceChildren(el("p", "error", d.result));
  else result.replaceChildren(el("p", "muted", "This task stopped before it finished (cancelled, or Otto quit)."));

  $("steps-box").hidden = !d.steps.length;
  $("steps-box").querySelector("summary")!.textContent = `Steps (${d.steps.length})`;
  $("steps").replaceChildren(
    ...d.steps.map((s) => {
      const li = el("li", "", s.label);
      if (!s.label.includes(s.tool)) li.append(el("span", "tool", s.tool));
      return li;
    }),
  );

  $("approvals-box").hidden = !d.approvals.length;
  $("approvals-box").querySelector("summary")!.textContent = `Approvals (${d.approvals.length})`;
  $("approvals").replaceChildren(
    ...d.approvals.map((a) => {
      const li = el("li");
      const head = el("div", "ahead");
      const allowed = a.decision === "allow";
      head.append(
        el("span", `badge ${allowed ? "ok" : "no"}`, allowed ? "Allowed" : "Denied"),
        el("span", "tool", a.tool),
        el("span", "muted small", `${a.by === "user" ? "by you" : `by ${a.by}`} · ${new Date(a.ts * 1000).toLocaleTimeString()}`),
      );
      li.append(head);
      if (a.input) li.append(el("pre", "input", a.input));
      return li;
    }),
  );

  $("files").replaceChildren(
    ...d.files.map((name) => {
      const b = el("button", "chip", name);
      b.type = "button";
      b.setAttribute("aria-label", `Open ${name}`);
      b.addEventListener("click", () => invoke("open_history_file", { folder, name }));
      return b;
    }),
  );
  $("finder").onclick = () => invoke("show_history_folder", { folder });
}

async function refresh() {
  entries = await invoke<Entry[]>("list_history");
  $("empty").hidden = entries.length > 0;
  if (selected && !entries.some((e) => e.folder === selected)) selected = null;
  renderList();
  if (!selected && shown.length) select(shown[0].folder);
  else if (selected) select(selected); // its status may have changed
  else $("task").hidden = true;
}

function move(by: number) {
  if (!shown.length) return;
  const at = shown.findIndex((e) => e.folder === selected);
  const next = Math.min(shown.length - 1, Math.max(0, at < 0 ? 0 : at + by));
  select(shown[next].folder);
}

function onKey(e: KeyboardEvent) {
  if (e.key === "ArrowDown" || e.key === "ArrowUp") {
    e.preventDefault();
    move(e.key === "ArrowDown" ? 1 : -1);
  }
}

search.addEventListener("input", () => {
  renderList();
  if (shown.length && !shown.some((e) => e.folder === selected)) select(shown[0].folder);
});
search.addEventListener("keydown", onKey);
list.addEventListener("keydown", onKey);

getCurrentWindow().onFocusChanged(({ payload: focused }) => focused && refresh());
listen<{ status: string }>("otto://task", ({ payload }) => {
  if (payload.status === "done" || payload.status === "failed") refresh();
});
refresh();
