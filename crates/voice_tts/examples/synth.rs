//! Synthesize text to a WAV file for validating a Piper voice end-to-end.
//!
//! Usage:
//!   cargo run -p voice_tts --example synth -- \
//!     <model.onnx> <config.onnx.json> <espeak-ng-bin> <out.wav> [speaker_id] [text...]

use anyhow::{ensure, Context, Result};
use voice_tts::{EspeakPhonemizer, PiperEngine, PiperTts, TtsEngine};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    ensure!(
        args.len() >= 4,
        "usage: synth <model.onnx> <config.json> <espeak-ng-bin> <out.wav> [speaker_id] [text...]"
    );
    let model = &args[0];
    let config = &args[1];
    let espeak = &args[2];
    let out = &args[3];
    let speaker_id: i64 = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(0);
    let text = if args.len() > 5 {
        args[5..].join(" ")
    } else {
        "Sure, I checked your home directory. There are about seventy three files, \
         including a few shell scripts and a large Jenkins jar. Want me to open any of them?"
            .to_string()
    };

    let mut tts = PiperTts::from_paths(model, config).context("loading Piper voice")?;
    tts.set_speaker(speaker_id);
    let phonemizer = EspeakPhonemizer::new(espeak, "en-us");
    let engine = PiperEngine::new(tts, phonemizer);

    let start = std::time::Instant::now();
    let pcm = engine.synthesize(&text).context("synthesizing")?;
    let secs = pcm.samples.len() as f32 / pcm.sample_rate as f32;
    eprintln!(
        "synth: {} samples @ {} Hz ({:.2}s audio) in {:?}",
        pcm.samples.len(),
        pcm.sample_rate,
        secs,
        start.elapsed()
    );

    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: pcm.sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(out, spec).context("creating WAV")?;
    for &s in &pcm.samples {
        let v = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        writer.write_sample(v)?;
    }
    writer.finalize()?;
    eprintln!("wrote {out}");
    Ok(())
}
