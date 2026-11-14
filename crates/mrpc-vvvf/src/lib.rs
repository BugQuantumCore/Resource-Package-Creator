//! Safe Rust wrapper around the C++ VVVF synthesis engine (`cpp/vvvf.*`).
//!
//! The engine models a two-level VVVF inverter driving an RL motor load and
//! renders the audible result: SPWM (natural sampling) or SHE modulation, a
//! synchronous pulse-count band at low speed, V/f control with field
//! weakening, DC-blocked current/voltage mix and speed-scaled rolling noise.

use std::os::raw::{c_double, c_float, c_int};

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct VvvfParams {
    /// 0 = SPWM, 1 = SHE.
    pub scheme: i32,
    pub sample_rate: f64,
    /// Asynchronous carrier frequency (Hz).
    pub carrier_freq: f64,
    /// Pole pairs — informational; the fundamental is commanded directly.
    pub pole_pairs: f64,
    /// V/f base frequency (Hz).
    pub base_freq_hz: f64,
    /// Peak modulation index at base frequency (≤ ~1.1).
    pub max_mod_depth: f64,
    /// DC link voltage (V).
    pub dc_bus: f64,
    pub sync_min_ratio: std::os::raw::c_int,
    pub sync_max_ratio: std::os::raw::c_int,
    /// Phase resistance (Ω).
    pub load_r: f64,
    /// Phase inductance (H).
    pub load_l: f64,
    /// Rolling/fan noise mix 0..1.
    pub noise_gain: f64,
    /// 0..1 speed fraction for noise scaling.
    pub speed_fraction: f64,
}

impl Default for VvvfParams {
    fn default() -> Self {
        Self {
            scheme: 0,
            sample_rate: 48_000.0,
            carrier_freq: 1000.0,
            pole_pairs: 4.0,
            base_freq_hz: 60.0,
            max_mod_depth: 1.0,
            dc_bus: 1500.0,
            sync_min_ratio: 3,
            sync_max_ratio: 15,
            load_r: 0.8,
            load_l: 0.004,
            noise_gain: 0.25,
            speed_fraction: 0.0,
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct VvvfState {
    shaft_angle: f64,
    carrier_phase: f64,
    current: f64,
    hp_state: f64,
    sample_index: u64,
}

#[link(name = "mrpcvvvf")]
extern "C" {
    fn mrpc_vvvf_reset(state: *mut VvvfState);
    fn mrpc_vvvf_render(
        params: *const VvvfParams,
        state: *mut VvvfState,
        fundamental_hz: c_double,
        power: c_double,
        out: *mut c_float,
        n: c_int,
    );
    fn mrpc_vvvf_abi_version() -> c_int;
}

/// A synthesiser instance. `render` can be called repeatedly with changing
/// frequency/effort — phase and load state carry across calls.
#[derive(Debug)]
pub struct Engine {
    params: VvvfParams,
    state: VvvfState,
}

impl Engine {
    pub fn new(params: VvvfParams) -> Engine {
        Engine { params, state: VvvfState::default() }
    }

    pub fn params(&self) -> &VvvfParams {
        &self.params
    }

    pub fn params_mut(&mut self) -> &mut VvvfParams {
        &mut self.params
    }

    pub fn reset(&mut self) {
        unsafe { mrpc_vvvf_reset(&mut self.state) };
    }

    /// Render `out.len()` samples at the given electrical fundamental (Hz)
    /// and traction effort `power` ∈ [-1, 1].
    pub fn render(&mut self, fundamental_hz: f64, power: f64, out: &mut [f32]) {
        unsafe {
            mrpc_vvvf_render(
                &self.params,
                &mut self.state,
                fundamental_hz as c_double,
                power as c_double,
                out.as_mut_ptr(),
                out.len() as c_int,
            );
        }
    }

    /// ABI version of the linked C++ engine.
    pub fn abi_version() -> i32 {
        unsafe { mrpc_vvvf_abi_version() as i32 }
    }
}

/// Electrical fundamental frequency for a train speed. MTR speeds are in
/// blocks-per-second internally; `speed_kmh / 3.6 / block_length_m` gives
/// blocks/s. This helper turns a shaft rps and pole pairs into the fundamental.
pub fn fundamental_hz(shaft_rps: f64, pole_pairs: f64) -> f64 {
    (shaft_rps * pole_pairs).max(0.0)
}

/// Discrete Fourier magnitude of `data` at `freq_hz` (Goertzel).
pub fn goertzel_magnitude(data: &[f32], sample_rate: f64, freq_hz: f64) -> f32 {
    let n = data.len();
    if n == 0 {
        return 0.0;
    }
    let k = (freq_hz * n as f64 / sample_rate).round() as usize;
    if k == 0 || k > n / 2 {
        return 0.0;
    }
    let w = 2.0 * std::f64::consts::PI * k as f64 / n as f64;
    let (s, c) = w.sin_cos();
    let coeff = 2.0 * c;
    let mut s_prev = 0.0f64;
    let mut s_prev2 = 0.0f64;
    for &x in data {
        let s = x as f64 + coeff * s_prev - s_prev2;
        s_prev2 = s_prev;
        s_prev = s;
    }
    let real = s_prev - c * s_prev2;
    let imag = s * s_prev2;
    ((real * real + imag * imag).sqrt() / n as f64) as f32 * 2.0
}

/// Write a mono 16-bit WAV file.
pub fn write_wav_mono16(path: &std::path::Path, sample_rate: u32, samples: &[f32]) -> std::io::Result<()> {
    use std::io::Write;
    let mut data = Vec::with_capacity(samples.len() * 2);
    for s in samples {
        let v = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
        data.extend_from_slice(&v.to_le_bytes());
    }
    let mut file = std::fs::File::create(path)?;
    let byte_rate = sample_rate * 2;
    file.write_all(b"RIFF")?;
    file.write_all(&(36 + data.len() as u32).to_le_bytes())?;
    file.write_all(b"WAVEfmt ")?;
    file.write_all(&16u32.to_le_bytes())?; // PCM chunk size
    file.write_all(&1u16.to_le_bytes())?; // PCM
    file.write_all(&1u16.to_le_bytes())?; // mono
    file.write_all(&sample_rate.to_le_bytes())?;
    file.write_all(&byte_rate.to_le_bytes())?;
    file.write_all(&2u16.to_le_bytes())?; // block align
    file.write_all(&16u16.to_le_bytes())?; // bits
    file.write_all(b"data")?;
    file.write_all(&(data.len() as u32).to_le_bytes())?;
    file.write_all(&data)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abi_version_positive() {
        assert!(Engine::abi_version() >= 1);
    }

    #[test]
    fn fundamental_present() {
        let mut engine = Engine::new(VvvfParams::default());
        let n = 48_000;
        let mut buf = vec![0f32; n];
        engine.render(120.0, 1.0, &mut buf);
        let mag = goertzel_magnitude(&buf, 48_000.0, 120.0);
        assert!(mag > 0.05, "fundamental magnitude too small: {mag}");
        // Energy present at all?
        let rms = (buf.iter().map(|x| x * x).sum::<f32>() / n as f32).sqrt();
        assert!(rms > 0.01, "rms too small: {rms}");
    }

    #[test]
    fn silence_when_coasting_and_stopped() {
        let mut engine = Engine::new(VvvfParams { noise_gain: 0.0, ..Default::default() });
        let mut buf = vec![0f32; 4800];
        engine.render(0.0, 0.0, &mut buf);
        let peak = buf.iter().fold(0f32, |a, b| a.max(b.abs()));
        assert!(peak < 1e-4, "expected silence, peak {peak}");
    }

    #[test]
    fn fundamental_tracks_command() {
        // Sweep the frequency and confirm the peak follows via Goertzel.
        for f in [40.0, 90.0, 150.0] {
            let mut engine = Engine::new(VvvfParams {
                base_freq_hz: 200.0, // stay in the constant-flux region
                ..Default::default()
            });
            let mut buf = vec![0f32; 48_000];
            engine.render(f, 1.0, &mut buf);
            let here = goertzel_magnitude(&buf, 48_000.0, f);
            let off = goertzel_magnitude(&buf, 48_000.0, f * 1.5);
            assert!(here > off * 1.5, "f={f}: {here} vs off {off}");
        }
    }

    #[test]
    fn wav_written() {
        let dir = std::env::temp_dir().join("mrpc_vvvf_test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.wav");
        write_wav_mono16(&path, 48_000, &[0.0, 0.5, -0.5]).unwrap();
        assert!(std::fs::metadata(&path).unwrap().len() > 44);
    }
}
