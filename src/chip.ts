import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

type TaskEvent = { id: number; title: string; status: "queued" | "running" | "done" | "failed"; step: string | null; summary: string | null };
type Approval = { task_id: number; summary: string };

const root = document.getElementById("root")!;
const tasks = new Map<number, TaskEvent>(); // queued/running only
const asks = new Map<number, string>(); // task id -> what it wants to do

const BODY = `M50 12 C74 10 88 26 86 48 C84 68 68 78 48 78 C26 78 12 64 14 44 C16 24 30 13 50 12 Z`;
const svg = (inner: string) => `<svg viewBox="-10 -20 120 120" aria-hidden="true">${inner}</svg>`;
// Static markup only; task text never goes through innerHTML.
const WORKING = svg(`<g class="m bob"><path d="${BODY}" fill="#CFE6F3"/><circle class="blink" cx="76" cy="44" r="5.5" fill="#2F6F62"/><path d="M58 73 L66 78 L58 83 Z" fill="#8CC9B2" stroke="#8CC9B2" stroke-width="6" stroke-linejoin="round"/></g>`);
const NEEDS_YOU = svg(`<g class="qm"><path d="M44 -12 Q44 -19 50.5 -19 Q57 -19 57 -13 Q57 -9 51 -6 V-2" fill="none" stroke="#2B5677" stroke-width="5" stroke-linecap="round" stroke-linejoin="round"/><circle cx="51" cy="5" r="3" fill="#2B5677"/></g><g class="m nudge"><path d="${BODY}" fill="#CFE6F3"/><circle cx="39" cy="44" r="6.5" fill="#1F3E5A"/><circle cx="61" cy="44" r="6.5" fill="#1F3E5A"/><path d="M38 76 L48 81 L38 86 Z M62 76 L52 81 L62 86 Z" fill="#7FAED6" stroke="#7FAED6" stroke-width="6" stroke-linejoin="round"/></g>`);

function el<K extends keyof HTMLElementTagNameMap>(tag: K, cls: string, text?: string) {
  const e = document.createElement(tag);
  e.className = cls;
  if (text !== undefined) e.textContent = text;
  return e;
}

function row(t: TaskEvent) {
  const needs = asks.has(t.id);
  const r = el("div", needs ? "row needs-you" : "row");
  const top = el("div", "top");
  const status = needs ? "Waiting for you" : t.status === "queued" ? "Waiting for a free slot" : (t.step ?? "Working");
  top.append(el("span", "title", t.title), el("span", "sub", status));
  r.append(top);
  if (needs) r.append(el("span", "sub", asks.get(t.id)));
  else if (t.status === "running") r.append(el("div", "bar"));
  return r;
}

function render() {
  const all = [...tasks.values()];
  const needing = all.filter((t) => asks.has(t.id)).length;
  const running = all.length - needing;
  root.replaceChildren();
  if (!all.length) return;

  const chip = el("div", "chip");
  const mascot = el("span", "");
  mascot.innerHTML = needing ? NEEDS_YOU : WORKING; // static markup above
  chip.append(mascot);
  if (running) chip.append(el("span", "", `${running} running`));
  if (running && needing) chip.append(el("span", "sep"));
  if (needing) chip.append(el("span", "needs", `${needing} needs you`));

  // Needs-you rows first.
  const panel = el("div", "panel");
  panel.append(...all.sort((a, b) => Number(asks.has(b.id)) - Number(asks.has(a.id))).map(row));
  root.append(chip, panel);
}

// The window follows #root, which shows the chip or (on hover) the panel; 0 hides it.
new ResizeObserver(() => {
  const r = root.getBoundingClientRect();
  invoke("set_chip_size", { width: tasks.size ? Math.ceil(r.width) : 0, height: tasks.size ? Math.ceil(r.height) : 0 });
}).observe(root);

listen<TaskEvent>("otto://task", ({ payload: t }) => {
  asks.delete(t.id); // the next task event means the ask was answered or moved on
  if (t.status === "queued" || t.status === "running") tasks.set(t.id, t);
  else tasks.delete(t.id);
  render();
});
listen<Approval>("otto://approval", ({ payload: a }) => {
  if (!tasks.has(a.task_id)) return;
  asks.set(a.task_id, a.summary);
  render();
});
