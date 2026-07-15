//! Text → phoneme conversion.
//!
//! Piper (VITS) models are trained on eSpeak-NG IPA phonemes, so we must produce
//! the same phoneme units the model expects. eSpeak-NG is GPL-3.0, so — to keep
//! the app itself proprietary and publicly shippable — we invoke it strictly
//! **out-of-process** (a bundled `espeak-ng` helper binary) rather than linking
//! it. That process boundary is what isolates the GPL code; see the Phase 1
//! plan. The [`Phonemizer`] trait keeps this swappable (e.g. a permissive
//! OpenPhonemizer impl later) without touching synthesis or callers.

use std::io::Write;
use std::path::PathBuf;
use std::process::Stdio;

use anyhow::{ensure, Context, Result};
use command::blocking::Command;

/// Converts text into a flat sequence of phoneme units (each a Unicode scalar
/// matching a `phoneme_id_map` key in the Piper config).
pub trait Phonemizer: Send + Sync {
    fn phonemize(&self, text: &str) -> Result<Vec<char>>;
}

/// Phonemizer that shells out to a bundled `espeak-ng` binary (GPL, isolated
/// across the process boundary) and returns its IPA output as phoneme units.
pub struct EspeakPhonemizer {
    binary: PathBuf,
    voice: String,
}

impl EspeakPhonemizer {
    /// `binary` is the path to the `espeak-ng` executable (bundled with the app,
    /// or on `PATH` in dev); `voice` is the eSpeak voice the model expects
    /// (e.g. "en-us", from the model config's `espeak.voice`).
    pub fn new(binary: impl Into<PathBuf>, voice: impl Into<String>) -> Self {
        let voice = voice.into();
        Self {
            binary: binary.into(),
            voice: if voice.is_empty() {
                "en-us".to_string()
            } else {
                voice
            },
        }
    }
}

impl Phonemizer for EspeakPhonemizer {
    fn phonemize(&self, text: &str) -> Result<Vec<char>> {
        // `-q` quiet (no audio), `--ipa` IPA phonemes (no `=3` tie bars, which
        // would inject zero-width joiners not in the phoneme map). Text is fed on
        // stdin to avoid argument escaping/injection issues.
        let mut child = Command::new(&self.binary)
            .args(["-q", "--ipa", "-v", &self.voice])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("failed to spawn espeak-ng at {:?}", self.binary))?;

        child
            .stdin
            .take()
            .context("espeak-ng stdin unavailable")?
            .write_all(text.as_bytes())
            .context("failed to write text to espeak-ng")?;

        let output = child
            .wait_with_output()
            .context("failed to read espeak-ng output")?;
        ensure!(
            output.status.success(),
            "espeak-ng exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );

        // eSpeak emits one line per clause (it splits on punctuation). Join the
        // clauses with a space so word boundaries survive, then keep the phoneme
        // codepoints and drop control chars / stray zero-width joiners.
        let text = String::from_utf8_lossy(&output.stdout);
        let joined = text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        Ok(joined
            .chars()
            .filter(|c| !c.is_control() && *c != '\u{200d}')
            .collect())
    }
}
