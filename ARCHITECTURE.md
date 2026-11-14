# Architecture

```
┌──────────────────────────────── mrpc-app (egui + wgpu) ────────────────────────────────┐
│  menu · outliner · properties · viewport(3D) · anim timeline · VVVF · script · export  │
└────────────┬──────────────────────┬───────────────────────┬───────────────────────────┘
             │                      │                       │
      mrpc-core (Rust)       mrpc-vvvf (C++ DSP)      mrpc-script (Rust + QuickJS/C++)
      pack model & IO        SPWM/SHE synthesis       JS sandbox, JCM-compatible API,
      OBJ/MTL/BBModel        sync band, RL load       C++ native-module host (dlopen)
      door animation math    BVE5/BVE4 tables         script templates
      BVE parsing            noise, DC block
      validation, zip        WAV writer
      i18n (7 locales)
```

## Data flow

1. **Pack loading** — `ResourcePack::load_zip / load_dir` reads every file,
   parses `assets/mtr/mtr_custom_resources.json` into typed structures
   (`VehicleResource`, `VehicleModel`, `ModelProperties`, `PositionDefinitions`,
   `VehicleScriptEntry`) while **preserving unknown keys** (`serde` flatten
   maps) so third-party extensions round-trip untouched. The legacy MTR 3
   format (`custom_trains` maps) is kept verbatim in `CustomResources::legacy`
   and re-merged on export.
2. **Model resolution** — for the selected vehicle, `EditorState::reload_model`
   resolves `modelResource` / `textureResource` /
   `modelPropertiesResource` / `positionDefinitionsResource` exactly like MTR's
   `CustomResourceTools` (namespace default `mtr`, extension defaults,
   same-path MTL and textures, `default.png` → vehicle texture). `.bbmodel`
   files are converted to the same mesh representation as OBJ.
3. **Scene build** — `EditorState::build_scene` expands
   `parts × names × placements` (positions JSON), applies the door animation
   offsets from `DoorAnimationType` (a 1:1 port of the MTR curves) at the
   current `doorValue`, and produces CPU meshes + instances.
4. **Render** — `ViewportShared` (all fields behind `Mutex`) is read by the
   `egui_wgpu::CallbackTrait` implementation on the render thread: one
   instanced pipeline (per-instance model matrix + tint + flags), one line
   pipeline for the grid, and instance data CPU-sorted back-to-front because
   the egui render pass has no depth buffer.
5. **VVVF** — the C++ engine (`mrpc-vvvf/cpp/vvvf.{h,cpp}`, C ABI) renders
   blocks of PCM from (fundamental Hz, power). The GUI updates a 1024-sample
   waveform and a 48-bar log spectrum (Goertzel) every frame; with the
   `audio` feature a cpal stream drives the same engine; `render_wav` writes
   5 s auditions. BVE mode parses the vehicle's `sound.cfg` +
   `powervol/powerfreq/brakevol/brakefreq.csv` (type 5) or `train.dat`
   (type 4) and drives the synthesiser from the pitch/volume tables.
6. **Scripts** — `JsSandbox` embeds QuickJS (`rquickjs`), installs
   `print`/`console`, `Timing`, `TextUtil.cycleString`, `Resources.id`,
   `include` (resolving `scriptLocations`), `SCRIPT_INPUT`, then calls the
   script's global `create`/`render`/`dispose(ctx, state, vehicle)` with a
   `vehicle` wrapper implementing the JCM `VehicleWrapper` vocabulary
   (getSpeedKmh, getDoorValue, getNotchLevel, getStops,
   getDirectionOfDoorOpening, …) from simulated train data. `NativePreview`
   dlopens modules compiled against `crates/mrpc-script/cpp/mrpc_script.hpp`
   (`MRPC_REGISTER_VEHICLE_SCRIPT`) and drives the same lifecycle through a
   stable C snapshot struct.

## Format reference (encoded in `mrpc-core`)

- `vehicles[]`: id, name, color, transportMode, length, width,
  bogie1Position/bogie2Position, couplingPadding1/2, description,
  wikipediaArticle, tags, models[], bogie1Models[], bogie2Models[],
  hasGangway1/2, hasBarrier1/2, legacyRiderOffset, bveSoundBaseResource,
  legacy speed/door sound fields, **scriptId**, **hideDisplayParts**.
- `models[]`: modelResource, textureResource, modelPropertiesResource,
  positionDefinitionsResource, flipTextureV.
- model properties JSON: parts[] (names[], positionDefinitions[], condition,
  renderStage, type, display fields, doorXMultiplier, doorZMultiplier,
  doorAnimationType, renderFrom/UntilOpening/ClosingDoorTime, flashOn/OffTime,
  displayOptions[]), modelYOffset, gangway*/barrier* fields.
- position definitions JSON: positionDefinitions[] { name, positions[],
  positionsFlipped[] } (model units = 1/16 block, car axis = Z).
- `vehicleScripts[]` (JCM Yanyang): id, prependExpressions[], scriptLocations[]
  (`.js` = JavaScript, anything else = native module; per-platform maps
  allowed), input, dataFetchMode (SKIP|MANDATORY|ALL), forceLoad.
- BVE sound set: `sounds/<ns path>/sound.cfg` + CSVs (`type 5`) or
  `train.dat` (`type 4`).
- Door timing (TSC): `DOOR_MOVE_TIME = 3200 ms`, `DOOR_DELAY = 1000 ms`;
  doorCloseTime = max(dwell/2, dwell − move − delay).

## Importers (v0.2, in `mrpc-core`)

- `import.rs`: shared `ImportedModel` (ObjModel + MTL + PNG textures) and
  the OBJ/MTL serializers importers produce.
- `mqo.rs`: Metasequoia `.mqo`/`.mqoz` (gzip) — Material `col`/`tex`,
  `vertex`/`BVertex` (UVs), `face` `V`/`M`/`UV`; left-handed to right-handed
  (negated z + reversed winding); UV origin stays top-left.
- `gltf.rs`: glTF 2.0 `.gltf`/`.glb` — buffers (data URI / external / GLB
  BIN), accessors (with byteStride), node TRS/matrix baking, materials
  `baseColorFactor` to Kd (linear to sRGB) and `baseColorTexture` to PNG.
- `bve_playback.rs`: real-sound-source playback — RIFF WAV reader (PCM
  8/16/24/32-bit, IEEE float 32/64, WAVE_FORMAT_EXTENSIBLE, mono mixdown)
  and a curve-driven loop resampling engine (per-channel rate/gain,
  one-pole smoothing, `clone_shared` for the audio thread).

## Testing

54 unit tests cover the JSON round-trips (including JCM script entries and
unknown-key preservation), OBJ/MTL/BBModel parsing, the twelve door animation
curves and the TSC cycle, BVE5/BVE4 motor data (plus SoundList), pack zip
round-trip + generated `pack.png`, validation rules, the seven locales, the
MQO/MQOZ and glTF/GLB importers, the WAV reader and curve-driven BVE playback
engine, the C++ VVVF engine (fundamental presence/frequency tracking/
silence), and the QuickJS sandbox (lifecycle, SCRIPT_INPUT, include, Timing,
vehicle wrapper, error reporting). The demo pack generator is itself tested
to produce a zero-error pack that parses back cleanly.
