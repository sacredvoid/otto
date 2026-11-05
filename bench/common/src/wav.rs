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
