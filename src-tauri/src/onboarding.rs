//! First-launch onboarding: window control, live checks, and the config write.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use serde::Serialize;
use serde_json::{Map, Value, json};
use tauri::{AppHandle, Manager};

fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_default()
}

fn config_path() -> PathBuf {
    home().join("Otto/config.json")
}

fn read_config() -> Map<String, Value> {
    std::fs::read(config_path())
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn is_onboarded() -> bool {
    read_config().get("onboarded") == Some(&Value::Bool(true))
}

/// Accessory apps have no Dock icon, so the app must be un-hidden and the
/// window focused explicitly or it opens behind everything.
pub fn show_onboarding(app: &AppHandle) {
    let Some(win) = app.get_webview_window("onboarding") else { return };
    #[cfg(target_os = "macos")]
    let _ = app.show();
    let _ = win.center();
    let _ = win.show();
    let _ = win.set_focus();
}

#[derive(Serialize)]
pub struct Info {
    /// TODO: real download. For now "ready" just means the file exists.
    model_ready: bool,
    folders: Vec<String>,
    /// Current "disabled" list, so reopening setup shows the saved toggles.
    disabled: Vec<String>,
    /// Plugins named in config.json ("plugins").
    plugins: Vec<String>,
}

#[tauri::command]
pub fn onboarding_info() -> Info {
    let home = home();
    let mut folders: Vec<String> = ["Documents", "Desktop", "Downloads"]
        .iter()
        .filter(|n| home.join(n).is_dir())
        .map(|n| format!("~/{n}"))
        .collect();
    let mut extra: Vec<String> = std::fs::read_dir(&home)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.starts_with("Documents-") && home.join(n).is_dir())
        .map(|n| format!("~/{n}"))
        .collect();
    extra.sort();
    folders.extend(extra);
    let config = crate::tasks::config();
    let list = |k: &str| -> Vec<String> {
        config[k].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect()).unwrap_or_default()
    };
    Info { model_ready: crate::stt::default_model_path().is_file(), folders, disabled: list("disabled"), plugins: list("plugins") }
}

#[derive(Serialize)]
pub struct Detected {
    claude: bool,
    codex: bool,
    gemini: bool,
    ollama: bool,
}

/// Runs on the async pool: a login shell and a socket probe can take a moment.
#[tauri::command]
pub async fn onboarding_detect() -> Detected {
    Detected {
        claude: crate::tasks::claude_bin().is_some(),
        codex: cli_exists("codex"),
        gemini: cli_exists("gemini"),
        ollama: ollama_up(),
    }
}

/// Same lookup order as `tasks::claude_bin`: login shell, then the usual dirs.
fn cli_exists(name: &str) -> bool {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
    if let Ok(out) = Command::new(shell).args(["-lc", &format!("command -v {name}")]).output() {
        let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if p.starts_with('/') && Path::new(&p).is_file() {
            return true;
        }
    }
    [home().join(".local/bin"), "/opt/homebrew/bin".into(), "/usr/local/bin".into()]
        .iter()
        .any(|d| d.join(name).is_file())
}

fn ollama_up() -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], 11434));
    let t = Duration::from_secs(1);
    let Ok(mut s) = TcpStream::connect_timeout(&addr, t) else { return false };
    let _ = s.set_read_timeout(Some(t));
    let _ = s.set_write_timeout(Some(t));
    if s.write_all(b"GET /api/tags HTTP/1.0\r\nHost: 127.0.0.1\r\n\r\n").is_err() {
        return false;
    }
    let mut head = [0u8; 13]; // "HTTP/1.0 200 "
    s.read_exact(&mut head).is_ok() && head[8..] == *b" 200 "
}

/// "granted" | "denied" | "undetermined"
#[tauri::command]
pub fn mic_status() -> &'static str {
    #[cfg(target_os = "macos")]
    return mac::mic_status();
    #[cfg(not(target_os = "macos"))]
    "granted"
}

/// First ask: opening the input stream makes macOS show its own prompt.
/// Already decided: only System Settings can change it, so send them there.
#[tauri::command]
pub fn request_mic() {
    if mic_status() == "undetermined" {
        std::thread::spawn(|| {
            if let Ok(r) = crate::audio::Recording::start(|_| {}) {
                std::thread::sleep(Duration::from_millis(200));
                r.stop();
            }
        });
    } else {
        open_settings("microphone".into());
    }
}

#[tauri::command]
pub fn open_settings(pane: String) {
    let url = match pane.as_str() {
        "microphone" => "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone",
        "notifications" => "x-apple.systempreferences:com.apple.Notifications-Settings.extension",
        _ => return,
    };
    let _ = Command::new("open").arg(url).spawn();
}

/// A connector or plugin server Claude Code loaded for this account.
#[derive(Serialize)]
pub struct Server {
    /// Tool prefix, e.g. "mcp__claude_ai_Gmail". Switching it off denies every tool under it.
    key: String,
    name: String,
    tools: Vec<String>,
}

/// Starts Claude Code the way a task would and reads its first event, which lists the
/// connectors it loaded. Killed right after, before any request reaches the model.
#[tauri::command]
pub async fn detect_connectors() -> Result<Vec<Server>, String> {
    use std::io::BufRead;
    let claude = crate::tasks::claude_bin().ok_or("Claude Code isn't installed")?;
    let config = crate::tasks::config();
    let plugins: Map<String, Value> = config["plugins"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| p.as_str())
        .map(|p| (p.to_string(), Value::Bool(true)))
        .collect();
    let mut child = Command::new(claude)
        .args(["-p", "ok", "--output-format", "stream-json", "--verbose", "--restricted", "--tools", "Read"])
        .arg("--settings")
        .arg(json!({ "enabledPlugins": plugins }).to_string())
        .current_dir(std::env::temp_dir())
        .env("MCP_CONNECTION_NONBLOCKING", "false")
        .env_remove("CLAUDE_CODE_PLUGIN_DIRS")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("couldn't start Claude Code: {e}"))?;
    let stdout = child.stdout.take().ok_or("no output")?;
    // ponytail: no timeout of our own; init arrives in a few seconds or the CLI exits on error.
    let init = std::io::BufReader::new(stdout)
        .lines()
        .map_while(Result::ok)
        .filter_map(|l| serde_json::from_str::<Value>(&l).ok())
        .find(|v| v["subtype"] == "init");
    let _ = child.kill();
    let _ = child.wait();
    let init = init.ok_or("Claude Code didn't start (is it logged in?)")?;
    let mut servers: Vec<Server> = Vec::new();
    for tool in init["tools"].as_array().into_iter().flatten().filter_map(|t| t.as_str()) {
        let Some(rest) = tool.strip_prefix("mcp__") else { continue };
        let Some((server, name)) = rest.split_once("__") else { continue };
        let key = format!("mcp__{server}");
        match servers.iter_mut().find(|s| s.key == key) {
            Some(s) => s.tools.push(name.replace('_', " ")),
            None => {
                let label = server.strip_prefix("claude_ai_").unwrap_or(server).replace('_', " ");
                servers.push(Server { key, name: label, tools: vec![name.replace('_', " ")] });
            }
        }
    }
    // Plugin servers are switched off as plugins (see the Plugins list), not here.
    servers.retain(|s| s.key.starts_with("mcp__claude_ai_"));
    Ok(servers)
}

/// Saves the choices, keeping any other keys, then closes the window.
#[tauri::command]
pub fn finish_onboarding(app: AppHandle, readable_dirs: Vec<String>, disabled: Vec<String>) -> Result<(), String> {
    let home = home();
    // Only "~/Name" folders that exist; the webview is not trusted to name paths.
    let dirs: Vec<&String> = readable_dirs
        .iter()
        .filter(|d| {
            d.strip_prefix("~/")
                .is_some_and(|r| !r.is_empty() && !r.split('/').any(|p| p.is_empty() || p == "..") && home.join(r).is_dir())
        })
        .collect();
    // Tool names, server prefixes or "plugin:name"; nothing that could be a flag or a path.
    let ok = |d: &&String| {
        !d.is_empty() && d.len() <= 200 && !d.starts_with('-')
            && d.chars().all(|c| c.is_ascii_alphanumeric() || "_-:@.".contains(c))
    };
    let disabled: Vec<&String> = disabled.iter().filter(ok).take(200).collect();
    let mut config = read_config();
    config.insert("readable_dirs".into(), json!(dirs));
    config.insert("disabled".into(), json!(disabled));
    config.insert("onboarded".into(), json!(true));
    let path = config_path();
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    let body = serde_json::to_vec_pretty(&config).map_err(|e| e.to_string())?;
    std::fs::write(&tmp, body)
        .and_then(|_| std::fs::rename(&tmp, &path))
        .map_err(|e| format!("could not save settings: {e}"))?;
    if let Some(w) = app.get_webview_window("onboarding") {
        let _ = w.hide();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn mic_status_is_known() {
        assert!(["granted", "denied", "undetermined"].contains(&super::mic_status()));
    }

    #[test]
    fn ollama_probe_does_not_hang() {
        let _ = super::ollama_up();
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use objc2::msg_send;
    use objc2::runtime::{AnyClass, AnyObject};

    #[link(name = "AVFoundation", kind = "framework")]
    unsafe extern "C" {}

    pub fn mic_status() -> &'static str {
        let (Some(device), Some(string)) = (AnyClass::get(c"AVCaptureDevice"), AnyClass::get(c"NSString")) else {
            return "undetermined";
        };
        // AVMediaTypeAudio is the string "soun".
        let status: isize = unsafe {
            let media: *mut AnyObject = msg_send![string, stringWithUTF8String: c"soun".as_ptr()];
            msg_send![device, authorizationStatusForMediaType: media]
        };
        match status {
            3 => "granted",
            1 | 2 => "denied",
            _ => "undetermined",
        }
    }
}
