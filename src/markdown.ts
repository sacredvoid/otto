// A tiny, safe Markdown renderer for task results.
// Builds DOM nodes with textContent only (no innerHTML), never loads remote
// content: images become a text placeholder and links show their full URL.

type Block =
  | { kind: "code"; text: string }
  | { kind: "heading"; text: string }
  | { kind: "list"; ordered: boolean; items: string[] }
  | { kind: "para"; text: string };

function parse(src: string): Block[] {
  const lines = src.replace(/\r\n?/g, "\n").split("\n");
  const blocks: Block[] = [];
  let para: string[] = [];
  const flush = () => {
    if (para.length) blocks.push({ kind: "para", text: para.join(" ") });
    para = [];
  };
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    if (/^\s*```/.test(line)) {
      flush();
      const body: string[] = [];
      while (++i < lines.length && !/^\s*```/.test(lines[i])) body.push(lines[i]);
      blocks.push({ kind: "code", text: body.join("\n") });
      continue;
    }
    const heading = line.match(/^#{1,6}\s+(.*)$/);
    if (heading) {
      flush();
      blocks.push({ kind: "heading", text: heading[1] });
      continue;
    }
    const item = line.match(/^\s*(?:([-*+])|(\d+)[.)])\s+(.*)$/);
    if (item) {
      flush();
      const ordered = !item[1];
      const last = blocks[blocks.length - 1];
      if (last?.kind === "list" && last.ordered === ordered) last.items.push(item[3]);
      else blocks.push({ kind: "list", ordered, items: [item[3]] });
      continue;
    }
    if (!line.trim()) flush();
    else para.push(line.trim());
  }
  flush();
  return blocks;
}

/** Inline: `code`, **bold**, [text](url), ![alt](url). */
function inline(parent: HTMLElement, text: string) {
  const re = /`([^`]+)`|\*\*([^*]+)\*\*|!\[([^\]]*)\]\(([^)\s]+)\)|\[([^\]]+)\]\(([^)\s]+)\)/g;
  let at = 0;
  for (const m of text.matchAll(re)) {
    if (m.index! > at) parent.append(text.slice(at, m.index));
    if (m[1] !== undefined) {
      const c = document.createElement("code");
      c.textContent = m[1];
      parent.append(c);
    } else if (m[2] !== undefined) {
      const b = document.createElement("strong");
      b.textContent = m[2];
      parent.append(b);
    } else if (m[4] !== undefined) {
      parent.append(`[image: ${m[3] || m[4]}]`);
    } else {
      const span = document.createElement("span");
      span.className = "url";
      span.textContent = ` (${m[6]})`;
      parent.append(m[5], span);
    }
    at = m.index! + m[0].length;
  }
  if (at < text.length) parent.append(text.slice(at));
}

/** Claude ends every answer with a one-line recap for the notification. When there's a
 * body above it, the recap only repeats it, so the expanded view drops it. */
export function withoutRecap(src: string): string {
  const paras = src.trim().split(/\n\s*\n/);
  const last = paras[paras.length - 1].trimStart();
  return paras.length > 1 && !last.startsWith("```") ? paras.slice(0, -1).join("\n\n") : src;
}

export function renderMarkdown(src: string): DocumentFragment {
  const frag = document.createDocumentFragment();
  for (const b of parse(src)) {
    if (b.kind === "code") {
      const pre = document.createElement("pre");
      pre.textContent = b.text;
      frag.append(pre);
    } else if (b.kind === "heading") {
      const h = document.createElement("p");
      h.className = "h";
      inline(h, b.text);
      frag.append(h);
    } else if (b.kind === "list") {
      const list = document.createElement(b.ordered ? "ol" : "ul");
      for (const it of b.items) {
        const li = document.createElement("li");
        inline(li, it);
        list.append(li);
      }
      frag.append(list);
    } else {
      const p = document.createElement("p");
      inline(p, b.text);
      frag.append(p);
    }
  }
  return frag;
}
