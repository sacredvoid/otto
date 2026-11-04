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
pub struct Step {
    label: String,
    tool: String,
}

#[derive(Serialize)]
pub struct Approval {
    ts: u64,
    tool: String,
    decision: String,
    by: String,
    input: String,
}

#[derive(Serialize)]
pub struct Detail {
    transcript: String,
    created_unix: u64,
    status: &'static str,
    result: String,
    steps: Vec<Step>,
    approvals: Vec<Approval>,
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
    Some((v["transcript"].as_str()?.to_string(), v["created_unix"].as_u64().unwrap_or(0)))
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

fn entries(root: &Path) -> Vec<Entry> {
    let mut out: Vec<Entry> = std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| {
            let dir = e.path();
            let (transcript, created_unix) = meta(&dir)?;
            let (status, text) = outcome(&dir);
            let preview = if status == "done" { tasks::preview(&text) } else { text };
            Some(Entry { folder: e.file_name().to_string_lossy().into_owned(), created_unix, transcript, status, preview })
        })
        .collect();
    out.sort_by(|a, b| b.created_unix.cmp(&a.created_unix).then_with(|| b.folder.cmp(&a.folder)));
    out
}

fn steps(dir: &Path) -> Vec<Step> {
    let log = std::fs::read_to_string(dir.join("log.jsonl")).unwrap_or_default();
    log.lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|v| v["type"] == "assistant")
        .flat_map(|v| v["message"]["content"].as_array().cloned().unwrap_or_default())
        .filter(|c| c["type"] == "tool_use")
        .map(|c| {
            let tool = c["name"].as_str().unwrap_or("tool").to_string();
            Step { label: tasks::describe_tool(&tool, &c["input"]), tool }
        })
        .collect()
}

/// Audit lines tagged with this task's folder. Older lines have no folder and are skipped.
fn approvals(audit: &Path, folder: &str) -> Vec<Approval> {
    let text = std::fs::read_to_string(audit).unwrap_or_default();
    text.lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|v| v["folder"].as_str() == Some(folder))
        .map(|v| {
            let s = |k: &str| v[k].as_str().unwrap_or_default().to_string();
            let input = match &v["input"] {
                Value::String(s) => s.clone(),
                Value::Null => String::new(),
                other => other.to_string(),
            };
            let input = if input.chars().count() > INPUT_MAX {
                input.chars().take(INPUT_MAX).collect::<String>() + "\u{2026}"
            } else {
                input
            };
            Approval { ts: v["ts"].as_u64().unwrap_or(0), tool: s("tool"), decision: s("decision"), by: s("by"), input }
        })
        .collect()
}

fn audit_path() -> PathBuf {
    dirs::home_dir().unwrap_or_default().join("Otto/audit.jsonl")
}

// Async so the disk reads run off the main thread.
#[tauri::command]
pub async fn list_history() -> Vec<Entry> {
    entries(&tasks::tasks_root())
}

#[tauri::command]
pub async fn history_detail(folder: String) -> Result<Detail, String> {
    let dir = dir_of(&folder)?;
    let (transcript, created_unix) = meta(&dir).ok_or("task.json is missing or unreadable")?;
    let (status, result) = outcome(&dir);
    Ok(Detail {
        transcript,
        created_unix,
        status,
        result,
        steps: steps(&dir),
        approvals: approvals(&audit_path(), &folder),
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
        assert_eq!(steps(&failed)[0].label, "Writing a.md");
        assert_eq!(entries(&root).len(), 3);
        let _ = std::fs::remove_dir_all(root);
    }
}
