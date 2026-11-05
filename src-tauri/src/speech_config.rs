//! Speech-to-text settings read from ~/Otto/config.json.

use serde_json::Value;

pub const DEFAULT_MODEL: &str = "parakeet-tdt-0.6b-v2-Q8_0.gguf";

#[derive(Debug, Clone, PartialEq)]
pub enum DevicePref {
    Auto,
    Cpu,
    Named(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct SpeechConfig {
    pub model: String,
    pub device: DevicePref,
}

pub fn from_config(v: &Value) -> SpeechConfig {
    let model = match v.get("speech_model").and_then(Value::as_str) {
        Some(m) if !m.is_empty() => m.to_string(),
        _ => DEFAULT_MODEL.to_string(),
    };
    let device = match v.get("speech_device").and_then(Value::as_str) {
        Some(d) if d.is_empty() || d.eq_ignore_ascii_case("auto") => DevicePref::Auto,
        Some(d) if d.eq_ignore_ascii_case("cpu") => DevicePref::Cpu,
        Some(d) => DevicePref::Named(d.to_string()),
        None => DevicePref::Auto,
    };
    SpeechConfig { model, device }
}

#[allow(dead_code)]
pub fn load() -> SpeechConfig {
    from_config(&crate::tasks::config())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn empty_gives_defaults() {
        let c = from_config(&json!({}));
        assert_eq!(c.model, DEFAULT_MODEL);
        assert_eq!(c.device, DevicePref::Auto);
    }

    #[test]
    fn model_and_cpu() {
        let c = from_config(&json!({"speech_model":"ggml-base.en.bin","speech_device":"cpu"}));
        assert_eq!(c.model, "ggml-base.en.bin");
        assert_eq!(c.device, DevicePref::Cpu);
    }

    #[test]
    fn named_device() {
        let c = from_config(&json!({"speech_device":"RTX"}));
        assert_eq!(c.device, DevicePref::Named("RTX".into()));
    }

    #[test]
    fn bad_values_ignored() {
        let c = from_config(&json!({"speech_model":"","speech_device":42}));
        assert_eq!(c.model, DEFAULT_MODEL);
        assert_eq!(c.device, DevicePref::Auto);
    }

    #[test]
    fn auto_and_case() {
        assert_eq!(from_config(&json!({"speech_device":"AUTO"})).device, DevicePref::Auto);
        assert_eq!(from_config(&json!({"speech_device":"CPU"})).device, DevicePref::Cpu);
        assert_eq!(from_config(&json!({"speech_device":""})).device, DevicePref::Auto);
    }
}
