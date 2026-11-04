import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { renderMarkdown, withoutRecap } from "./markdown";
import { setLine, toolIcon } from "./icons";
import { trackGlass } from "./glass";

type Status = "queued" | "running" | "done" | "failed" | "needs_you";
type TaskEvent = { id: number; title: string; status: Exclude<Status, "needs_you">; step: string | null; summary: string | null; icon: string | null };
type Approval = { task_id: number; approval_id: number; title: string; summary: string; detail: string; icon: string };
type Details = { summary: string; files: string[] };

const DONE_DISMISS_MS = 8000;
const CANCELLED_DISMISS_MS = 2000;
const CANCELLED = "Cancelled."; // the summary tasks.rs emits for a cancelled task
const TUCK_MS = 2500; // a running card shows this long, then condenses into the pill
const TUCK_ANIM_MS = 260; // matches .tucking in toasts.css
const stack = document.getElementById("stack")!;
const pill = document.getElementById("pill")!;
const wrap = document.getElementById("wrap")!;
const tuckTimers = new Map<number, number>();
const latestIcon = new Map<number, string | null>();
const cards = new Map<number, HTMLElement>();
const timers = new Map<number, number>();
const titles = new Map<number, string>();
// A task can fire several asks at once (parallel tool calls); show them one at a time.
const approvals = new Map<number, Approval[]>();

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

function cancelButton(id: number) {
  return button("Cancel", "link", () => invoke("cancel_task", { id }));
}

function getCard(id: number): HTMLElement {
  let card = cards.get(id);
  if (card) return card;
  card = el("article", "card glass");
  card.innerHTML = `<div class="avatar"><div class="mascot"></div><span class="app-badge"></span></div><div class="body"><p class="title"></p><p class="sub"></p><div class="extra"></div><div class="actions"></div></div>`;
  // Closing a task that is still going tucks it into the pill instead of losing it.
  const close = button("×", "link", () => (isWorking(card!) ? tuck(id, true) : remove(id)));
  close.className = "close";
  close.setAttribute("aria-label", "Dismiss");
  card.append(close);
  // Hovering pauses the auto-dismiss of a finished task.
  card.addEventListener("mouseenter", () => {
    clearTimeout(timers.get(id));
    clearTimeout(tuckTimers.get(id));
  });
  card.addEventListener("mouseleave", () => {
    if (card!.dataset.status === "done" && !card!.dataset.expanded) armDismiss(id);
    if (card!.dataset.cancelled) armDismiss(id, CANCELLED_DISMISS_MS);
    if (isWorking(card!) && !card!.classList.contains("tucked")) armTuck(id);
  });
  stack.prepend(card);
  cards.set(id, card);
  return card;
}

/** Auto-dismiss, unless the card was opened (kept until ×) or is under the cursor right now. */
function armDismiss(id: number, ms = DONE_DISMISS_MS) {
  clearTimeout(timers.get(id));
  timers.set(id, window.setTimeout(() => {
    const card = cards.get(id);
    if (!card || card.dataset.kept) return;
    // Checked at fire time: hover events don't always reach a panel that never takes focus.
    if (card.matches(":hover")) return armDismiss(id, 1500);
    remove(id);
  }, ms));
}

function remove(id: number) {
  clearTimeout(timers.get(id));
  timers.delete(id);
  untuck(id);
  approvals.delete(id);
  latestIcon.delete(id);
  cards.get(id)?.remove();
  cards.delete(id);
  updatePill();
  report();
}

const isWorking = (card: HTMLElement) => card.dataset.status === "running" || card.dataset.status === "queued";

function armTuck(id: number) {
  clearTimeout(tuckTimers.get(id));
  tuckTimers.set(id, window.setTimeout(() => tuck(id), TUCK_MS));
}

/** Condense a working card into the pill. */
function tuck(id: number, now = false) {
  const card = cards.get(id);
  if (!card || !isWorking(card) || approvals.get(id)?.length) return;
  if (!now && card.matches(":hover")) return armTuck(id); // never yank a card from under the cursor
  clearTimeout(tuckTimers.get(id));
  card.classList.add("tucking");
  tuckTimers.set(id, window.setTimeout(() => {
    card.classList.replace("tucking", "tucked");
    updatePill();
    report();
  }, TUCK_ANIM_MS));
}

/** Pour a card back out (it needs you, or it finished). */
function untuck(id: number) {
  clearTimeout(tuckTimers.get(id));
  tuckTimers.delete(id);
  cards.get(id)?.classList.remove("tucking", "tucked");
  updatePill();
}

const RING = `<svg viewBox="0 0 16 16" aria-hidden="true"><circle cx="8" cy="8" r="6" fill="none" stroke="rgba(47,111,98,0.18)" stroke-width="2"/><circle class="arc" cx="8" cy="8" r="6" fill="none" stroke="#2F6F62" stroke-width="2" stroke-linecap="round" stroke-dasharray="11 27"/></svg>`;

// The island's parts are built once and updated in place, so step updates
// never restart its animations.
const pillMascot = el("span", "pill-mascot");
pillMascot.innerHTML = MASCOT.running; // static markup
const pillTitle = el("span", "pill-title");
const pillIcon = el("span", "pill-icon");
const pillRing = el("span", "ring");
pillRing.innerHTML = RING; // static markup
const pillMore = el("span", "more");
const pillEnd = el("span", "pill-end");
pillEnd.append(pillIcon, pillRing, pillMore);
pill.append(pillMascot, pillTitle, pillEnd);

/** The island: mascot, the newest tucked task's title, its tool's logo, a working ring, +N for the rest. */
function updatePill() {
  const tucked = [...cards].filter(([, c]) => c.classList.contains("tucked")).map(([id]) => id);
  if (!tucked.length) {
    pill.hidden = true;
    return;
  }
  pill.hidden = false; // un-hiding replays the grow-in
  const newest = tucked[tucked.length - 1];
  const icon = latestIcon.get(newest) ?? "";
  if (pillIcon.dataset.key !== icon) {
    pillIcon.dataset.key = icon;
    pillIcon.replaceChildren(...(icon ? [toolIcon(icon)] : []));
  }
  pillTitle.textContent = titles.get(newest) ?? "Working";
  pillMore.textContent = tucked.length > 1 ? `+${tucked.length - 1}` : "";
  pill.setAttribute("aria-label", `${tucked.length} running`);
}

/** Connector logos go on the mascot badge; built-in glyphs stay inline. */
const glyphOnly = (icon: string | null) => (icon?.startsWith("app:") ? null : icon);

/** A connector's logo pops onto the mascot while a step uses it. */
function setBadge(card: HTMLElement, icon: string | null) {
  const badge = card.querySelector<HTMLElement>(".app-badge")!;
  const key = icon?.startsWith("app:") ? icon : "";
  if (badge.dataset.key === key) return;
  badge.dataset.key = key;
  badge.replaceChildren(...(key ? [toolIcon(key)] : []));
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
  const isNew = !cards.has(t.id);
  titles.set(t.id, t.title);
  if (t.status === "done" || t.status === "failed") approvals.delete(t.id);
  else if (approvals.get(t.id)?.length) return; // an open ask owns the card until answered
  const card = getCard(t.id);
  card.querySelector(".title")!.textContent = t.title;
  const sub = t.status === "queued" ? "Waiting for a free slot" : t.status === "running" ? (t.step ?? "Working") : (t.summary ?? "");
  setLine(card.querySelector<HTMLElement>(".sub")!, t.status === "running" ? glyphOnly(t.icon) : null, sub);
  setBadge(card, t.status === "running" ? t.icon : null);
  if (t.status === "running" && t.icon) latestIcon.set(t.id, t.icon);
  if (!setStatus(card, t.status)) {
    if (card.classList.contains("tucked")) updatePill(); // the pill shows the newest tool
    return; // keep animations smooth between step updates
  }

  const actions = card.querySelector(".actions")!;
  if (t.status === "done") {
    actions.append(
      button("Open", "primary", () => toggleDetails(t.id)),
      button("Show in Finder", "link", () => invoke("show_task_folder", { id: t.id })),
    );
    untuck(t.id);
    armDismiss(t.id);
  } else if (t.status === "failed") {
    untuck(t.id);
    if (t.summary === CANCELLED) {
      card.dataset.cancelled = "1";
      return armDismiss(t.id, CANCELLED_DISMISS_MS);
    }
    actions.append(button("Show in Finder", "link", () => invoke("show_task_folder", { id: t.id })));
  } else {
    actions.append(cancelButton(t.id));
    // A new task goes straight into the island; hover it for the card.
    if (isNew) {
      card.classList.add("tucked");
      updatePill();
    } else if (!card.classList.contains("tucked")) armTuck(t.id);
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
  card.dataset.kept = "1"; // opened once: stays until you close it
  const d = await invoke<Details>("task_details", { id });
  const result = el("div", "result");
  result.append(d.summary ? renderMarkdown(withoutRecap(d.summary)) : "No summary.");
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

function queueApproval(a: Approval) {
  const q = approvals.get(a.task_id) ?? [];
  q.push(a);
  approvals.set(a.task_id, q);
  if (q.length === 1) {
    untuck(a.task_id);
    renderApproval(a);
  }
}

function renderApproval(a: Approval) {
  const card = getCard(a.task_id);
  card.querySelector(".title")!.textContent = titles.get(a.task_id) ?? a.title;
  setStatus(card, "needs_you");
  clearTimeout(timers.get(a.task_id));
  setLine(card.querySelector<HTMLElement>(".sub")!, glyphOnly(a.icon), a.summary);
  setBadge(card, a.icon);
  card.querySelector(".extra")!.replaceChildren(el("pre", "detail", a.detail));
  const answer = (allow: boolean) => {
    invoke("answer_approval", { approvalId: a.approval_id, allow });
    const q = approvals.get(a.task_id) ?? [];
    q.shift();
    if (q.length) return renderApproval(q[0]);
    approvals.delete(a.task_id);
    setStatus(card, "running");
    card.querySelector(".actions")!.append(cancelButton(a.task_id));
    armTuck(a.task_id);
    card.querySelector(".sub")!.textContent = allow ? "Allowed, continuing" : "Skipped, continuing";
  };
  card.querySelector(".actions")!.replaceChildren(
    button("Allow", "solid", () => answer(true)),
    button("Don't allow", "link", () => answer(false)),
    cancelButton(a.task_id),
  );
}

// The window is sized to its content; height 0 hides it. Measured directly after
// every change too: a hidden window does no layout, so the observer alone never
// fires for the first task.
let reported = -1;
function report() {
  const height = cards.size ? Math.ceil(wrap.getBoundingClientRect().height) : 0;
  if (height === reported) return;
  reported = height;
  invoke("set_toasts_height", { height });
}
new ResizeObserver(report).observe(wrap);

listen<TaskEvent>("otto://task", ({ payload }) => {
  render(payload);
  report();
});
listen<Approval>("otto://approval", ({ payload }) => {
  queueApproval(payload);
  report();
});

trackGlass();
