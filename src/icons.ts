/// <reference types="vite/client" />
// Turns a step's icon key (from tasks::tool_icon) into a tiny logo or glyph.
import "./icons.css";

// Bundled connector logos, keyed by file name ("gmail", "slack", ...).
const LOGOS: Record<string, string> = Object.fromEntries(
  Object.entries(import.meta.glob<string>("./logos/apps/*.png", { eager: true, query: "?url", import: "default" })).map(
    ([path, url]) => [path.split("/").pop()!.replace(".png", ""), url],
  ),
);
// Connector names that don't match their logo's file name.
const ALIASES: Record<string, string> = { jira: "atlassian", confluence: "atlassian", calendar: "googlecalendar", drive: "googledrive", gdrive: "googledrive" };

const glyph = (d: string) =>
  `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${d}</svg>`;
// Static markup only.
const GLYPHS: Record<string, [string, string]> = {
  file: ["Files", glyph('<path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z"/><path d="M14 3v5h5"/>')],
  search: ["Search", glyph('<circle cx="11" cy="11" r="6"/><path d="m20 20-4.5-4.5"/>')],
  web: ["Web", glyph('<circle cx="12" cy="12" r="9"/><path d="M3 12h18M12 3a14 14 0 0 1 0 18M12 3a14 14 0 0 0 0 18"/>')],
  terminal: ["Terminal", glyph('<rect x="3" y="4" width="18" height="16" rx="2"/><path d="m7 9 3 3-3 3M13 15h4"/>')],
  mac: ["Mac app", glyph('<rect x="3" y="4" width="18" height="16" rx="2"/><path d="M3 9h18"/><circle cx="6" cy="6.5" r=".6" fill="currentColor"/><circle cx="8.5" cy="6.5" r=".6" fill="currentColor"/>')],
  memory: ["Memory", glyph('<path d="M6 3h12v18l-6-4-6 4z"/>')],
  plan: ["Plan", glyph('<path d="M9 6h11M9 12h11M9 18h11"/><path d="m3.5 6 1 1 2-2M3.5 12l1 1 2-2M3.5 18l1 1 2-2"/>')],
  tool: ["Tool", glyph('<path d="M14.7 6.3a4 4 0 0 0-5.4 5.4L3 18v3h3l6.3-6.3a4 4 0 0 0 5.4-5.4l-2.5 2.5-2.5-2.5z"/>')],
};

function logoFor(app: string): string | undefined {
  if (LOGOS[app]) return LOGOS[app];
  if (ALIASES[app]) return LOGOS[ALIASES[app]];
  // "googlecalendarv2", "githubenterprise": the longest logo name it contains.
  const hit = Object.keys(LOGOS).filter((n) => app.includes(n)).sort((a, b) => b.length - a.length)[0];
  return hit && LOGOS[hit];
}

/** A 16px icon that pops in when inserted. */
export function toolIcon(key: string): HTMLElement {
  const box = document.createElement("span");
  box.className = "tool-icon";
  box.dataset.key = key;
  if (key.startsWith("app:")) {
    const app = key.slice(4);
    const src = logoFor(app);
    box.title = app;
    if (src) {
      const img = document.createElement("img");
      img.src = src;
      img.alt = "";
      box.append(img);
    } else {
      box.classList.add("letter");
      box.textContent = (app[0] ?? "?").toUpperCase();
    }
  } else {
    const [label, svg] = GLYPHS[key] ?? GLYPHS.tool;
    box.title = label;
    box.classList.add("glyph");
    box.innerHTML = svg; // static markup above
  }
  return box;
}

/** Sets "icon + text" on a line, swapping (and re-animating) the icon only when it changes. */
export function setLine(line: HTMLElement, icon: string | null | undefined, text: string) {
  const key = icon ?? "";
  if (line.dataset.icon !== key || !line.lastElementChild) {
    line.dataset.icon = key;
    const label = document.createElement("span");
    line.replaceChildren(...(key ? [toolIcon(key)] : []), label);
  }
  line.lastElementChild!.textContent = text;
}
