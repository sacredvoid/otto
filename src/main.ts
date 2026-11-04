import "./platform";
import { listen } from "@tauri-apps/api/event";
import { trackGlass } from "./glass";

type PillState = "idle" | "listening" | "transcribing" | "heard" | "miss";
type Pill = { state: PillState; text: string | null; ms: number | null; follow_up: boolean };

const pill = document.getElementById("pill")!;
const label = document.getElementById("label")!;
const detail = document.getElementById("detail")!;
const wave = document.getElementById("wave")!;

const BARS = 14;
// Fixed per-bar shape so the wave looks like speech, scaled by mic level.
const shape = Array.from({ length: BARS }, (_, i) => 0.35 + 0.65 * Math.sin((i / (BARS - 1)) * Math.PI));
const bars = shape.map(() => wave.appendChild(document.createElement("span")));

const LABELS: Record<PillState, string> = {
  idle: "",
  listening: "Listening",
  transcribing: "Transcribing",
  heard: "Got it",
  miss: "Didn't catch that",
};

function clip(s: string, n = 52) {
  return s.length > n ? s.slice(0, n - 1).trimEnd() + "…" : s;
}

listen<Pill>("otto://pill", ({ payload }) => {
  pill.dataset.state = payload.state;
  pill.toggleAttribute("data-follow-up", payload.follow_up);
  label.textContent = payload.follow_up ? "Following up" : LABELS[payload.state];
  if (payload.state === "heard" && payload.text) {
    detail.textContent = `"${clip(payload.text)}"`;
    detail.title = payload.ms ? `${payload.ms} ms` : "";
  } else if (payload.state === "miss") {
    detail.textContent = payload.text ? clip(payload.text) : "hold and try again";
  } else {
    detail.textContent = payload.state === "listening" ? "release to send" : "";
  }
});

listen<number>("otto://level", ({ payload: level }) => {
  pill.style.setProperty("--level", Math.min(1, level * 1.6).toFixed(2));
  bars.forEach((b, i) => {
    const jitter = 0.75 + Math.random() * 0.5;
    b.style.height = `${Math.max(3, Math.min(22, 3 + level * 22 * shape[i] * jitter))}px`;
  });
});

trackGlass();
