use tauri::{AppHandle, LogicalPosition, Manager, WebviewWindow};

const WIDTH: f64 = 560.0;

fn window(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window("overlay")
}

/// Top-center, flush under the menu bar like the task island; clicks pass
/// through to the app below. Must run on the main thread (Tauri's `setup` does).
pub fn place(app: &AppHandle) {
    let Some(win) = window(app) else { return };
    if let Ok(Some(m)) = win.primary_monitor() {
        let s = m.scale_factor();
        let size = m.size().to_logical::<f64>(s);
        let pos = m.position().to_logical::<f64>(s);
        let top = m.work_area().position.to_logical::<f64>(s).y;
        let _ = win.set_position(LogicalPosition::new(pos.x + (size.width - WIDTH) / 2.0, top));
    }
    let _ = win.set_ignore_cursor_events(true);
    #[cfg(target_os = "macos")]
    mac::make_panel(&win);
    #[cfg(windows)]
    let _ = win.set_focusable(false);
}

const TOAST_WIDTH: f64 = 380.0;
const TOAST_MAX_HEIGHT: f64 = 720.0;

/// Top-center stack for task cards, flush under the menu bar so cards drop
/// out of the notch. Unlike the pill, it takes clicks (Open, Show in Finder),
/// but as a non-activating panel it still never steals keyboard focus.
pub fn place_toasts(app: &AppHandle) {
    let Some(win) = app.get_webview_window("toasts") else { return };
    if let Ok(Some(m)) = win.primary_monitor() {
        let s = m.scale_factor();
        let size = m.size().to_logical::<f64>(s);
        let pos = m.position().to_logical::<f64>(s);
        // The work area starts below the menu bar (taller on notched Macs).
        let top = m.work_area().position.to_logical::<f64>(s).y;
        let _ = win.set_position(LogicalPosition::new(pos.x + (size.width - TOAST_WIDTH) / 2.0, top));
    }
    #[cfg(target_os = "macos")]
    mac::make_panel(&win);
    // Takes clicks (Allow, Open) but never the keyboard, like the Mac panel.
    #[cfg(windows)]
    let _ = win.set_focusable(false);
}

/// The webview reports its content height; 0 means no cards, so hide.
pub fn set_toasts_height(app: &AppHandle, height: f64) {
    let Some(win) = app.get_webview_window("toasts") else { return };
    if !(height >= 1.0) {
        return hide_window(&win);
    }
    let _ = win.set_size(tauri::LogicalSize::new(TOAST_WIDTH, height.min(TOAST_MAX_HEIGHT)));
    show_window(app, win);
}

/// One `.glass` element in a webview: its box in CSS px (= points), corner
/// radius and current opacity.
#[derive(serde::Deserialize, Clone, Debug)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))] // native glass is macOS only
pub struct GlassRect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    r: f64,
    a: f64,
}

/// Puts native glass behind the given boxes of `win`'s transparent webview.
/// The webview sends a fresh list whenever a glass element moves or fades.
pub fn set_glass(app: &AppHandle, win: WebviewWindow, rects: Vec<GlassRect>) {
    #[cfg(target_os = "macos")]
    let _ = app.run_on_main_thread(move || mac::set_glass(&win, &rects));
    #[cfg(not(target_os = "macos"))]
    let _ = (app, win, rects);
}

#[cfg(target_os = "macos")]
mod mac {
    use objc2::msg_send;
    use objc2::rc::{Allocated, Retained};
    use objc2::runtime::{AnyClass, AnyObject};
    use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
    use std::collections::HashMap;
    use std::sync::Mutex;
    use tauri::WebviewWindow;

    /// Glass views per window label, back to front. The superview owns them;
    /// they are only touched on the main thread.
    static GLASS: Mutex<Option<HashMap<String, Vec<usize>>>> = Mutex::new(None);

    /// Liquid Glass on macOS 26, the older blur material before that.
    fn glass_class() -> Option<(&'static AnyClass, bool)> {
        AnyClass::get(c"NSGlassEffectView").map(|c| (c, true)).or_else(|| AnyClass::get(c"NSVisualEffectView").map(|c| (c, false)))
    }

    pub fn set_glass(win: &WebviewWindow, rects: &[super::GlassRect]) {
        let Ok(ptr) = win.ns_window() else { return };
        let Some((cls, liquid)) = glass_class() else { return };
        let mut guard = GLASS.lock().unwrap();
        let views = guard.get_or_insert_with(HashMap::new).entry(win.label().to_string()).or_default();
        unsafe {
            let w = &*(ptr as *mut AnyObject);
            let content: *mut AnyObject = msg_send![w, contentView];
            let Some(content) = content.as_ref() else { return };
            let bounds: NSRect = msg_send![content, bounds];
            let flipped: bool = msg_send![content, isFlipped];
            // Extra views from a longer list go away; missing ones are made.
            while views.len() > rects.len() {
                let v = &*(views.pop().unwrap() as *mut AnyObject);
                let _: () = msg_send![v, removeFromSuperview];
            }
            while views.len() < rects.len() {
                let v: Allocated<AnyObject> = msg_send![cls, alloc];
                let v: Retained<AnyObject> = msg_send![v, initWithFrame: NSRect::ZERO];
                if !liquid {
                    let _: () = msg_send![&*v, setMaterial: 6isize]; // popover
                    let _: () = msg_send![&*v, setBlendingMode: 0isize]; // behind window
                    let _: () = msg_send![&*v, setState: 1isize]; // always active
                    let _: () = msg_send![&*v, setWantsLayer: true];
                }
                // Below the webview, so the page draws on top of the glass.
                let _: () = msg_send![content, addSubview: &*v, positioned: -1isize, relativeTo: std::ptr::null::<AnyObject>()];
                views.push(Retained::as_ptr(&v) as usize); // the superview keeps it alive
            }
            for (r, &v) in rects.iter().zip(views.iter()) {
                let v = &*(v as *mut AnyObject);
                let y = if flipped { r.y } else { bounds.size.height - r.y - r.h };
                let _: () = msg_send![v, setFrame: NSRect::new(NSPoint::new(r.x, y), NSSize::new(r.w, r.h))];
                // Keep the distance to the window's top when it resizes, like the page does.
                let _: () = msg_send![v, setAutoresizingMask: if flipped { 32usize } else { 8usize }];
                let _: () = msg_send![v, setAlphaValue: r.a.clamp(0.0, 1.0)];
                if liquid {
                    let _: () = msg_send![v, setCornerRadius: r.r];
                } else {
                    let layer: *mut AnyObject = msg_send![v, layer];
                    if let Some(layer) = layer.as_ref() {
                        let _: () = msg_send![layer, setCornerRadius: r.r];
                        let _: () = msg_send![layer, setMasksToBounds: true];
                    }
                }
            }
        }
    }

    /// Otto's ink is dark, so its glass stays light even in dark mode.
    fn force_light(w: &AnyObject) {
        let Some(cls) = AnyClass::get(c"NSAppearance") else { return };
        unsafe {
            let name = NSString::from_str("NSAppearanceNameAqua");
            let appearance: *mut AnyObject = msg_send![cls, appearanceNamed: &*name];
            if !appearance.is_null() {
                let _: () = msg_send![w, setAppearance: appearance];
            }
        }
    }

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
        force_light(unsafe { &*(ptr as *mut AnyObject) });
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
pub fn show(app: &AppHandle) {
    if let Some(win) = window(app) {
        show_window(app, win);
    }
}

pub fn hide(app: &AppHandle) {
    if let Some(win) = window(app) {
        hide_window(&win);
    }
}

/// Brings an overlay window up without taking focus from the user's app.
fn show_window(app: &AppHandle, win: WebviewWindow) {
    #[cfg(target_os = "macos")]
    let _ = app.run_on_main_thread(move || mac::order_front(&win));
    #[cfg(windows)]
    let _ = app.run_on_main_thread(move || win32::show(&win));
    #[cfg(not(any(target_os = "macos", windows)))]
    let _ = (app, win.show());
}

fn hide_window(win: &WebviewWindow) {
    // Shown with a raw ShowWindow on Windows, so hidden the same way.
    #[cfg(windows)]
    win32::hide(win);
    #[cfg(not(windows))]
    let _ = win.hide();
}

/// Tauri's show() activates the window on Windows, which would pull focus out
/// of whatever the user is typing in. ShowWindow(SW_SHOWNOACTIVATE) doesn't.
#[cfg(windows)]
mod win32 {
    use tauri::WebviewWindow;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        HWND_TOPMOST, SW_HIDE, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetWindowPos, ShowWindow,
    };

    pub fn show(win: &WebviewWindow) {
        let Ok(hwnd) = win.hwnd() else { return };
        let h = hwnd.0 as _;
        unsafe {
            ShowWindow(h, SW_SHOWNOACTIVATE);
            SetWindowPos(h, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        }
    }

    pub fn hide(win: &WebviewWindow) {
        if let Ok(hwnd) = win.hwnd() {
            unsafe { ShowWindow(hwnd.0 as _, SW_HIDE) };
        }
    }
}
