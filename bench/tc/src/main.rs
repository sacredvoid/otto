//! Candidate bench: transcribe-cpp (Parakeet GGUF) raw, no VAD, no vocabulary, default RunOptions.
//!
//! usage: tc <commands.tsv> <clips_dir> <out_dir> --models <gguf>... [--devices cpu,nvidia,intel]
//! Each model x device runs in a child process (`--one`) so peak memory is isolated.
use std::path::{Path, PathBuf};
use std::process::Command as Proc;
use std::time::Instant;
use stt_bench_common::{
    Kind, Row, entity_hit, load_commands, peak_memory_mb, read_wav_16k_mono, wer, write_reports,
};
use transcribe_cpp::{Backend, Device, Model, ModelOptions, RunOptions, devices, init_backends_default};

const PASSES: usize = 3;
const WARMUPS: usize = 2;
const EXIT_NO_DEVICE: i32 = 3;

/// "parakeet-tdt-0.6b-v2-Q8_0.gguf" -> "parakeet-0.6b-v2 Q8_0"; "parakeet-tdt_ctc-110m-Q8_0.gguf" -> "parakeet-110m Q8_0".
fn model_label(path: &Path) -> String {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("model");
    let (name, quant) = match stem.rsplit_once('-') {
        Some((n, q)) => (n, q),
        None => (stem, ""),
    };
    let name = name.replace("-tdt_ctc", "").replace("-tdt", "");
    format!("{name} {quant}").trim().to_string()
}

/// Case-insensitive substring match: `cpu` -> the CPU device; anything else -> a Vulkan
/// device whose hardware description (or name) contains it.
fn pick_device(want: &str) -> Option<(Backend, Option<Device>)> {
    let want = want.to_lowercase();
    let all = devices();
    if want == "cpu" {
        return all.iter().find(|d| d.kind == "cpu").map(|_| (Backend::Cpu, None));
    }
    all.into_iter()
        .filter(|d| d.kind != "cpu")
        .find(|d| d.description.to_lowercase().contains(&want) || d.name.to_lowercase().contains(&want))
        .map(|d| (Backend::Vulkan, Some(d)))
}

fn transcribe(session: &mut transcribe_cpp::Session, pcm: &[f32]) -> Result<String, String> {
    session
        .run(pcm, &RunOptions::default())
        .map(|t| t.text.trim().to_string())
        .map_err(|e| e.to_string())
}

/// Child: run one model x device, write {rows, peak_mb, first_run_ms, ...} JSON to `out_json`.
fn run_one(a: &[String]) -> Result<(), String> {
    let [model_path, dev, tsv, clips, out_json] = a else {
        return Err("--one <model> <device> <tsv> <clips> <out.json>".into());
    };
    let model_path = Path::new(model_path);
    let label = model_label(model_path);
    let commands = load_commands(Path::new(tsv))?;
    init_backends_default().map_err(|e| e.to_string())?;
    let Some((backend, device)) = pick_device(dev) else {
        eprintln!("warning: device '{dev}' not found, skipping {label}");
        std::process::exit(EXIT_NO_DEVICE);
    };
    let t = Instant::now();
    let model = Model::load_with(model_path, &ModelOptions { backend, device, ..Default::default() })
        .map_err(|e| format!("load {}: {e}", model_path.display()))?;
    let load_ms = t.elapsed().as_millis();
    let backend_name = model.backend().to_string();
    let mut session = model.session().map_err(|e| e.to_string())?;

    let mut rows = Vec::new();
    let mut skipped = 0usize;
    let mut first_run_ms: Option<u128> = None;
    for c in &commands {
        let wav = Path::new(clips).join(format!("{}.wav", c.id));
        if !wav.is_file() {
            eprintln!("warning: missing {}, skipping", wav.display());
            skipped += 1;
            continue;
        }
        let pcm = read_wav_16k_mono(&wav)?;
        if first_run_ms.is_none() {
            // First GPU run compiles Vulkan pipelines (seconds); keep it out of Row.ms.
            for i in 0..WARMUPS {
                let t = Instant::now();
                transcribe(&mut session, &pcm)?;
                if i == 0 {
                    first_run_ms = Some(t.elapsed().as_millis());
                }
            }
        }
        let mut times = Vec::with_capacity(PASSES);
        let mut text = String::new();
        for _ in 0..PASSES {
            let t = Instant::now();
            text = transcribe(&mut session, &pcm)?;
            times.push(t.elapsed().as_millis());
        }
        times.sort_unstable();
        let ms = times[PASSES / 2];
        let (w, hit) = if c.kind == Kind::Speech {
            (wer(&c.text, &text), entity_hit(&c.entity, &text))
        } else {
            (0.0, false)
        };
        println!("[{label} {dev}] {} {ms}ms {text:?}", c.id);
        rows.push(serde_json::json!({
            "id": c.id, "silence": c.kind == Kind::Silence, "ms": ms as u64,
            "text": text, "wer": w, "hit": hit,
        }));
    }
    let out = serde_json::json!({
        "label": label, "device": dev, "backend": backend_name, "load_ms": load_ms as u64,
        "first_run_ms": first_run_ms.map(|v| v as u64), "skipped": skipped,
        "peak_mb": peak_memory_mb(), "rows": rows,
    });
    std::fs::write(out_json, out.to_string()).map_err(|e| e.to_string())
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--one") {
        return run_one(&args[1..]);
    }
    let mut pos = Vec::new();
    let mut models: Vec<PathBuf> = Vec::new();
    let mut devs = "cpu,nvidia,intel".to_string();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--models" => {
                i += 1;
                while i < args.len() && !args[i].starts_with("--") {
                    models.push(PathBuf::from(&args[i]));
                    i += 1;
                }
            }
            "--devices" => {
                devs = args.get(i + 1).ok_or("--devices needs a list")?.clone();
                i += 2;
            }
            _ => {
                pos.push(args[i].clone());
                i += 1;
            }
        }
    }
    if pos.len() != 3 || models.is_empty() {
        return Err("usage: tc <commands.tsv> <clips_dir> <out_dir> --models <gguf>... [--devices cpu,nvidia,intel]".into());
    }
    let (tsv, clips, out) = (&pos[0], &pos[1], Path::new(&pos[2]));
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let tmp = std::env::temp_dir();
    let commands = load_commands(Path::new(tsv))?;

    let mut rows: Vec<Row> = Vec::new();
    let mut mem: Vec<(String, String, u64)> = Vec::new();
    let mut first_runs = String::from("model,device,backend,load_ms,first_run_ms,skipped\n");
    for m in &models {
        if !m.is_file() {
            return Err(format!("model not found: {}", m.display()));
        }
        for dev in devs.split(',').map(str::trim).filter(|d| !d.is_empty()) {
            let out_json = tmp.join(format!("tc-bench-{}-{dev}.json", std::process::id()));
            let _ = std::fs::remove_file(&out_json);
            let status = Proc::new(&exe)
                .arg("--one")
                .arg(m)
                .args([dev, tsv, clips])
                .arg(&out_json)
                .status()
                .map_err(|e| e.to_string())?;
            match status.code() {
                Some(0) => {}
                Some(EXIT_NO_DEVICE) => {
                    eprintln!("warning: no '{dev}' device, skipped {}", m.display());
                    continue;
                }
                _ => {
                    eprintln!("warning: {} on {dev} failed ({status}), skipped", m.display());
                    continue;
                }
            }
            let v: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&out_json).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
            let label = v["label"].as_str().unwrap_or("").to_string();
            println!(
                "== {label} / {dev} ({}): load {} ms, first_run {} ms, peak {} MB, {} skipped",
                v["backend"], v["load_ms"], v["first_run_ms"], v["peak_mb"], v["skipped"]
            );
            first_runs.push_str(&format!(
                "{label},{dev},{},{},{},{}\n",
                v["backend"].as_str().unwrap_or(""),
                v["load_ms"],
                v["first_run_ms"],
                v["skipped"]
            ));
            mem.push((label.clone(), dev.to_string(), v["peak_mb"].as_u64().unwrap_or(0)));
            for r in v["rows"].as_array().into_iter().flatten() {
                let id = r["id"].as_str().unwrap_or("").to_string();
                let kind = commands.iter().find(|c| c.id == id).map(|c| c.kind).unwrap_or(Kind::Speech);
                rows.push(Row {
                    model: label.clone(),
                    device: dev.to_string(),
                    id,
                    kind,
                    ms: r["ms"].as_u64().unwrap_or(0) as u128,
                    text: r["text"].as_str().unwrap_or("").to_string(),
                    wer: r["wer"].as_f64().unwrap_or(0.0),
                    hit: r["hit"].as_bool().unwrap_or(false),
                });
            }
            let _ = std::fs::remove_file(&out_json);
        }
    }
    write_reports(&rows, &mem, out)?;
    std::fs::write(out.join("first_run_ms.csv"), first_runs).map_err(|e| e.to_string())?;
    println!("done: {} rows -> {}", rows.len(), out.display());
    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
