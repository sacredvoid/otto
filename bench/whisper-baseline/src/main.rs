//! Baseline: whisper-rs 0.16 + ggml-base.en exactly as Otto ships it (src-tauri/src/stt.rs).
use std::path::{Path, PathBuf};
use std::time::Instant;
use stt_bench_common::{
    Kind, Row, entity_hit, load_commands, peak_memory_mb, read_wav_16k_mono, wer, write_reports,
};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

const MODEL_NAME: &str = "base.en";
const PASSES: usize = 3;

#[cfg(target_os = "macos")]
const DEVICE: &str = "metal";
#[cfg(not(target_os = "macos"))]
const DEVICE: &str = "cpu";

/// Mirrors `Stt::transcribe`: fresh state per call, identical FullParams.
fn transcribe(ctx: &WhisperContext, samples: &[f32]) -> Result<String, String> {
    let mut state = ctx.create_state().map_err(|e| e.to_string())?;
    let mut p = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    p.set_language(Some("en"));
    p.set_initial_prompt("Otto");
    p.set_suppress_blank(true);
    p.set_print_progress(false);
    p.set_print_realtime(false);
    p.set_print_special(false);
    p.set_print_timestamps(false);
    state.full(p, samples).map_err(|e| e.to_string())?;
    let mut text = String::new();
    for seg in state.as_iter() {
        text.push_str(&seg.to_str_lossy().map_err(|e| e.to_string())?);
    }
    Ok(text.trim().to_string())
}

fn run() -> Result<(), String> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut model: Option<PathBuf> = None;
    if let Some(i) = args.iter().position(|a| a == "--model") {
        if i + 1 >= args.len() {
            return Err("--model needs a path".into());
        }
        model = Some(PathBuf::from(args.remove(i + 1)));
        args.remove(i);
    }
    let mut label = MODEL_NAME.to_string();
    if let Some(i) = args.iter().position(|a| a == "--label") {
        if i + 1 >= args.len() {
            return Err("--label needs a name".into());
        }
        label = args.remove(i + 1);
        args.remove(i);
    }
    if args.len() != 3 {
        return Err("usage: whisper-baseline <commands.tsv> <clips_dir> <out_dir> [--model <path>] [--label <name>]".into());
    }
    let (tsv, clips, out) = (Path::new(&args[0]), Path::new(&args[1]), Path::new(&args[2]));
    // Default: the repo's bundled model (bench/ is one level below the repo root).
    let model = model.unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../models/ggml-base.en.bin"));
    if !model.is_file() {
        return Err(format!("model not found at {}", model.display()));
    }

    let commands = load_commands(tsv)?;
    whisper_rs::install_logging_hooks();
    let ctx = WhisperContext::new_with_params(
        model.to_str().ok_or("model path is not UTF-8")?,
        WhisperContextParameters::default(),
    )
    .map_err(|e| format!("could not load speech model: {e}"))?;

    let mut rows = Vec::new();
    let mut skipped = 0usize;
    let mut warmed = false;
    for c in &commands {
        let wav = clips.join(format!("{}.wav", c.id));
        if !wav.is_file() {
            eprintln!("warning: missing {}, skipping", wav.display());
            skipped += 1;
            continue;
        }
        let samples = read_wav_16k_mono(&wav)?;
        if !warmed {
            transcribe(&ctx, &samples)?;
            warmed = true;
        }
        let mut times = Vec::with_capacity(PASSES);
        let mut text = String::new();
        for _ in 0..PASSES {
            let t = Instant::now();
            text = transcribe(&ctx, &samples)?;
            times.push(t.elapsed().as_millis());
        }
        times.sort_unstable();
        let ms = times[PASSES / 2];
        let (w, hit) = if c.kind == Kind::Speech {
            (wer(&c.text, &text), entity_hit(&c.entity, &text))
        } else {
            (0.0, false)
        };
        println!("{} {}ms {:?}", c.id, ms, text);
        rows.push(Row {
            model: label.clone(),
            device: DEVICE.into(),
            id: c.id.clone(),
            kind: c.kind,
            ms,
            text,
            wer: w,
            hit,
        });
    }
    let mem = vec![(label.clone(), DEVICE.to_string(), peak_memory_mb())];
    write_reports(&rows, &mem, out)?;
    println!("done: {} transcribed, {skipped} skipped -> {}", rows.len(), out.display());
    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
