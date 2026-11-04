//! The few things that differ between macOS and Windows when Otto starts
//! other programs: opening files, finding CLIs, stopping a task.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The hotkey as people read it on this platform.
#[cfg(target_os = "macos")]
pub const HOTKEY_LABEL: &str = "Ctrl+Option+Space";
#[cfg(not(target_os = "macos"))]
pub const HOTKEY_LABEL: &str = "Ctrl+Alt+Space";

/// A command that never flashes a console window on Windows.
pub fn command(program: impl AsRef<OsStr>) -> Command {
    #[allow(unused_mut)]
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// Opens a file, folder or URL with its default app (Finder / Explorer for folders).
pub fn open(target: &OsStr) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let mut cmd = command("open");
    // Explorer opens files with their app, folders in a window, and URLs in the browser.
    #[cfg(not(target_os = "macos"))]
    let mut cmd = command("explorer");
    cmd.arg(target).spawn().map(|_| ()).map_err(|e| e.to_string())
}

/// Stops a task's process and everything it started.
pub fn kill_tree(pid: u32) {
    #[cfg(windows)]
    let _ = command("taskkill").args(["/PID", &pid.to_string(), "/T", "/F"]).status();
    #[cfg(not(windows))]
    let _ = command("kill").arg("-TERM").arg(pid.to_string()).status();
}

/// Finds a CLI the way a terminal would. GUI apps on a Mac don't inherit the
/// shell PATH, so ask a login shell there; then try the usual install folders.
pub fn find_cli(name: &str) -> Option<PathBuf> {
    let home = dirs::home_dir().unwrap_or_default();
    #[cfg(windows)]
    {
        if let Ok(out) = command("where.exe").arg(name).output() {
            // `where` lists every match; prefer a real .exe over npm's .cmd shim.
            let found: Vec<PathBuf> = String::from_utf8_lossy(&out.stdout).lines().map(|l| PathBuf::from(l.trim())).filter(|p| p.is_file()).collect();
            let exe = found.iter().find(|p| has_ext(p, "exe")).or(found.first());
            if let Some(p) = exe {
                return Some(p.clone());
            }
        }
        let appdata = dirs::data_dir().unwrap_or_default();
        let local = dirs::data_local_dir().unwrap_or_default();
        return [
            home.join(format!(".local/bin/{name}.exe")),
            local.join(format!("Programs/{name}/{name}.exe")),
            appdata.join(format!("npm/{name}.cmd")),
        ]
        .into_iter()
        .find(|p| p.is_file());
    }
    #[cfg(not(windows))]
    {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
        if let Ok(out) = command(shell).args(["-lc", &format!("command -v {name}")]).output() {
            let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if p.starts_with('/') && Path::new(&p).is_file() {
                return Some(p.into());
            }
        }
        [home.join(".local/bin"), "/opt/homebrew/bin".into(), "/usr/local/bin".into()]
            .into_iter()
            .map(|d| d.join(name))
            .find(|p| p.is_file())
    }
}

/// npm installs Claude Code on Windows as a .cmd shim, and Windows can't pass
/// multi-line arguments (Otto's prompt) through a batch file. Run the package's
/// JavaScript entry with node instead when the shim is all there is.
pub fn cli_command(bin: &Path) -> Command {
    if has_ext(bin, "cmd") {
        if let Some(js) = npm_entry(bin) {
            let mut cmd = command("node");
            cmd.arg(js);
            return cmd;
        }
    }
    command(bin)
}

fn npm_entry(shim: &Path) -> Option<PathBuf> {
    let js = shim.parent()?.join("node_modules/@anthropic-ai/claude-code/cli.js");
    js.is_file().then_some(js)
}

fn has_ext(p: &Path, ext: &str) -> bool {
    p.extension().is_some_and(|e| e.eq_ignore_ascii_case(ext))
}

/// curl ships with macOS and with Windows 10 and later.
pub fn curl() -> Command {
    #[cfg(windows)]
    return command("curl.exe");
    #[cfg(not(windows))]
    command("/usr/bin/curl")
}
