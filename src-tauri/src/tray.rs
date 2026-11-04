use std::sync::OnceLock;

use tauri::image::Image;
use tauri::menu::{Menu, MenuBuilder, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Wry};

/// Release page of a newer version, once one is found.
static UPDATE_URL: OnceLock<String> = OnceLock::new();

const ICON: &[u8] = include_bytes!("../icons/tray/pebbleTemplate@2x.png");

fn menu(app: &AppHandle, update: Option<&str>) -> tauri::Result<Menu<Wry>> {
    let hint = MenuItem::with_id(app, "hint", "Otto is listening: hold Ctrl+Option+Space", false, None::<&str>)?;
    let history = MenuItem::with_id(app, "history", "History\u{2026}", true, None::<&str>)?;
    let folder = MenuItem::with_id(app, "folder", "Open tasks folder", true, None::<&str>)?;
    let setup = MenuItem::with_id(app, "setup", "Set up Otto\u{2026}", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Otto", true, None::<&str>)?;
    let mut b = MenuBuilder::new(app);
    if let Some(v) = update {
        let item = MenuItem::with_id(app, "update", format!("Update available: Otto {v}\u{2026}"), true, None::<&str>)?;
        b = b.item(&item).separator();
    }
    b.items(&[&hint, &history, &folder, &setup, &PredefinedMenuItem::separator(app)?, &quit]).build()
}

/// Puts "Update available" at the top of the menu.
pub fn show_update(app: &AppHandle, version: &str, url: &str) {
    let _ = UPDATE_URL.set(url.to_string());
    if let (Some(tray), Ok(m)) = (app.tray_by_id("otto"), menu(app, Some(version))) {
        let _ = tray.set_menu(Some(m));
        let _ = tray.set_tooltip(Some(format!("Otto {version} is available")));
    }
}

/// Menu-bar item. Template image, so macOS tints it for light and dark bars.
pub fn start(app: &AppHandle) -> tauri::Result<()> {
    let menu = menu(app, None)?;
    TrayIconBuilder::with_id("otto")
        .icon(Image::from_bytes(ICON)?)
        .icon_as_template(true)
        .menu(&menu)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "history" => crate::history::show(app),
            "folder" => {
                let dir = dirs::home_dir().unwrap_or_default().join("Otto/tasks");
                let _ = std::fs::create_dir_all(&dir);
                let _ = std::process::Command::new("open").arg(dir).spawn();
            }
            "setup" => {
                let _ = app.emit("otto://show-onboarding", ());
            }
            "update" => {
                if let Some(url) = UPDATE_URL.get().filter(|u| u.starts_with(crate::updates::RELEASES)) {
                    let _ = std::process::Command::new("open").arg(url).spawn();
                }
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;
    Ok(())
}
