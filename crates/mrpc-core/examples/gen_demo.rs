//! Generate the demonstration resource pack and a VVVF audition WAV.
//!
//! Usage: `cargo run -p mrpc-core --example gen_demo [output-dir]`
//! Writes `mrpc-demo-pack.zip` and `vvvf-demo.wav` into the output directory
//! (default: the current directory).

use mrpc_core::demo::create_demo_pack;
use mrpc_vvvf::{write_wav_mono16, Engine, VvvfParams};

fn main() {
    let out_dir = std::env::args().nth(1).unwrap_or_else(|| ".".to_string());
    let out_dir = std::path::PathBuf::from(out_dir);
    std::fs::create_dir_all(&out_dir).expect("create output dir");

    // ---- demo pack ---------------------------------------------------------
    let pack = create_demo_pack();
    let validation = mrpc_core::validate::validate_pack(&pack);
    let (errors, warnings, hints) = mrpc_core::validate::summarize(&validation);
    for item in &validation {
        println!("[{:?}] {} {}", item.severity, item.key, item.detail);
    }
    println!(
        "validation: {errors} error(s), {warnings} warning(s), {hints} hint(s); {} file(s)",
        pack.files.len()
    );

    let zip_path = out_dir.join("mrpc-demo-pack.zip");
    pack.export_zip(&zip_path).expect("export demo pack");
    println!("pack written: {}", zip_path.display());

    // ---- VVVF audition WAV -------------------------------------------------
    // A 12-second run: accelerate 0→80 km/h (P3), cruise, coast, brake (B3),
    // stop and dwell. Drives the C++ synthesis engine block-wise.
    let sr = 48_000u32;
    let mut params = VvvfParams::default();
    params.noise_gain = 0.2;
    let mut engine = Engine::new(params);
    let mut samples: Vec<f32> = Vec::new();
    let block = sr as usize / 10; // 100 ms blocks
    let total_s = 12.0f64;
    let mut t = 0.0f64;
    while t < total_s {
        let (speed_kmh, power) = if t < 6.0 {
            ((t / 6.0) * 80.0, (t / 6.0).min(1.0) * 0.6)
        } else if t < 8.0 {
            (80.0, 0.0)
        } else if t < 10.5 {
            (80.0 - (t - 8.0) * 32.0, -((t - 8.0) / 2.5) * 0.8)
        } else {
            (0.0, 0.0)
        };
        // Wheel circumference 0.85 m, gear ratio 6.2, 4 pole pairs.
        let speed_ms = speed_kmh / 3.6;
        let fundamental = mrpc_vvvf::fundamental_hz(speed_ms / 0.85 / 6.2, params.pole_pairs);
        let mut buf = vec![0f32; block];
        engine.render(fundamental, power, &mut buf);
        samples.extend_from_slice(&buf);
        t += 0.1;
    }
    let wav_path = out_dir.join("vvvf-demo.wav");
    write_wav_mono16(&wav_path, sr, &samples).expect("write wav");
    println!("vvvf written: {} ({:.1} s)", wav_path.display(), total_s);
}
