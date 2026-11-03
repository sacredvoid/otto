use std::path::{Path, PathBuf};

use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

pub const SAMPLE_RATE: u32 = 16_000;

pub struct Stt {
    ctx: WhisperContext,
}

pub fn default_model_path() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_default()
        .join("app.otto.desktop/models/ggml-base.en.bin")
}

impl Stt {
    pub fn load(path: &Path) -> Result<Self, String> {
        if !path.exists() {
            return Err(format!("speech model not found at {}", path.display()));
        }
        let path = path.to_str().ok_or("model path is not UTF-8")?;
        let ctx = WhisperContext::new_with_params(path, WhisperContextParameters::default())
            .map_err(|e| format!("could not load speech model: {e}"))?;
        Ok(Self { ctx })
    }

    /// `samples` must be mono f32 at 16 kHz.
    pub fn transcribe(&self, samples: &[f32]) -> Result<String, String> {
        let mut state = self.ctx.create_state().map_err(|e| e.to_string())?;
        let mut p = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        p.set_language(Some("en"));
        p.set_initial_prompt("Otto"); // vocabulary hint so "hey Otto" isn't "hey auto"
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    /// Run with: cargo test -- --ignored --nocapture
    #[test]
    #[ignore = "needs the downloaded model and macOS `say`"]
    fn transcribes_a_spoken_command() {
        let wav = std::env::temp_dir().join("otto-stt-test.wav");
        let ok = std::process::Command::new("say")
            .arg("-o")
            .arg(&wav)
            .args(["--file-format=WAVE", "--data-format=LEF32@16000"])
            .arg("Hey Otto, make a five slide deck about hiring.")
            .status()
            .expect("run say")
            .success();
        assert!(ok, "say failed");

        let samples = wav_data_f32(&std::fs::read(&wav).unwrap());
        let stt = Stt::load(&default_model_path()).unwrap();
        let t = Instant::now();
        let text = stt.transcribe(&samples).unwrap().to_lowercase();
        println!("heard {:?} in {} ms", text, t.elapsed().as_millis());
        assert!(text.contains("deck") && text.contains("hiring"), "{text}");
    }

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
