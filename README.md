<p align="center">
  <img src="docs/otto.svg" width="110" alt="Otto, a small pebble mascot">
</p>

<h1 align="center">Otto</h1>

<p align="center">
  <b>Hold a key, say what you need, keep working.</b><br>
  A voice-first background assistant for macOS, powered by the Claude Code you already have.
</p>

<p align="center">
  <img alt="macOS" src="https://img.shields.io/badge/macOS-14%2B-1c2622?style=flat-square&logo=apple&logoColor=white">
  <img alt="Tauri 2" src="https://img.shields.io/badge/Tauri-2-24594a?style=flat-square&logo=tauri&logoColor=white">
  <img alt="Rust" src="https://img.shields.io/badge/Rust-2024-2b5677?style=flat-square&logo=rust&logoColor=white">
  <img alt="Local speech" src="https://img.shields.io/badge/speech-on--device-5b6b65?style=flat-square">
  <img alt="Status" src="https://img.shields.io/badge/status-early%20preview-c98a2b?style=flat-square">
</p>

---

You're in the middle of something and remember another job: *"make a five-slide outline on Q3 hiring"*, *"find that lease PDF and tell me what it says about pets"*, *"what's on my calendar Thursday?"* Normally you'd switch windows and lose your place.

With Otto you hold **Ctrl + Option + Space**, say it, and let go. The speech is transcribed on your Mac and handed to Claude Code as a background task. You go back to what you were doing. A small chip in the corner shows what's running, and the result shows up as a card when it's ready.

## What it does

| Feature | How it works |
|---|---|
| Hold to talk | A global hotkey. Whisper runs on your Mac (Metal), so your audio never leaves it. |
| Background tasks | Up to three tasks at once, each in its own folder under `~/Otto/tasks`. Files a task produces stay there. |
| Permission gate | Every risky action (running a command, writing outside the task folder, sending email, going online after reading your files) waits for an *Allow* click. Each decision is logged to `~/Otto/audit.jsonl`. |
| Your connectors | Uses the Gmail, Google Calendar and other claude.ai connectors your Claude account already has. Read-only calls run on their own. Sending, creating and deleting always ask first. |
| Memory | Otto knows your recent requests, keeps a plain-text notes file (`~/Otto/memory.md`), and can load a memory plugin you already use, such as [claude-mem](https://github.com/thedotmack/claude-mem), so a vague request like "send it to Michael" lands. |
| Mac control | Drives Mail, Messages, Reminders, Notes, Finder and other apps through AppleScript and Shortcuts, and can take a screenshot to check its work. Every action is approved by you. |
| History | A window listing everything you've asked, with the answer, the steps taken, the approvals, and the files produced. |

## How it works

```
 hotkey ─▶ mic ─▶ whisper.cpp (local) ─▶ transcript
                                            │
                                            ▼
                          claude -p --restricted  (one per task)
                                            │
             every permission prompt ──▶ Otto's MCP gate (localhost)
                                            │            │
                              allow / ask you / block    audit.jsonl
                                            │
                       chip · approval cards · result cards · History
```

Otto has no model of its own. For each task it starts the `claude` CLI in restricted mode. In that mode Claude Code ignores your personal allow-rules and hooks, and every permission question goes to the gate built into Otto. The gate's rules live in one pure, unit-tested function (`src-tauri/src/gate.rs`). A task that has read your files or email is marked *tainted*, and after that any web access needs your OK. So a poisoned web page or email can't quietly send your data anywhere without you seeing it first.

## Getting started

**You'll need:** macOS 14+, [Rust](https://rustup.rs), Node 20+, and [Claude Code](https://docs.claude.com/claude-code) installed and logged in.

```bash
git clone https://github.com/sacredvoid/otto.git
cd otto
npm install

# Speech model (~140 MB, one time)
mkdir -p ~/Library/Application\ Support/app.otto.desktop/models
curl -L -o ~/Library/Application\ Support/app.otto.desktop/models/ggml-base.en.bin \
  https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin

npm run tauri dev
```

The setup window opens on first launch. It walks you through microphone access, picking which folders Otto may read, and a practice run of the hotkey.

## Configuration

Everything lives in `~/Otto/config.json`. Every key is optional.

```jsonc
{
  "readable_dirs": ["~/Documents", "~/Desktop"],          // read without asking
  "plugins": ["claude-mem@thedotmack"],                    // Claude Code plugins to load
  "mcp_servers": { "notion": { "type": "http", "url": "https://..." } },  // extra MCP servers
  "allow_tools": ["mcp__plugin_claude-mem_mcp-search__search"]           // read-only tools that skip the prompt
}
```

To edit what Otto remembers about you, open `~/Otto/memory.md`. It's one fact per line, and Otto adds lines when you tell it something worth keeping.

## Project layout

```
src-tauri/src/
  lib.rs        hotkey, recording, app wiring
  stt.rs        whisper.cpp transcription
  tasks.rs      task queue, claude CLI invocation, system prompt, memory
  gate.rs       MCP server + permission policy + audit log
  history.rs    reads past tasks for the History window
  onboarding.rs first-run setup
src/            TypeScript for each window (pill, cards, chip, setup, history)
DESIGN.md       the design doc and build plan
```

## Roadmap

- [x] Hold-to-talk, local speech, background Claude Code tasks
- [x] Permission gate with taint tracking and audit log
- [x] Corner chip, approval cards, onboarding, History
- [x] Connectors, memory, Mac app control
- [ ] Other AI backends: Codex CLI, Gemini CLI, OpenAI-compatible APIs, Ollama
- [ ] Download the speech model from inside the app
- [ ] Signed, notarized DMG with auto-update

## Status

Early preview. I use it daily, it has rough edges, and I've only run it on Apple Silicon.
