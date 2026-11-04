# Contributing to Otto

Thanks for taking a look. Otto is small and early, so bug reports and focused pull requests both help a lot.

## Run it locally

You need macOS on Apple Silicon, [Rust](https://rustup.rs), Node 20+, and [Claude Code](https://docs.claude.com/claude-code) installed. Otto signs in with its own profile in `~/Otto/claude` (setup window, step 3).

```bash
git clone https://github.com/sacredvoid/otto.git && cd otto
npm install          # also downloads the speech model (~148 MB, once)
npm run tauri dev    # hold Ctrl + Option + Space to talk
```

The first launch opens the setup window. Your settings live in `~/Otto/config.json`, and every task gets a folder in `~/Otto/tasks/`, which is the first place to look when something goes wrong.

## Where things are

```
src-tauri/src/
  lib.rs         hotkey, recording, app wiring
  tasks.rs       task queue, how the claude CLI is started, system prompt, memory, follow-ups
  gate.rs        the permission gate (MCP server + policy + audit log)
  history.rs     turns task logs into the History trace
  onboarding.rs  setup window, connector detection
  updates.rs     daily "is there a newer release" check
src/             each window's page, script and styles
DESIGN.md        why things are the way they are
```

## Tests

```bash
cd src-tauri
cargo test                                            # fast, no network
cargo test e2e_ -- --ignored --test-threads=1         # runs the real Claude CLI, costs a few cents
npm run build                                         # type-checks the frontend
```

CI runs `cargo test` and builds the DMG on every push and pull request.

## The gate is the part to be careful with

Everything Claude Code wants to do goes through `classify()` in `gate.rs`. It's a pure function so it's easy to test. If your change touches it, or touches anything that decides what runs without asking:

- add a case to the `policy` test showing the new behavior,
- keep it failing closed: when unsure, ask the user,
- run the e2e tests above at least once.

Pull requests that loosen a rule need a sentence on why it's safe.

## Pull requests

- One change per pull request. Small is easier to review and to revert.
- Prefer what's already here (or in the standard library) over a new dependency.
- UI changes: include a screenshot.
- Explain the why in the description; the diff already shows the what.

## Reporting a bug

Open an issue with what you said, what you expected, and what happened. The task's folder in `~/Otto/tasks/` has `task.json` and `log.jsonl`, which usually show exactly where it went wrong. Read them before attaching, since they can include file contents, emails, or other personal data from that task.

If you find a way to get something past the permission gate, please don't open a public issue. Report it privately from the repo's **Security** tab (Report a vulnerability) instead.

## Releases

Maintainers tag a version (`git tag v0.3.2 && git push origin v0.3.2`), then bump `version` in `package.json`, `src-tauri/tauri.conf.json` and `src-tauri/Cargo.toml` to the next patch so local builds never look older than the latest release. CI builds the DMG and publishes the GitHub Release, and installed copies show "Update available" in the menu bar within a day.
