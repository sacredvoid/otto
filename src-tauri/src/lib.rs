mod audio;
mod gate;
mod history;
mod onboarding;
mod overlay;
mod stt;
mod tasks;
mod tray;
mod updates;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_global_shortcut::ShortcutState;

const HOTKEY: &str = "ctrl+alt+space";
/// Shorter than this is an accidental tap, not speech.
const MIN_SAMPLES: usize = stt::SAMPLE_RATE as usize * 3 / 10;
const RELEASE_TAIL: Duration = Duration::from_millis(300);

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
    follow_up: bool,
}

fn pill(app: &AppHandle, state: &'static str, text: Option<String>, ms: Option<u128>) {
    let _ = app.emit("otto://pill", Pill { state, text, ms, follow_up: false });
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
    let app = app.clone();
    std::thread::spawn(move || {
        // People let go of the key on their last syllable; keep listening briefly.
        std::thread::sleep(RELEASE_TAIL);
        let samples = recording.stop();
        if samples.len() < MIN_SAMPLES {
            pill(&app, "miss", None, None);
            hide_later(&app);
            return;
        }
        pill(&app, "transcribing", None, None);
        let started = Instant::now();
        let result = match app.state::<Otto>().stt.get() {
            Some(Ok(stt)) => stt.transcribe(&samples),
            Some(Err(e)) => Err(e.clone()),
            None => Err("Speech model is still loading".into()),
        };
        let ms = started.elapsed().as_millis();
        match result {
            Ok(text) if is_speech(&text) => {
                println!("[otto] heard in {ms} ms: {text}");
                // During setup the hotkey is a practice run: show what was heard, start nothing.
                let practicing = app
                    .get_webview_window("onboarding")
                    .is_some_and(|w| w.is_visible().unwrap_or(false));
                if practicing {
                    pill(&app, "heard", Some(text), Some(ms));
                    hide_later(&app);
                    return;
                }
                match tasks::Tasks::submit(&app, &text) {
                    Ok(s) => {
                        let _ = app.emit("otto://pill", Pill { state: "heard", text: Some(s.title), ms: Some(ms), follow_up: s.follow_up });
                    }
                    Err(e) => {
                        eprintln!("[otto] could not queue task: {e}");
                        pill(&app, "miss", Some(e), None);
                    }
                }
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

/// Whisper renders silence as tags like "[BLANK_AUDIO]" or "(wind blowing)".
fn is_speech(text: &str) -> bool {
    let t = text.trim();
    !t.is_empty() && !(t.starts_with(['[', '(']) && t.ends_with([']', ')']))
}

fn task_dir(tasks: &tasks::Tasks, id: u64) -> Result<std::path::PathBuf, String> {
    tasks.dir(id).ok_or_else(|| "unknown task".to_string())
}

fn open(args: &[&std::ffi::OsStr]) -> Result<(), String> {
    std::process::Command::new("open").args(args).spawn().map(|_| ()).map_err(|e| e.to_string())
}

#[tauri::command]
fn task_details(tasks: tauri::State<tasks::Tasks>, id: u64) -> Result<tasks::Details, String> {
    Ok(tasks::details(&task_dir(&tasks, id)?))
}

#[tauri::command]
fn open_task_file(tasks: tauri::State<tasks::Tasks>, id: u64, name: String) -> Result<(), String> {
    let file = tasks::task_file(&task_dir(&tasks, id)?, &name).ok_or("not a file in this task")?;
    open(&[file.as_os_str()])
}

/// Opens the task's folder as a Finder window.
#[tauri::command]
fn show_task_folder(tasks: tauri::State<tasks::Tasks>, id: u64) -> Result<(), String> {
    open(&[task_dir(&tasks, id)?.as_os_str()])
}

#[tauri::command]
fn cancel_task(app: AppHandle, id: u64) {
    tasks::Tasks::cancel(&app, id);
}

#[tauri::command]
fn answer_approval(gate: tauri::State<std::sync::Arc<gate::Gate>>, approval_id: u64, allow: bool) {
    gate.answer(approval_id, allow);
}

#[tauri::command]
fn set_toasts_height(app: AppHandle, height: f64) {
    overlay::set_toasts_height(&app, height);
}

#[tauri::command]
fn set_chip_size(app: AppHandle, width: f64, height: f64) {
    overlay::set_chip_size(&app, width, height);
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
        .manage(tasks::Tasks::default())
        .invoke_handler(tauri::generate_handler![task_details, open_task_file, show_task_folder, cancel_task, answer_approval, set_toasts_height, set_chip_size,
            onboarding::onboarding_info, onboarding::onboarding_detect, onboarding::mic_status, onboarding::request_mic,
            onboarding::open_settings, onboarding::finish_onboarding, onboarding::detect_connectors,
            onboarding::claude_account, onboarding::claude_login, onboarding::claude_login_code, onboarding::claude_logout,
            history::list_history, history::history_detail, history::open_history_file, history::show_history_folder])
        // Onboarding and History are reopened later from the tray, so closing only hides them.
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if matches!(window.label(), "onboarding" | "history") {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
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
            overlay::place_toasts(app.handle());
            overlay::place_chip(app.handle());
            tray::start(app.handle())?;
            updates::start(app.handle());
            let handle = app.handle().clone();
            tauri::Listener::listen_any(app.handle(), "otto://show-onboarding", move |_| {
                onboarding::show_onboarding(&handle);
            });

            let asks = app.handle().clone();
            let audit = dirs::home_dir().unwrap_or_default().join("Otto/audit.jsonl");
            let gate = gate::Gate::start(&audit, move |ask| {
                println!("[otto] task {} needs you: {}", ask.task_id, ask.summary);
                let _ = asks.emit("otto://approval", ask);
            })?;
            app.manage(gate);

            let handle = app.handle().clone();
            std::thread::spawn(move || {
                let loaded = stt::Stt::load(&stt::default_model_path());
                match &loaded {
                    Ok(_) => println!("[otto] speech model ready"),
                    Err(e) => eprintln!("[otto] {e}"),
                }
                let _ = handle.state::<Otto>().stt.set(loaded);
            });
            if !onboarding::is_onboarded() {
                onboarding::show_onboarding(app.handle());
            } else {
                // Set up before Otto had its own sign-in (or signed out since): ask once at launch.
                let handle = app.handle().clone();
                std::thread::spawn(move || {
                    if !onboarding::account().logged_in() {
                        let h = handle.clone();
                        let _ = handle.run_on_main_thread(move || onboarding::show_onboarding(&h));
                    }
                });
            }
            println!("[otto] hold Ctrl+Option+Space to talk");
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building Otto")
        .run(|app, event| {
            // No Dock icon and the menu bar icon can hide behind the notch, so
            // opening Otto.app again shows a window instead of doing nothing.
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = event {
                if onboarding::is_onboarded() {
                    history::show(app);
                } else {
                    onboarding::show_onboarding(app);
                }
            }
        });
}
