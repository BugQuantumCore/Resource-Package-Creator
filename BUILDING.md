# Building

## Prerequisites

| Component | Requirement |
| --- | --- |
| Rust | 1.82 or newer (`rustup default stable`) |
| C++ compiler | C++17: g++ 9+, clang++ 10+, or MSVC 19.28+ (VS 2022) |
| Linux audio *(optional)* | `libasound2-dev` for the `audio` feature (cpal/ALSA) |

The Rust toolchain downloads and compiles QuickJS (via `rquickjs`) and the
VVVF DSP (`cc` crate compiles `crates/mrpc-vvvf/cpp`), so there are no other
external dependencies.

## Build & run

```sh
cargo run -p mrpc-app --release                 # GUI (no audio output)
cargo run -p mrpc-app --release --features audio  # GUI + live VVVF playback
cargo test --workspace                           # unit tests
cargo run -p mrpc-core --example gen_demo -- ./out
```

First builds take a while (wgpu/egui dependency tree). Subsequent builds are
incremental.

## Platform notes

### Windows

- Install [Build Tools for Visual Studio](https://visualstudio.microsoft.com/downloads/)
  (Desktop development with C++).
- `cargo run -p mrpc-app --release` produces
  `target\release\mtr-resource-package-creator.exe`.
- The GUI font loader picks up Microsoft YaHei / SimHei automatically; no
  bundling needed.

### Linux

```sh
sudo apt install build-essential libasound2-dev   # Debian/Ubuntu
# or: sudo dnf install gcc-c++ alsa-lib-devel     # Fedora
```

The font loader searches common Noto Sans CJK paths. If your distribution
installs it elsewhere, drop any CJK TTF/OTF at `~/.fonts/`.

### macOS

```sh
xcode-select --install
cargo run -p mrpc-app --release
```

PingFang is picked up automatically. Build universal binaries with
`cargo build --target aarch64-apple-darwin` plus
`--target x86_64-apple-darwin` and `lipo`.

## Feature flags (`mrpc-app`)

| Feature | Default | Effect |
| --- | --- | --- |
| `audio` | off | Live VVVF playback through cpal (WASAPI/CoreAudio/ALSA). Without it the VVVF panel still shows the live waveform/spectrum and can render WAV files. |

## Release packaging (all platforms)

```sh
cargo build -p mrpc-app --release
# The binary is self-contained; ship it together with assets/i18n only if you
# intend to edit translations at runtime (they are embedded by default).
```

CI builds for the three platforms are defined in
`.github/workflows/ci.yml`.
