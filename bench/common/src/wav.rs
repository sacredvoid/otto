use std::path::Path;

/// Read a 16 kHz mono WAV (16-bit int or 32-bit float) as f32 samples in [-1, 1].
pub fn read_wav_16k_mono(path: &Path) -> Result<Vec<f32>, String> {
    let mut r = hound::WavReader::open(path).map_err(|e| format!("open {}: {e}", path.display()))?;
    let spec = r.spec();
    if spec.sample_rate != 16_000 || spec.channels != 1 {
        return Err(format!(
            "{}: expected 16 kHz mono, got {} Hz x {} ch",
            path.display(),
            spec.sample_rate,
            spec.channels
        ));
    }
    match (spec.sample_format, spec.bits_per_sample) {
        (hound::SampleFormat::Int, 16) => r
            .samples::<i16>()
            .map(|s| s.map(|v| v as f32 / 32768.0).map_err(|e| e.to_string()))
            .collect(),
        (hound::SampleFormat::Float, 32) => r
            .samples::<f32>()
            .map(|s| s.map_err(|e| e.to_string()))
            .collect(),
        (f, b) => Err(format!("{}: unsupported sample format {f:?} {b}-bit", path.display())),
    }
}

/// Write f32 samples as a 16 kHz mono 32-bit float WAV.
pub fn write_wav_16k_mono(path: &Path, samples: &[f32]) -> Result<(), String> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 16_000,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut w = hound::WavWriter::create(path, spec).map_err(|e| format!("create {}: {e}", path.display()))?;
    for &s in samples {
        w.write_sample(s).map_err(|e| e.to_string())?;
    }
    w.finalize().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_read_round_trip() {
        let path = std::env::temp_dir().join("stt_bench_roundtrip.wav");
        let samples: Vec<f32> = (0..1600).map(|i| (i as f32 / 1600.0 * 6.28).sin() * 0.8).collect();
        write_wav_16k_mono(&path, &samples).unwrap();
        let back = read_wav_16k_mono(&path).unwrap();
        std::fs::remove_file(&path).ok();
        assert_eq!(back.len(), samples.len());
        assert!(samples.iter().zip(&back).all(|(a, b)| (a - b).abs() < 1e-6));
    }
}
