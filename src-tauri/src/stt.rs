//! Speech to text through transcribe-cpp (Parakeet TDT by default; Whisper GGML/GGUF also load).
//!
//! One model and one session stay resident and are reused for every utterance.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;

use transcribe_cpp::{Backend, DeviceType, ErrorKind, Feature, Model, ModelOptions, RunOptions, Session};

pub const SAMPLE_RATE: u32 = 16_000;

/// Words the recognizer should prefer when a model can be biased.
const VOCABULARY: &str = "Otto";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceKind {
    Discrete,
    Integrated,
    Cpu,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    /// Human-readable hardware name, e.g. "NVIDIA GeForce RTX 4060 Laptop GPU".
    pub name: String,
    pub kind: DeviceKind,
    /// ggml registry name ("Vulkan0", "CPU", "Metal"), used to find the device again.
    ggml_name: String,
}

impl DeviceInfo {
    /// The CPU, for when enumeration found nothing (backend init failed).
    pub fn cpu() -> Self {
        DeviceInfo { name: "CPU".into(), kind: DeviceKind::Cpu, ggml_name: "CPU".into() }
    }

    /// A made-up device for tests that never reach the native library.
    #[cfg(test)]
    pub fn fake(name: &str, kind: DeviceKind) -> Self {
        DeviceInfo { name: name.into(), kind, ggml_name: name.into() }
    }
}

fn kind_of(d: &transcribe_cpp::Device) -> Option<DeviceKind> {
    match d.device_type {
        DeviceType::Cpu => Some(DeviceKind::Cpu),
        DeviceType::Gpu => Some(DeviceKind::Discrete),
        DeviceType::Igpu => Some(DeviceKind::Integrated),
        // Host-memory accelerators (BLAS) are not a device a user picks.
        DeviceType::Accel => None,
        DeviceType::Unknown if d.kind == "cpu" => Some(DeviceKind::Cpu),
        // A GPU class newer than this binding: treat it as the cautious choice.
        DeviceType::Unknown => Some(DeviceKind::Integrated),
    }
}

/// Load the compute backends (CPU variants, Vulkan on Windows). Call once at startup, before
/// `devices()` or `Stt::load`. Never panics: a PC without a Vulkan loader just gets the CPU.
/// Later calls are no-ops (the native registry can't be re-initialized).
pub fn init_backends() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(init_backends_once);
}

fn init_backends_once() {
    let t = Instant::now();
    let result = init_backends_inner();
    match result {
        Ok(()) => {
            let names: Vec<String> = devices().iter().map(|d| format!("{} ({:?})", d.name, d.kind)).collect();
            println!("[otto] speech backends ready in {} ms: {}", t.elapsed().as_millis(), names.join(", "));
        }
        Err(e) => eprintln!("[otto] speech backends failed to load ({e}); continuing, speech may be unavailable"),
    }
}

fn init_backends_inner() -> Result<(), String> {
    // Windows ships the backend modules next to Otto.exe (and build.rs stages them next to
    // dev builds and test binaries), so point the loader there explicitly.
    #[cfg(windows)]
    {
        quiet_dll_errors();
        if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
            return transcribe_cpp::init_backends(&dir).map_err(|e| format!("{e} (looked in {})", dir.display()));
        }
    }
    // Static builds (Metal, CPU): a no-op; the backends are compiled in.
    transcribe_cpp::init_backends_default().map_err(|e| e.to_string())
}

/// A broken GPU driver (missing or old vulkan-1.dll) must fail ggml-vulkan.dll's load quietly
/// instead of popping a modal "entry point not found" / "DLL missing" dialog. Process-wide,
/// added to whatever mode is already set.
#[cfg(windows)]
fn quiet_dll_errors() {
    const SEM_FAILCRITICALERRORS: u32 = 0x0001;
    const SEM_NOOPENFILEERRORBOX: u32 = 0x8000;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn SetErrorMode(mode: u32) -> u32;
    }
    // SAFETY: SetErrorMode only swaps a process flag word; setting it returns the old value,
    // which is the documented way to read and combine the flags.
    unsafe {
        let old = SetErrorMode(SEM_FAILCRITICALERRORS | SEM_NOOPENFILEERRORBOX);
        SetErrorMode(old | SEM_FAILCRITICALERRORS | SEM_NOOPENFILEERRORBOX);
    }
}

/// Compute devices for speech, in registry order (discrete GPU, integrated GPU, CPU).
pub fn devices() -> Vec<DeviceInfo> {
    transcribe_cpp::devices()
        .iter()
        .filter_map(|d| {
            Some(DeviceInfo {
                name: if d.description.is_empty() { d.name.clone() } else { d.description.clone() },
                kind: kind_of(d)?,
                ggml_name: d.name.clone(),
            })
        })
        .collect()
}

fn user_models_dir() -> PathBuf {
    dirs::data_dir().unwrap_or_default().join("app.otto.desktop/models")
}

/// A model the user placed in `<data_dir>/app.otto.desktop/models/` wins (lets them swap
/// models); otherwise the one bundled with the app. If neither exists, the user path.
pub fn model_path(name: &str) -> PathBuf {
    // Bundled with the app: Otto.app/Contents/Resources on a Mac, next to Otto.exe on Windows.
    #[cfg(target_os = "macos")]
    let resources = |exe: &Path| Some(exe.parent()?.parent()?.join("Resources"));
    #[cfg(not(target_os = "macos"))]
    let resources = |exe: &Path| exe.parent().map(Path::to_path_buf);
    let bundled = std::env::current_exe().ok().and_then(|exe| Some(resources(&exe)?.join("models")));
    resolve_model(&user_models_dir(), bundled.as_deref(), name)
}

/// User folder, then bundled folder, then the user path (so errors name where to put it).
fn resolve_model(user_dir: &Path, bundled_dir: Option<&Path>, name: &str) -> PathBuf {
    let user = user_dir.join(name);
    if user.is_file() {
        return user;
    }
    bundled_dir.map(|d| d.join(name)).filter(|p| p.is_file()).unwrap_or(user)
}

pub struct Stt {
    model: Model,
    session: Mutex<Session>,
    options: RunOptions,
    device: DeviceInfo,
    path: PathBuf,
}

/// Biasing toward "Otto": vocabulary when the model takes it (Whisper), else a context
/// prompt when the model has a conditioning slot, else nothing (Parakeet supports neither). ITN is never set:
/// no supported model has a toggle, so Otto normalizes numbers itself.
fn run_options(model: &Model) -> RunOptions {
    if model.supports(Feature::Vocabulary) {
        RunOptions { vocabulary: vec![VOCABULARY.into()], ..Default::default() }
    } else if model.supports(Feature::ContextPrompt) {
        RunOptions { prompt: Some(VOCABULARY.into()), ..Default::default() }
    } else {
        RunOptions::default()
    }
}

fn backend_for(d: &transcribe_cpp::Device) -> Option<Backend> {
    Some(match d.kind.as_str() {
        "vulkan" => Backend::Vulkan,
        "metal" => Backend::Metal,
        "cuda" => Backend::Cuda,
        "rocm" => Backend::Rocm,
        _ => return None,
    })
}

impl Stt {
    /// Loads the model on `device` (no silent fallback) and warms it up, so the first
    /// utterance doesn't pay for GPU pipeline compilation.
    pub fn load(path: &Path, device: &DeviceInfo) -> Result<Self, String> {
        if !path.is_file() {
            return Err(format!("speech model not found at {}", path.display()));
        }
        let options = if device.kind == DeviceKind::Cpu {
            ModelOptions { backend: Backend::Cpu, ..Default::default() }
        } else {
            let d = transcribe_cpp::devices()
                .into_iter()
                .find(|d| d.name == device.ggml_name)
                .ok_or_else(|| format!("speech device {} not found", device.name))?;
            let backend = backend_for(&d).ok_or_else(|| format!("no backend for speech device {} ({})", device.name, d.kind))?;
            ModelOptions { backend, device: Some(d), ..Default::default() }
        };
        let t = Instant::now();
        let model = Model::load_with(path, &options).map_err(|e| format!("could not load speech model: {e}"))?;
        let load_ms = t.elapsed().as_millis();
        let session = model.session().map_err(|e| format!("could not start speech session: {e}"))?;
        let stt = Stt { options: run_options(&model), model, session: Mutex::new(session), device: device.clone(), path: path.to_path_buf() };

        let t = Instant::now();
        let silence = vec![0.0f32; SAMPLE_RATE as usize / 2];
        match stt.run(&silence) {
            Ok(_) => {}
            Err(e) if e.kind() == ErrorKind::InputTooShort => {}
            Err(e) => return Err(format!("speech model warm-up failed: {e}")),
        }
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("?");
        println!(
            "[otto] speech model {name} ({} {}) on {} [{}]: load {load_ms} ms, warm-up {} ms",
            stt.model.arch(),
            stt.model.variant(),
            stt.device.name,
            stt.model.backend(),
            t.elapsed().as_millis()
        );
        Ok(stt)
    }

    /// Like `transcribe`, but keeps the error kind (device fault or bad input).
    pub fn run(&self, samples: &[f32]) -> transcribe_cpp::Result<String> {
        let mut session = self.session.lock().unwrap_or_else(|p| p.into_inner());
        session.run(samples, &self.options).map(|t| t.text.trim().to_string())
    }

    /// `samples` must be mono f32 at 16 kHz.
    pub fn transcribe(&self, samples: &[f32]) -> Result<String, String> {
        self.run(samples).map_err(|e| e.to_string())
    }

    pub fn device(&self) -> &DeviceInfo {
        &self.device
    }

    /// The model file this was loaded from.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::speech_config::DEFAULT_MODEL;

    #[test]
    fn missing_model_is_an_error() {
        let err = Stt::load(Path::new("no-such-model.gguf"), &DeviceInfo::cpu()).err().unwrap();
        assert!(err.contains("not found"), "{err}");
    }

    #[test]
    fn model_path_falls_back_to_user_folder() {
        let p = model_path("no-such-model-xyz.gguf");
        assert!(p.ends_with("app.otto.desktop/models/no-such-model-xyz.gguf"), "{}", p.display());
    }

    #[test]
    fn user_models_folder_wins_over_bundled() {
        let root = std::env::temp_dir().join(format!("otto-model-path-{}", std::process::id()));
        let (user, bundled) = (root.join("user"), root.join("bundled"));
        std::fs::create_dir_all(&user).unwrap();
        std::fs::create_dir_all(&bundled).unwrap();
        let name = "m.gguf";
        // Neither: the user path.
        assert_eq!(resolve_model(&user, Some(&bundled), name), user.join(name));
        // Bundled only.
        std::fs::write(bundled.join(name), b"b").unwrap();
        assert_eq!(resolve_model(&user, Some(&bundled), name), bundled.join(name));
        // Both: the user copy wins.
        std::fs::write(user.join(name), b"u").unwrap();
        assert_eq!(resolve_model(&user, Some(&bundled), name), user.join(name));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn backends_init_and_cpu_listed() {
        init_backends();
        let all = devices();
        assert!(all.iter().any(|d| d.kind == DeviceKind::Cpu), "{all:?}");
    }

    /// Run with: OTTO_TEST_CLIP=G:\otto-work\clips\app01.wav cargo test -- --ignored stt::tests
    /// (app01 is "Hey Otto, open Notion."). Without OTTO_TEST_CLIP it uses macOS `say`.
    /// OTTO_TEST_ENTITY overrides the expected word ("notion").
    #[test]
    #[ignore = "needs the downloaded model and a clip (OTTO_TEST_CLIP) or macOS `say`"]
    fn transcribes_a_spoken_command() {
        let (wav, entity) = match std::env::var_os("OTTO_TEST_CLIP") {
            Some(clip) => (PathBuf::from(clip), std::env::var("OTTO_TEST_ENTITY").unwrap_or_else(|_| "notion".into())),
            None => {
                let wav = std::env::temp_dir().join("otto-stt-test.wav");
                let ok = std::process::Command::new("say")
                    .arg("-o")
                    .arg(&wav)
                    .args(["--file-format=WAVE", "--data-format=LEF32@16000"])
                    .arg("Hey Otto, open Notion.")
                    .status()
                    .expect("run say")
                    .success();
                assert!(ok, "say failed");
                (wav, "notion".to_string())
            }
        };

        let samples = wav_data_f32(&std::fs::read(&wav).unwrap());
        init_backends();
        let cpu = devices().into_iter().find(|d| d.kind == DeviceKind::Cpu).unwrap_or_else(DeviceInfo::cpu);
        // Installed/user copy first, else the one scripts/fetch-model.sh put in the repo.
        // OTTO_TEST_MODEL picks another model file name (e.g. ggml-base.en.bin).
        let name = std::env::var("OTTO_TEST_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.into());
        let path = Some(model_path(&name))
            .filter(|p| p.is_file())
            .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../models").join(&name));
        let stt = Stt::load(&path, &cpu).unwrap();
        let t = Instant::now();
        let text = stt.transcribe(&samples).unwrap().to_lowercase();
        println!("heard {:?} in {} ms on {}", text, t.elapsed().as_millis(), stt.device().name);
        assert!(text.contains(&entity.to_lowercase()), "{text}");
    }

    /// 32-bit float WAV data (plain or WAVE_FORMAT_EXTENSIBLE), 16 kHz mono.
    fn wav_data_f32(b: &[u8]) -> Vec<f32> {
        let mut i = 12;
        while i + 8 <= b.len() {
            let n = u32::from_le_bytes(b[i + 4..i + 8].try_into().unwrap()) as usize;
            if &b[i..i + 4] == b"data" {
                let end = (i + 8 + n).min(b.len());
                return b[i + 8..end]
                    .chunks_exact(4)
                    .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
                    .collect();
            }
            i += 8 + n + (n & 1);
        }
        panic!("no data chunk in WAV");
    }
}
