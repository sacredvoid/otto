import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { renderMarkdown, withoutRecap } from "./markdown";

type Status = "done" | "failed" | "incomplete" | "running";
type Entry = { folder: string; created_unix: number; transcript: string; status: Status; preview: string };
type Approval = { decision: string; by: string } | null;
type Item =
  | { kind: "said"; text: string; unix: number; follow_up: boolean }
  | { kind: "note"; text: string; ts: string }
  | { kind: "tool"; tool: string; label: string; detail: string; approval: Approval; output: string | null; error: boolean; ts: string }
  | { kind: "result"; ok: boolean; text: string; duration_ms: number | null };
type Detail = {
  transcript: string;
  created_unix: number;
  status: Status;
  result: string;
  trace: Item[];
  files: string[];
};

const STATUS_LABEL: Record<Status, string> = { done: "Done", failed: "Failed", incomplete: "Stopped", running: "Running" };
// One glyph per status so it reads without relying on color alone.
const STATUS_ICON: Record<Status, string> = { done: "\u2713", failed: "\u2715", incomplete: "\u25A0", running: "" };

function statusBadge(s: Status, big = false) {
  const b = el("span", `status ${s}${big ? " big" : ""}`);
  b.append(el("span", "icon", STATUS_ICON[s]));
  if (big) b.append(STATUS_LABEL[s]);
  else b.setAttribute("aria-label", STATUS_LABEL[s]);
  b.title = STATUS_LABEL[s];
  return b;
}

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
      const dot = statusBadge(e.status);
      dot.setAttribute("role", "img");
      const text = el("div", "text");
      const when = el("span", `when ${e.status}`, e.status === "done" ? relative(e.created_unix) : `${STATUS_LABEL[e.status]} \u00b7 ${relative(e.created_unix)}`);
      text.append(el("span", "title", firstLine(e.transcript)), when);
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
    $("meta").textContent = "";
    $("trace").replaceChildren(el("p", "error", `Couldn't load this task: ${e}`));
    return;
  }
  if (selected !== folder) return; // a newer click won
  renderDetail(folder, d);
}

function approvalBadge(a: Approval) {
  if (!a) return null;
  const [cls, text] =
    a.decision === "allow" ? (a.by === "user" ? ["ok", "You allowed"] : ["auto", "Auto-allowed"])
    : a.decision === "block" ? ["no", "Blocked"]
    : a.by === "timeout" ? ["no", "No answer"]
    : ["no", "You declined"];
  return el("span", `badge ${cls}`, text);
}

function seconds(ms: number) {
  return ms < 60_000 ? `${Math.round(ms / 1000)}s` : `${Math.floor(ms / 60_000)}m ${Math.round((ms % 60_000) / 1000)}s`;
}

function node(kind: string, ...children: (Node | null)[]) {
  const li = el("li", `step ${kind}`);
  li.append(el("span", "marker"), ...children.filter((c): c is Node => c !== null));
  return li;
}

function renderItem(it: Item, start: number): HTMLLIElement {
  if (it.kind === "said") {
    const body = el("div", "body");
    body.append(el("div", "kicker", it.follow_up ? "You followed up" : "You said"), el("blockquote", "", it.text.trim()));
    return node("said", body);
  }
  if (it.kind === "note") {
    return node("note", el("p", "body", it.text));
  }
  if (it.kind === "tool") {
    const body = el("div", "body");
    const head = el("div", "head");
    head.append(el("span", "label", it.label), approvalBadge(it.approval) ?? el("span"));
    const at = Date.parse(it.ts);
    if (start && at) head.append(el("span", "at", `+${seconds(at - start)}`));
    body.append(head);
    if (it.detail) body.append(el("code", "detail", it.detail));
    if (it.output) {
      const out = el("details", "output");
      out.append(el("summary", "", it.error ? "Error" : "Output"), el("pre", "", it.output));
      body.append(out);
    }
    return node(`tool${it.error ? " failed" : ""}`, body);
  }
  const body = el("div", "body");
  const head = el("div", "kicker", it.ok ? "Result" : "Failed");
  if (it.duration_ms) head.append(el("span", "at", ` · took ${seconds(it.duration_ms)}`));
  const text = el("div", "result");
  text.tabIndex = 0;
  text.append(it.ok ? (it.text ? renderMarkdown(withoutRecap(it.text)) : el("p", "muted", "No summary.")) : el("p", "error", it.text));
  body.append(head, text);
  return node(`outcome${it.ok ? "" : " failed"}`, body);
}

function renderDetail(folder: string, d: Detail) {
  $("empty").hidden = true;
  $("task").hidden = false;
  $("meta").replaceChildren(statusBadge(d.status, true), el("span", "", new Date(d.created_unix * 1000).toLocaleString()));

  const firstTs = d.trace.find((i) => i.kind === "tool" || i.kind === "note") as { ts: string } | undefined;
  const start = firstTs ? Date.parse(firstTs.ts) : 0;
  const items = d.trace.map((it) => renderItem(it, start));
  if (d.status === "incomplete") items.push(node("stopped", el("p", "body muted", "Stopped here (cancelled, or Otto quit).")));
  if (d.status === "running") items.push(node("live", el("p", "body muted", "Still working\u2026")));
  $("trace").replaceChildren(...items);

  $("files-box").hidden = !d.files.length;
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
  if (payload.status !== "queued") refresh(); // running, done or failed changes a badge
});
refresh();
