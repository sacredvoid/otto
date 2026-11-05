//! Silero voice-activity gate: a tap with no speech in it should not reach the recognizer.

use std::path::Path;
use std::sync::Mutex;

use transcribe_cpp::{Backend, Model, ModelOptions, VadOptions, VadSession, VadSessionOptions};

use crate::stt::SAMPLE_RATE;

/// Context kept before the first and after the last detected segment.
pub const PAD_MS: u32 = 200;

pub struct Vad {
    // Keeps the weights alive for the session's lifetime.
    _model: Model,
    session: Mutex<VadSession>,
}

impl Vad {
    /// CPU, one thread: the model is 1 MB and a clip scores in milliseconds.
    pub fn load(path: &Path) -> Result<Self, String> {
        if !path.is_file() {
            return Err(format!("VAD model not found at {}", path.display()));
        }
        let model = Model::load_with(path, &ModelOptions { backend: Backend::Cpu, ..Default::default() })
            .map_err(|e| format!("could not load VAD model: {e}"))?;
        let session = model
            .vad_session_with(&VadSessionOptions { n_threads: 1, ..Default::default() })
            .map_err(|e| format!("could not start VAD session: {e}"))?;
        Ok(Vad { _model: model, session: Mutex::new(session) })
    }

    /// The part of `samples` (16 kHz mono) from 200 ms before the first speech segment to 200 ms
    /// after the last, or None when there is no speech. An error from the VAD itself keeps the
    /// whole clip: never lose dictation to a gate fault.
    pub fn speech(&self, samples: &[f32]) -> Option<Vec<f32>> {
        let mut session = self.session.lock().unwrap_or_else(|p| p.into_inner());
        let result = match session.run(samples, &VadOptions::default()) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("[otto] VAD failed, keeping the whole clip: {e}");
                return Some(samples.to_vec());
            }
        };
        let segments: Vec<(usize, usize)> =
            result.segments.iter().map(|s| (s.start_sample.max(0) as usize, s.end_sample.max(0) as usize)).collect();
        let (start, end) = padded_range(&segments, samples.len())?;
        Some(samples[start..end].to_vec())
    }
}

/// First segment start minus the pad to last segment end plus the pad, clamped to `0..len`.
fn padded_range(segments: &[(usize, usize)], len: usize) -> Option<(usize, usize)> {
    let first = segments.first()?.0;
    let last = segments.last()?.1;
    let pad = SAMPLE_RATE as usize * PAD_MS as usize / 1000;
    let end = last.saturating_add(pad).min(len);
    let start = first.saturating_sub(pad).min(end);
    Some((start, end))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stt::{self, DeviceInfo, DeviceKind, Stt};
    use std::path::PathBuf;

    const VAD_MODEL: &str = "silero-vad-v6.2-F32.gguf";

    #[test]
    fn pad_clamps_to_clip_bounds() {
        let ms = |n: usize| n * 16;
        // Segment at 0..100 ms in a 1 s clip: start clamps to 0, end gains the pad.
        assert_eq!(padded_range(&[(0, ms(100))], ms(1000)), Some((0, ms(300))));
        // Segment near the end: end clamps to the clip length.
        assert_eq!(padded_range(&[(ms(900), ms(990))], ms(1000)), Some((ms(700), ms(1000))));
        // Several segments span first start to last end.
        assert_eq!(padded_range(&[(ms(400), ms(500)), (ms(600), ms(700))], ms(1000)), Some((ms(200), ms(900))));
        assert_eq!(padded_range(&[], ms(1000)), None);
    }

    fn clips() -> PathBuf {
        PathBuf::from(std::env::var_os("OTTO_CLIPS").expect("set OTTO_CLIPS to the clips folder"))
    }

    fn rows(kind: &str) -> Vec<(String, String)> {
        let tsv = Path::new(env!("CARGO_MANIFEST_DIR")).join("../bench/commands.tsv");
        std::fs::read_to_string(tsv)
            .unwrap()
            .lines()
            .skip(1)
            .filter_map(|l| {
                let c: Vec<&str> = l.split('\t').collect();
                (c.get(3).map(|k| k.trim()) == Some(kind)).then(|| (c[0].to_string(), c[2].to_string()))
            })
            .collect()
    }

    fn model_file(name: &str) -> PathBuf {
        Some(stt::model_path(name))
            .filter(|p| p.is_file())
            .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../models").join(name))
    }

    fn load_vad() -> Vad {
        stt::init_backends();
        Vad::load(&model_file(VAD_MODEL)).unwrap()
    }

    /// 32-bit float WAV data (plain or WAVE_FORMAT_EXTENSIBLE), 16 kHz mono.
    fn read_clip(id: &str) -> Vec<f32> {
        let b = std::fs::read(clips().join(format!("{id}.wav"))).unwrap();
        let mut i = 12;
        while i + 8 <= b.len() {
            let n = u32::from_le_bytes(b[i + 4..i + 8].try_into().unwrap()) as usize;
            if &b[i..i + 4] == b"data" {
                let end = (i + 8 + n).min(b.len());
                return b[i + 8..end].chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect();
            }
            i += 8 + n + (n & 1);
        }
        panic!("no data chunk in {id}.wav");
    }

    #[test]
    #[ignore = "needs OTTO_CLIPS and the VAD model"]
    fn silence_clip_has_no_speech() {
        let vad = load_vad();
        let silent = rows("silence");
        assert_eq!(silent.len(), 5, "{silent:?}");
        for (id, _) in silent {
            let s = read_clip(&id);
            assert!(vad.speech(&s).is_none(), "{id} was treated as speech");
        }
    }

    #[test]
    #[ignore = "needs OTTO_CLIPS, the VAD model and the speech model"]
    fn one_word_clip_keeps_speech() {
        let vad = load_vad();
        let cpu = stt::devices().into_iter().find(|d| d.kind == DeviceKind::Cpu).unwrap_or_else(DeviceInfo::cpu);
        let stt = Stt::load(&model_file(crate::speech_config::DEFAULT_MODEL), &cpu).unwrap();
        let words: Vec<_> = rows("speech").into_iter().filter(|(id, _)| id.starts_with("word")).collect();
        assert_eq!(words.len(), 4, "{words:?}");
        for (id, entity) in words {
            let s = read_clip(&id);
            let kept = vad.speech(&s).unwrap_or_else(|| panic!("{id} ({entity}) was dropped as silence"));
            let text = stt.transcribe(&kept).unwrap().to_lowercase();
            println!("{id}: {} -> {} samples, heard {text:?}", s.len(), kept.len());
            assert!(text.contains(&entity.to_lowercase()), "{id}: {text}");
        }
    }
}
