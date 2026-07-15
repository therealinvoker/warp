//! Software acoustic echo cancellation (WebRTC APM / AEC3).
//!
//! Enables hands-free barge-in: while the agent's answer is read aloud, the
//! spoken audio (which we own as PCM) is fed as the far-end *reference* and
//! subtracted from the mic input, so what reaches the transcriber's VAD is only
//! the user's voice. Unlike the OS VoiceProcessing path, this is device- and
//! output-independent (works on speakers) and BSD-licensed for public shipping.
//!
//! The WebRTC APM runs at a fixed 48 kHz in 10 ms frames. Our mic (24 kHz, for
//! the Realtime transcriber) and TTS (22.05 kHz) streams are resampled to/from
//! 48 kHz here, with internal buffering so callers can push arbitrary chunk
//! sizes.

use anyhow::Result;
use webrtc_audio_processing::config::EchoCanceller;
use webrtc_audio_processing::{Config, Processor};

/// The APM's fixed operating rate.
const APM_RATE: u32 = 48_000;
/// Samples per 10 ms mono frame at [`APM_RATE`].
const APM_FRAME: usize = (APM_RATE as usize) / 100;

/// Echo canceller: push the far-end reference (played TTS) and process near-end
/// (mic) frames to get echo-removed audio. Kept as a trait so the overlay/app
/// depend on the capability, not the WebRTC implementation.
pub trait Aec: Send {
    /// Feed far-end reference samples (what's being played to the speakers), at
    /// the reference sample rate given at construction.
    fn push_reference(&mut self, samples: &[f32]);
    /// Process near-end mic samples (at the mic rate) and return the
    /// echo-cancelled samples at the same mic rate.
    fn process_capture(&mut self, mic: &[f32]) -> Vec<f32>;
    /// Drop adapted echo state (e.g. when a new spoken answer starts).
    fn reset(&mut self);
}

/// Streaming linear resampler that carries one sample of history across calls so
/// consecutive chunks join without boundary discontinuities. Adequate for the
/// AEC reference/mic paths (the canceller and VAD tolerate linear resampling);
/// can be upgraded to windowed-sinc later if cancellation quality demands it.
struct Resampler {
    from: f64,
    to: f64,
    /// Fractional input index for the next output sample, relative to the start
    /// of the next input buffer (can be negative → into `last`).
    pos: f64,
    /// Final sample of the previous input buffer (history for interpolation).
    last: f32,
    identity: bool,
}

impl Resampler {
    fn new(from: u32, to: u32) -> Self {
        Self {
            from: from as f64,
            to: to as f64,
            pos: 0.0,
            last: 0.0,
            identity: from == to,
        }
    }

    fn process(&mut self, input: &[f32]) -> Vec<f32> {
        if self.identity {
            return input.to_vec();
        }
        if input.is_empty() {
            return Vec::new();
        }
        let step = self.from / self.to; // input samples advanced per output sample
        let n = input.len();
        let mut out = Vec::with_capacity(((n as f64) * self.to / self.from) as usize + 2);
        let get = |k: i64| -> f32 {
            if k < 0 {
                self.last
            } else {
                let k = k as usize;
                input.get(k).copied().unwrap_or(input[n - 1])
            }
        };
        let max = (n as f64) - 1.0;
        let mut pos = self.pos;
        while pos <= max {
            let floor = pos.floor();
            let frac = (pos - floor) as f32;
            let a = get(floor as i64);
            let b = get(floor as i64 + 1);
            out.push(a + (b - a) * frac);
            pos += step;
        }
        // Carry: index 0 of the next buffer sits `n` input samples ahead.
        self.pos = pos - n as f64;
        self.last = input[n - 1];
        out
    }
}

/// WebRTC APM echo canceller with the resampling/framing glue.
pub struct WebRtcAec {
    apm: Processor,
    // Capture (near-end) path.
    cap_up: Resampler,   // mic_rate -> 48k
    cap_down: Resampler, // 48k -> mic_rate
    cap_accum: Vec<f32>, // 48k samples awaiting full frames
    // Render (far-end reference) path.
    ref_up: Resampler,   // tts_rate -> 48k
    ref_accum: Vec<f32>, // 48k samples awaiting full frames
}

impl WebRtcAec {
    /// `mic_rate` is the near-end (mic/transcriber) rate; `reference_rate` is the
    /// far-end (played TTS) rate.
    pub fn new(mic_rate: u32, reference_rate: u32) -> Result<Self> {
        let apm = Processor::new(APM_RATE)
            .map_err(|e| anyhow::anyhow!("failed to create WebRTC APM: {e}"))?;
        // Full AEC3 with adaptive delay estimation (no fixed stream delay hint).
        apm.set_config(Config {
            echo_canceller: Some(EchoCanceller::Full {
                stream_delay_ms: None,
            }),
            ..Default::default()
        });
        Ok(Self {
            apm,
            cap_up: Resampler::new(mic_rate, APM_RATE),
            cap_down: Resampler::new(APM_RATE, mic_rate),
            cap_accum: Vec::new(),
            ref_up: Resampler::new(reference_rate, APM_RATE),
            ref_accum: Vec::new(),
        })
    }
}

impl Aec for WebRtcAec {
    fn push_reference(&mut self, samples: &[f32]) {
        let up = self.ref_up.process(samples);
        self.ref_accum.extend_from_slice(&up);
        while self.ref_accum.len() >= APM_FRAME {
            let mut frame: Vec<f32> = self.ref_accum.drain(..APM_FRAME).collect();
            // analyze_render_frame doesn't modify, but takes the frame by ref.
            let _ = self.apm.analyze_render_frame([frame.as_slice()]);
            frame.clear();
        }
    }

    fn process_capture(&mut self, mic: &[f32]) -> Vec<f32> {
        let up = self.cap_up.process(mic);
        self.cap_accum.extend_from_slice(&up);
        let mut cleaned_48k: Vec<f32> = Vec::with_capacity(self.cap_accum.len());
        while self.cap_accum.len() >= APM_FRAME {
            let mut frame: Vec<f32> = self.cap_accum.drain(..APM_FRAME).collect();
            if self
                .apm
                .process_capture_frame([frame.as_mut_slice()])
                .is_ok()
            {
                cleaned_48k.extend_from_slice(&frame);
            }
        }
        self.cap_down.process(&cleaned_48k)
    }

    fn reset(&mut self) {
        self.apm.reinitialize();
        self.cap_accum.clear();
        self.ref_accum.clear();
    }
}

/// A snapshot of the echo canceller's runtime stats, for on-device tuning.
#[derive(Debug, Clone, Copy, Default)]
pub struct AecStats {
    /// Echo return loss enhancement (dB): how much echo the AEC is removing.
    /// Higher is better; `None` until the AEC has adapted.
    pub erle: Option<f64>,
    /// Echo return loss (dB): far/echo ratio at the mic.
    pub erl: Option<f64>,
    /// Estimated far→near delay the AEC locked onto (ms).
    pub delay_ms: Option<u32>,
    /// Residual echo likelihood (0..1); high means echo is leaking through.
    pub residual_echo: Option<f64>,
}

impl WebRtcAec {
    /// Current echo-cancellation stats (populated after a short warmup).
    pub fn stats(&self) -> AecStats {
        let s = self.apm.get_stats();
        AecStats {
            erle: s.echo_return_loss_enhancement,
            erl: s.echo_return_loss,
            delay_ms: s.delay_ms,
            residual_echo: s.residual_echo_likelihood,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resampler_roughly_scales_length() {
        let mut up = Resampler::new(24_000, 48_000);
        let out = up.process(&vec![0.5; 240]);
        // ~2x upsample.
        assert!((out.len() as i64 - 480).abs() <= 2, "got {}", out.len());
    }

    #[test]
    fn resampler_identity_passthrough() {
        let mut r = Resampler::new(24_000, 24_000);
        let input: Vec<f32> = (0..100).map(|i| i as f32).collect();
        assert_eq!(r.process(&input), input);
    }

    /// Feeding the played reference as the (scaled) echo in the mic should be
    /// substantially cancelled after the AEC warms up.
    #[test]
    fn cancels_echo_after_warmup() {
        let mut aec = WebRtcAec::new(24_000, 24_000).unwrap();
        let frame = 240usize; // 10ms @ 24k
        let make_ref = |phase: usize| -> Vec<f32> {
            (0..frame)
                .map(|i| ((i + phase) as f32 / 12.0).sin() * 0.5)
                .collect()
        };

        // Warm up: reference played, mic hears it as echo (0.8x).
        let mut residual_energy = 0.0f32;
        let mut echo_energy = 0.0f32;
        for iter in 0..200 {
            let reference = make_ref(iter * frame);
            let echo: Vec<f32> = reference.iter().map(|s| s * 0.8).collect();
            aec.push_reference(&reference);
            let cleaned = aec.process_capture(&echo);
            if iter >= 150 {
                echo_energy += echo.iter().map(|s| s * s).sum::<f32>();
                residual_energy += cleaned.iter().map(|s| s * s).sum::<f32>();
            }
        }
        assert!(echo_energy > 0.0);
        let reduction_db = 10.0 * (echo_energy / residual_energy.max(1e-9)).log10();
        assert!(
            reduction_db > 6.0,
            "expected >6dB echo reduction after warmup, got {reduction_db:.1}dB"
        );
    }
}
