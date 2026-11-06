//! Interactive recorder: `record <commands.tsv> <out_dir> [--redo-all]`.
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use stt_bench_common::{Kind, load_commands, write_wav_16k_mono};

const SAMPLE_RATE: u32 = 16_000;

// Copied from src-tauri/src/audio.rs so the bench captures audio the way Otto does.
fn downmix(x: &[f32], channels: u16) -> Vec<f32> {
    let c = channels.max(1) as usize;
    if c == 1 {
        return x.to_vec();
    }
    x.chunks_exact(c).map(|f| f.iter().sum::<f32>() / c as f32).collect()
}

fn resample(x: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || x.is_empty() {
        return x.to_vec();
    }
    let n = (x.len() as u64 * to as u64 / from as u64) as usize;
    let step = from as f64 / to as f64;
    (0..n)
        .map(|i| {
            let p = i as f64 * step;
            let j = p as usize;
            let a = x[j];
            let b = *x.get(j + 1).unwrap_or(&a);
            a + (b - a) * (p - j as f64) as f32
        })
        .collect()
}

struct Capture {
    stream: cpal::Stream,
    buf: Arc<Mutex<Vec<f32>>>,
    rate: u32,
    channels: u16,
}

fn start_capture() -> Result<Capture, String> {
    let device = cpal::default_host()
        .default_input_device()
        .ok_or("no input device found: plug in or enable a microphone")?;
    let config = device.default_input_config().map_err(|e| e.to_string())?;
    let (rate, channels, format) = (config.sample_rate(), config.channels(), config.sample_format());
    let config: cpal::StreamConfig = config.into();
    let buf = Arc::new(Mutex::new(Vec::<f32>::new()));
    let (b1, b2) = (buf.clone(), buf.clone());
    let on_err = |e| eprintln!("mic stream error: {e}");
    let stream = match format {
        cpal::SampleFormat::F32 => device.build_input_stream(
            config,
            move |d: &[f32], _: &cpal::InputCallbackInfo| b1.lock().unwrap().extend_from_slice(d),
            on_err,
            None,
        ),
        cpal::SampleFormat::I16 => device.build_input_stream(
            config,
            move |d: &[i16], _: &cpal::InputCallbackInfo| {
                b2.lock().unwrap().extend(d.iter().map(|&s| s as f32 / i16::MAX as f32))
            },
            on_err,
            None,
        ),
        other => return Err(format!("unsupported mic sample format {other:?}")),
    }
    .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    Ok(Capture { stream, buf, rate, channels })
}

fn read_line() -> Option<String> {
    let mut s = String::new();
    match io::stdin().lock().read_line(&mut s) {
        Ok(0) | Err(_) => None,
        Ok(_) => Some(s.trim().to_lowercase()),
    }
}

fn prompt(msg: &str) -> Option<String> {
    print!("{msg}");
    io::stdout().flush().ok();
    read_line()
}

fn record_one(path: &Path) -> Result<Option<f32>, String> {
    match prompt("  [Enter]=start recording  q=quit: ").as_deref() {
        None | Some("q") => return Ok(None),
        _ => {}
    }
    let cap = start_capture()?;
    println!("  RECORDING - press [Enter] to stop");
    read_line();
    drop(cap.stream);
    let raw = std::mem::take(&mut *cap.buf.lock().unwrap());
    let samples = resample(&downmix(&raw, cap.channels), cap.rate, SAMPLE_RATE);
    write_wav_16k_mono(path, &samples)?;
    Ok(Some(samples.len() as f32 / SAMPLE_RATE as f32))
}

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let redo_all = args.iter().any(|a| a == "--redo-all");
    args.retain(|a| a != "--redo-all");
    if args.len() != 2 {
        return Err("usage: record <commands.tsv> <out_dir> [--redo-all]".into());
    }
    let cmds = load_commands(Path::new(&args[0]))?;
    let out_dir = PathBuf::from(&args[1]);
    std::fs::create_dir_all(&out_dir).map_err(|e| format!("create {}: {e}", out_dir.display()))?;
    let total = cmds.len();

    for (i, c) in cmds.iter().enumerate() {
        let path = out_dir.join(format!("{}.wav", c.id));
        if path.exists() && !redo_all {
            println!("[{}/{}] {} already recorded, skipping", i + 1, total, c.id);
            continue;
        }
        println!();
        match c.kind {
            Kind::Speech => println!("[{}/{}] {}\n  SAY: {}", i + 1, total, c.id, c.text),
            Kind::Silence => println!("[{}/{}] {}\n  DO: {}", i + 1, total, c.id, c.text),
        }
        loop {
            match record_one(&path)? {
                None => return Ok(()),
                Some(secs) => println!("  saved {} ({secs:.1} s)", path.display()),
            }
            match prompt("[Enter]=next  r=redo  q=quit: ").as_deref() {
                Some("r") => continue,
                Some("q") | None => return Ok(()),
                _ => break,
            }
        }
    }
    println!("\nAll {total} commands recorded.");
    Ok(())
}
