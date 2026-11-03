use tauri::{AppHandle, LogicalPosition, Manager, WebviewWindow};

const WIDTH: f64 = 560.0;
const HEIGHT: f64 = 120.0;
const BOTTOM_GAP: f64 = 72.0;

fn window(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window("overlay")
}

/// Bottom-center of the primary screen; clicks pass through to the app below.
pub fn place(app: &AppHandle) {
    let Some(win) = window(app) else { return };
    if let Ok(Some(m)) = win.primary_monitor() {
        let s = m.scale_factor();
        let size = m.size().to_logical::<f64>(s);
        let pos = m.position().to_logical::<f64>(s);
        let _ = win.set_position(LogicalPosition::new(
            pos.x + (size.width - WIDTH) / 2.0,
            pos.y + size.height - HEIGHT - BOTTOM_GAP,
        ));
    }
    let _ = win.set_ignore_cursor_events(true);
}

/// Shows the pill WITHOUT activating Otto, so keyboard focus stays in the
/// user's app. A normal `show()` would make the window key and steal focus.
#[cfg(target_os = "macos")]
pub fn show(app: &AppHandle) {
    use objc2::msg_send;
    use objc2::runtime::AnyObject;

    let Some(win) = window(app) else { return };
    let _ = app.run_on_main_thread(move || {
        let Ok(ptr) = win.ns_window() else { return };
        // NSWindowCollectionBehaviorCanJoinAllSpaces | FullScreenAuxiliary
        let behavior: usize = (1 << 0) | (1 << 8);
        unsafe {
            let ns_window = &*(ptr as *mut AnyObject);
            let _: () = msg_send![ns_window, setCollectionBehavior: behavior];
            let _: () = msg_send![ns_window, orderFrontRegardless];
        }
    });
}

#[cfg(not(target_os = "macos"))]
pub fn show(app: &AppHandle) {
    if let Some(win) = window(app) {
        let _ = win.show();
    }
}

pub fn hide(app: &AppHandle) {
    if let Some(win) = window(app) {
        let _ = win.hide();
    }
}
