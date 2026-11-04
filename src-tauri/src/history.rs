//! The History window: every past task, read straight from ~/Otto/tasks.

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Manager};

use crate::tasks;

const INPUT_MAX: usize = 300;

/// Same dance as onboarding: an Accessory app must un-hide itself first.
pub fn show(app: &AppHandle) {
    let Some(win) = app.get_webview_window("history") else { return };
    #[cfg(target_os = "macos")]
    let _ = app.show();
    let _ = win.show();
    let _ = win.set_focus();
}

#[derive(Serialize)]
pub struct Entry {
    folder: String,
    created_unix: u64,
    transcript: String,
    status: &'static str, // done | failed | incomplete
    preview: String,
}

#[derive(Serialize)]
pub struct Detail {
    transcript: String,
    created_unix: u64,
    status: &'static str,
    result: String,
    /// The thread in order: what you said, each step and its output, notes, results.
    trace: Vec<Value>,
    files: Vec<String>,
}

/// Resolves a folder name from the webview to a task folder directly under
/// `root`. Rejects separators, "." / "..", and symlinks that lead elsewhere.
fn task_dir(root: &Path, folder: &str) -> Option<PathBuf> {
    if folder.is_empty() || folder.contains(['/', '\\']) || folder == "." || folder == ".." {
        return None;
    }
    let dir = root.join(folder).canonicalize().ok()?;
    (dir.parent()? == root.canonicalize().ok()?.as_path() && dir.is_dir()).then_some(dir)
}

fn dir_of(folder: &str) -> Result<PathBuf, String> {
    task_dir(&tasks::tasks_root(), folder).ok_or_else(|| "not a task folder".into())
}

fn meta(dir: &Path) -> Option<(String, u64)> {
    let v: Value = serde_json::from_slice(&std::fs::read(dir.join("task.json")).ok()?).ok()?;
    // Follow-ups show under the original request, one per line.
    let mut said = v["transcript"].as_str()?.to_string();
    for f in v["followups"].as_array().into_iter().flatten() {
        if let Some(t) = f["transcript"].as_str() {
            said.push_str("\n\u{21b3} ");
            said.push_str(t);
        }
    }
    Some((said, v["created_unix"].as_u64().unwrap_or(0)))
}

/// Status plus the result text: result.md on success, else the log's error.
fn outcome(dir: &Path) -> (&'static str, String) {
    if let Ok(md) = std::fs::read_to_string(dir.join("result.md")) {
        return ("done", md);
    }
    let log = std::fs::read_to_string(dir.join("log.jsonl")).unwrap_or_default();
    let result = log
        .lines()
        .rev()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .find(|v| v["type"] == "result");
    match result {
        Some(v) if v["is_error"] == true || v["subtype"] != "success" => {
            let msg = v["result"].as_str().map(str::trim).filter(|s| !s.is_empty());
            ("failed", msg.or(v["subtype"].as_str()).unwrap_or("Failed").to_string())
        }
        _ => ("incomplete", String::new()),
    }
}

fn entries(root: &Path, active: &std::collections::HashSet<PathBuf>) -> Vec<Entry> {
    let mut out: Vec<Entry> = std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| {
            let dir = e.path();
            let (transcript, created_unix) = meta(&dir)?;
            let (status, text) = if active.contains(&dir) { ("running", String::new()) } else { outcome(&dir) };
            let preview = if status == "done" { tasks::preview(&text) } else { text };
            Some(Entry { folder: e.file_name().to_string_lossy().into_owned(), created_unix, transcript, status, preview })
        })
        .collect();
    out.sort_by(|a, b| b.created_unix.cmp(&a.created_unix).then_with(|| b.folder.cmp(&a.folder)));
    out
}

const OUTPUT_MAX: usize = 2_000;

fn clip(s: &str, max: usize) -> String {
    if s.chars().count() > max { s.chars().take(max).collect::<String>() + "\u{2026}" } else { s.to_string() }
}

/// The one input field worth showing for a step (the command, path, query...).
fn key_input(input: &Value) -> String {
    for k in ["command", "file_path", "path", "pattern", "query", "url", "fact", "prompt"] {
        if let Some(v) = input[k].as_str() {
            return clip(v, INPUT_MAX);
        }
    }
    if input.as_object().is_some_and(|o| o.is_empty()) { String::new() } else { clip(&input.to_string(), INPUT_MAX) }
}

struct Decision {
    tool: String,
    input: String,
    decision: String,
    by: String,
    used: bool,
}

/// Gate decisions for this task. Lines from before the folder tag are matched by tool + input.
fn decisions(audit: &Path, folder: &str) -> Vec<Decision> {
    std::fs::read_to_string(audit)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|v| v["folder"].is_null() || v["folder"].as_str() == Some(folder))
        .map(|v| {
            let s = |k: &str| v[k].as_str().unwrap_or_default().to_string();
            Decision { tool: s("tool"), input: s("input"), decision: s("decision"), by: s("by"), used: false }
        })
        .collect()
}

/// The gate logs `input.to_string()` (sorted keys, cut at 4000 chars), so the same serialization matches.
fn take_decision(ds: &mut [Decision], tool: &str, input: &Value) -> Option<Value> {
    let want = input.to_string();
    let d = ds.iter_mut().find(|d| {
        !d.used && d.tool == tool && (d.input == want || d.input.strip_suffix('\u{2026}').is_some_and(|p| want.starts_with(p)))
    })?;
    d.used = true;
    Some(serde_json::json!({ "decision": d.decision, "by": d.by }))
}

fn tool_output(c: &Value) -> String {
    let text = match &c["content"] {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts.iter().filter_map(|p| p["text"].as_str()).collect::<Vec<_>>().join("\n"),
        _ => String::new(),
    };
    clip(text.trim(), OUTPUT_MAX)
}

/// Replays log.jsonl (one Claude run per request, follow-ups appended) into a trace.
fn trace(dir: &Path, audit: &Path, folder: &str) -> Vec<Value> {
    use serde_json::json;
    let meta: Value = std::fs::read(dir.join("task.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let mut said = vec![(meta["transcript"].as_str().unwrap_or("").to_string(), meta["created_unix"].as_u64().unwrap_or(0))];
    for f in meta["followups"].as_array().into_iter().flatten() {
        said.push((f["transcript"].as_str().unwrap_or("").to_string(), f["created_unix"].as_u64().unwrap_or(0)));
    }
    let mut ds = decisions(audit, folder);
    let mut items: Vec<Value> = Vec::new();
    let mut by_id: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut runs = 0;
    let log = std::fs::read_to_string(dir.join("log.jsonl")).unwrap_or_default();
    for ev in log.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()) {
        let ts = ev["timestamp"].as_str().unwrap_or("");
        match (ev["type"].as_str(), ev["subtype"].as_str()) {
            (Some("system"), Some("init")) => {
                if let Some((text, at)) = said.get(runs) {
                    items.push(json!({ "kind": "said", "text": text, "unix": at, "follow_up": runs > 0 }));
                }
                runs += 1;
            }
            (Some("assistant"), _) => {
                for c in ev["message"]["content"].as_array().into_iter().flatten() {
                    match c["type"].as_str() {
                        Some("text") if !c["text"].as_str().unwrap_or("").trim().is_empty() => {
                            items.push(json!({ "kind": "note", "text": c["text"].as_str().unwrap_or("").trim(), "ts": ts }));
                        }
                        Some("tool_use") => {
                            let tool = c["name"].as_str().unwrap_or("tool");
                            let input = &c["input"];
                            by_id.insert(c["id"].as_str().unwrap_or("").to_string(), items.len());
                            items.push(json!({
                                "kind": "tool", "tool": tool, "label": tasks::describe_tool(tool, input),
                                "detail": key_input(input), "approval": take_decision(&mut ds, tool, input),
                                "output": Value::Null, "error": false, "ts": ts,
                            }));
                        }
                        _ => {}
                    }
                }
            }
            (Some("user"), _) => {
                for c in ev["message"]["content"].as_array().into_iter().flatten().filter(|c| c["type"] == "tool_result") {
                    if let Some(&i) = c["tool_use_id"].as_str().and_then(|id| by_id.get(id)) {
                        items[i]["output"] = tool_output(c).into();
                        items[i]["error"] = (c["is_error"] == true).into();
                    }
                }
            }
            (Some("result"), _) => {
                let text = ev["result"].as_str().unwrap_or("").trim().to_string();
                // The closing message is the result itself; don't show it twice.
                if items.last().is_some_and(|l| l["kind"] == "note" && l["text"].as_str() == Some(text.as_str())) {
                    items.pop();
                }
                let ok = ev["subtype"] == "success" && ev["is_error"] != true;
                items.push(json!({ "kind": "result", "ok": ok, "text": text, "duration_ms": ev["duration_ms"] }));
            }
            _ => {}
        }
    }
    if runs == 0 {
        // Never started (or no log): still show what was said.
        for (i, (text, at)) in said.iter().enumerate() {
            items.push(json!({ "kind": "said", "text": text, "unix": at, "follow_up": i > 0 }));
        }
    }
    items
}

fn audit_path() -> PathBuf {
    dirs::home_dir().unwrap_or_default().join("Otto/audit.jsonl")
}

// Async so the disk reads run off the main thread.
#[tauri::command]
pub async fn list_history(tasks: tauri::State<'_, tasks::Tasks>) -> Result<Vec<Entry>, String> {
    Ok(entries(&tasks::tasks_root(), &tasks.active_dirs()))
}

#[tauri::command]
pub async fn history_detail(tasks: tauri::State<'_, tasks::Tasks>, folder: String) -> Result<Detail, String> {
    let dir = dir_of(&folder)?;
    let (transcript, created_unix) = meta(&dir).ok_or("task.json is missing or unreadable")?;
    let (status, result) = if tasks.active_dirs().contains(&dir) { ("running", String::new()) } else { outcome(&dir) };
    Ok(Detail {
        transcript,
        created_unix,
        status,
        result,
        trace: trace(&dir, &audit_path(), &folder),
        files: tasks::details(&dir).files,
    })
}

#[tauri::command]
pub fn open_history_file(folder: String, name: String) -> Result<(), String> {
    let file = tasks::task_file(&dir_of(&folder)?, &name).ok_or("not a file in this task")?;
    crate::open(&[file.as_os_str()])
}

#[tauri::command]
pub fn show_history_folder(folder: String) -> Result<(), String> {
    crate::open(&[dir_of(&folder)?.as_os_str()])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trace_replays_the_thread_in_order() {
        let d = std::env::temp_dir().join(format!("otto-trace-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("task.json"), r#"{"transcript":"run ls","created_unix":1,"followups":[{"transcript":"no, ls -a","created_unix":2}]}"#).unwrap();
        let log = [
            r#"{"type":"system","subtype":"init"}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Listing."},{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"ls"}}]}}"#,
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"a.txt"}]}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Done: a.txt"}]}}"#,
            r#"{"type":"result","subtype":"success","result":"Done: a.txt"}"#,
            r#"{"type":"system","subtype":"init"}"#,
            r#"{"type":"result","subtype":"success","result":"Hidden files too."}"#,
        ];
        std::fs::write(d.join("log.jsonl"), log.join("\n")).unwrap();
        std::fs::write(d.join("audit.jsonl"), r#"{"tool":"Bash","input":"{\"command\":\"ls\"}","decision":"allow","by":"user"}"#).unwrap();
        let t = trace(&d, &d.join("audit.jsonl"), "x");
        let kinds: Vec<&str> = t.iter().map(|i| i["kind"].as_str().unwrap()).collect();
        assert_eq!(kinds, ["said", "note", "tool", "result", "said", "result"], "closing note folded into result");
        assert_eq!(t[2]["output"], "a.txt");
        assert_eq!(t[2]["approval"]["by"], "user");
        assert_eq!(t[4]["follow_up"], true);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn folder_must_be_a_direct_child() {
        let root = std::env::temp_dir().join(format!("otto-history-{}", std::process::id()));
        std::fs::create_dir_all(root.join("tasks/2026-11-03-a")).unwrap();
        std::fs::create_dir_all(root.join("outside")).unwrap();
        let tasks = root.join("tasks");
        assert!(task_dir(&tasks, "2026-11-03-a").is_some());
        for bad in ["", ".", "..", "../outside", "2026-11-03-a/..", "a\\b", "missing"] {
            assert!(task_dir(&tasks, bad).is_none(), "{bad:?} should be rejected");
        }
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(root.join("outside"), tasks.join("link")).unwrap();
            assert!(task_dir(&tasks, "link").is_none());
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn status_from_files() {
        let root = std::env::temp_dir().join(format!("otto-status-{}", std::process::id()));
        let (done, failed, partial) = (root.join("done"), root.join("failed"), root.join("partial"));
        for d in [&done, &failed, &partial] {
            std::fs::create_dir_all(d).unwrap();
            std::fs::write(d.join("task.json"), r#"{"transcript":"hi","created_unix":1}"#).unwrap();
        }
        std::fs::write(done.join("result.md"), "All set.").unwrap();
        std::fs::write(
            failed.join("log.jsonl"),
            "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"tool_use\",\"name\":\"Write\",\"input\":{\"file_path\":\"/x/a.md\"}}]}}\n\
             {\"type\":\"result\",\"subtype\":\"success\",\"is_error\":true,\"result\":\"Out of credits\"}\n",
        )
        .unwrap();
        std::fs::write(partial.join("log.jsonl"), "{\"type\":\"system\"}\n").unwrap();
        assert_eq!(outcome(&done), ("done", "All set.".into()));
        assert_eq!(outcome(&failed), ("failed", "Out of credits".into()));
        assert_eq!(outcome(&partial).0, "incomplete");
        assert_eq!(entries(&root, &Default::default()).len(), 3);
        let busy = entries(&root, &[partial.clone()].into_iter().collect());
        assert!(busy.iter().any(|e| e.status == "running"), "an active folder shows as running");
        let _ = std::fs::remove_dir_all(root);
    }
}

