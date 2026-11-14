# MTR Resource Package Creator (English)

A **standalone desktop** custom-resource-pack creator for Minecraft Transit
Railway 4.0.x — Windows, Linux and macOS, no Minecraft installation required.
Written in Rust + C++ (egui + wgpu UI, C++ DSP and native script host).

Features: Blender-style dark 3D model preview (OBJ + MTL, Blockbench
`.bbmodel`), a full door open/close animation timeline (the twelve MTR door
curves ported 1:1, TSC stop timings), a three-mode VVVF audition module
(C++ SPWM/SHE synthesiser with live waveform + spectrum, BVE5/BVE4 motor-data
tables, legacy MTR3 speed-sound emulation, WAV rendering), and JCM
(Yanyang) train-script support: editing of `vehicleScripts` entries,
JavaScript test-running in a QuickJS sandbox with a simulated train, and a
C++ native-module preview host (`MRPC_REGISTER_VEHICLE_SCRIPT`).

UI languages: 简体中文（中国大陆）, 繁體中文（香港特別行政區）, 繁體中文（台灣省）,
文言（華夏）, English (United States), English (United Kingdom), Français (France).

```sh
cargo run -p mrpc-app --release                 # GUI
cargo run -p mrpc-app --release --features audio  # with live audio (Linux: libasound2-dev)
cargo test --workspace                           # 37 unit tests
cargo run -p mrpc-core --example gen_demo -- ./out
```

See [README.md](README.md) (Chinese, the main document), [BUILDING.md](BUILDING.md)
and [ARCHITECTURE.md](ARCHITECTURE.md). MIT licensed; format knowledge derived
from reading the open-source MTR, TSC and Joban Client Mod (Yanyang) projects —
no code or assets from them are included.
