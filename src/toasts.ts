import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

type Status = "queued" | "running" | "done" | "failed";
type TaskEvent = { id: number; title: string; status: Status; step: string | null; summary: string | null };

const DONE_DISMISS_MS = 8000;
const stack = document.getElementById("stack")!;
const cards = new Map<number, HTMLElement>();
const timers = new Map<number, number>();

const BODY = `M50 12 C74 10 88 26 86 48 C84 68 68 78 48 78 C26 78 12 64 14 44 C16 24 30 13 50 12 Z`;
const TIE = `<path d="M38 76 L48 81 L38 86 Z M62 76 L52 81 L62 86 Z" fill="#8CC9B2" stroke="#8CC9B2" stroke-width="6" stroke-linejoin="round"/>`;
const svg = (inner: string) => `<svg viewBox="-10 -20 120 120" aria-hidden="true">${inner}</svg>`;

// Static markup only; user text never goes through innerHTML.
const MASCOT: Record<Status, string> = {
  queued: svg(`<g class="m breathe"><path d="${BODY}" fill="#CFE6F3"/><path d="M34 47 H44 M56 47 H66" stroke="#2F6F62" stroke-width="5" stroke-linecap="round"/>${TIE}</g>`),
  running: svg(`<g class="m bob"><path d="${BODY}" fill="#CFE6F3"/><circle class="blink" cx="76" cy="44" r="5.5" fill="#2F6F62"/><path d="M58 73 L66 78 L58 83 Z" fill="#8CC9B2" stroke="#8CC9B2" stroke-width="6" stroke-linejoin="round"/></g>`),
  done: svg(`<g class="m hop"><path d="${BODY}" fill="#BFE5D2"/><path d="M33 47 Q39 39 45 47 M55 47 Q61 39 67 47" fill="none" stroke="#2F6F62" stroke-width="5" stroke-linecap="round"/>${TIE}</g>`),
  failed: svg(`<g class="m droop"><path d="${BODY}" fill="#E4ECE8"/><path d="M35 41 L43 49 M43 41 L35 49 M57 41 L65 49 M65 41 L57 49" stroke="#5B6B65" stroke-width="5" stroke-linecap="round"/><path d="M38 76 L48 81 L38 86 Z M62 76 L52 81 L62 86 Z" fill="#E3A29C" stroke="#E3A29C" stroke-width="6" stroke-linejoin="round"/></g>`),
};

function button(label: string, primary: boolean, onClick: () => void) {
  const b = document.createElement("button");
  b.type = "button";
  b.textContent = label;
  b.className = primary ? "link primary" : "link";
  b.addEventListener("click", onClick);
  return b;
}

function getCard(id: number): HTMLElement {
  let el = cards.get(id);
  if (el) return el;
  el = document.createElement("article");
  el.className = "card";
  el.innerHTML = `<div class="mascot"></div><div class="body"><p class="title"></p><p class="sub"></p><div class="actions"></div></div>`;
  const close = button("×", false, () => remove(id));
  close.className = "close";
  close.setAttribute("aria-label", "Dismiss");
  el.append(close);
  // Hovering pauses the auto-dismiss of a finished task.
  el.addEventListener("mouseenter", () => clearTimeout(timers.get(id)));
  el.addEventListener("mouseleave", () => el!.dataset.status === "done" && armDismiss(id));
  stack.prepend(el);
  cards.set(id, el);
  return el;
}

function armDismiss(id: number) {
  clearTimeout(timers.get(id));
  timers.set(id, window.setTimeout(() => remove(id), DONE_DISMISS_MS));
}

function remove(id: number) {
  clearTimeout(timers.get(id));
  timers.delete(id);
  cards.get(id)?.remove();
  cards.delete(id);
}

function render(t: TaskEvent) {
  const el = getCard(t.id);
  const changed = el.dataset.status !== t.status;
  el.dataset.status = t.status;
  el.querySelector(".title")!.textContent = t.title;
  const sub =
    t.status === "queued" ? "Waiting for a free slot"
    : t.status === "running" ? (t.step ?? "Working")
    : (t.summary ?? "");
  el.querySelector(".sub")!.textContent = sub;
  if (!changed) return; // keep the running animation smooth between step updates

  el.querySelector(".mascot")!.innerHTML = MASCOT[t.status];
  const actions = el.querySelector(".actions")!;
  actions.replaceChildren();
  if (t.status === "done") {
    actions.append(
      button("Open", true, () => invoke("open_task", { id: t.id, reveal: false })),
      button("Show in Finder", false, () => invoke("open_task", { id: t.id, reveal: true })),
    );
    armDismiss(t.id);
  } else if (t.status === "failed") {
    actions.append(button("Show log", false, () => invoke("open_task", { id: t.id, reveal: true })));
  }
}

// The window is sized to its content; height 0 hides it.
new ResizeObserver(() => {
  invoke("set_toasts_height", { height: cards.size ? document.documentElement.scrollHeight : 0 });
}).observe(stack);

listen<TaskEvent>("otto://task", ({ payload }) => render(payload));
