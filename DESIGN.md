# Otto: Design (v0.1 draft)

Hold a hotkey, say what you want, keep working. Otto does it in the background with tools (shell, files, APIs, MCP), never by taking over your screen, and reports back by notification or voice.

**Who it's for:** one person on their own laptop, juggling personal and work tasks. You're focused on thing A, and thing B pops into your head ("make a 5-slide deck on Q3 hiring", "find that lease PDF and summarize the pet clause", "draft a reply to Jordan"). You say it once and go back to A. Otto works on B in the background and hands you the result when it's done. It's an assistant you delegate to, not a voice command line.

Status: design locked 2026-11-03. Implementation started.

Mockups: https://claude.ai/artifact/8RCMzh2ifPQypHJ191iBbZ (source copied to `design/mockups/`, the `.dc.html` files are the visual spec).

## Build plan

| Milestone | What | Proves | Who |
|---|---|---|---|
| M0 spike | Tauri scaffold, hold-to-talk hotkey, mic capture, local whisper, overlay pill as non-activating panel | The 4 riskiest bits (hotkey release, mic permission, STT speed, focus-safe overlay) | Opus (lead) |
| M1 runner | Task queue + workdirs, CliRunner for `claude -p` stream-json, done toast | End-to-end: speak, task runs, result lands | Opus |
| M2 gate | Otto MCP server, tiers, approvals, audit log | Every action routed through the gate | Opus, Fable reviews |
| M3 UI | Chip, toasts, mascot states, 5-screen onboarding from mockups | Matches the design | Sonnet (parallel, from mockups) |
| M4 providers | NativeRunner: OpenAI-compatible + Anthropic | Bring-your-own-AI | Sonnet, Opus reviews |
| M5 ship | Signing, notarization, updater, DMG | Installable by anyone | Opus |

## Goals

- Hold-to-talk global hotkey, speak, release.
- Fire and forget: tasks can take minutes, several can run at once, results wait for you.
- Runs entirely in the background. No mouse, keyboard, or screen control.
- Bring your own AI: Claude, OpenAI, Gemini, OpenRouter, local Ollama/LM Studio, or an agent CLI you already pay for (`claude -p`, Codex, Gemini CLI).
- Installable by anyone: signed installer, onboarding, no terminal required.
- Safe by default: risky actions need a click to confirm, everything is logged.

## Non-goals (v0.1)

- Pixel-level computer use. Otto drives Mac apps through AppleScript, Shortcuts and System Events instead, with each action approved (added 2026-11-03).
- Wake word / always listening.
- Multi-turn conversation. (Lightweight memory landed 2026-11-03: `~/Otto/memory.md`, recent requests, and optional memory plugins like claude-mem.)
- Windows and Linux (designed for, not shipped).

## Architecture

```
 [hotkey] -> [mic capture] -> [STT] -> transcript
                                          |
                                          v
                                   Runner (task, policy)
                         +----------------+-----------------+
                         |                                  |
                   NativeRunner                        CliRunner
              (our agent loop)                 (claude -p, codex exec, ...)
                 |       |                                  |
       ModelAdapter   MCP client                 generated MCP config
       (OpenAI-compat |                                     |
        or Anthropic) +----------------+--------------------+
                                       v
                         Otto MCP server (localhost)
                         = built-in tools + PERMISSION GATE
                                       |
                         user-added MCP servers (proxied through gate)
                                       |
                                       v
                      notification + TTS + confirm HUD + audit log
```

### Task model

Each utterance becomes a **Task**, not a chat turn.

```
Task { id, transcript, status, runner, created_at, workdir, outputs[], log }
status: queued -> running -> needs_approval -> done | failed | cancelled
```

- **Parallel:** up to N tasks run at once (default 3), and the rest wait in a queue.
- **Workdir:** each task gets its own `~/Otto/tasks/<date>-<slug>/`. Generated files (decks, docs, notes) land there, so nothing scatters across the disk.
- **Tray list:** the menu bar shows running, waiting-for-approval and done tasks. Click one for the transcript, a live step log and its output files.
- **Done notification:** a short spoken or written summary plus "Open", e.g. "Deck ready: Q3 hiring, 5 slides."
- **Approvals queue up:** a task that hits a T2 action pauses there, and other tasks keep running. Approvals never steal focus with a modal. They show as a badge and a notification.
- **Follow-ups:** a hotkey with a task selected adds to that task ("make it 3 slides"). v0.2.

Artifact-heavy tasks (decks, docs, spreadsheets) favor the CliRunner. Agent CLIs like Claude Code already have file and document skills, so we don't rebuild them.

### Onboarding (first launch)

Five short screens. Every one except Permissions can be skipped and revisited in Settings. The speech model downloads in the background from screen 1, so nobody waits on a progress bar at the end.

1. **Welcome.** One line on what Otto does and one on what it never does ("never touches your screen, asks before sending or deleting").
2. **Permissions.** One card per permission, each with why it's needed and a Grant button. Each card turns green live when granted (poll its status):
   - Microphone (required).
   - Notifications (recommended; this is the fallback when the overlay is hidden).
   - Folder access: Documents / Desktop / Downloads (optional; explains the macOS prompt before it appears).
   - Input Monitoring: only shown if the user picks a modifier-only hotkey.
3. **Pick your AI.** Auto-detect first, then let the user choose:
   - Installed CLIs: `which claude codex gemini opencode`.
   - Local models: Ollama on `localhost:11434`, LM Studio on `localhost:1234`.
   - API key: paste for Anthropic / OpenAI / Gemini / OpenRouter, stored in Keychain.
   - **Test** button sends a tiny prompt and shows the reply, so a bad key fails here, not on the first real task.
4. **Voice + hotkey.** Pick the hotkey (default Ctrl+Opt+Space), then a live try-it: "Hold the key and say 'hi Otto'". The transcript appears on screen, which proves mic, hotkey and speech-to-text in one step.
5. **Extras (skippable).**
   - Task folder (default `~/Otto/tasks`).
   - Tools: **auto-detected, read-only.** With a CLI runner, Otto reads the connectors and MCP servers that CLI already has (e.g. `claude mcp list`) and shows them. To change them, the user changes them in that tool. With an API-key runner, there's no tool to read from, so this screen offers presets plus "paste JSON" instead.
   - Safety defaults (tier table, with T2 confirm locked on).

Finish screen: a suggested first task ("Try: 'make a one-page summary of the newest PDF in Downloads'").

### Overlay and notifications

Otto has its own on-screen overlay. It is a floating, always-on-top, **non-activating** panel: you can click its buttons, but keyboard focus stays in whatever app you're typing in. macOS notifications are only the fallback (overlay turned off, or the app hidden).

Original visual style. Not modeled on any specific product's look.

Three pieces:

```
   (top-right stack)                     +---------------------------+
                                         | done  Q3 hiring deck      |
                                         | 5 slides   [Open] [Show]  |
                                         +---------------------------+
                                         | !  Send email to          |
                                         | jon.smith@acme.com?       |
                                         | "send jon the notes"      |
                                         | [Approve]  [Deny]  [View] |
                                         +---------------------------+

   (bottom-center pill, while holding hotkey)

            (  o  ||||||||||||  listening...  )
            (  ...  transcribing              )
            (  +  Got it: "Q3 hiring deck"    )   -> shrinks into the chip

   (corner chip, idle)        ( 2 running  [ring] )   hover -> task list
```

1. **Pill.** Shows only while you talk. It has a live mic waveform, then "transcribing", then a one-line "Got it" with the task title, then it shrinks into the chip. Position is configurable (bottom center by default, or top center near the notch). The waveform is the feedback that it actually heard you.
2. **Chip.** A tiny corner indicator for running tasks: a count plus a progress ring. It hides itself when nothing is running. Hover expands it into the task list, and clicking a task shows its live step log and output files.
3. **Toasts.** These stack in a corner:
   - **Done:** the summary plus Open / Show in Finder. Auto-dismisses after about 6s; hovering pauses it.
   - **Needs approval:** stays until handled. Shows the resolved entities and the original transcript, with Approve / Deny / View.
   - **Failed:** a one-line cause plus Retry / View log.

Rules:
- Nothing steals focus or blocks the screen. No modals.
- Click-through everywhere except the visible cards.
- Respects macOS Focus / Do Not Disturb: toasts queue silently and the chip shows a badge.
- Spoken summaries are off by default and turned on per user in Settings.
- Approvals can also be handled from the chip's task list, so dismissing a toast never loses one.

Implementation: a Tauri transparent window with `alwaysOnTop`, `skipTaskbar` and `focus: false`, converted to a non-activating `NSPanel` (community `tauri-nspanel` plugin, or a small Objective-C shim if the plugin lags behind Tauri v2). Each overlay piece is a web component inside that one window.

### Brand

- **Mascot / logo:** Otto, a pebble in a bow tie (locked 2026-11-03). Pebble body in pastel sky, bow tie in mint, eyes in deep teal.
- **States** (same character, different face + motion):

  | State | Face | Motion |
  |---|---|---|
  | Resting | flat closed eyes | slow breathe |
  | Listening | open eyes, ear pops out on the side | head tilts toward you, small sound arcs at the ear |
  | Working | side profile (one eye, bow tie seen from the side) | busy bob, occasional blink |
  | Done | happy closed eyes | one hop |
  | Needs you | wide eyes, blue bow tie, "?" above head | wiggle, "?" bounces |
  | Didn't catch that | one eye squints | head tilt |
  | Stuck (failed) | crosses for eyes, rose bow tie, grey body | slump |

- **Where it appears:** the overlay chip (mirrors the busiest state across tasks), the listening pill, the toast icon, the app icon, and the onboarding welcome screen.
- **Menu bar:** a monochrome silhouette (macOS template image), no animation.
- Motion is CSS/SVG only, and turns off under "Reduce motion".
- **Palette:** mint `#DDF1E6` / `#8CC9B2`, sky `#DEEBF6` / `#9CC2E3`, teal `#2F6F62` (primary buttons), blue `#2B5677` (needs you), rose `#E3A29C` (failed), ink `#1C2622`.
- **Font:** Geist (Geist Mono for paths and keys).

### App shell

**Tauri v2, Rust core. macOS (Apple Silicon) first.**

| Piece | Choice |
|---|---|
| Hotkey | `global-hotkey` crate (Carbon `RegisterEventHotKey`, press + release) |
| Mic | `cpal` |
| STT | `whisper-rs` (whisper.cpp bindings) |
| HTTP / models | `reqwest` |
| MCP | `rmcp` (official Rust SDK) |
| Secrets | `keyring` crate (Keychain / Credential Manager / Secret Service) |
| TTS | macOS `AVSpeechSynthesizer` |

Runner-up: Electron + TypeScript (better MCP and AI SDK ecosystem, but hold-to-talk needs `uiohook-napi`, which needs Accessibility permission, plus a ~150MB binary).

Default hotkey is a chord (Ctrl+Opt+Space). Modifier-only keys (hold Right Option, Fn) need Input Monitoring permission, so they are opt-in.

### Provider abstraction

Unify at the task level, not the model level.

```
trait Runner {
    fn run(task: Task, policy: Policy) -> Stream<Event>;
    fn cancel();
}

enum Event {
    ToolStart { name, args, tier },
    NeedsApproval { id, summary },
    ToolResult { id, ok, preview },
    Final { spoken, detail },
    Error { msg },
}
```

- **NativeRunner** owns the agent loop. It talks to models through a thin `ModelAdapter.stream(messages, tools)` with two wire formats:
  - OpenAI-compatible: OpenAI, OpenRouter, Ollama, LM Studio, Gemini (compat endpoint).
  - Anthropic Messages.
- **CliRunner** spawns an agent CLI and maps its JSON stream onto `Event`.
  - `claude -p --output-format stream-json --mcp-config <gen> --strict-mcp-config --disallowedTools Bash,Edit,Write --permission-prompt-tool <ours>`
  - `codex exec --json`, Gemini CLI, opencode: later. If a CLI cannot route its own tools through our gate, show it as "trusted mode" in the UI.

**Everything is MCP.** Built-in tools live in a localhost MCP server that contains the permission gate. The native loop and every CLI connect to it, so policy is enforced in one place no matter which AI the user picks.

ToS note: spawn the official CLI binaries only. Never extract or reuse subscription OAuth tokens.

### Speech-to-text

- Default: local whisper.cpp, `large-v3-turbo` quantized (~550MB), downloaded on first run. On CPU-only machines default to `small.en`.
- Batch transcription on key release. No VAD or streaming needed for hold-to-talk.
- Pass contact names and app names as the initial prompt to cut entity mishearing.
- Opt-in cloud: Groq `whisper-large-v3-turbo`.
- Later: Parakeet TDT 0.6B via sherpa-onnx (faster, English-centric).
- TODO: benchmark latency on an M1 Air and a mid-range Intel laptop. Current numbers are estimates.

## Safety model

| Tier | Examples | Behavior |
|---|---|---|
| T0 | list, search, read files, read calendar | Auto |
| T1 | draft email, create reminder, write file in allowlisted dir | Auto, notify, undo where possible |
| T2 | send, delete, post, push, **any shell command** | Confirm with a click or hotkey |
| T3 | sudo, credential export, security settings | Blocked |

Rules:
1. Confirmation is never by voice. Mishearing "yes" is how an email reaches the wrong Jon.
2. The confirm card shows the transcript and resolved entities (`jon.smith@acme.com`, not "Jon").
3. Prompt injection: once a task reads untrusted content (email, web, downloads), every T1 escalates to T2 for the rest of that task. Never auto-fetch URLs found in content.
4. Audit log: append-only JSONL of every tool call (args, tier, approver, result), viewable in-app.
5. Secrets live in the OS keychain and never enter LLM context.

## v0.1 scope

In:
- macOS Apple Silicon, signed + notarized DMG, auto-update.
- Chord hotkey, local whisper.
- NativeRunner (both wire formats) + CliRunner for `claude -p`.
- Built-in MCP server, ~6 tools: list/read/search files, write in allowlisted dirs, shell (always T2), `open` app/URL.
- User-added MCP servers via pasted JSON.
- Overlay: pill, chip, toasts (done / approval / failed). Native notifications as fallback. Audit log.
- 5-screen onboarding: permissions, AI detect + test, hotkey try-it, extras.

Later:
- Windows (needs low-level hook for key release, signing cert) and Linux (Wayland: XDG GlobalShortcuts portal or a `otto toggle` CLI bound by the user).
- Codex / Gemini CLI / opencode adapters.
- Cloud STT, follow-up turns, memory.
- Calendar without full access: default to macOS write-only calendar access (add events, can't read them). Fallback for users who refuse any calendar permission: Otto maintains `otto.ics` and serves it at `webcal://localhost:<port>/otto.ics`; the user subscribes once in Calendar.app (auto-refreshes, no re-upload). Limits: read-only for the user, only on that Mac while Otto runs, no clash checking.
- Native Gmail/Calendar (Google restricted scopes need a paid security assessment, so rely on user-supplied MCP servers until then).

## macOS permission gotchas

- Mic: `NSMicrophoneUsageDescription` + audio-input entitlement under hardened runtime.
- Distribute as DMG, not Mac App Store (App Sandbox blocks spawning shells and MCP servers).
- Apple Developer account ($99/yr) required for notarization.
- Child processes inherit the app's privacy permissions (TCC): `ls ~/Documents` triggers "Otto wants access to Documents". Mail/Messages data needs Full Disk Access. Each `osascript` target triggers an Automation prompt. Onboarding must explain these.

## Risks

1. **Scope creep.** Deciding: a personal side-task assistant. Email/calendar still means OAuth pain, so v0.1 relies on user-added MCP servers for those.
2. **Latency.** STT + multi-step loop + TTS = 5 to 20s. Fine because it runs in the background; make that the pitch.
3. **MCP setup burden** for non-developers (needs Node or uv).
4. **Competition.** Block's Goose (provider-agnostic, MCP-native desktop agent) is one voice feature away from this. Also Claude/ChatGPT desktop voice, Raycast AI, Wispr Flow / Superwhisper command modes, Apple Intelligence Siri. Open Interpreter 01 is the closest prior art and stalled; study why.
5. **Wedge:** "hold a key, keep working, it reports back" plus real bring-your-own-AI.

## M2 gate: what's verified (2026-11-03)

- `--permission-prompt-tool mcp__otto__approve` over a local HTTP MCP endpoint works with `--restricted` (e2e tests in `src-tauri/src/gate.rs`): approved command runs, declined command doesn't, gate unreachable means nothing runs (fails closed, but slowly: ~7 min before Claude Code gives up).
- `--restricted` drops the user's CLAUDE.md, hooks and allow-rules (canary checked). Skills and plugins still load.
- Fable review (MUST-FIX) applied: unknown MCP tools always ask; taint covers WebSearch too and waits for the stream parser; Bash cards show any script the task wrote; `.git`/`.claude`/`.mcp.json` writes always ask; symlinks and `..` refused; hidden/bidi characters shown on cards; output rendered as plain text only.
- LATER: Claude Code Bash sandbox via `--settings` (the T3 blocklist is a speed bump, not a control); connectors with an explicit per-tool allowlist; "allow for this task" only for exact repeats; narrow `--add-dir` to subfolders; minimum Claude Code version check.

## Unverified (check before building)

- `global-hotkey` key-release events on Windows.
- Whether Codex exec / Gemini CLI can route approvals to an external tool.
- MCP Bundles support outside Claude Desktop.
- ~~STT latency numbers~~ Verified 2026-11-03: `base.en` on Apple Silicon, ~160 ms for a short command (debug build). Larger model still to benchmark.
- `claude mcp list` (or a config file) shows claude.ai connectors like Gmail/Calendar, not just local MCP servers. Codex/Gemini equivalents.
- ~~`tauri-nspanel` works with current Tauri v2~~ Not needed: M0 swaps the overlay's class to a non-activating `NSPanel` directly (`src-tauri/src/overlay.rs`). Verified 2026-11-03: shows over full-screen apps.

## Open decisions

None right now.

Decided:
- Audience: personal side-task assistant on your own laptop (2026-11-03).
- Stack: Tauri v2 + Rust (2026-11-03).
- Name: Otto (2026-11-03). Not yet checked against trademarks or app stores.
