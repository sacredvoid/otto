use std::sync::OnceLock;

use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuBuilder, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Wry};

/// Release page of a newer version, once one is found.
static UPDATE_URL: OnceLock<String> = OnceLock::new();
static UPDATE_VERSION: OnceLock<String> = OnceLock::new();

// A black template glyph macOS tints for the menu bar; Windows taskbars are often
// dark, so it gets the colored app icon.
#[cfg(target_os = "macos")]
const ICON: &[u8] = include_bytes!("../icons/tray/pebbleTemplate@2x.png");
#[cfg(not(target_os = "macos"))]
const ICON: &[u8] = include_bytes!("../icons/32x32.png");

fn menu(app: &AppHandle, update: Option<&str>) -> tauri::Result<Menu<Wry>> {
    let hint = MenuItem::with_id(app, "hint", format!("Otto is listening: hold {}", crate::platform::HOTKEY_LABEL), false, None::<&str>)?;
    let home = MenuItem::with_id(app, "home", "Open Otto\u{2026}", true, None::<&str>)?;
    let history = MenuItem::with_id(app, "history", "History\u{2026}", true, None::<&str>)?;
    let login = CheckMenuItem::with_id(app, "login", "Open at Login", true, crate::power::login_enabled(), None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Otto", true, None::<&str>)?;
    let mut b = MenuBuilder::new(app);
    if let Some(v) = update {
        let item = MenuItem::with_id(app, "update", format!("Update available: Otto {v}\u{2026}"), true, None::<&str>)?;
        b = b.item(&item).separator();
    }
    b.items(&[&hint, &home, &history, &PredefinedMenuItem::separator(app)?, &login, &quit]).build()
}

/// Puts "Update available" at the top of the menu.
pub fn show_update(app: &AppHandle, version: &str, url: &str) {
    let _ = UPDATE_URL.set(url.to_string());
    let _ = UPDATE_VERSION.set(version.to_string());
    if let (Some(tray), Ok(m)) = (app.tray_by_id("otto"), menu(app, Some(version))) {
        let _ = tray.set_menu(Some(m));
        let _ = tray.set_tooltip(Some(format!("Otto {version} is available")));
    }
}

/// Menu-bar item. Template image, so macOS tints it for light and dark bars.
pub fn start(app: &AppHandle) -> tauri::Result<()> {
    TrayIconBuilder::with_id("otto")
        .icon(Image::from_bytes(ICON)?)
        .icon_as_template(cfg!(target_os = "macos"))
        .tooltip("Otto")
        .menu(&menu(app, None)?)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "home" => crate::history::show(app, "home"),
            "history" => crate::history::show(app, "history"),
            "update" => {
                if let Some(url) = UPDATE_URL.get().filter(|u| u.starts_with(crate::updates::RELEASES)) {
                    let _ = crate::platform::open(url.as_ref());
                }
            }
            "login" => {
                crate::power::set_login(!crate::power::login_enabled());
                // Rebuild so the checkmark shows what macOS actually has.
                if let (Some(tray), Ok(m)) = (app.tray_by_id("otto"), menu(app, UPDATE_VERSION.get().map(String::as_str))) {
                    let _ = tray.set_menu(Some(m));
                }
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;
    Ok(())
}
