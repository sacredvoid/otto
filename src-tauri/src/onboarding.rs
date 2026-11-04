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

/// Sets one key in ~/Otto/config.json, keeping the rest. Atomic: write, then rename.
pub fn set_config(key: &str, value: Value) {
    let mut config = read_config();
    config.insert(key.into(), value);
    if let Err(e) = write_config(&config) {
        eprintln!("[otto] {e}");
    }
}

fn write_config(config: &Map<String, Value>) -> Result<(), String> {
    let path = config_path();
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    let body = serde_json::to_vec_pretty(config).map_err(|e| e.to_string())?;
    std::fs::write(&tmp, body)
        .and_then(|_| std::fs::rename(&tmp, &path))
        .map_err(|e| format!("could not save settings: {e}"))
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
    /// Memory saves ask every time ("ask_memory").
    ask_memory: bool,
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
    Info { model_ready: crate::stt::default_model_path().is_file(), folders, disabled: list("disabled"), plugins: list("plugins"), ask_memory: config["ask_memory"] == true }
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
        // Ask through AVFoundation, the same API mic_status reads. Prompting by
        // opening a CoreAudio stream left that status stuck on "undetermined".
        #[cfg(target_os = "macos")]
        mac::request_mic();
    } else {
        open_settings("microphone".into());
    }
}

#[tauri::command]
pub fn open_settings(pane: String) {
    let url = match pane.as_str() {
        "microphone" => "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone",
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
    let plugins = crate::tasks::strings(&config["plugins"]);
    let mut cmd = crate::tasks::claude_cli(&claude);
    for d in crate::tasks::plugin_dirs(&plugins) {
        cmd.arg("--plugin-dir").arg(d);
    }
    let mut child = cmd
        .args(["-p", "ok", "--output-format", "stream-json", "--verbose", "--restricted", "--tools", "Read"])
        .current_dir(std::env::temp_dir())
        .env("MCP_CONNECTION_NONBLOCKING", "false")
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
    let init = init.ok_or("Claude Code didn't start. Sign in on the previous step.")?;
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
pub fn finish_onboarding(app: AppHandle, readable_dirs: Vec<String>, disabled: Vec<String>, ask_memory: bool) -> Result<(), String> {
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
    config.insert("ask_memory".into(), json!(ask_memory));
    config.insert("onboarded".into(), json!(true));
    write_config(&config)?;
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

    /// AVCaptureDevice and AVMediaTypeAudio (the string "soun").
    fn device_and_audio() -> Option<(&'static AnyClass, *mut AnyObject)> {
        let device = AnyClass::get(c"AVCaptureDevice")?;
        let string = AnyClass::get(c"NSString")?;
        let media: *mut AnyObject = unsafe { msg_send![string, stringWithUTF8String: c"soun".as_ptr()] };
        Some((device, media))
    }

    pub fn request_mic() {
        let Some((device, media)) = device_and_audio() else { return };
        let done = block2::RcBlock::new(|_granted: objc2::runtime::Bool| {});
        unsafe {
            let _: () = msg_send![device, requestAccessForMediaType: media, completionHandler: &*done];
        }
    }

    pub fn mic_status() -> &'static str {
        let Some((device, media)) = device_and_audio() else { return "undetermined" };
        let status: isize = unsafe { msg_send![device, authorizationStatusForMediaType: media] };
        match status {
            3 => "granted",
            1 | 2 => "denied",
            _ => "undetermined",
        }
    }
}

/// Who Otto's Claude Code profile is signed in as. Otto never reads the token:
/// the CLI keeps it in the macOS Keychain and every task runs through the CLI.
#[derive(Serialize, Default)]
pub struct Account {
    installed: bool,
    logged_in: bool,
    email: Option<String>,
    method: Option<String>,
}

impl Account {
    pub fn logged_in(&self) -> bool {
        self.logged_in
    }
}

#[tauri::command]
pub async fn claude_account() -> Account {
    account()
}

pub fn account() -> Account {
    let Some(bin) = crate::tasks::claude_bin() else { return Account::default() };
    let out = crate::tasks::claude_cli(&bin).args(["auth", "status"]).output();
    let v: Value = out.ok().and_then(|o| serde_json::from_slice(&o.stdout).ok()).unwrap_or_default();
    let s = |k: &str| v[k].as_str().map(String::from);
    Account { installed: true, logged_in: v["loggedIn"] == true, email: s("email"), method: s("authMethod") }
}

/// stdin of the running `claude auth login`, for the paste-a-code fallback.
static LOGIN: std::sync::Mutex<Option<std::process::ChildStdin>> = std::sync::Mutex::new(None);

/// Starts the CLI's own sign-in: it opens the browser and listens on localhost,
/// so most people never paste anything. Emits otto://claude-login with
/// null on success or the CLI's error line.
#[tauri::command]
pub fn claude_login(app: AppHandle) -> Result<(), String> {
    use std::io::BufRead;
    use std::process::Stdio;
    use tauri::Emitter;
    let bin = crate::tasks::claude_bin().ok_or("Claude Code isn't installed")?;
    let mut slot = LOGIN.lock().map_err(|_| "login state poisoned")?;
    if slot.is_some() {
        return Ok(()); // already waiting on the browser
    }
    let mut child = crate::tasks::claude_cli(&bin)
        .args(["auth", "login", "--claudeai"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("couldn't start claude: {e}"))?;
    *slot = child.stdin.take();
    let stdout = child.stdout.take();
    std::thread::spawn(move || {
        let mut last = String::new();
        if let Some(out) = stdout {
            for line in std::io::BufReader::new(out).lines().map_while(Result::ok) {
                if line.contains("failed") || line.contains("rror") {
                    last = line;
                }
            }
        }
        let ok = child.wait().map(|s| s.success()).unwrap_or(false);
        if let Ok(mut s) = LOGIN.lock() {
            *s = None;
        }
        let err = (!ok).then(|| if last.is_empty() { "Sign-in didn't finish.".to_string() } else { last.replace("Login failed: ", "") });
        let _ = app.emit("otto://claude-login", err);
    });
    Ok(())
}

/// The browser sometimes shows a code instead of finishing by itself.
#[tauri::command]
pub fn claude_login_code(code: String) -> Result<(), String> {
    let code = code.trim();
    if code.is_empty() || code.len() > 512 || code.contains(['\n', '\r']) {
        return Err("That doesn't look like a sign-in code.".into());
    }
    let mut slot = LOGIN.lock().map_err(|_| "login state poisoned")?;
    let stdin = slot.as_mut().ok_or("Click Sign in first.")?;
    writeln!(stdin, "{code}").map_err(|e| e.to_string())
}

/// Signs Otto's profile out. The terminal Claude Code login is untouched.
#[tauri::command]
pub async fn claude_logout() -> Result<(), String> {
    let bin = crate::tasks::claude_bin().ok_or("Claude Code isn't installed")?;
    let out = crate::tasks::claude_cli(&bin).args(["auth", "logout"]).output().map_err(|e| e.to_string())?;
    if out.status.success() { Ok(()) } else { Err("Couldn't sign out.".into()) }
}
