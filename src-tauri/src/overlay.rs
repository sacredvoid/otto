use tauri::{AppHandle, LogicalPosition, Manager, WebviewWindow};

const WIDTH: f64 = 560.0;
const HEIGHT: f64 = 120.0;
const BOTTOM_GAP: f64 = 72.0;

fn window(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window("overlay")
}

/// Bottom-center of the primary screen; clicks pass through to the app below.
/// Must run on the main thread (Tauri's `setup` does).
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
    #[cfg(target_os = "macos")]
    mac::make_panel(&win);
}

const TOAST_WIDTH: f64 = 380.0;
const TOAST_MAX_HEIGHT: f64 = 720.0;

/// Top-right stack for task cards. Unlike the pill, it takes clicks
/// (Open, Show in Finder), but as a non-activating panel it still never
/// steals keyboard focus.
pub fn place_toasts(app: &AppHandle) {
    let Some(win) = app.get_webview_window("toasts") else { return };
    if let Ok(Some(m)) = win.primary_monitor() {
        let s = m.scale_factor();
        let size = m.size().to_logical::<f64>(s);
        let pos = m.position().to_logical::<f64>(s);
        let _ = win.set_position(LogicalPosition::new(pos.x + size.width - TOAST_WIDTH - 12.0, pos.y + 36.0));
    }
    #[cfg(target_os = "macos")]
    mac::make_panel(&win);
}

/// The webview reports its content height; 0 means no cards, so hide.
pub fn set_toasts_height(app: &AppHandle, height: f64) {
    let Some(win) = app.get_webview_window("toasts") else { return };
    if !(height >= 1.0) {
        let _ = win.hide();
        return;
    }
    let _ = win.set_size(tauri::LogicalSize::new(TOAST_WIDTH, height.min(TOAST_MAX_HEIGHT)));
    #[cfg(target_os = "macos")]
    {
        let w = win.clone();
        let _ = app.run_on_main_thread(move || mac::order_front(&w));
    }
    #[cfg(not(target_os = "macos"))]
    let _ = win.show();
}

#[cfg(target_os = "macos")]
mod mac {
    use objc2::msg_send;
    use objc2::runtime::{AnyClass, AnyObject};
    use tauri::WebviewWindow;

    const NONACTIVATING_PANEL: usize = 1 << 7;
    const CAN_JOIN_ALL_SPACES: usize = 1 << 0;
    const STATIONARY: usize = 1 << 4;
    const IGNORES_CYCLE: usize = 1 << 6;
    const FULL_SCREEN_AUXILIARY: usize = 1 << 8;
    const STATUS_WINDOW_LEVEL: isize = 25;

    /// Turns the overlay into a non-activating NSPanel. A plain NSWindow can't
    /// float over another app's full-screen Space and would steal focus when
    /// clicked; a non-activating panel does neither.
    pub fn make_panel(win: &WebviewWindow) {
        let Ok(ptr) = win.ns_window() else { return };
        let Some(panel) = AnyClass::get(c"NSPanel") else { return };
        unsafe {
            let w = &*(ptr as *mut AnyObject);
            AnyObject::set_class(w, panel);
            let mask: usize = msg_send![w, styleMask];
            let _: () = msg_send![w, setStyleMask: mask | NONACTIVATING_PANEL];
            let _: () = msg_send![w, setLevel: STATUS_WINDOW_LEVEL];
            let behavior = CAN_JOIN_ALL_SPACES | STATIONARY | IGNORES_CYCLE | FULL_SCREEN_AUXILIARY;
            let _: () = msg_send![w, setCollectionBehavior: behavior];
            let _: () = msg_send![w, setHidesOnDeactivate: false];
        }
    }

    pub fn order_front(win: &WebviewWindow) {
        let Ok(ptr) = win.ns_window() else { return };
        unsafe {
            let w = &*(ptr as *mut AnyObject);
            let _: () = msg_send![w, orderFrontRegardless];
        }
    }
}

/// Shows the pill WITHOUT activating Otto, so keyboard focus stays in the
/// user's app.
#[cfg(target_os = "macos")]
pub fn show(app: &AppHandle) {
    let Some(win) = window(app) else { return };
    let _ = app.run_on_main_thread(move || mac::order_front(&win));
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
