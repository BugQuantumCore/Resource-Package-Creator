//! BVE real-sound-source playback: curve-driven loop resampling.
//!
//! Instead of synthesising a VVVF motor, this engine plays actual PCM motor
//! recordings ("real sound sources") the way BVE does: each motor channel is
//! a looping WAV whose playback rate follows the frequency tables
//! (`powerfreq` / `brakefreq`) and whose gain follows the volume tables
//! (`powervol` / `brakevol`), both keyed by speed and selected by traction
//! effort. Gains are one-pole smoothed to avoid clicks when the curves step.
//!
//! Channel WAVs come from the sound set's `sound.cfg` `[SoundList]` section
//! (index → file) or can be supplied manually.

use std::sync::Arc;

use crate::bve::BveMotorData;

/// A decoded mono PCM loop.
#[derive(Debug, Clone, Default)]
pub struct WavSample {
    pub data: Vec<f32>,
    pub sample_rate: u32,
}

impl WavSample {
    /// Duration in seconds.
    pub fn duration(&self) -> f32 {
        if self.sample_rate == 0 {
            0.0
        } else {
            self.data.len() as f32 / self.sample_rate as f32
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum WavError {
    #[error("WAV: {0}")]
    Parse(String),
    #[error("WAV: unsupported format {0}")]
    Unsupported(String),
}

/// Read a RIFF WAVE file (PCM 8/16/24/32-bit, IEEE float 32/64-bit, and the
/// WAVE_FORMAT_EXTENSIBLE wrapper) into mono f32 samples.
pub fn read_wav(bytes: &[u8]) -> Result<WavSample, WavError> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(WavError::Parse("not a RIFF/WAVE file".into()));
    }
    let mut pos = 12;
    let mut format: Option<u16> = None;
    let mut channels: Option<u16> = None;
    let mut sample_rate: Option<u32> = None;
    let mut bits: Option<u16> = None;
    let mut data: Option<&[u8]> = None;
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let size = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().unwrap()) as usize;
        let body_start = pos + 8;
        let body_end = body_start
            .checked_add(size)
            .ok_or_else(|| WavError::Parse("chunk length overflow".into()))?;
        if body_end > bytes.len() {
            return Err(WavError::Parse("chunk overruns file".into()));
        }
        let body = &bytes[body_start..body_end];
        match id {
            b"fmt " => {
                if body.len() < 16 {
                    return Err(WavError::Parse("fmt chunk too short".into()));
                }
                let mut f = u16::from_le_bytes([body[0], body[1]]);
                if f == 0xFFFE {
                    // WAVE_FORMAT_EXTENSIBLE: real format in the SubFormat GUID.
                    if body.len() >= 40 {
                        f = u16::from_le_bytes([body[24], body[25]]);
                    }
                }
                format = Some(f);
                channels = Some(u16::from_le_bytes([body[2], body[3]]));
                sample_rate = Some(u32::from_le_bytes(body[4..8].try_into().unwrap()));
                bits = Some(u16::from_le_bytes([body[14], body[15]]));
            }
            b"data" => {
                data = Some(body);
            }
            _ => {}
        }
        pos = body_start + size + (size & 1); // chunks are word-aligned
    }
    let format = format.ok_or_else(|| WavError::Parse("missing fmt chunk".into()))?;
    let channels = channels.unwrap_or(1).max(1) as usize;
    let sample_rate = sample_rate.ok_or_else(|| WavError::Parse("missing sample rate".into()))?;
    let bits = bits.unwrap_or(16);
    let data = data.ok_or_else(|| WavError::Parse("missing data chunk".into()))?;

    let bytes_per_sample = match (format, bits) {
        (1, 8) | (1, 16) | (1, 24) | (1, 32) => (bits / 8) as usize,
        (3, 32) | (3, 64) => (bits / 8) as usize,
        _ => {
            return Err(WavError::Unsupported(format!("format {format}, {bits}-bit")));
        }
    };
    let frame_size = bytes_per_sample * channels;
    let frames = data.len() / frame_size;
    let mut mono = Vec::with_capacity(frames);
    for i in 0..frames {
        let frame = &data[i * frame_size..(i + 1) * frame_size];
        let mut acc = 0f32;
        for c in 0..channels {
            let s = &frame[c * bytes_per_sample..(c + 1) * bytes_per_sample];
            let v = match (format, bytes_per_sample) {
                (1, 1) => (s[0] as f32 - 128.0) / 128.0,
                (1, 2) => i16::from_le_bytes([s[0], s[1]]) as f32 / 32768.0,
                (1, 3) => {
                    let v = (s[0] as i32) | ((s[1] as i32) << 8) | ((s[2] as i32) << 16);
                    // Sign-extend 24-bit.
                    let v = if v & 0x80_0000 != 0 { v - 0x100_0000 } else { v };
                    v as f32 / 8_388_608.0
                }
                (1, 4) => i32::from_le_bytes([s[0], s[1], s[2], s[3]]) as f32 / 2_147_483_648.0,
                (3, 4) => f32::from_le_bytes([s[0], s[1], s[2], s[3]]),
                (3, 8) => {
                    // 64-bit float: truncate to f32.
                    let mut b = [0u8; 8];
                    b.copy_from_slice(s);
                    f64::from_le_bytes(b) as f32
                }
                _ => 0.0,
            };
            acc += v;
        }
        mono.push(acc / channels as f32);
    }
    Ok(WavSample { data: mono, sample_rate })
}

#[derive(Debug, Clone)]
struct Channel {
    buf: Arc<Vec<f32>>,
    /// Playback positions advance at `rate × pitch` per output sample.
    pos: f64,
    /// Source rate relative to the output rate.
    rate: f64,
    gain: f32,
}

/// Curve-driven real-sound-source playback engine.
#[derive(Debug, Clone)]
pub struct BvePlaybackEngine {
    channels: Vec<Channel>,
    out_rate: f64,
    /// One-pole gain smoothing coefficient (~10 ms).
    smooth: f32,
}

impl BvePlaybackEngine {
    /// Build an engine from per-channel loops.
    pub fn new(samples: &[WavSample], out_rate: f64) -> BvePlaybackEngine {
        let channels = samples
            .iter()
            .filter(|s| !s.data.is_empty() && s.sample_rate > 0)
            .map(|s| Channel {
                buf: Arc::new(s.data.clone()),
                pos: 0.0,
                rate: s.sample_rate as f64 / out_rate.max(1.0),
                gain: 0.0,
            })
            .collect();
        let smooth = (1.0 - (-1.0 / (0.01 * out_rate.max(1.0))).exp()) as f32;
        BvePlaybackEngine { channels, out_rate, smooth }
    }

    pub fn channel_count(&self) -> usize {
        self.channels.len()
    }

    /// Total loop duration of the longest channel.
    pub fn loop_duration(&self) -> f32 {
        self.channels
            .iter()
            .map(|c| c.buf.len() as f32 / (self.out_rate * c.rate) as f32)
            .fold(0.0, f32::max)
    }

    /// Share buffers with a clone for another thread (audio callback).
    pub fn clone_shared(&self) -> BvePlaybackEngine {
        BvePlaybackEngine {
            channels: self
                .channels
                .iter()
                .map(|c| Channel {
                    buf: Arc::clone(&c.buf),
                    pos: 0.0,
                    rate: c.rate,
                    gain: 0.0,
                })
                .collect(),
            out_rate: self.out_rate,
            smooth: self.smooth,
        }
    }

    /// Replace all channels with copies of one loop.
    pub fn set_single_loop(&mut self, sample: &WavSample) {
        let buf = Arc::new(sample.data.clone());
        let rate = sample.sample_rate as f64 / self.out_rate.max(1.0);
        let n = self.channels.len().max(1);
        let channels: Vec<Channel> = (0..n)
            .map(|_| Channel {
                buf: Arc::clone(&buf),
                pos: 0.0,
                rate,
                gain: 0.0,
            })
            .collect();
        self.channels = channels;
    }

    /// Render with rates/gains driven directly by the caller.
    pub fn render(&mut self, rates: &[f32], gains: &[f32], out: &mut [f32]) {
        for x in out.iter_mut() {
            *x = 0.0;
        }
        for (i, ch) in self.channels.iter_mut().enumerate() {
            let rate = rates.get(i).copied().unwrap_or(0.0).max(0.0) as f64;
            let target = gains.get(i).copied().unwrap_or(0.0).clamp(0.0, 4.0);
            let len = ch.buf.len();
            if len == 0 {
                continue;
            }
            let step = ch.rate * rate;
            for x in out.iter_mut() {
                // Smooth the gain per-sample (cheap, click-free).
                ch.gain += (target - ch.gain) * self.smooth;
                let advance = |ch: &mut Channel| {
                    ch.pos += step;
                    if ch.pos >= len as f64 {
                        ch.pos -= (len as f64) * (ch.pos / len as f64).floor();
                    }
                };
                if ch.gain <= 1e-5 && target <= 1e-5 {
                    // Silent: keep the playhead moving for phase realism.
                    advance(ch);
                    continue;
                }
                let i0 = ch.pos as usize;
                let frac = (ch.pos - i0 as f64) as f32;
                let i1 = if i0 + 1 < len { i0 + 1 } else { 0 };
                let s0 = ch.buf[i0];
                let s1 = ch.buf[i1];
                *x += (s0 + (s1 - s0) * frac) * ch.gain;
                advance(ch);
            }
        }
        // Soft clip.
        for x in out.iter_mut() {
            if *x > 1.0 {
                *x = 1.0;
            } else if *x < -1.0 {
                *x = -1.0;
            }
        }
    }

    /// Render with rates/gains derived from BVE motor data curves.
    pub fn render_curves(
        &mut self,
        data: &BveMotorData,
        speed_ms: f32,
        power: f32,
        out: &mut [f32],
    ) {
        let n = self.channels.len();
        let mut rates = vec![0f32; n];
        let mut gains = vec![0f32; n];
        for i in 0..n {
            rates[i] = data.get_pitch(i, speed_ms, power);
            gains[i] = data.get_volume(i, speed_ms, power);
        }
        self.render(&rates, &gains, out);
    }

    /// Current rate/gain pairs from BVE curves (for the audio thread).
    pub fn curve_drive(data: &BveMotorData, channel_count: usize, speed_ms: f32, power: f32) -> (Vec<f32>, Vec<f32>) {
        let mut rates = vec![0f32; channel_count];
        let mut gains = vec![0f32; channel_count];
        for i in 0..channel_count {
            rates[i] = data.get_pitch(i, speed_ms, power);
            gains[i] = data.get_volume(i, speed_ms, power);
        }
        (rates, gains)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bve::{BveMotorData5, FloatSplines};

    fn wav_bytes(sr: u32, samples: &[f32]) -> Vec<u8> {
        let mut data = Vec::new();
        for s in samples {
            data.extend_from_slice(&(s.clamp(-1.0, 1.0) as f32).to_le_bytes());
        }
        let mut out = Vec::new();
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&3u16.to_le_bytes()); // IEEE float
        out.extend_from_slice(&1u16.to_le_bytes()); // mono
        out.extend_from_slice(&sr.to_le_bytes());
        out.extend_from_slice(&(sr * 4).to_le_bytes());
        out.extend_from_slice(&4u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&data);
        out
    }

    #[test]
    fn wav_roundtrip_float32() {
        let bytes = wav_bytes(48_000, &[0.0, 0.5, -0.5, 0.25]);
        let wav = read_wav(&bytes).unwrap();
        assert_eq!(wav.sample_rate, 48_000);
        assert_eq!(wav.data.len(), 4);
        assert!((wav.data[1] - 0.5).abs() < 1e-6);
        assert!((wav.data[2] + 0.5).abs() < 1e-6);
    }

    #[test]
    fn wav_pcm16_stereo_mixdown() {
        let mut data = Vec::new();
        // Frames: (L=0.5, R=-0.5) and (L=1.0, R=-1.0) → mono 0.0 both.
        for v in [0.5f32, -0.5, 1.0, -1.0] {
            data.extend_from_slice(&((v * 32767.0) as i16).to_le_bytes());
        }
        let mut out = Vec::new();
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes()); // PCM
        out.extend_from_slice(&2u16.to_le_bytes()); // stereo
        out.extend_from_slice(&48_000u32.to_le_bytes());
        out.extend_from_slice(&(48_000u32 * 4).to_le_bytes());
        out.extend_from_slice(&4u16.to_le_bytes());
        out.extend_from_slice(&16u16.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&data);
        let wav = read_wav(&out).unwrap();
        assert_eq!(wav.data.len(), 2);
        assert!(wav.data.iter().all(|x| x.abs() < 1e-3));
    }

    #[test]
    fn engine_follows_curves() {
        // A 100 Hz sine single-cycle loop at 48 kHz.
        let sr = 48_000u32;
        let period = sr / 100;
        let loop_samples: Vec<f32> = (0..period)
            .map(|i| ((i as f32 / period as f32) * 2.0 * std::f32::consts::PI).sin() * 0.8)
            .collect();
        let wav = WavSample { data: loop_samples.clone(), sample_rate: sr };

        let data = BveMotorData::V5(BveMotorData5 {
            power_frequency: FloatSplines::parse("0,0.5\n10,1.0"),
            power_volume: FloatSplines::parse("0,0.5\n10,1.0"),
            ..Default::default()
        });
        let mut engine = BvePlaybackEngine::new(&[wav], sr as f64);
        assert_eq!(engine.channel_count(), 1);

        // Render 1 s at speed 0 (pitch 0.5) → ~50 Hz.
        let mut out = vec![0f32; sr as usize];
        engine.render_curves(&data, 0.0, 1.0, &mut out);
        // Skip the gain-smoothing attack, then measure zero crossings.
        let tail = &out[sr as usize / 10..];
        let crossings = tail
            .windows(2)
            .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
            .count();
        // 50 Hz → 50 positive-going crossings per second; allow ±20%.
        let measured_hz = crossings as f32 / 2.0 / (tail.len() as f32 / sr as f32);
        assert!(
            (measured_hz - 50.0).abs() < 10.0,
            "measured {measured_hz:.1} Hz, expected ~50"
        );
        // Non-silent.
        let peak = tail.iter().fold(0f32, |a, b| a.max(b.abs()));
        assert!(peak > 0.1, "peak {peak}");

        // Coast: silence.
        let mut out2 = vec![0f32; 4800];
        engine.render_curves(&data, 5.0, 0.0, &mut out2);
        let peak2 = out2.iter().fold(0f32, |a, b| a.max(b.abs()));
        assert!(peak2 < 1e-3, "coasting should be silent, peak {peak2}");
    }

    #[test]
    fn clone_shared_renders_identically() {
        let wav = WavSample {
            data: vec![0.5f32; 480],
            sample_rate: 48_000,
        };
        let data = BveMotorData::V5(BveMotorData5 {
            power_frequency: FloatSplines::parse("0,1.0"),
            power_volume: FloatSplines::parse("0,1.0"),
            ..Default::default()
        });
        let mut a = BvePlaybackEngine::new(&[wav], 48_000.0);
        let mut b = a.clone_shared();
        let mut oa = vec![0f32; 1000];
        let mut ob = vec![0f32; 1000];
        // Run a for a while so smoothing settles, then compare fresh renders.
        let mut warm = vec![0f32; 4800];
        a.render_curves(&data, 0.0, 1.0, &mut warm);
        b.render_curves(&data, 0.0, 1.0, &mut warm);
        a.render_curves(&data, 0.0, 1.0, &mut oa);
        b.render_curves(&data, 0.0, 1.0, &mut ob);
        assert_eq!(oa, ob);
    }
}
