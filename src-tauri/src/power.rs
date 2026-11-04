//! Launch at login, and staying sane across sleep, wake and display changes.

use tauri::AppHandle;

/// True when Otto is registered to open at login.
pub fn login_enabled() -> bool {
    #[cfg(target_os = "macos")]
    return mac::login_status() == 1;
    #[cfg(not(target_os = "macos"))]
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
    if let Err(e) = mac::set_login(on) {
        eprintln!("[otto] login item: {e}");
    }
}

/// On by default; a saved "launch_at_login": false keeps it off.
/// `on_sleep` runs before the Mac sleeps, `on_change` after it wakes or the displays change.
pub fn start(app: &AppHandle, on_sleep: fn(&AppHandle), on_change: fn(&AppHandle)) {
    apply_login(crate::tasks::config()["launch_at_login"] != false);
    #[cfg(target_os = "macos")]
    mac::observe(app, on_sleep, on_change);
    #[cfg(not(target_os = "macos"))]
    let _ = (app, on_sleep, on_change);
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
