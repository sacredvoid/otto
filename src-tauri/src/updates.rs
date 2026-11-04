//! Tells the user when a newer Otto is on GitHub. Checks on launch and once a day;
//! sends nothing but a plain request for the latest release. Off with "check_updates": false.

use std::time::Duration;

use serde_json::Value;
use tauri::AppHandle;

const LATEST: &str = "https://api.github.com/repos/sacredvoid/otto/releases/latest";
/// Only release pages of this repo are ever opened from the menu.
pub const RELEASES: &str = "https://github.com/sacredvoid/otto/releases/";
const EVERY: Duration = Duration::from_secs(24 * 60 * 60);

pub fn start(app: &AppHandle) {
    if crate::tasks::config()["check_updates"] == false {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || loop {
        if let Some((version, url)) = latest().filter(|(v, _)| newer(v, env!("CARGO_PKG_VERSION"))) {
            crate::tray::show_update(&app, &version, &url);
            return; // the menu item stays until the next launch
        }
        std::thread::sleep(EVERY);
    });
}

/// (version without "v", release page). None when offline or the answer is odd.
fn latest() -> Option<(String, String)> {
    let out = std::process::Command::new("/usr/bin/curl")
        .args(["-fsSL", "-m", "15", "-H", "Accept: application/vnd.github+json", LATEST])
        .output()
        .ok()?;
    let v: Value = serde_json::from_slice(&out.stdout).ok()?;
    let tag = v["tag_name"].as_str()?.trim_start_matches('v').to_string();
    let url = v["html_url"].as_str()?.to_string();
    url.starts_with(RELEASES).then_some((tag, url))
}

fn parse(v: &str) -> Option<(u64, u64, u64)> {
    let core = v.trim_start_matches('v').split(['-', '+']).next()?;
    let mut n = core.split('.').map(|p| p.parse::<u64>().ok());
    Some((n.next()??, n.next().flatten().unwrap_or(0), n.next().flatten().unwrap_or(0)))
}

fn newer(candidate: &str, current: &str) -> bool {
    matches!((parse(candidate), parse(current)), (Some(a), Some(b)) if a > b)
}

#[cfg(test)]
mod tests {
    use super::newer;

    #[test]
    fn compares_versions_numerically() {
        assert!(newer("0.2.0", "0.1.0"));
        assert!(newer("v0.10.0", "0.9.3"), "10 > 9, not text order");
        assert!(newer("1.0", "0.9.9"));
        assert!(!newer("0.1.0", "0.1.0"));
        assert!(!newer("0.1.0-beta", "0.1.0"));
        assert!(!newer("garbage", "0.1.0"));
    }
}
