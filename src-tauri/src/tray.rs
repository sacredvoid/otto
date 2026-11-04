use tauri::image::Image;
use tauri::menu::{MenuBuilder, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter};

const ICON: &[u8] = include_bytes!("../icons/tray/pebbleTemplate@2x.png");

/// Menu-bar item. Template image, so macOS tints it for light and dark bars.
pub fn start(app: &AppHandle) -> tauri::Result<()> {
    let hint = MenuItem::with_id(app, "hint", "Otto is listening: hold Ctrl+Option+Space", false, None::<&str>)?;
    let folder = MenuItem::with_id(app, "folder", "Open tasks folder", true, None::<&str>)?;
    let setup = MenuItem::with_id(app, "setup", "Set up Otto\u{2026}", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Otto", true, None::<&str>)?;
    let menu = MenuBuilder::new(app)
        .items(&[&hint, &folder, &setup, &PredefinedMenuItem::separator(app)?, &quit])
        .build()?;
    TrayIconBuilder::with_id("otto")
        .icon(Image::from_bytes(ICON)?)
        .icon_as_template(true)
        .menu(&menu)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "folder" => {
                let dir = dirs::home_dir().unwrap_or_default().join("Otto/tasks");
                let _ = std::fs::create_dir_all(&dir);
                let _ = std::process::Command::new("open").arg(dir).spawn();
            }
            "setup" => {
                let _ = app.emit("otto://show-onboarding", ());
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;
    Ok(())
}
