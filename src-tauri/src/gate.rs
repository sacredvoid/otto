//! The permission gate. Every action Claude Code would normally ask about is
//! routed here (`--permission-prompt-tool mcp__otto__approve`) over a local MCP
//! endpoint. The policy decides allow / ask the user / block, and every
//! decision is appended to ~/Otto/audit.jsonl.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::{json, Value};

pub const APPROVAL_TIMEOUT: Duration = Duration::from_secs(10 * 60);
/// How long an outbound call waits for the stream parser to catch up before
/// we assume the task may have seen private data.
const SEEN_WAIT: Duration = Duration::from_secs(2);

/// claude.ai connector tools that only read, judged by name since the gate
/// doesn't see MCP annotations. Read-only means a read verb near the start
/// ("search_threads", "slack_read_channel", "notion-fetch") and no word that
/// changes anything. Unknown shapes fall through to asking. They run without
/// asking but taint the task, so anything outbound afterwards still asks.
const READ_VERBS: [&str; 13] =
    ["get", "list", "search", "read", "fetch", "find", "query", "describe", "view", "lookup", "show", "count", "suggest"];
const WRITE_WORDS: [&str; 54] = [
    "send", "create", "update", "delete", "remove", "trash", "untrash", "archive", "post", "write", "edit", "set",
    "add", "move", "mark", "unmark", "label", "unlabel", "reply", "forward", "respond", "invite", "share", "upload",
    "modify", "apply", "cancel", "schedule", "publish", "comment", "react", "approve", "assign", "close", "merge",
    "rename", "copy", "import", "run", "execute", "invoke", "trigger", "start", "stop", "submit", "save", "patch",
    "put", "insert", "replace", "clear", "accept", "decline", "spam",
];

pub(crate) fn is_read_only_connector(tool: &str) -> bool {
    let Some(name) = tool.strip_prefix("mcp__claude_ai_").and_then(|r| r.split_once("__")).map(|(_, n)| n) else {
        return false;
    };
    let words: Vec<String> = name.split(['_', '-']).filter(|w| !w.is_empty()).map(str::to_ascii_lowercase).collect();
    // The verb may follow the app's name ("slack_read_channel").
    words.iter().take(2).any(|w| READ_VERBS.contains(&w.as_str())) && !words.iter().any(|w| WRITE_WORDS.contains(&w.as_str()))
}


const REMEMBER_MAX: usize = 500;

/// Tool-config and VCS paths that `--restricted` protects; never auto-written.
const PROTECTED: [&str; 5] = [".git", ".claude", ".mcp.json", ".claude.json", ".vscode"];

#[derive(Clone, Serialize, Debug)]
pub struct Ask {
    pub task_id: u64,
    pub approval_id: u64,
    pub title: String,
    pub summary: String,
    pub detail: String,
}

#[derive(Debug, PartialEq)]
pub enum Verdict {
    Allow,
    Ask { summary: String, detail: String },
    Block(String),
}

#[derive(Default)]
struct TaskCtx {
    dir: PathBuf,
    title: String,
    /// Set once the task may hold private or untrusted data; outbound calls then ask.
    tainted: bool,
    seen: HashSet<String>,
    /// File names this task wrote, shown alongside any command that mentions them.
    written: HashSet<String>,
    /// Folders find_files may search.
    readable: Vec<PathBuf>,
    /// Extra tools the user allowed in config.json ("allow_tools").
    allow: Vec<String>,
    /// The user allowed a web search in this task; later ones run without asking.
    web_ok: bool,
    /// Tools and servers switched off in setup ("disabled" in config.json).
    disabled: Vec<String>,
}

pub struct Gate {
    token: String,
    port: u16,
    tasks: Mutex<HashMap<u64, TaskCtx>>,
    seen_cv: Condvar,
    pending: Mutex<HashMap<u64, mpsc::Sender<bool>>>,
    next_approval: AtomicU64,
    on_ask: Box<dyn Fn(Ask) + Send + Sync>,
    audit: Mutex<Option<File>>,
}

impl Gate {
    pub fn start(audit_path: &Path, on_ask: impl Fn(Ask) + Send + Sync + 'static) -> Result<Arc<Self>, String> {
        let server = tiny_http::Server::http("127.0.0.1:0").map_err(|e| format!("gate server: {e}"))?;
        let port = server.server_addr().to_ip().map(|a| a.port()).ok_or("gate server has no port")?;
        if let Some(parent) = audit_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let audit = std::fs::OpenOptions::new().create(true).append(true).open(audit_path).ok();
        let gate = Arc::new(Gate {
            token: random_hex(16)?,
            port,
            tasks: Mutex::default(),
            seen_cv: Condvar::new(),
            pending: Mutex::default(),
            next_approval: AtomicU64::new(1),
            on_ask: Box::new(on_ask),
            audit: Mutex::new(audit),
        });
        let g = gate.clone();
        std::thread::spawn(move || {
            for req in server.incoming_requests() {
                let g = g.clone();
                std::thread::spawn(move || g.handle(req));
            }
        });
        Ok(gate)
    }

    pub fn url(&self, task_id: u64) -> String {
        format!("http://127.0.0.1:{}/mcp/{}/{}", self.port, self.token, task_id)
    }

    pub fn register(&self, task_id: u64, dir: &Path, title: &str, readable: Vec<PathBuf>, allow: Vec<String>, disabled: Vec<String>) {
        let dir = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
        let ctx = TaskCtx { dir, title: title.into(), readable, allow, disabled, ..Default::default() };
        self.tasks.lock().unwrap().insert(task_id, ctx);
    }

    pub fn taint(&self, task_id: u64) {
        if let Some(c) = self.tasks.lock().unwrap().get_mut(&task_id) {
            c.tainted = true;
        }
    }

    pub fn unregister(&self, task_id: u64) {
        self.tasks.lock().unwrap().remove(&task_id);
    }

    /// Fed by the stream-json parser for every tool_use, in order.
    pub fn observe(&self, task_id: u64, tool_use_id: &str, tool: &str, input: &Value) {
        let mut tasks = self.tasks.lock().unwrap();
        let Some(ctx) = tasks.get_mut(&task_id) else { return };
        ctx.seen.insert(tool_use_id.to_string());
        match tool {
            "Read" | "Glob" | "Grep" | "LS" => {
                let p = input["file_path"].as_str().or(input["path"].as_str());
                if p.is_some_and(|p| !resolves_inside(Path::new(p), &ctx.dir)) {
                    ctx.tainted = true;
                }
            }
            // Command output and fetched pages can carry private or hostile text.
            "Bash" | "WebFetch" => ctx.tainted = true,
            t if t.starts_with("mcp__") && t != "mcp__otto__approve" => ctx.tainted = true,
            "Write" | "Edit" | "MultiEdit" => {
                if let Some(name) = input["file_path"].as_str().and_then(|p| Path::new(p).file_name()) {
                    ctx.written.insert(name.to_string_lossy().into_owned());
                }
            }
            _ => {}
        }
        drop(tasks);
        self.seen_cv.notify_all();
    }

    /// Answer from the user's Allow / Don't allow click.
    pub fn answer(&self, approval_id: u64, allow: bool) {
        if let Some(tx) = self.pending.lock().unwrap().remove(&approval_id) {
            let _ = tx.send(allow);
        }
    }

    /// Ok(()) = allow. Err(message) = deny, told to the model.
    fn decide(&self, task_id: u64, tool: &str, input: &Value, tool_use_id: Option<&str>) -> Result<(), String> {
        let (verdict, title) = {
            let mut tasks = self.tasks.lock().unwrap();
            if !tasks.contains_key(&task_id) {
                return Err("Unknown task; denied.".into());
            }
            // Outbound calls: make sure the parser has caught up so taint is current.
            if matches!(tool, "WebSearch" | "WebFetch") {
                let mut caught_up = false;
                if let Some(id) = tool_use_id {
                    let (guard, _) = self
                        .seen_cv
                        .wait_timeout_while(tasks, SEEN_WAIT, |t| !t.get(&task_id).is_some_and(|c| c.seen.contains(id)))
                        .unwrap();
                    tasks = guard;
                    caught_up = tasks.get(&task_id).is_some_and(|c| c.seen.contains(id));
                }
                if !caught_up {
                    if let Some(c) = tasks.get_mut(&task_id) {
                        c.tainted = true;
                    }
                }
            }
            let Some(ctx) = tasks.get(&task_id) else { return Err("Task ended.".into()) };
            (classify(ctx, tool, input), ctx.title.clone())
        };

        match verdict {
            Verdict::Allow => {
                self.log(task_id, tool, input, "allow", "policy");
                Ok(())
            }
            Verdict::Block(why) => {
                self.log(task_id, tool, input, "block", "policy");
                Err(format!("Blocked by Otto: {why}. Do not retry; tell the user what you skipped."))
            }
            Verdict::Ask { summary, detail } => {
                let approval_id = self.next_approval.fetch_add(1, Ordering::SeqCst);
                let (tx, rx) = mpsc::channel();
                self.pending.lock().unwrap().insert(approval_id, tx);
                (self.on_ask)(Ask { task_id, approval_id, title, summary, detail: clean(&detail) });
                let answer = rx.recv_timeout(APPROVAL_TIMEOUT);
                self.pending.lock().unwrap().remove(&approval_id);
                match answer {
                    Ok(true) => {
                        if tool == "WebSearch" {
                            if let Some(c) = self.tasks.lock().unwrap().get_mut(&task_id) {
                                c.web_ok = true;
                            }
                        }
                        self.log(task_id, tool, input, "allow", "user");
                        Ok(())
                    }
                    Ok(false) => {
                        self.log(task_id, tool, input, "deny", "user");
                        Err("The user declined this action. Continue without it and mention what you skipped.".into())
                    }
                    Err(_) => {
                        self.log(task_id, tool, input, "deny", "timeout");
                        Err("No answer from the user in time, so this action was skipped.".into())
                    }
                }
            }
        }
    }

    fn log(&self, task_id: u64, tool: &str, input: &Value, decision: &str, by: &str) {
        let mut input = input.to_string();
        if input.len() > 4000 {
            input = format!("{}…", &input[..input.floor_char_boundary(4000)]);
        }
        // The task folder name is the stable id (task numbers restart every launch); History matches on it.
        let folder = self.tasks.lock().unwrap().get(&task_id).and_then(|c| c.dir.file_name().map(|n| n.to_string_lossy().into_owned()));
        let line = json!({ "ts": unix_now(), "task": task_id, "folder": folder, "tool": tool, "input": input, "decision": decision, "by": by });
        if let Some(f) = self.audit.lock().unwrap().as_mut() {
            let _ = writeln!(f, "{line}");
        }
    }

    // ---- minimal MCP (JSON-RPC over streamable HTTP, JSON responses only) ----

    fn handle(&self, mut req: tiny_http::Request) {
        let task_id = req
            .url()
            .strip_prefix("/mcp/")
            .and_then(|rest| rest.split_once('/'))
            .filter(|(token, _)| ct_eq(token.as_bytes(), self.token.as_bytes()))
            .and_then(|(_, id)| id.parse::<u64>().ok());
        let Some(task_id) = task_id else {
            let _ = req.respond(tiny_http::Response::empty(404));
            return;
        };
        if *req.method() != tiny_http::Method::Post {
            let _ = req.respond(tiny_http::Response::empty(405));
            return;
        }
        let mut body = String::new();
        if req.as_reader().take(1 << 20).read_to_string(&mut body).is_err() {
            let _ = req.respond(tiny_http::Response::empty(400));
            return;
        }
        let Ok(msg) = serde_json::from_str::<Value>(&body) else {
            let _ = req.respond(tiny_http::Response::empty(400));
            return;
        };
        let Some(id) = msg.get("id").cloned() else {
            let _ = req.respond(tiny_http::Response::empty(202)); // notification
            return;
        };
        let reply = match self.rpc(task_id, msg["method"].as_str().unwrap_or(""), &msg["params"]) {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err((code, message)) => json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }),
        };
        let header = tiny_http::Header::from_bytes("Content-Type", "application/json").unwrap();
        let _ = req.respond(tiny_http::Response::from_string(reply.to_string()).with_header(header));
    }

    fn rpc(&self, task_id: u64, method: &str, params: &Value) -> Result<Value, (i64, String)> {
        match method {
            "initialize" => Ok(json!({
                "protocolVersion": params["protocolVersion"].as_str().unwrap_or("2025-06-18"),
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "otto", "version": env!("CARGO_PKG_VERSION") },
            })),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": [{
                "name": "find_files",
                "description": "Find files on the user's Mac with Spotlight, limited to the folders Otto may read. \
Much faster than Grep or Glob across big folders. Matches file contents and names; set name_only to match names only. \
Returns up to 50 paths, skipping dependency and build folders.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "Words to look for, e.g. attention sink" },
                        "name_only": { "type": "boolean" }
                    },
                    "required": ["query"]
                }
            }, {
                "name": "remember",
                "description": "Save one short lasting fact or preference about the user to Otto's memory \
(e.g. 'Michael Chen is my manager; message him on Slack'). Shown to you at the start of every future task.",
                "inputSchema": {
                    "type": "object",
                    "properties": { "fact": { "type": "string", "description": "One line, under 500 characters" } },
                    "required": ["fact"]
                }
            }, {
                "name": "approve",
                "description": "Otto's permission check. Called by Claude Code, not by the model.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "tool_name": { "type": "string" },
                        "input": { "type": "object" },
                        "tool_use_id": { "type": "string" }
                    },
                    "required": ["tool_name", "input"]
                }
            }]})),
            "tools/call" if params["name"] == "find_files" => {
                let args = &params["arguments"];
                let query = args["query"].as_str().unwrap_or("").trim();
                if query.is_empty() || query.len() > 200 {
                    return Err((-32602, "query must be 1 to 200 characters".into()));
                }
                let readable = self.tasks.lock().unwrap().get(&task_id).map(|c| c.readable.clone()).unwrap_or_default();
                let found = find_files(&readable, query, args["name_only"] == true);
                let text = if found.is_empty() { "No matches.".to_string() } else { found.join("\n") };
                Ok(json!({ "content": [{ "type": "text", "text": text }] }))
            }
            "tools/call" if params["name"] == "remember" => {
                let fact = params["arguments"]["fact"].as_str().unwrap_or("").split_whitespace().collect::<Vec<_>>().join(" ");
                if fact.is_empty() || fact.len() > REMEMBER_MAX {
                    return Err((-32602, format!("fact must be 1 to {REMEMBER_MAX} characters")));
                }
                remember(&crate::tasks::memory_path(), &fact).map_err(|e| (-32603, format!("could not save: {e}")))?;
                Ok(json!({ "content": [{ "type": "text", "text": "Saved." }] }))
            }
            "tools/call" if params["name"] == "approve" => {
                let args = &params["arguments"];
                let tool = args["tool_name"].as_str().unwrap_or("");
                let input = &args["input"];
                // Side-effect free for any caller: it only answers, never changes policy.
                let decision = self.decide(task_id, tool, input, args["tool_use_id"].as_str());
                let body = match decision {
                    Ok(()) => json!({ "behavior": "allow", "updatedInput": input }),
                    Err(message) => json!({ "behavior": "deny", "message": message }),
                };
                Ok(json!({ "content": [{ "type": "text", "text": body.to_string() }] }))
            }
            "tools/call" => Err((-32602, "unknown tool".into())),
            _ => Err((-32601, "method not found".into())),
        }
    }
}

const SKIP_DIRS: [&str; 9] =
    ["/node_modules/", "/.venv/", "/venv/", "/site-packages/", "/target/", "/.git/", "/dist/", "/build/", "/Library/"];

/// Spotlight search across the readable folders. No shell: the query is a plain argument.
// ponytail: no timeout on mdfind; it answers from the index in well under a second.
fn find_files(readable: &[PathBuf], query: &str, name_only: bool) -> Vec<String> {
    let mut out = Vec::new();
    for dir in readable {
        let mut cmd = std::process::Command::new("mdfind");
        cmd.arg("-onlyin").arg(dir);
        if name_only {
            cmd.arg("-name");
        }
        let Ok(o) = cmd.arg(query).output() else { continue };
        out.extend(
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .filter(|p| !SKIP_DIRS.iter().any(|s| p.contains(s)))
                .map(String::from),
        );
    }
    out.sort();
    out.dedup();
    out.truncate(50);
    out
}

/// Appends one line to the memory file. Small O_APPEND writes don't interleave.
fn remember(path: &Path, fact: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(f, "- {}", clean(fact))
}

/// "mcp__claude_ai_Gmail__send_message" -> "Gmail: send message".
fn tool_label(tool: &str) -> String {
    let mut parts = tool.splitn(3, "__").skip(1);
    let (server, name) = (parts.next().unwrap_or("?"), parts.next().unwrap_or("?"));
    let server = server.strip_prefix("claude_ai_").unwrap_or(server).replace('_', " ");
    format!("{server}: {}", name.replace('_', " "))
}

/// The policy. Pure, so it's unit tested below.
fn classify(ctx: &TaskCtx, tool: &str, input: &Value) -> Verdict {
    let s = |k: &str| input[k].as_str().unwrap_or("").to_string();
    let ask = |summary: &str, detail: String| Verdict::Ask { summary: summary.into(), detail };
    if crate::tasks::is_disabled(&ctx.disabled, tool) {
        return Verdict::Block("you turned this off in Otto's setup".into());
    }
    match tool {
        // Otto's own read-only Spotlight search, confined to the readable folders.
        "TodoWrite" | "mcp__otto__find_files" => Verdict::Allow,
        t if is_read_only_connector(t) || ctx.allow.iter().any(|a| a == t) => Verdict::Allow,
        // Saving a memory shapes every later task, so a task that read untrusted text must ask.
        "mcp__otto__remember" if !ctx.tainted => Verdict::Allow,
        "mcp__otto__remember" => ask("Save to Otto's memory (this task has read your files)", s("fact")),
        "Write" | "Edit" | "MultiEdit" | "NotebookEdit" => {
            let path = s("file_path");
            let p = Path::new(&path);
            if path.is_empty() {
                ask("Change a file", input.to_string())
            } else if is_protected(p) {
                ask("Change a protected config file", path)
            } else if resolves_inside(p, &ctx.dir) {
                Verdict::Allow
            } else {
                ask("Change a file outside this task's folder", path)
            }
        }
        "Read" | "Glob" | "Grep" | "LS" => {
            let p = input["file_path"].as_str().or(input["path"].as_str()).unwrap_or("");
            ask("Read outside your allowed folders", if p.is_empty() { input.to_string() } else { p.to_string() })
        }
        "Bash" => {
            let cmd = s("command");
            if let Some(why) = blocked_command(&cmd) {
                return Verdict::Block(why.into());
            }
            let mut detail = cmd.clone();
            if touches_network(&cmd) {
                detail = format!("[uses the network]\n{detail}");
            }
            // Show any file this task wrote that the command runs or reads.
            for name in &ctx.written {
                if cmd.contains(name.as_str()) {
                    let body = std::fs::read_to_string(ctx.dir.join(name)).unwrap_or_default();
                    let head: Vec<&str> = body.lines().take(60).collect();
                    detail.push_str(&format!("\n\n--- {name} (written by this task) ---\n{}", head.join("\n")));
                    if body.lines().count() > 60 {
                        detail.push_str("\n… (more lines in the task folder)");
                    }
                }
            }
            let summary = if cmd.contains("osascript") { "Control a Mac app" } else { "Run a command" };
            ask(summary, detail)
        }
        // ponytail: one OK covers every search in the task; queries stay visible in History.
        "WebSearch" if !ctx.tainted || ctx.web_ok => Verdict::Allow,
        "WebFetch" if !ctx.tainted => Verdict::Allow,
        "WebSearch" => ask("Search the web (this task has read your files)", s("query")),
        "WebFetch" => ask("Open a web page (this task has read your files)", s("url")),
        t if t.starts_with("mcp__") => ask(&tool_label(t), serde_json::to_string_pretty(input).unwrap_or_default()),
        other => ask(&format!("Use {other}"), serde_json::to_string_pretty(input).unwrap_or_default()),
    }
}

// ponytail: a speed bump, not a control. Trivially bypassed (quoting, base64,
// python -c). Real containment is the Claude Code Bash sandbox via --settings (planned).
fn blocked_command(cmd: &str) -> Option<&'static str> {
    let c = cmd.to_lowercase();
    if c.split(|ch: char| !ch.is_alphanumeric()).any(|w| w == "sudo") {
        Some("admin (sudo) commands are never allowed")
    } else if c.contains(".ssh") || c.contains("security find") || c.contains("security dump") {
        Some("keys and passwords are off limits")
    } else {
        None
    }
}

fn touches_network(cmd: &str) -> bool {
    let c = cmd.to_lowercase();
    ["curl", "wget", "http://", "https://", "nc ", "ssh ", "scp ", "rsync ", "ftp", "git push", "npm publish"]
        .iter()
        .any(|k| c.contains(k))
}

fn is_protected(p: &Path) -> bool {
    p.components().any(|c| matches!(c, Component::Normal(n) if PROTECTED.iter().any(|x| n == *x)))
}

/// True if `p` (absolute, or relative to `dir`) lands inside `dir` after
/// resolving symlinks on the nearest existing ancestor. An existing symlink at
/// `p` itself, or any "..", is refused.
pub fn resolves_inside(p: &Path, dir: &Path) -> bool {
    let abs = if p.is_absolute() { p.to_path_buf() } else { dir.join(p) };
    if abs.symlink_metadata().is_ok_and(|m| m.file_type().is_symlink()) {
        return false;
    }
    if abs.components().any(|c| c == Component::ParentDir) {
        return false;
    }
    let mut existing = abs.as_path();
    let mut rest = Vec::new();
    while !existing.exists() {
        match (existing.file_name(), existing.parent()) {
            (Some(name), Some(parent)) => {
                rest.push(name.to_owned());
                existing = parent;
            }
            _ => return false,
        }
    }
    let (Ok(mut real), Ok(dir)) = (existing.canonicalize(), dir.canonicalize()) else { return false };
    real.extend(rest.iter().rev());
    real.starts_with(dir)
}

/// Makes hidden characters visible on the approval card so the command the
/// user reads is the command that runs.
pub fn clean(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '\n' | '\t' => c.to_string(),
            c if c.is_control() => format!("⟨U+{:04X}⟩", c as u32),
            '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}' => {
                format!("⟨U+{:04X}⟩", c as u32)
            }
            c => c.to_string(),
        })
        .collect()
}

fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn random_hex(n: usize) -> Result<String, String> {
    let mut buf = vec![0u8; n];
    File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut buf)).map_err(|e| format!("no randomness: {e}"))?;
    Ok(buf.iter().map(|b| format!("{b:02x}")).collect())
}

fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> (TaskCtx, PathBuf) {
        let root = std::env::temp_dir().join(format!("otto-gate-{}-{}", std::process::id(), random_hex(4).unwrap()));
        let dir = root.join("task");
        std::fs::create_dir_all(&dir).unwrap();
        let dir = dir.canonicalize().unwrap();
        (TaskCtx { dir: dir.clone(), ..Default::default() }, root)
    }

    fn is_ask(v: &Verdict) -> bool {
        matches!(v, Verdict::Ask { .. })
    }

    #[test]
    fn policy() {
        let (mut c, root) = ctx();
        let inside = c.dir.join("notes.md");
        assert_eq!(classify(&c, "Write", &json!({ "file_path": inside })), Verdict::Allow);
        assert!(is_ask(&classify(&c, "Write", &json!({ "file_path": "/etc/hosts" }))));
        assert!(is_ask(&classify(&c, "Write", &json!({ "file_path": c.dir.join("../escape.md") }))));
        assert!(is_ask(&classify(&c, "Write", &json!({ "file_path": c.dir.join(".git/hooks/pre-commit") }))));
        assert!(is_ask(&classify(&c, "Bash", &json!({ "command": "ls" }))));
        assert!(matches!(classify(&c, "Bash", &json!({ "command": "sudo rm -rf /" })), Verdict::Block(_)));
        assert!(is_ask(&classify(&c, "mcp__gmail__label_thread", &json!({}))), "unknown MCP tools ask");
        assert_eq!(classify(&c, "WebSearch", &json!({ "query": "x" })), Verdict::Allow);
        assert_eq!(classify(&c, "mcp__otto__find_files", &json!({ "query": "x" })), Verdict::Allow);
        assert_eq!(classify(&c, "mcp__claude_ai_Gmail__search_threads", &json!({})), Verdict::Allow);
        assert!(is_ask(&classify(&c, "mcp__claude_ai_Gmail__send_message", &json!({ "to": "m@x.com" }))), "sends ask");
        for t in ["mcp__claude_ai_Gmail__get_thread", "mcp__claude_ai_Google_Calendar__list_events", "mcp__claude_ai_Google_Calendar__suggest_time",
                  "mcp__claude_ai_Slack__slack_read_channel", "mcp__claude_ai_Slack__slack_search_public", "mcp__claude_ai_Notion__notion-fetch"] {
            assert_eq!(classify(&c, t, &json!({})), Verdict::Allow, "{t} reads");
        }
        for t in ["mcp__claude_ai_Gmail__create_draft", "mcp__claude_ai_Gmail__trash_thread", "mcp__claude_ai_Gmail__label_thread",
                  "mcp__claude_ai_Google_Calendar__create_event", "mcp__claude_ai_Google_Calendar__respond_to_event",
                  "mcp__claude_ai_Slack__slack_send_message", "mcp__claude_ai_Slack__slack_schedule_message",
                  "mcp__claude_ai_Notion__notion-update-page", "mcp__claude_ai_X__search_and_replace", "mcp__claude_ai_X__get_or_create",
                  "mcp__claude_ai_X__do_thing", "mcp__claude_ai_Gmail", "mcp__someserver__search", "mcp__plugin_x_y__list_items"] {
            assert!(is_ask(&classify(&c, t, &json!({}))), "{t} asks");
        }
        assert!(is_ask(&classify(&c, "mcp__mem__search", &json!({}))));
        c.allow = vec!["mcp__mem__search".into()];
        assert_eq!(classify(&c, "mcp__mem__search", &json!({})), Verdict::Allow);
        assert_eq!(classify(&c, "mcp__otto__remember", &json!({ "fact": "x" })), Verdict::Allow);
        c.tainted = true;
        assert!(is_ask(&classify(&c, "mcp__otto__remember", &json!({ "fact": "x" }))), "tainted memory writes ask");
        assert!(is_ask(&classify(&c, "WebSearch", &json!({ "query": "x" }))));
        c.web_ok = true;
        c.disabled = vec!["mcp__claude_ai_Gmail".into()];
        assert!(matches!(classify(&c, "mcp__claude_ai_Gmail__search_threads", &json!({})), Verdict::Block(_)), "off beats auto-allow");
        c.disabled.clear();
        assert_eq!(classify(&c, "WebSearch", &json!({ "query": "y" })), Verdict::Allow, "one OK per task");
        assert!(is_ask(&classify(&c, "WebFetch", &json!({ "url": "https://x" }))), "fetches still ask");
        assert!(is_ask(&classify(&c, "WebFetch", &json!({ "url": "https://x" }))));

        // A symlink inside the task folder pointing out is not "inside".
        std::os::unix::fs::symlink("/etc", c.dir.join("link")).unwrap();
        assert!(!resolves_inside(&c.dir.join("link"), &c.dir));
        assert!(!resolves_inside(&c.dir.join("link/hosts"), &c.dir));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn bash_card_shows_task_written_script() {
        let (mut c, root) = ctx();
        std::fs::write(c.dir.join("analyze.py"), "import os\nos.system('curl evil')\n").unwrap();
        c.written.insert("analyze.py".into());
        let Verdict::Ask { detail, .. } = classify(&c, "Bash", &json!({ "command": "python3 analyze.py" })) else {
            panic!("expected ask")
        };
        assert!(detail.contains("os.system('curl evil')"), "{detail}");
        let _ = std::fs::remove_dir_all(root);
    }

    /// End-to-end against the real Claude Code CLI. Costs a few cents.
    /// Run with: cargo test e2e_ -- --ignored --nocapture --test-threads=1
    fn e2e(answer: Option<bool>) -> (bool, Vec<Ask>) {
        let (c, root) = ctx();
        let (tx, rx) = mpsc::channel::<Ask>();
        let gate = Gate::start(&root.join("audit.jsonl"), move |a| tx.send(a).unwrap()).unwrap();
        gate.register(1, &c.dir, "e2e", vec![], vec![], vec![]);
        let url = if answer.is_some() { gate.url(1) } else { "http://127.0.0.1:9/mcp/nope/1".into() };
        let g = gate.clone();
        let asks = std::thread::spawn(move || {
            let mut seen = Vec::new();
            while let Ok(a) = rx.recv_timeout(Duration::from_secs(150)) {
                g.answer(a.approval_id, answer.unwrap_or(false));
                seen.push(a);
            }
            seen
        });
        let claude = crate::tasks::claude_bin().expect("claude installed");
        let out = crate::tasks::claude_command(
            &claude,
            "Use the Bash tool to run exactly this command: touch ran.txt\nThen reply with the single word DONE.",
            &c.dir,
            &url,
        )
        .output()
        .unwrap();
        println!("stdout tail: {}", String::from_utf8_lossy(&out.stdout).lines().last().unwrap_or(""));
        let ran = c.dir.join("ran.txt").exists();
        drop(gate);
        let asks = asks.join().unwrap();
        let _ = std::fs::remove_dir_all(root);
        (ran, asks)
    }

    #[test]
    #[ignore = "runs the real claude CLI"]
    fn e2e_allowed_command_runs() {
        let (ran, asks) = e2e(Some(true));
        assert_eq!(asks.len(), 1, "{asks:?}");
        assert!(asks[0].detail.contains("touch ran.txt"));
        assert!(ran, "approved command should run");
    }

    #[test]
    #[ignore = "runs the real claude CLI"]
    fn e2e_denied_command_does_not_run() {
        let (ran, asks) = e2e(Some(false));
        assert!(!asks.is_empty());
        assert!(!ran, "denied command must not run");
    }

    #[test]
    #[ignore = "runs the real claude CLI"]
    fn e2e_gate_down_fails_closed() {
        let (ran, _) = e2e(None);
        assert!(!ran, "with the gate unreachable nothing may run");
    }

    #[test]
    fn remember_appends_lines() {
        let (_, root) = ctx();
        let path = root.join("memory.md");
        remember(&path, "Michael is my manager").unwrap();
        remember(&path, "Prefers short emails").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "- Michael is my manager\n- Prefers short emails\n");
        assert_eq!(tool_label("mcp__claude_ai_Gmail__send_message"), "Gmail: send message");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn hidden_characters_are_shown() {
        assert_eq!(clean("rm\u{202E}fdp.txt"), "rm⟨U+202E⟩fdp.txt");
        assert_eq!(clean("a\nb"), "a\nb");
    }
}
