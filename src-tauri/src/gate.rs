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

    pub fn register(&self, task_id: u64, dir: &Path, title: &str) {
        let dir = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
        self.tasks.lock().unwrap().insert(task_id, TaskCtx { dir, title: title.into(), ..Default::default() });
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
        let line = json!({ "ts": unix_now(), "task": task_id, "tool": tool, "input": input, "decision": decision, "by": by });
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

/// The policy. Pure, so it's unit tested below.
fn classify(ctx: &TaskCtx, tool: &str, input: &Value) -> Verdict {
    let s = |k: &str| input[k].as_str().unwrap_or("").to_string();
    let ask = |summary: &str, detail: String| Verdict::Ask { summary: summary.into(), detail };
    match tool {
        "TodoWrite" => Verdict::Allow,
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
            ask("Run a command", detail)
        }
        "WebSearch" if !ctx.tainted => Verdict::Allow,
        "WebFetch" if !ctx.tainted => Verdict::Allow,
        "WebSearch" => ask("Search the web (this task has read your files)", s("query")),
        "WebFetch" => ask("Open a web page (this task has read your files)", s("url")),
        t if t.starts_with("mcp__") => {
            let mut parts = t.splitn(3, "__").skip(1);
            let (server, name) = (parts.next().unwrap_or("?"), parts.next().unwrap_or("?"));
            ask(&format!("Use {server}: {name}"), serde_json::to_string_pretty(input).unwrap_or_default())
        }
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
        c.tainted = true;
        assert!(is_ask(&classify(&c, "WebSearch", &json!({ "query": "x" }))));
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
        gate.register(1, &c.dir, "e2e");
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
    fn hidden_characters_are_shown() {
        assert_eq!(clean("rm\u{202E}fdp.txt"), "rm⟨U+202E⟩fdp.txt");
        assert_eq!(clean("a\nb"), "a\nb");
    }
}
