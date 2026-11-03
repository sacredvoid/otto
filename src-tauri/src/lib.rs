mod audio;
mod overlay;
mod stt;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_global_shortcut::ShortcutState;

const HOTKEY: &str = "ctrl+alt+space";
/// Shorter than this is an accidental tap, not speech.
const MIN_SAMPLES: usize = stt::SAMPLE_RATE as usize * 3 / 10;

#[derive(Default)]
struct Otto {
    recording: Mutex<Option<audio::Recording>>,
    stt: OnceLock<Result<stt::Stt, String>>,
    /// Bumped on every hotkey press so a stale "hide" timer can't close a newer pill.
    generation: AtomicU64,
}

#[derive(Clone, Serialize)]
struct Pill {
    state: &'static str,
    text: Option<String>,
    ms: Option<u128>,
}

fn pill(app: &AppHandle, state: &'static str, text: Option<String>, ms: Option<u128>) {
    let _ = app.emit("otto://pill", Pill { state, text, ms });
}

fn on_press(app: &AppHandle) {
    let otto = app.state::<Otto>();
    let mut recording = otto.recording.lock().unwrap();
    if recording.is_some() {
        return; // key repeat while held
    }
    otto.generation.fetch_add(1, Ordering::SeqCst);
    let levels = app.clone();
    match audio::Recording::start(move |level| {
        let _ = levels.emit("otto://level", level);
    }) {
        Ok(r) => {
            *recording = Some(r);
            pill(app, "listening", None, None);
            overlay::show(app);
        }
        Err(e) => {
            eprintln!("[otto] mic error: {e}");
            pill(app, "miss", Some(format!("Mic unavailable: {e}")), None);
            overlay::show(app);
            hide_later(app);
        }
    }
}

fn on_release(app: &AppHandle) {
    let otto = app.state::<Otto>();
    let Some(recording) = otto.recording.lock().unwrap().take() else {
        return;
    };
    let samples = recording.stop();
    if samples.len() < MIN_SAMPLES {
        pill(app, "miss", None, None);
        hide_later(app);
        return;
    }
    pill(app, "transcribing", None, None);
    let app = app.clone();
    std::thread::spawn(move || {
        let started = Instant::now();
        let result = match app.state::<Otto>().stt.get() {
            Some(Ok(stt)) => stt.transcribe(&samples),
            Some(Err(e)) => Err(e.clone()),
            None => Err("Speech model is still loading".into()),
        };
        let ms = started.elapsed().as_millis();
        match result {
            Ok(text) if !text.is_empty() => {
                println!("[otto] heard in {ms} ms: {text}");
                pill(&app, "heard", Some(text), Some(ms));
            }
            Ok(_) => pill(&app, "miss", None, None),
            Err(e) => {
                eprintln!("[otto] speech error: {e}");
                pill(&app, "miss", Some(e), None);
            }
        }
        hide_later(&app);
    });
}

fn hide_later(app: &AppHandle) {
    let generation = app.state::<Otto>().generation.load(Ordering::SeqCst);
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(2500));
        if app.state::<Otto>().generation.load(Ordering::SeqCst) == generation {
            overlay::hide(&app);
        }
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(Otto::default())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_shortcuts([HOTKEY])
                .expect("hotkey string is valid")
                .with_handler(|app, _shortcut, event| match event.state {
                    ShortcutState::Pressed => on_press(app),
                    ShortcutState::Released => on_release(app),
                })
                .build(),
        )
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            overlay::place(app.handle());

            let handle = app.handle().clone();
            std::thread::spawn(move || {
                let loaded = stt::Stt::load(&stt::default_model_path());
                match &loaded {
                    Ok(_) => println!("[otto] speech model ready"),
                    Err(e) => eprintln!("[otto] {e}"),
                }
                let _ = handle.state::<Otto>().stt.set(loaded);
            });
            println!("[otto] hold Ctrl+Option+Space to talk");
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Otto");
}
