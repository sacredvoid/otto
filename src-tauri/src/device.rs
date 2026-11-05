//! Which device runs speech: the configured preference, checked by timing a warm run on the
//! GPU against the CPU, plus the runtime rule that a failing GPU hands the work to the CPU.

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use crate::speech_config::DevicePref;
use transcribe_cpp::ErrorKind;

use crate::stt::{DeviceInfo, DeviceKind, Stt, SAMPLE_RATE};

/// Devices worth trying, best first, always ending with the CPU.
/// Auto: discrete GPUs, then integrated (Apple's Metal GPU may list as either), then CPU.
/// Cpu: the CPU only. Named: the first GPU whose name contains the text (any case), then CPU.
pub fn candidates(pref: &DevicePref, all: &[DeviceInfo]) -> Vec<DeviceInfo> {
    let of = |kind| all.iter().filter(move |d| d.kind == kind).cloned();
    let mut out: Vec<DeviceInfo> = match pref {
        DevicePref::Auto => of(DeviceKind::Discrete).chain(of(DeviceKind::Integrated)).collect(),
        DevicePref::Cpu => vec![],
        DevicePref::Named(want) => {
            let want = want.to_lowercase();
            all.iter().filter(|d| d.kind != DeviceKind::Cpu && d.name.to_lowercase().contains(&want)).take(1).cloned().collect()
        }
    };
    out.push(of(DeviceKind::Cpu).next().unwrap_or_else(DeviceInfo::cpu));
    out
}

/// Whether Auto races the GPU against the CPU. Windows PCs can have a weak iGPU or a broken
/// driver, so the race guards them. A Mac has one GPU (Metal), which is the right choice there.
const RACE_CPU_IN_AUTO: bool = !cfg!(target_os = "macos");

/// Picks the device to run speech on.
/// Auto on Windows/Linux: the CPU is timed first, then the first GPU candidate; the GPU is kept
/// only when strictly faster (a tie keeps the CPU). Auto on macOS, and Named: the GPU is used
/// without a race; it only has to load and run. Any GPU error gives the CPU. Nothing is timed
/// when there's no GPU candidate.
pub fn choose(pref: &DevicePref, all: &[DeviceInfo], time_warmup: impl FnMut(&DeviceInfo) -> Result<u128, String>) -> DeviceInfo {
    choose_with(pref, all, RACE_CPU_IN_AUTO, time_warmup)
}

fn choose_with(
    pref: &DevicePref,
    all: &[DeviceInfo],
    race_cpu_in_auto: bool,
    mut time_warmup: impl FnMut(&DeviceInfo) -> Result<u128, String>,
) -> DeviceInfo {
    let list = candidates(pref, all);
    let cpu = list.last().cloned().unwrap_or_else(DeviceInfo::cpu);
    let Some(gpu) = list.into_iter().find(|d| d.kind != DeviceKind::Cpu) else {
        return cpu;
    };
    let race = race_cpu_in_auto && matches!(pref, DevicePref::Auto);
    if !race {
        return match time_warmup(&gpu) {
            Ok(ms) => {
                let why = if matches!(pref, DevicePref::Named(_)) { "named in config" } else { "the only GPU path here" };
                println!("[otto] speech device check: {} {ms} ms, using it ({why})", gpu.name);
                gpu
            }
            Err(e) => {
                println!("[otto] speech moved to CPU: {} failed its check: {e}", gpu.name);
                cpu
            }
        };
    }
    // CPU first, so its model can be dropped before the GPU's loads (one model in memory at a time).
    let cpu_ms = time_warmup(&cpu);
    let gpu_ms = match time_warmup(&gpu) {
        Ok(ms) => ms,
        Err(e) => {
            println!("[otto] speech moved to CPU: {} failed its check: {e}", gpu.name);
            return cpu;
        }
    };
    match cpu_ms {
        Ok(cpu_ms) if gpu_ms < cpu_ms => {
            println!("[otto] speech device check: {} {gpu_ms} ms vs CPU {cpu_ms} ms, using {}", gpu.name, gpu.name);
            gpu
        }
        Ok(cpu_ms) => {
            println!("[otto] speech device check: {} {gpu_ms} ms vs CPU {cpu_ms} ms, using CPU", gpu.name);
            cpu
        }
        // The CPU couldn't run the check, but the GPU did: keep the one that works.
        Err(e) => {
            println!("[otto] speech device check: {} {gpu_ms} ms, CPU failed ({e}), using {}", gpu.name, gpu.name);
            gpu
        }
    }
}

/// Times the second transcription of ~2 s of quiet noise. The first run of a new input shape
/// compiles GPU pipelines (up to 13 s on first use; Stt::load's 0.5 s warm-up is a different
/// shape), so it runs once untimed and only the repeat is counted.
pub fn time_transcribe(stt: &Stt) -> Result<u128, String> {
    // Low-level deterministic noise: closer to a real (quiet) recording than exact zeros.
    let mut x: u32 = 0x9E37_79B9;
    let noise: Vec<f32> = (0..SAMPLE_RATE as usize * 2)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            (x as f32 / u32::MAX as f32 - 0.5) * 0.002
        })
        .collect();
    stt.transcribe(&noise)?;
    let t = Instant::now();
    stt.transcribe(&noise)?;
    Ok(t.elapsed().as_millis())
}

/// Loads the model on the device `pref` and the timed check pick. Only one model is in memory
/// at a time: the CPU's is dropped right after its check, the GPU's is kept, and the CPU is
/// loaded again (~300 ms) only if it wins. Slow (seconds): call off the UI thread.
pub fn load_best(path: &Path, pref: &DevicePref, all: &[DeviceInfo]) -> Result<Stt, String> {
    let mut gpu_model: Option<Stt> = None;
    let chosen = choose(pref, all, |d| {
        let stt = Stt::load(path, d)?;
        let ms = time_transcribe(&stt)?;
        if d.kind != DeviceKind::Cpu {
            gpu_model = Some(stt);
        }
        Ok(ms)
    });
    match gpu_model.filter(|s| *s.device() == chosen) {
        Some(stt) => Ok(stt),
        None => Stt::load(path, &chosen),
    }
}

/// A failed transcription. `device_fault` means the compute device itself failed (backend
/// error, out of memory, unknown native failure), not that this input was unusable.
#[derive(Debug, Clone, PartialEq)]
pub struct SpeechError {
    pub message: String,
    pub device_fault: bool,
}

/// Only backend-class failures justify leaving the GPU; input problems (too short, too long,
/// aborted, repetitive output) would fail on the CPU too.
pub fn is_device_fault(kind: ErrorKind) -> bool {
    matches!(kind, ErrorKind::Backend | ErrorKind::OutOfMemory | ErrorKind::Other)
}

/// Something that turns 16 kHz mono samples into text.
pub trait Transcriber {
    fn transcribe(&self, samples: &[f32]) -> Result<String, SpeechError>;
    fn on_gpu(&self) -> bool;
}

impl Transcriber for Stt {
    fn transcribe(&self, samples: &[f32]) -> Result<String, SpeechError> {
        self.run(samples).map_err(|e| SpeechError { device_fault: is_device_fault(e.kind()), message: e.to_string() })
    }
    fn on_gpu(&self) -> bool {
        self.device().kind != DeviceKind::Cpu
    }
}

impl<T: Transcriber + ?Sized> Transcriber for Arc<T> {
    fn transcribe(&self, samples: &[f32]) -> Result<String, SpeechError> {
        (**self).transcribe(samples)
    }
    fn on_gpu(&self) -> bool {
        (**self).on_gpu()
    }
}

/// Transcribes on `stt`; if that is a GPU and the device itself failed, `reload_cpu` (given the
/// GPU's error) supplies a CPU transcriber and the same samples are tried once more there.
/// An input error fails this utterance only and leaves the GPU in place.
pub fn transcribe_with_fallback(
    stt: &dyn Transcriber,
    reload_cpu: impl FnOnce(&str) -> Result<Box<dyn Transcriber>, String>,
    samples: &[f32],
) -> Result<String, String> {
    match stt.transcribe(samples) {
        Ok(text) => Ok(text),
        Err(e) if e.device_fault && stt.on_gpu() => reload_cpu(&e.message)?.transcribe(samples).map_err(|e| e.message),
        Err(e) => Err(e.message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn intel() -> DeviceInfo {
        DeviceInfo::fake("Intel(R) UHD Graphics", DeviceKind::Integrated)
    }
    fn rtx() -> DeviceInfo {
        DeviceInfo::fake("NVIDIA GeForce RTX 4060 Laptop GPU", DeviceKind::Discrete)
    }
    fn cpu() -> DeviceInfo {
        DeviceInfo::fake("13th Gen Intel(R) Core(TM) i7-13700H", DeviceKind::Cpu)
    }

    /// A timer that answers from a table and records which devices it was asked about.
    fn timer<'a>(table: &'a [(&'a str, Result<u128, String>)], asked: &'a RefCell<Vec<String>>) -> impl FnMut(&DeviceInfo) -> Result<u128, String> + 'a {
        move |d| {
            asked.borrow_mut().push(d.name.clone());
            table.iter().find(|(n, _)| d.name.contains(n)).map(|(_, r)| r.clone()).unwrap_or(Err("not in table".into()))
        }
    }

    /// `choose` as it runs on Windows and Linux (Auto races the CPU), on any build target.
    fn race(pref: &DevicePref, all: &[DeviceInfo], t: impl FnMut(&DeviceInfo) -> Result<u128, String>) -> DeviceInfo {
        choose_with(pref, all, true, t)
    }

    /// `choose` as it runs on macOS (Auto takes the GPU without a race).
    fn mac(pref: &DevicePref, all: &[DeviceInfo], t: impl FnMut(&DeviceInfo) -> Result<u128, String>) -> DeviceInfo {
        choose_with(pref, all, false, t)
    }

    #[test]
    fn auto_races_cpu_off_macos_only() {
        assert_eq!(RACE_CPU_IN_AUTO, !cfg!(target_os = "macos"));
    }

    fn names(ds: &[DeviceInfo]) -> Vec<String> {
        ds.iter().map(|d| d.name.clone()).collect()
    }

    #[test]
    fn auto_prefers_discrete_over_integrated() {
        let c = candidates(&DevicePref::Auto, &[intel(), rtx(), cpu()]);
        assert_eq!(c, vec![rtx(), intel(), cpu()]);
    }

    #[test]
    fn auto_drops_gpu_slower_than_cpu() {
        let asked = RefCell::new(vec![]);
        let table = [("RTX", Ok(900)), ("Core", Ok(400))];
        assert_eq!(race(&DevicePref::Auto, &[intel(), rtx(), cpu()], timer(&table, &asked)), cpu());
        // CPU first (so only one model is loaded at a time), then only the first GPU candidate.
        assert_eq!(*asked.borrow(), names(&[cpu(), rtx()]));
    }

    #[test]
    fn auto_keeps_faster_gpu() {
        let asked = RefCell::new(vec![]);
        let table = [("RTX", Ok(80)), ("Core", Ok(400))];
        assert_eq!(race(&DevicePref::Auto, &[intel(), rtx(), cpu()], timer(&table, &asked)), rtx());
        assert_eq!(*asked.borrow(), names(&[cpu(), rtx()]));
    }

    #[test]
    fn auto_tie_keeps_cpu() {
        let asked = RefCell::new(vec![]);
        let table = [("RTX", Ok(400)), ("Core", Ok(400))];
        assert_eq!(race(&DevicePref::Auto, &[rtx(), cpu()], timer(&table, &asked)), cpu());
    }

    #[test]
    fn auto_falls_back_to_cpu_when_no_gpu_listed() {
        let asked = RefCell::new(vec![]);
        assert_eq!(race(&DevicePref::Auto, &[cpu()], timer(&[], &asked)), cpu());
        assert!(asked.borrow().is_empty(), "{:?}", asked.borrow());
    }

    #[test]
    fn auto_with_nothing_listed_gives_cpu() {
        let asked = RefCell::new(vec![]);
        assert_eq!(race(&DevicePref::Auto, &[], timer(&[], &asked)).kind, DeviceKind::Cpu);
        assert!(asked.borrow().is_empty());
    }

    #[test]
    fn gpu_warmup_error_gives_cpu() {
        let asked = RefCell::new(vec![]);
        let table = [("RTX", Err("vulkan backend requested but not available".to_string())), ("Core", Ok(400))];
        assert_eq!(race(&DevicePref::Auto, &[rtx(), cpu()], timer(&table, &asked)), cpu());
    }

    #[test]
    fn cpu_check_error_keeps_working_gpu() {
        let asked = RefCell::new(vec![]);
        let table = [("RTX", Ok(900)), ("Core", Err("cpu failed".to_string()))];
        assert_eq!(race(&DevicePref::Auto, &[rtx(), cpu()], timer(&table, &asked)), rtx());
    }

    #[test]
    fn named_device_missing_gives_cpu() {
        let asked = RefCell::new(vec![]);
        let table = [("RTX", Ok(80)), ("Core", Ok(400))];
        let pref = DevicePref::Named("Radeon".into());
        assert_eq!(candidates(&pref, &[rtx(), cpu()]), vec![cpu()]);
        assert_eq!(race(&pref, &[rtx(), cpu()], timer(&table, &asked)), cpu());
        assert!(asked.borrow().is_empty(), "{:?}", asked.borrow());
    }

    #[test]
    fn named_device_is_honoured_without_cpu_race() {
        let asked = RefCell::new(vec![]);
        // Slower than the CPU, but the user asked for it.
        let table = [("UHD", Ok(300)), ("RTX", Ok(80)), ("Core", Ok(100))];
        // "intel" also appears in the CPU's name; only GPUs match a name.
        let pref = DevicePref::Named("intel".into());
        assert_eq!(candidates(&pref, &[rtx(), intel(), cpu()]), vec![intel(), cpu()]);
        assert_eq!(race(&pref, &[rtx(), intel(), cpu()], timer(&table, &asked)), intel());
        assert_eq!(*asked.borrow(), names(&[intel()]));
    }

    #[test]
    fn named_device_error_gives_cpu() {
        let asked = RefCell::new(vec![]);
        let table = [("UHD", Err("warm-up failed".to_string())), ("Core", Ok(100))];
        let pref = DevicePref::Named("UHD".into());
        assert_eq!(race(&pref, &[intel(), cpu()], timer(&table, &asked)), cpu());
    }

    #[test]
    fn cpu_pref_never_times_gpu() {
        let asked = RefCell::new(vec![]);
        let table = [("RTX", Ok(1)), ("Core", Ok(400))];
        assert_eq!(candidates(&DevicePref::Cpu, &[intel(), rtx(), cpu()]), vec![cpu()]);
        assert_eq!(race(&DevicePref::Cpu, &[intel(), rtx(), cpu()], timer(&table, &asked)), cpu());
        assert!(asked.borrow().is_empty(), "{:?}", asked.borrow());
    }

    /// Apple Silicon: one Metal GPU (unified memory, may be listed as integrated) and the CPU.
    /// Auto takes Metal without timing the CPU, even if the CPU would have won the race.
    #[test]
    fn mac_auto_takes_lone_integrated_gpu_without_race() {
        let metal = DeviceInfo::fake("Apple M2", DeviceKind::Integrated);
        let mac_cpu = DeviceInfo::fake("CPU", DeviceKind::Cpu);
        let asked = RefCell::new(vec![]);
        let table = [("Apple", Ok(600)), ("CPU", Ok(500))];
        assert_eq!(mac(&DevicePref::Auto, &[metal.clone(), mac_cpu], timer(&table, &asked)), metal);
        assert_eq!(*asked.borrow(), vec!["Apple M2".to_string()]);
    }

    #[test]
    fn mac_auto_metal_error_gives_cpu() {
        let metal = DeviceInfo::fake("Apple M2", DeviceKind::Discrete);
        let mac_cpu = DeviceInfo::fake("CPU", DeviceKind::Cpu);
        let asked = RefCell::new(vec![]);
        let table = [("Apple", Err("metal warm-up failed".to_string())), ("CPU", Ok(500))];
        assert_eq!(mac(&DevicePref::Auto, &[metal, mac_cpu.clone()], timer(&table, &asked)), mac_cpu);
    }

    #[test]
    fn mac_cpu_pref_still_cpu() {
        let metal = DeviceInfo::fake("Apple M2", DeviceKind::Integrated);
        let mac_cpu = DeviceInfo::fake("CPU", DeviceKind::Cpu);
        let asked = RefCell::new(vec![]);
        assert_eq!(mac(&DevicePref::Cpu, &[metal, mac_cpu.clone()], timer(&[], &asked)), mac_cpu);
        assert!(asked.borrow().is_empty());
    }

    #[test]
    fn device_fault_kinds() {
        assert!(is_device_fault(ErrorKind::Backend));
        assert!(is_device_fault(ErrorKind::OutOfMemory));
        assert!(!is_device_fault(ErrorKind::InputTooShort));
        assert!(!is_device_fault(ErrorKind::InputTooLong));
        assert!(!is_device_fault(ErrorKind::Aborted));
    }

    struct Fake {
        gpu: bool,
        answer: Result<String, SpeechError>,
    }
    impl Transcriber for Fake {
        fn transcribe(&self, _: &[f32]) -> Result<String, SpeechError> {
            self.answer.clone()
        }
        fn on_gpu(&self) -> bool {
            self.gpu
        }
    }
    fn fault(m: &str) -> SpeechError {
        SpeechError { message: m.into(), device_fault: true }
    }
    fn input_error(m: &str) -> SpeechError {
        SpeechError { message: m.into(), device_fault: false }
    }

    #[test]
    fn runtime_error_reloads_on_cpu() {
        let gpu = Fake { gpu: true, answer: Err(fault("vulkan device lost")) };
        let reason = RefCell::new(String::new());
        let text = transcribe_with_fallback(
            &gpu,
            |why| {
                *reason.borrow_mut() = why.to_string();
                Ok(Box::new(Fake { gpu: false, answer: Ok("open notion".into()) }))
            },
            &[0.0; 16],
        );
        assert_eq!(text, Ok("open notion".into()));
        assert_eq!(*reason.borrow(), "vulkan device lost");
    }

    #[test]
    fn gpu_input_error_does_not_reload() {
        let gpu = Fake { gpu: true, answer: Err(input_error("input too short")) };
        let text = transcribe_with_fallback(&gpu, |_| panic!("must not reload"), &[0.0; 16]);
        assert_eq!(text, Err("input too short".into()));
    }

    #[test]
    fn failed_cpu_reload_fails_this_utterance() {
        let gpu = Fake { gpu: true, answer: Err(fault("backend error")) };
        let text = transcribe_with_fallback(&gpu, |_| Err("could not load speech model".into()), &[0.0; 16]);
        assert_eq!(text, Err("could not load speech model".into()));
    }

    #[test]
    fn cpu_error_is_not_retried() {
        let cpu = Fake { gpu: false, answer: Err(fault("bad audio")) };
        let text = transcribe_with_fallback(&cpu, |_| panic!("must not reload"), &[0.0; 16]);
        assert_eq!(text, Err("bad audio".into()));
    }

    #[test]
    fn working_gpu_is_not_reloaded() {
        let gpu = Fake { gpu: true, answer: Ok("hi".into()) };
        assert_eq!(transcribe_with_fallback(&gpu, |_| panic!("must not reload"), &[0.0; 16]), Ok("hi".into()));
    }

    /// Run with: cargo test real_device_choice -- --ignored --nocapture
    /// Prints the devices, each timed check and the pick for Auto and for "Intel".
    #[test]
    #[ignore = "needs the downloaded model and real GPUs"]
    fn real_device_choice() {
        use crate::speech_config::DEFAULT_MODEL;
        crate::stt::init_backends();
        let all = crate::stt::devices();
        println!("devices: {all:?}");
        let path = Some(crate::stt::model_path(DEFAULT_MODEL))
            .filter(|p| p.is_file())
            .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../models").join(DEFAULT_MODEL));
        for pref in [DevicePref::Auto, DevicePref::Named("Intel".into())] {
            let t = Instant::now();
            let stt = load_best(&path, &pref, &all).unwrap();
            println!("{pref:?} -> {} (choice took {} ms)", stt.device().name, t.elapsed().as_millis());
            println!("{pref:?} -> again: {} ms", time_transcribe(&stt).unwrap());
        }
    }
}
