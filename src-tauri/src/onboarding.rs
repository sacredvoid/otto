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
    Info { model_ready: crate::stt::default_model_path().is_file(), folders }
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

/// Saves the choices, keeping any other keys, then closes the window.
#[tauri::command]
pub fn finish_onboarding(app: AppHandle, readable_dirs: Vec<String>) -> Result<(), String> {
    let home = home();
    // Only "~/Name" folders that exist; the webview is not trusted to name paths.
    let dirs: Vec<&String> = readable_dirs
        .iter()
        .filter(|d| {
            d.strip_prefix("~/")
                .is_some_and(|r| !r.is_empty() && !r.split('/').any(|p| p.is_empty() || p == "..") && home.join(r).is_dir())
        })
        .collect();
    let mut config = read_config();
    config.insert("readable_dirs".into(), json!(dirs));
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
