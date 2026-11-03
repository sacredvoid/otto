//! Spoken requests become background tasks, each run by an agent CLI in its own folder.

use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::Value;
use std::sync::Arc;

use crate::gate::{self, Gate};
use tauri::{AppHandle, Emitter, Manager};

pub const MAX_RUNNING: usize = 3;

const SYSTEM_PROMPT: &str = "You are Otto, a background assistant on the user's Mac. \
The user said this request out loud while focused on other work, and it was transcribed by \
speech recognition, so quietly fix obvious mis-hearings. Do the task without asking follow-up \
questions: make reasonable assumptions and mention them. Save anything you produce (documents, \
notes, drafts, slide outlines) as files in the current directory. Some actions need the user's \
OK (running commands, changing files outside this folder, and web access after reading their \
files); Otto asks them for you. If an action is declined, continue without it and say what you \
skipped. Finish with a one or two sentence plain-text summary suitable for a notification, no markdown.";

/// Files Otto writes into every task folder; never offered as "the result".
const META_FILES: [&str; 3] = ["task.json", "log.jsonl", "result.md"];

#[derive(Clone, Serialize)]
pub struct TaskEvent {
    id: u64,
    title: String,
    status: &'static str, // queued | running | done | failed
    step: Option<String>,
    summary: Option<String>,
}

struct Job {
    id: u64,
    title: String,
    prompt: String,
    dir: PathBuf,
}

#[derive(Default)]
pub struct Tasks {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    next_id: u64,
    running: usize,
    queue: VecDeque<Job>,
    dirs: HashMap<u64, PathBuf>,
}

fn emit(app: &AppHandle, id: u64, title: &str, status: &'static str, step: Option<String>, summary: Option<String>) {
    let _ = app.emit("otto://task", TaskEvent { id, title: title.to_string(), status, step, summary });
}

impl Tasks {
    /// Queues a spoken request and returns its short title.
    pub fn submit(app: &AppHandle, transcript: &str) -> Result<String, String> {
        let title = title_of(transcript);
        let dir = new_task_dir(&tasks_root(), transcript)?;
        let meta = serde_json::json!({ "transcript": transcript, "created_unix": unix_now() });
        std::fs::write(dir.join("task.json"), serde_json::to_vec_pretty(&meta).unwrap())
            .map_err(|e| format!("could not write task file: {e}"))?;

        let tasks = app.state::<Tasks>();
        let mut inner = tasks.inner.lock().unwrap();
        inner.next_id += 1;
        let id = inner.next_id;
        inner.dirs.insert(id, dir.clone());
        inner.queue.push_back(Job { id, title: title.clone(), prompt: transcript.to_string(), dir });
        drop(inner);

        emit(app, id, &title, "queued", None, None);
        start_ready(app);
        Ok(title)
    }

    pub fn dir(&self, id: u64) -> Option<PathBuf> {
        self.inner.lock().unwrap().dirs.get(&id).cloned()
    }
}

/// What a finished task produced: its summary and the files it wrote.
#[derive(Serialize)]
pub struct Details {
    summary: String,
    files: Vec<String>,
}

pub fn details(dir: &Path) -> Details {
    let summary = std::fs::read_to_string(dir.join("result.md")).unwrap_or_default();
    let mut files: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_file())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| !META_FILES.contains(&n.as_str()))
        .collect();
    files.sort();
    Details { summary, files }
}

/// Resolves a file name from the webview to a file directly inside the task
/// folder. Rejects anything that could escape it (separators, "..", symlinks out).
pub fn task_file(dir: &Path, name: &str) -> Option<PathBuf> {
    if name.is_empty() || name.contains(['/', '\\']) || name == ".." || name == "." {
        return None;
    }
    let path = dir.join(name).canonicalize().ok()?;
    (path.parent()? == dir.canonicalize().ok()?.as_path() && path.is_file()).then_some(path)
}

fn start_ready(app: &AppHandle) {
    let tasks = app.state::<Tasks>();
    let mut inner = tasks.inner.lock().unwrap();
    while inner.running < MAX_RUNNING {
        let Some(job) = inner.queue.pop_front() else { break };
        inner.running += 1;
        let app = app.clone();
        std::thread::spawn(move || {
            run(&app, &job);
            app.state::<Tasks>().inner.lock().unwrap().running -= 1;
            start_ready(&app);
        });
    }
}

// ponytail: no timeout or cancel yet; a hung CLI holds its slot until it exits. Add with the task list UI (M3).
fn run(app: &AppHandle, job: &Job) {
    emit(app, job.id, &job.title, "running", Some("Starting".into()), None);
    let fail = |msg: String| {
        eprintln!("[otto] task {} failed: {msg}", job.id);
        emit(app, job.id, &job.title, "failed", None, Some(msg));
    };

    let Some(claude) = claude_bin() else {
        return fail("Claude Code isn't installed (or set OTTO_CLAUDE to its path).".into());
    };
    let gate = app.state::<Arc<Gate>>().inner().clone();
    gate.register(job.id, &job.dir, &job.title);
    let child = claude_command(&claude, &job.prompt, &job.dir, &gate.url(job.id))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) => {
            gate.unregister(job.id);
            return fail(format!("Couldn't start Claude Code: {e}"));
        }
    };

    let mut stderr = child.stderr.take().unwrap();
    let stderr_reader = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stderr.read_to_string(&mut s);
        s
    });

    let mut log = std::fs::File::create(job.dir.join("log.jsonl")).ok();
    let mut outcome: Option<(bool, String)> = None;
    for line in BufReader::new(child.stdout.take().unwrap()).lines().map_while(Result::ok) {
        if let Some(f) = log.as_mut() {
            let _ = writeln!(f, "{line}");
        }
        let Ok(ev) = serde_json::from_str::<Value>(&line) else { continue };
        match ev["type"].as_str() {
            Some("assistant") => {
                for c in ev["message"]["content"].as_array().into_iter().flatten() {
                    if c["type"] == "tool_use" {
                        let name = c["name"].as_str().unwrap_or("tool");
                        gate.observe(job.id, c["id"].as_str().unwrap_or(""), name, &c["input"]);
                        let step = describe_tool(name, &c["input"]);
                        emit(app, job.id, &job.title, "running", Some(step), None);
                    }
                }
            }
            Some("result") => {
                let ok = ev["subtype"] == "success" && ev["is_error"] != true;
                outcome = Some((ok, ev["result"].as_str().unwrap_or_default().trim().to_string()));
            }
            _ => {}
        }
    }
    let status = child.wait();
    let stderr = stderr_reader.join().unwrap_or_default();
    gate.unregister(job.id);

    match outcome {
        Some((true, summary)) => {
            let _ = std::fs::write(job.dir.join("result.md"), &summary);
            println!("[otto] task {} done: {}", job.id, job.title);
            emit(app, job.id, &job.title, "done", None, Some(summary));
        }
        Some((false, msg)) if !msg.is_empty() => fail(msg),
        _ => {
            let tail = stderr.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
            let code = status.map(|s| s.to_string()).unwrap_or_else(|e| e.to_string());
            fail(if tail.is_empty() { format!("Claude Code exited ({code})") } else { tail.to_string() })
        }
    }
}

/// Tools Claude Code may use. Anything it would normally ask about goes to Otto's gate.
const TOOLS: &str = "Read,Write,Edit,Glob,Grep,Bash,WebSearch,WebFetch,TodoWrite";

/// Folders Otto may read without asking (still taints the task for web access).
fn readable_dirs() -> Vec<PathBuf> {
    let home = dirs::home_dir().unwrap_or_default();
    ["Documents", "Desktop", "Downloads"].iter().map(|d| home.join(d)).filter(|p| p.is_dir()).collect()
}

/// `--restricted` ignores the user's own Claude settings, hooks and allow-rules,
/// so nothing bypasses the gate. `--strict-mcp-config` keeps the tool set fixed:
/// connectors come later with an explicit per-tool allowlist.
pub(crate) fn claude_command(claude: &Path, prompt: &str, dir: &Path, gate_url: &str) -> Command {
    let mcp = serde_json::json!({ "mcpServers": { "otto": { "type": "http", "url": gate_url } } });
    let mut cmd = Command::new(claude);
    cmd.arg("-p")
        .arg(prompt)
        .args(["--output-format", "stream-json", "--verbose"])
        .args(["--restricted", "--strict-mcp-config", "--tools", TOOLS])
        .arg("--mcp-config")
        .arg(mcp.to_string())
        .args(["--permission-prompt-tool", "mcp__otto__approve"])
        .args(["--append-system-prompt", SYSTEM_PROMPT])
        .current_dir(dir)
        // Approvals can take minutes; don't let Claude Code time out the gate call first.
        .env("MCP_TOOL_TIMEOUT", (gate::APPROVAL_TIMEOUT.as_millis() + 60_000).to_string());
    for d in readable_dirs() {
        cmd.arg("--add-dir").arg(d);
    }
    cmd
}

fn describe_tool(name: &str, input: &Value) -> String {
    let file = || {
        input["file_path"]
            .as_str()
            .and_then(|p| Path::new(p).file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    };
    match name {
        "Write" => format!("Writing {}", file()),
        "Edit" | "MultiEdit" => format!("Editing {}", file()),
        "Read" => format!("Reading {}", file()),
        "Glob" | "Grep" | "LS" => "Searching files".into(),
        "WebSearch" => "Searching the web".into(),
        "WebFetch" => "Reading a web page".into(),
        "TodoWrite" => "Planning".into(),
        other => format!("Using {other}"),
    }
}

/// GUI apps don't inherit the shell PATH, so look in the usual places.
pub(crate) fn claude_bin() -> Option<PathBuf> {
    static BIN: OnceLock<Option<PathBuf>> = OnceLock::new();
    BIN.get_or_init(|| {
        if let Some(p) = std::env::var_os("OTTO_CLAUDE").map(PathBuf::from).filter(|p| p.is_file()) {
            return Some(p);
        }
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
        if let Ok(out) = Command::new(shell).args(["-lc", "command -v claude"]).output() {
            let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if p.starts_with('/') && Path::new(&p).is_file() {
                return Some(p.into());
            }
        }
        let home = dirs::home_dir().unwrap_or_default();
        [home.join(".local/bin/claude"), "/opt/homebrew/bin/claude".into(), "/usr/local/bin/claude".into()]
            .into_iter()
            .find(|p| p.is_file())
    })
    .clone()
}

pub fn tasks_root() -> PathBuf {
    dirs::home_dir().unwrap_or_default().join("Otto/tasks")
}

fn new_task_dir(root: &Path, transcript: &str) -> Result<PathBuf, String> {
    std::fs::create_dir_all(root).map_err(|e| format!("could not create {}: {e}", root.display()))?;
    let base = format!("{}-{}", utc_date(unix_now()), slug(transcript));
    for n in 1.. {
        let dir = root.join(if n == 1 { base.clone() } else { format!("{base}-{n}") });
        match std::fs::create_dir(&dir) {
            Ok(()) => return Ok(dir),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("could not create task folder: {e}")),
        }
    }
    unreachable!()
}

fn slug(text: &str) -> String {
    let s: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .take(6)
        .map(str::to_lowercase)
        .collect();
    if s.is_empty() { "task".into() } else { s.join("-").chars().take(48).collect() }
}

fn title_of(text: &str) -> String {
    let t = text.trim().trim_end_matches(['.', '!', '?']);
    if t.chars().count() <= 48 {
        t.to_string()
    } else {
        format!("{}…", t.chars().take(47).collect::<String>().trim_end())
    }
}

fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

// ponytail: UTC date for folder names (no tz dependency); near midnight it can show tomorrow's date.
fn utc_date(secs: u64) -> String {
    // Howard Hinnant's civil_from_days
    let z = (secs / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn naming() {
        assert_eq!(utc_date(0), "1970-01-01");
        assert_eq!(utc_date(1_793_750_400), "2026-11-04");
        assert_eq!(slug("Make a 5-slide deck on Q3 hiring, please!"), "make-a-5-slide-deck-on");
        assert_eq!(slug("?!"), "task");
        assert_eq!(title_of("Summarize the lease."), "Summarize the lease");
    }

    #[test]
    fn task_file_stays_inside_folder() {
        let root = std::env::temp_dir().join(format!("otto-file-{}", std::process::id()));
        std::fs::create_dir_all(root.join("t")).unwrap();
        std::fs::write(root.join("t/deck.md"), "x").unwrap();
        std::fs::write(root.join("secret.txt"), "x").unwrap();
        let dir = root.join("t");
        assert!(task_file(&dir, "deck.md").is_some());
        assert!(task_file(&dir, "../secret.txt").is_none());
        assert!(task_file(&dir, "..").is_none());
        assert!(task_file(&dir, "missing.md").is_none());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn task_dirs_never_collide() {
        let root = std::env::temp_dir().join(format!("otto-test-{}", std::process::id()));
        let a = new_task_dir(&root, "same words").unwrap();
        let b = new_task_dir(&root, "same words").unwrap();
        assert_ne!(a, b);
        assert!(b.to_string_lossy().ends_with("-2"));
        let _ = std::fs::remove_dir_all(root);
    }
}
