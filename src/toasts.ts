import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

type Status = "queued" | "running" | "done" | "failed" | "needs_you";
type TaskEvent = { id: number; title: string; status: Exclude<Status, "needs_you">; step: string | null; summary: string | null };
type Approval = { task_id: number; approval_id: number; title: string; summary: string; detail: string };
type Details = { summary: string; files: string[] };

const DONE_DISMISS_MS = 8000;
const stack = document.getElementById("stack")!;
const cards = new Map<number, HTMLElement>();
const timers = new Map<number, number>();
const titles = new Map<number, string>();

const BODY = `M50 12 C74 10 88 26 86 48 C84 68 68 78 48 78 C26 78 12 64 14 44 C16 24 30 13 50 12 Z`;
const TIE = `<path d="M38 76 L48 81 L38 86 Z M62 76 L52 81 L62 86 Z" fill="#8CC9B2" stroke="#8CC9B2" stroke-width="6" stroke-linejoin="round"/>`;
const svg = (inner: string) => `<svg viewBox="-10 -20 120 120" aria-hidden="true">${inner}</svg>`;

// Static markup only; user and model text never goes through innerHTML.
const MASCOT: Record<Status, string> = {
  queued: svg(`<g class="m breathe"><path d="${BODY}" fill="#CFE6F3"/><path d="M34 47 H44 M56 47 H66" stroke="#2F6F62" stroke-width="5" stroke-linecap="round"/>${TIE}</g>`),
  running: svg(`<g class="m bob"><path d="${BODY}" fill="#CFE6F3"/><circle class="blink" cx="76" cy="44" r="5.5" fill="#2F6F62"/><path d="M58 73 L66 78 L58 83 Z" fill="#8CC9B2" stroke="#8CC9B2" stroke-width="6" stroke-linejoin="round"/></g>`),
  done: svg(`<g class="m hop"><path d="${BODY}" fill="#BFE5D2"/><path d="M33 47 Q39 39 45 47 M55 47 Q61 39 67 47" fill="none" stroke="#2F6F62" stroke-width="5" stroke-linecap="round"/>${TIE}</g>`),
  failed: svg(`<g class="m droop"><path d="${BODY}" fill="#E4ECE8"/><path d="M35 41 L43 49 M43 41 L35 49 M57 41 L65 49 M65 41 L57 49" stroke="#5B6B65" stroke-width="5" stroke-linecap="round"/><path d="M38 76 L48 81 L38 86 Z M62 76 L52 81 L62 86 Z" fill="#E3A29C" stroke="#E3A29C" stroke-width="6" stroke-linejoin="round"/></g>`),
  needs_you: svg(`<g class="qm"><path d="M44 -12 Q44 -19 50.5 -19 Q57 -19 57 -13 Q57 -9 51 -6 V-2" fill="none" stroke="#2B5677" stroke-width="5" stroke-linecap="round" stroke-linejoin="round"/><circle cx="51" cy="5" r="3" fill="#2B5677"/></g><g class="m nudge"><path d="${BODY}" fill="#CFE6F3"/><circle cx="39" cy="44" r="6.5" fill="#1F3E5A"/><circle cx="61" cy="44" r="6.5" fill="#1F3E5A"/><path d="M38 76 L48 81 L38 86 Z M62 76 L52 81 L62 86 Z" fill="#7FAED6" stroke="#7FAED6" stroke-width="6" stroke-linejoin="round"/></g>`),
};

function button(label: string, kind: "link" | "primary" | "solid", onClick: () => void) {
  const b = document.createElement("button");
  b.type = "button";
  b.textContent = label;
  b.className = kind === "link" ? "link" : kind === "primary" ? "link primary" : "solid";
  b.addEventListener("click", onClick);
  return b;
}

function el<K extends keyof HTMLElementTagNameMap>(tag: K, cls: string, text?: string) {
  const e = document.createElement(tag);
  e.className = cls;
  if (text !== undefined) e.textContent = text;
  return e;
}

function getCard(id: number): HTMLElement {
  let card = cards.get(id);
  if (card) return card;
  card = el("article", "card");
  card.innerHTML = `<div class="mascot"></div><div class="body"><p class="title"></p><p class="sub"></p><div class="extra"></div><div class="actions"></div></div>`;
  const close = button("×", "link", () => remove(id));
  close.className = "close";
  close.setAttribute("aria-label", "Dismiss");
  card.append(close);
  // Hovering pauses the auto-dismiss of a finished task.
  card.addEventListener("mouseenter", () => clearTimeout(timers.get(id)));
  card.addEventListener("mouseleave", () => card!.dataset.status === "done" && !card!.dataset.expanded && armDismiss(id));
  stack.prepend(card);
  cards.set(id, card);
  return card;
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

function setStatus(card: HTMLElement, status: Status): boolean {
  if (card.dataset.status === status) return false;
  card.dataset.status = status;
  card.querySelector(".mascot")!.innerHTML = MASCOT[status];
  card.querySelector(".extra")!.replaceChildren();
  card.querySelector(".actions")!.replaceChildren();
  delete card.dataset.expanded;
  return true;
}

function render(t: TaskEvent) {
  titles.set(t.id, t.title);
  const card = getCard(t.id);
  card.querySelector(".title")!.textContent = t.title;
  const sub = t.status === "queued" ? "Waiting for a free slot" : t.status === "running" ? (t.step ?? "Working") : (t.summary ?? "");
  card.querySelector(".sub")!.textContent = sub;
  if (!setStatus(card, t.status)) return; // keep animations smooth between step updates

  const actions = card.querySelector(".actions")!;
  if (t.status === "done") {
    actions.append(
      button("Open", "primary", () => toggleDetails(t.id)),
      button("Show in Finder", "link", () => invoke("show_task_folder", { id: t.id })),
    );
    armDismiss(t.id);
  } else if (t.status === "failed") {
    actions.append(button("Show in Finder", "link", () => invoke("show_task_folder", { id: t.id })));
  }
}

/// "Open" grows the card into the full result plus the files the task made.
async function toggleDetails(id: number) {
  const card = getCard(id);
  const extra = card.querySelector(".extra")!;
  if (card.dataset.expanded) {
    delete card.dataset.expanded;
    extra.replaceChildren();
    return;
  }
  clearTimeout(timers.get(id));
  card.dataset.expanded = "1";
  const d = await invoke<Details>("task_details", { id });
  const result = el("div", "result", d.summary || "No summary.");
  result.tabIndex = 0;
  extra.replaceChildren(result);
  if (d.files.length) {
    const files = el("ul", "files");
    for (const name of d.files) {
      const li = document.createElement("li");
      li.append(button(name, "link", () => invoke("open_task_file", { id, name })));
      files.append(li);
    }
    extra.append(el("p", "files-label", "Files"), files);
  }
}

function renderApproval(a: Approval) {
  const card = getCard(a.task_id);
  card.querySelector(".title")!.textContent = titles.get(a.task_id) ?? a.title;
  setStatus(card, "needs_you");
  clearTimeout(timers.get(a.task_id));
  card.querySelector(".sub")!.textContent = a.summary;
  card.querySelector(".extra")!.replaceChildren(el("pre", "detail", a.detail));
  const answer = (allow: boolean) => {
    invoke("answer_approval", { approvalId: a.approval_id, allow });
    setStatus(card, "running");
    card.querySelector(".sub")!.textContent = allow ? "Allowed, continuing" : "Skipped, continuing";
  };
  card.querySelector(".actions")!.replaceChildren(
    button("Allow", "solid", () => answer(true)),
    button("Don't allow", "link", () => answer(false)),
  );
}

// The window is sized to its content; height 0 hides it.
new ResizeObserver(() => {
  invoke("set_toasts_height", { height: cards.size ? document.documentElement.scrollHeight : 0 });
}).observe(stack);

listen<TaskEvent>("otto://task", ({ payload }) => render(payload));
listen<Approval>("otto://approval", ({ payload }) => renderApproval(payload));
