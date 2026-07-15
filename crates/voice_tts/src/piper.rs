//! Piper VITS synthesis via ONNX Runtime (`ort`).
//!
//! Turns phoneme units into the model's id sequence (with the Piper
//! BOS/pad/EOS scheme), runs the VITS graph, and returns f32 PCM. Permissively
//! licensed end-to-end — the only GPL piece (phonemization) lives behind the
//! [`Phonemizer`](crate::Phonemizer) trait and runs out-of-process.

use std::sync::Mutex;

use anyhow::{ensure, Context, Result};
use ort::execution_providers::CPUExecutionProvider;
use ort::session::Session;
use ort::value::Value;

use crate::config::PiperConfig;
use crate::Pcm;

/// A loaded Piper voice: an ONNX Runtime session plus its parsed config.
pub struct PiperTts {
    session: Mutex<Session>,
    config: PiperConfig,
    /// Speaker id for multi-speaker models (ignored when single-speaker).
    speaker_id: i64,
}

impl PiperTts {
    /// Load a voice from a model file and its `<model>.onnx.json` config path.
    pub fn from_paths(
        model_path: impl AsRef<std::path::Path>,
        config_path: impl AsRef<std::path::Path>,
    ) -> Result<Self> {
        let config_json = std::fs::read_to_string(config_path.as_ref())
            .with_context(|| format!("reading config {:?}", config_path.as_ref()))?;
        let config = PiperConfig::from_json(&config_json)?;
        let session = Self::build_session_from_file(model_path.as_ref())?;
        Ok(Self::new(session, config))
    }

    /// Load a voice from in-memory model bytes + config JSON (bundled resources).
    pub fn from_bytes(model_bytes: &[u8], config_json: &str) -> Result<Self> {
        let config = PiperConfig::from_json(config_json)?;
        let session = Session::builder()?
            .with_execution_providers([CPUExecutionProvider::default().build()])?
            .commit_from_memory(model_bytes)?;
        Ok(Self::new(session, config))
    }

    fn build_session_from_file(path: &std::path::Path) -> Result<Session> {
        Ok(Session::builder()?
            .with_execution_providers([CPUExecutionProvider::default().build()])?
            .commit_from_file(path)?)
    }

    fn new(session: Session, config: PiperConfig) -> Self {
        Self {
            session: Mutex::new(session),
            config,
            speaker_id: 0,
        }
    }

    pub fn sample_rate(&self) -> u32 {
        self.config.audio.sample_rate
    }

    /// The eSpeak-NG voice this model expects (from config; defaults to "en-us").
    pub fn espeak_voice(&self) -> &str {
        if self.config.espeak.voice.is_empty() {
            "en-us"
        } else {
            &self.config.espeak.voice
        }
    }

    /// Select the speaker id for a multi-speaker model (e.g. libritts_r has 904).
    pub fn set_speaker(&mut self, speaker_id: i64) {
        self.speaker_id = speaker_id;
    }

    /// Synthesize already-phonemized text into PCM.
    pub fn synthesize_phonemes(&self, phonemes: &[char]) -> Result<Pcm> {
        let ids = self.phonemes_to_ids(phonemes);
        // BOS + pad + EOS is 3 ids; anything at/under that means nothing mapped.
        ensure!(
            ids.len() > 3,
            "no recognizable phonemes to synthesize (got {} ids)",
            ids.len()
        );
        let len = ids.len();

        let input = Value::from_array(([1, len], ids))?;
        let input_lengths = Value::from_array(([1], vec![len as i64]))?;
        let inference = &self.config.inference;
        let scales = Value::from_array((
            [3],
            vec![
                inference.noise_scale,
                inference.length_scale,
                inference.noise_w,
            ],
        ))?;

        let mut session = self
            .session
            .lock()
            .map_err(|_| anyhow::anyhow!("piper session mutex poisoned"))?;
        let outputs = if self.config.is_multi_speaker() {
            let sid = Value::from_array(([1], vec![self.speaker_id]))?;
            session.run(ort::inputs![
                "input" => input,
                "input_lengths" => input_lengths,
                "scales" => scales,
                "sid" => sid,
            ])?
        } else {
            session.run(ort::inputs![
                "input" => input,
                "input_lengths" => input_lengths,
                "scales" => scales,
            ])?
        };

        // Piper output is float PCM in [-1, 1], shape [1, 1, num_samples].
        let audio = outputs[0].try_extract_array::<f32>()?;
        let samples: Vec<f32> = audio.iter().copied().collect();
        ensure!(!samples.is_empty(), "piper produced no audio");
        Ok(Pcm {
            samples,
            sample_rate: self.config.audio.sample_rate,
        })
    }

    /// Build the model input id sequence using Piper's scheme:
    /// `BOS, pad, (phoneme, pad)*, EOS`, skipping phonemes not in the map.
    fn phonemes_to_ids(&self, phonemes: &[char]) -> Vec<i64> {
        let map = &self.config.phoneme_id_map;
        let pad = map.get("_");
        let mut ids: Vec<i64> = Vec::with_capacity(phonemes.len() * 2 + 4);
        if let Some(bos) = map.get("^") {
            ids.extend_from_slice(bos);
        }
        if let Some(pad) = pad {
            ids.extend_from_slice(pad);
        }
        let mut buf = [0u8; 4];
        for &ch in phonemes {
            let key: &str = ch.encode_utf8(&mut buf);
            if let Some(units) = map.get(key) {
                ids.extend_from_slice(units);
                if let Some(pad) = pad {
                    ids.extend_from_slice(pad);
                }
            }
        }
        if let Some(eos) = map.get("$") {
            ids.extend_from_slice(eos);
        }
        ids
    }
}
