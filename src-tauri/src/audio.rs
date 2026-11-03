use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use crate::stt::SAMPLE_RATE;

/// A live mic capture. The cpal stream is not `Send` on macOS, so it lives and
/// dies on its own thread; we only talk to that thread through channels.
pub struct Recording {
    stop_tx: mpsc::Sender<()>,
    done_rx: mpsc::Receiver<Vec<f32>>,
}

impl Recording {
    pub fn start(on_level: impl Fn(f32) + Send + 'static) -> Result<Self, String> {
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let (done_tx, done_rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();

        std::thread::spawn(move || {
            let buf = Arc::new(Mutex::new(Vec::<f32>::new()));
            let (stream, rate, channels) = match build_stream(buf.clone(), Box::new(on_level)) {
                Ok(s) => s,
                Err(e) => return drop(ready_tx.send(Err(e))),
            };
            if let Err(e) = stream.play() {
                return drop(ready_tx.send(Err(e.to_string())));
            }
            let _ = ready_tx.send(Ok(()));
            let _ = stop_rx.recv(); // also returns if the Recording is dropped
            drop(stream);
            let raw = std::mem::take(&mut *buf.lock().unwrap());
            let _ = done_tx.send(resample(&downmix(&raw, channels), rate, SAMPLE_RATE));
        });

        ready_rx.recv().map_err(|_| "audio thread exited".to_string())??;
        Ok(Self { stop_tx, done_rx })
    }

    /// Stops capture and returns mono 16 kHz samples.
    pub fn stop(self) -> Vec<f32> {
        let _ = self.stop_tx.send(());
        self.done_rx.recv().unwrap_or_default()
    }
}

struct Sink {
    buf: Arc<Mutex<Vec<f32>>>,
    on_level: Box<dyn Fn(f32) + Send>,
    last_level: Instant,
}

impl Sink {
    fn push(&mut self, samples: impl Iterator<Item = f32>) {
        let mut buf = self.buf.lock().unwrap();
        let start = buf.len();
        buf.extend(samples);
        let chunk = &buf[start..];
        if !chunk.is_empty() && self.last_level.elapsed() >= Duration::from_millis(40) {
            let rms = (chunk.iter().map(|s| s * s).sum::<f32>() / chunk.len() as f32).sqrt();
            (self.on_level)((rms * 8.0).min(1.0));
            self.last_level = Instant::now();
        }
    }
}

fn build_stream(
    buf: Arc<Mutex<Vec<f32>>>,
    on_level: Box<dyn Fn(f32) + Send>,
) -> Result<(cpal::Stream, u32, u16), String> {
    let device = cpal::default_host()
        .default_input_device()
        .ok_or("no microphone found")?;
    let config = device.default_input_config().map_err(|e| e.to_string())?;
    let (rate, channels, format) = (config.sample_rate(), config.channels(), config.sample_format());
    let config: cpal::StreamConfig = config.into();
    let mut sink = Sink { buf, on_level, last_level: Instant::now() };
    let on_err = |e| eprintln!("[otto] mic stream error: {e}");

    let stream = match format {
        cpal::SampleFormat::F32 => device.build_input_stream(
            config,
            move |d: &[f32], _: &cpal::InputCallbackInfo| sink.push(d.iter().copied()),
            on_err,
            None,
        ),
        cpal::SampleFormat::I16 => device.build_input_stream(
            config,
            move |d: &[i16], _: &cpal::InputCallbackInfo| {
                sink.push(d.iter().map(|&s| s as f32 / i16::MAX as f32))
            },
            on_err,
            None,
        ),
        other => return Err(format!("unsupported mic sample format {other:?}")),
    }
    .map_err(|e| e.to_string())?;
    Ok((stream, rate, channels))
}

fn downmix(x: &[f32], channels: u16) -> Vec<f32> {
    let c = channels.max(1) as usize;
    if c == 1 {
        return x.to_vec();
    }
    x.chunks_exact(c).map(|f| f.iter().sum::<f32>() / c as f32).collect()
}

// ponytail: linear resampling with no low-pass filter. Fine for speech into
// whisper; switch to `rubato` if accuracy suffers on 44.1/48 kHz mics.
pub fn resample(x: &[f32], from: u32, to: u32) -> Vec<f32> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resample_and_downmix() {
        let one_sec_48k = vec![0.5f32; 48_000];
        let out = resample(&one_sec_48k, 48_000, 16_000);
        assert_eq!(out.len(), 16_000);
        assert!(out.iter().all(|&s| (s - 0.5).abs() < 1e-6));
        assert_eq!(downmix(&[1.0, 0.0, 0.5, 0.5], 2), vec![0.5, 0.5]);
    }
}
