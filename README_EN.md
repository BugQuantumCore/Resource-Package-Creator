# MTR Resource Package Creator (English)

A **standalone desktop** custom-resource-pack creator for Minecraft Transit
Railway 4.0.x — Windows, Linux and macOS, no Minecraft installation required.
Written in Rust + C++ (egui + wgpu UI, C++ DSP and native script host).

Features: Blender-style dark 3D model preview (OBJ + MTL, Blockbench
`.bbmodel`), external model import (Metasequoia `.mqo`/`.mqoz` and glTF 2.0
`.gltf`/`.glb`, converted to OBJ + MTL + PNG resources attached to the
vehicle), a full door open/close animation timeline (the twelve MTR door
curves ported 1:1, TSC stop timings), a three-mode VVVF audition module
(C++ SPWM/SHE synthesiser with live waveform + spectrum, BVE5/BVE4 motor-data
tables with **real sound source playback** — WAV loops resampled at the
frequency-table rate and volume-table gain, legacy MTR3 speed-sound emulation,
WAV rendering), and JCM (Yanyang) train-script support: editing of
`vehicleScripts` entries, JavaScript test-running in a QuickJS sandbox with a
simulated train, and a C++ native-module preview host
(`MRPC_REGISTER_VEHICLE_SCRIPT`).

UI languages: 简体中文（中国大陆）, 繁體中文（香港特別行政區）, 繁體中文（台灣省）,
文言（華夏）, English (United States), English (United Kingdom), Français (France).

```sh
cargo run -p mrpc-app --release                 # GUI
cargo run -p mrpc-app --release --features audio  # with live audio (Linux: libasound2-dev)
cargo test --workspace                           # 54 unit tests
cargo run -p mrpc-core --example gen_demo -- ./out
```

See [README.md](README.md) (Chinese, the main document), [BUILDING.md](BUILDING.md)
and [ARCHITECTURE.md](ARCHITECTURE.md). MIT licensed; format knowledge derived
from reading the open-source MTR, TSC and Joban Client Mod (Yanyang) projects —
no code or assets from them are included.

## Downloads & installation

Prebuilt artifacts for every platform are published on
[GitHub Releases](https://github.com/BugQuantumCore/Resource-Package-Creator/releases)
(automatically built for `v*` tags): portable `.zip` archives plus native
installers — Windows `.msi` (Program Files + Start Menu shortcut), Linux
`.deb` / `.rpm` (depend on libasound2), and macOS `.dmg` containing the `.app`
bundle (Apple Silicon; first launch may need right-click → Open).

## Troubleshooting startup problems (v0.2.1)

Startup is now fail-visible instead of failing silently behind
`windows_subsystem`: startup errors raise a native dialog and are written to a
log file; the graphics device is retried with a software renderer
(WARP / llvmpipe); audio is opened on a background thread and can no longer
block the first frame (fixes the reported "a process exists but no window
appears" behaviour).

* Log file: `mtr-resource-package-creator.log` next to the executable, or
  `%LOCALAPPDATA%\mtr-resource-package-creator\app.log` when the exe directory
  is read-only (Program Files); on Linux
  `~/.local/state/mtr-resource-package-creator/app.log`.
* Diagnostic switches: `MRPC_LOG_LEVEL=debug`, `MRPC_NO_AUDIO=1`,
  `WGPU_BACKEND=gl`, `WGPU_POWER_PREF=low`.
* If a dialog reports a startup failure, please attach the log file at
  <https://github.com/BugQuantumCore/Resource-Package-Creator/issues>.
