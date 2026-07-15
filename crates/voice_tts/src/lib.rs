//! Local, embeddable neural text-to-speech for Bang's voice overlay.
//!
//! Piper (VITS) synthesis via ONNX Runtime, permissively licensed, with the
//! GPL phonemizer (eSpeak-NG) isolated behind the [`Phonemizer`] trait and run
//! out-of-process. The crate has no UI and no Bang/IDE dependencies so it can
//! back both the standalone overlay app and the in-Bang integration.
//!
//! ```no_run
//! use voice_tts::{EspeakPhonemizer, PiperEngine, PiperTts, TtsEngine};
//!
//! let phonemizer = EspeakPhonemizer::new("espeak-ng", "en-us");
//! let tts = PiperTts::from_paths("voice.onnx", "voice.onnx.json")?;
//! let engine = PiperEngine::new(tts, phonemizer);
//! let pcm = engine.synthesize("Hello from Bang.")?;
//! # anyhow::Ok(())
//! ```

#[cfg(feature = "aec")]
mod aec;
mod config;
mod phonemizer;
mod piper;

#[cfg(feature = "aec")]
pub use aec::{Aec, AecStats, WebRtcAec};
pub use config::{AudioConfig, EspeakConfig, InferenceConfig, PiperConfig};
pub use phonemizer::{EspeakPhonemizer, Phonemizer};
pub use piper::PiperTts;

/// Mono PCM audio: `samples` are f32 in [-1, 1] at `sample_rate` Hz.
#[derive(Debug, Clone)]
pub struct Pcm {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
}

impl Pcm {
    /// Convert to interleaved little-endian PCM16 bytes (for WAV / playback).
    pub fn to_pcm16_le(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.samples.len() * 2);
        for &s in &self.samples {
            let clamped = s.clamp(-1.0, 1.0);
            let v = (clamped * i16::MAX as f32) as i16;
            out.extend_from_slice(&v.to_le_bytes());
        }
        out
    }

    /// Encode as a self-contained mono 16-bit WAV (RIFF) byte buffer, suitable
    /// for handing to a native audio player (e.g. `AVAudioPlayer`).
    pub fn to_wav_bytes(&self) -> Vec<u8> {
        let pcm = self.to_pcm16_le();
        let data_len = pcm.len() as u32;
        let channels: u16 = 1;
        let bits: u16 = 16;
        let byte_rate = self.sample_rate * channels as u32 * (bits as u32 / 8);
        let block_align = channels * (bits / 8);

        let mut wav = Vec::with_capacity(44 + pcm.len());
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data_len).to_le_bytes());
        wav.extend_from_slice(b"WAVE");
        wav.extend_from_slice(b"fmt ");
        wav.extend_from_slice(&16u32.to_le_bytes()); // PCM fmt chunk size
        wav.extend_from_slice(&1u16.to_le_bytes()); // audio format = PCM
        wav.extend_from_slice(&channels.to_le_bytes());
        wav.extend_from_slice(&self.sample_rate.to_le_bytes());
        wav.extend_from_slice(&byte_rate.to_le_bytes());
        wav.extend_from_slice(&block_align.to_le_bytes());
        wav.extend_from_slice(&bits.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_len.to_le_bytes());
        wav.extend_from_slice(&pcm);
        wav
    }
}

/// Text-to-speech engine: text in, PCM out. The synthesis backend and its
/// phonemizer are implementation details, so callers (overlay, standalone app)
/// depend only on this.
pub trait TtsEngine: Send + Sync {
    fn synthesize(&self, text: &str) -> anyhow::Result<Pcm>;
}

/// A [`TtsEngine`] pairing a Piper voice with a [`Phonemizer`].
pub struct PiperEngine<P: Phonemizer> {
    tts: PiperTts,
    phonemizer: P,
}

impl<P: Phonemizer> PiperEngine<P> {
    pub fn new(tts: PiperTts, phonemizer: P) -> Self {
        Self { tts, phonemizer }
    }

    pub fn tts_mut(&mut self) -> &mut PiperTts {
        &mut self.tts
    }
}

/// Extra silence inserted after a sentence end (`.`/`!`/`?`, newline).
const SENTENCE_PAUSE_MS: u32 = 320;
/// Extra silence inserted after a dash used as a break (em/en dash, spaced `-`).
const DASH_PAUSE_MS: u32 = 240;

/// Split text into speakable segments, tagging each with the pause (ms) to
/// insert *after* it, so periods and dashes get a natural break. A `.`/`!`/`?`
/// only counts as a sentence end when followed by whitespace/end (so "3.5" and
/// "e.g." aren't split); a `-` only when spaced on both sides (so hyphenated
/// words aren't split).
fn split_for_pauses(text: &str) -> Vec<(String, u32)> {
    let chars: Vec<char> = text.chars().collect();
    let mut out: Vec<(String, u32)> = Vec::new();
    let mut cur = String::new();
    for i in 0..chars.len() {
        let c = chars[i];
        cur.push(c);
        let next = chars.get(i + 1).copied();
        let pause = match c {
            '.' | '!' | '?' => {
                if next.is_none_or(char::is_whitespace) {
                    Some(SENTENCE_PAUSE_MS)
                } else {
                    None
                }
            }
            '\n' => Some(SENTENCE_PAUSE_MS),
            '\u{2014}' | '\u{2013}' => Some(DASH_PAUSE_MS), // em / en dash
            '-' => {
                let prev_space = i > 0 && chars[i - 1].is_whitespace();
                if prev_space && next.is_some_and(char::is_whitespace) {
                    Some(DASH_PAUSE_MS)
                } else {
                    None
                }
            }
            _ => None,
        };
        if let Some(pause) = pause {
            out.push((std::mem::take(&mut cur), pause));
        }
    }
    if !cur.trim().is_empty() {
        out.push((cur, 0));
    }
    out
}

impl<P: Phonemizer> PiperEngine<P> {
    fn synthesize_segment(&self, text: &str) -> Option<Vec<f32>> {
        let phonemes = self.phonemizer.phonemize(text).ok()?;
        self.tts
            .synthesize_phonemes(&phonemes)
            .ok()
            .map(|p| p.samples)
    }
}

impl<P: Phonemizer> TtsEngine for PiperEngine<P> {
    fn synthesize(&self, text: &str) -> anyhow::Result<Pcm> {
        let sample_rate = self.tts.sample_rate();
        let segments = split_for_pauses(text);

        let mut samples: Vec<f32> = Vec::new();
        for (segment, pause_ms) in &segments {
            if segment.trim().is_empty() {
                continue;
            }
            if let Some(seg_samples) = self.synthesize_segment(segment.trim()) {
                samples.extend_from_slice(&seg_samples);
                let silence = sample_rate as usize * *pause_ms as usize / 1000;
                samples.extend(std::iter::repeat_n(0.0, silence));
            }
        }

        // Fallback to whole-text synthesis if segmentation produced nothing
        // (e.g. a single token with no recognizable phonemes per segment).
        if samples.is_empty() {
            let phonemes = self.phonemizer.phonemize(text)?;
            return self.tts.synthesize_phonemes(&phonemes);
        }
        Ok(Pcm {
            samples,
            sample_rate,
        })
    }
}
