//! Piper voice config (`<model>.onnx.json`) — the bits we need to turn phonemes
//! into model inputs.

use std::collections::HashMap;

use serde::Deserialize;

/// Parsed Piper model configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct PiperConfig {
    pub audio: AudioConfig,
    /// Number of speakers; > 1 means the model takes a `sid` (speaker id) input.
    #[serde(default = "default_num_speakers")]
    pub num_speakers: i64,
    /// Maps each phoneme unit (a string key, usually one Unicode scalar) to the
    /// model's input id(s).
    pub phoneme_id_map: HashMap<String, Vec<i64>>,
    #[serde(default)]
    pub inference: InferenceConfig,
    /// Usually "espeak"; recorded for completeness.
    #[serde(default)]
    pub phoneme_type: String,
    #[serde(default)]
    pub espeak: EspeakConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AudioConfig {
    pub sample_rate: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct InferenceConfig {
    pub noise_scale: f32,
    pub length_scale: f32,
    pub noise_w: f32,
}

impl Default for InferenceConfig {
    fn default() -> Self {
        // Piper defaults.
        Self {
            noise_scale: 0.667,
            length_scale: 1.0,
            noise_w: 0.8,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct EspeakConfig {
    /// The eSpeak-NG voice this model was trained with (e.g. "en-us").
    #[serde(default)]
    pub voice: String,
}

fn default_num_speakers() -> i64 {
    1
}

impl PiperConfig {
    pub fn from_json(json: &str) -> anyhow::Result<Self> {
        Ok(serde_json::from_str(json)?)
    }

    pub fn is_multi_speaker(&self) -> bool {
        self.num_speakers > 1
    }
}
