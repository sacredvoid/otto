import { invoke } from "@tauri-apps/api/core";
import { isWindows } from "./platform";

const px = (n: number) => Math.round(n * 2) / 2;

/**
 * Native glass sits behind this transparent webview (overlay.rs). Measure each
 * visible `.glass` element and send the list when it changed, so the glass
 * follows drops, tucks and fades.
 */
export function trackGlass() {
  if (isWindows) return; // no native glass there; the CSS draws a solid card instead
  let last = "";
  const measure = () => {
    const rects = [...document.querySelectorAll<HTMLElement>(".glass")].flatMap((e) => {
      const b = e.getBoundingClientRect();
      if (b.width < 1 || b.height < 1) return [];
      const cs = getComputedStyle(e);
      const r = Math.min(parseFloat(cs.borderBottomLeftRadius) || 0, b.height / 2, b.width / 2);
      return [{ x: px(b.left), y: px(b.top), w: px(b.width), h: px(b.height), r: px(r), a: Math.round(Number(cs.opacity) * 50) / 50 }];
    });
    // The window height is in the key: native y counts from the bottom, so a resize moves every box.
    const key = JSON.stringify(rects) + window.innerHeight;
    if (key !== last) {
      last = key;
      invoke("set_glass", { rects });
    }
  };
  const frame = () => {
    measure();
    requestAnimationFrame(frame);
  };
  requestAnimationFrame(frame);
  // WebKit pauses animation frames for panels it thinks are hidden, which would
  // freeze the glass mid-animation. Timers and animation ends still fire, so they
  // catch the final state.
  // ponytail: 100ms poll, no IPC unless a box changed.
  setInterval(measure, 100);
  for (const ev of ["animationend", "animationcancel", "transitionend"]) document.addEventListener(ev, measure, true);
}
