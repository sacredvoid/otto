//! Launch at login, and staying sane across sleep, wake and display changes.

use tauri::AppHandle;

/// True when Otto is registered to open at login.
pub fn login_enabled() -> bool {
    #[cfg(target_os = "macos")]
    return mac::login_status() == 1;
    #[cfg(windows)]
    return win::login_enabled();
    #[cfg(not(any(target_os = "macos", windows)))]
    false
}

/// Turns the login item on or off and remembers the choice ("launch_at_login").
pub fn set_login(on: bool) {
    crate::onboarding::set_config("launch_at_login", serde_json::json!(on));
    apply_login(on);
}

fn apply_login(on: bool) {
    // A dev build isn't an installed app; registering it would add a broken login item.
    if cfg!(debug_assertions) {
        println!("[otto] login item: skipped in dev builds (would set {on})");
        return;
    }
    #[cfg(target_os = "macos")]
    let result = mac::set_login(on);
    #[cfg(windows)]
    let result = win::set_login(on);
    #[cfg(not(any(target_os = "macos", windows)))]
    let result: Result<(), String> = Ok(());
    if let Err(e) = result {
        eprintln!("[otto] login item: {e}");
    }
}

/// On by default; a saved "launch_at_login": false keeps it off.
/// `on_sleep` runs before the Mac sleeps, `on_change` after it wakes or the displays change.
pub fn start(app: &AppHandle, on_sleep: fn(&AppHandle), on_change: fn(&AppHandle)) {
    apply_login(crate::tasks::config()["launch_at_login"] != false);
    #[cfg(target_os = "macos")]
    mac::observe(app, on_sleep, on_change);
    #[cfg(windows)]
    win::observe(app, on_sleep, on_change);
    #[cfg(not(any(target_os = "macos", windows)))]
    let _ = (app, on_sleep, on_change);
}

#[cfg(windows)]
mod win {
    use std::time::{Duration, SystemTime};

    use tauri::{AppHandle, Manager};

    use crate::platform::command;

    const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";

    pub fn login_enabled() -> bool {
        command("reg").args(["query", RUN_KEY, "/v", "Otto"]).output().is_ok_and(|o| o.status.success())
    }

    /// The per-user Run key starts Otto at sign-in; no admin rights needed.
    pub fn set_login(on: bool) -> Result<(), String> {
        let out = if on {
            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
            command("reg").args(["add", RUN_KEY, "/v", "Otto", "/t", "REG_SZ", "/f", "/d"]).arg(format!("\"{}\"", exe.display())).output()
        } else if login_enabled() {
            command("reg").args(["delete", RUN_KEY, "/v", "Otto", "/f"]).output()
        } else {
            return Ok(());
        };
        match out {
            Ok(o) if o.status.success() => Ok(()),
            Ok(o) => Err(String::from_utf8_lossy(&o.stderr).trim().to_string()),
            Err(e) => Err(e.to_string()),
        }
    }

    /// Windows reports sleep and display changes as window messages to a
    /// top-level window; a light watcher covers both without hooking one.
    /// The wall clock jumping past the tick means the PC slept; a moved work
    /// area means a display or the taskbar changed.
    // ponytail: notices sleep after waking, not before; any recording is dropped then.
    pub fn observe(app: &AppHandle, on_sleep: fn(&AppHandle), on_change: fn(&AppHandle)) {
        const TICK: Duration = Duration::from_secs(5);
        let app = app.clone();
        std::thread::spawn(move || {
            let area = |app: &AppHandle| {
                let win = app.get_webview_window("toasts")?;
                let m = win.primary_monitor().ok()??;
                let w = m.work_area();
                Some((w.position.x, w.position.y, w.size.width, w.size.height, m.scale_factor().to_bits()))
            };
            let mut last_area = area(&app);
            let mut last = SystemTime::now();
            loop {
                std::thread::sleep(TICK);
                let now = SystemTime::now();
                let slept = now.duration_since(last).is_ok_and(|d| d > TICK * 4);
                last = now;
                let now_area = area(&app);
                if slept {
                    let a = app.clone();
                    let _ = app.run_on_main_thread(move || on_sleep(&a));
                }
                if slept || now_area != last_area {
                    last_area = now_area;
                    let a = app.clone();
                    let _ = app.run_on_main_thread(move || on_change(&a));
                }
            }
        });
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use std::ptr::NonNull;

    use block2::RcBlock;
    use objc2::msg_send;
    use objc2::runtime::{AnyClass, AnyObject};
    use objc2_foundation::NSString;
    use tauri::AppHandle;

    #[link(name = "ServiceManagement", kind = "framework")]
    unsafe extern "C" {}

    fn main_app_service() -> Option<*mut AnyObject> {
        let cls = AnyClass::get(c"SMAppService")?;
        let s: *mut AnyObject = unsafe { msg_send![cls, mainAppService] };
        (!s.is_null()).then_some(s)
    }

    /// SMAppServiceStatus: 0 not registered, 1 enabled, 2 needs approval, 3 not found.
    pub fn login_status() -> isize {
        main_app_service().map_or(0, |s| unsafe { msg_send![s, status] })
    }

    pub fn set_login(on: bool) -> Result<(), String> {
        let s = main_app_service().ok_or("SMAppService unavailable")?;
        if (login_status() == 1) == on {
            return Ok(());
        }
        let mut err: *mut AnyObject = std::ptr::null_mut();
        let ok: bool = unsafe {
            if on { msg_send![s, registerAndReturnError: &mut err] } else { msg_send![s, unregisterAndReturnError: &mut err] }
        };
        if ok {
            return Ok(());
        }
        let msg = if err.is_null() {
            "unknown error".to_string()
        } else {
            let d: *mut NSString = unsafe { msg_send![err, localizedDescription] };
            unsafe { d.as_ref() }.map(|d| d.to_string()).unwrap_or_default()
        };
        Err(msg)
    }

    /// Workspace sleep/wake plus screen changes, delivered on the main queue.
    pub fn observe(app: &AppHandle, on_sleep: fn(&AppHandle), on_change: fn(&AppHandle)) {
        let (Some(ws), Some(nc), Some(queue)) = (AnyClass::get(c"NSWorkspace"), AnyClass::get(c"NSNotificationCenter"), AnyClass::get(c"NSOperationQueue"))
        else {
            return;
        };
        unsafe {
            let shared: *mut AnyObject = msg_send![ws, sharedWorkspace];
            let ws_center: *mut AnyObject = msg_send![shared, notificationCenter];
            let app_center: *mut AnyObject = msg_send![nc, defaultCenter];
            let main: *mut AnyObject = msg_send![queue, mainQueue];
            let add = |center: *mut AnyObject, name: &str, f: fn(&AppHandle)| {
                let app = app.clone();
                let block = RcBlock::new(move |_note: NonNull<AnyObject>| f(&app));
                let name = NSString::from_str(name);
                // The center keeps the block for the life of the app.
                let _: *mut AnyObject = msg_send![center, addObserverForName: &*name, object: std::ptr::null::<AnyObject>(), queue: main, usingBlock: &*block];
            };
            add(ws_center, "NSWorkspaceWillSleepNotification", on_sleep);
            add(ws_center, "NSWorkspaceDidWakeNotification", on_change);
            add(app_center, "NSApplicationDidChangeScreenParametersNotification", on_change);
        }
    }
}
