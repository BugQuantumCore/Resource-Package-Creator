// mrpc-vvvf — VVVF inverter / traction motor sound synthesiser (C++ DSP).
//
// Renders the audible result of a two-level VVVF inverter driving an RL motor
// load, with either natural-sampled SPWM or SHE (selected-harmonic-elimination)
// modulation, a synchronous-pulses-per-period band at low frequency, speed
// dependent rolling noise and a first-order DC block. Exposed through a pure
// C ABI so the Rust side can link it without any C++ runtime requirement.
//
// This is an original implementation; it mirrors the *behaviour* of real
// GTO/IGBT traction drives (and of BVE-style motor sound simulation) but is
// not derived from any third-party source code.

#pragma once

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct MrpcVvvfParams {
    int32_t scheme;         // 0 = SPWM (natural sampled), 1 = SHE
    double sample_rate;     // Hz, e.g. 48000
    double carrier_freq;    // asynchronous carrier frequency, Hz (e.g. 1000)
    double pole_pairs;      // informational (fundamental is passed directly)
    double base_freq_hz;    // V/f base frequency (field weakening above)
    double max_mod_depth;   // peak modulation index at base frequency (<= 1)
    double dc_bus;          // DC link voltage (V)
    int32_t sync_min_ratio;  // minimum pulses per fundamental period (sync band)
    int32_t sync_max_ratio;  // maximum pulses per fundamental period (sync band)
    double load_r;          // phase resistance (ohm)
    double load_l;          // phase inductance (henry)
    double noise_gain;      // rolling/fan noise mix 0..1
    double speed_fraction;  // 0..1 speed fraction for noise scaling
} MrpcVvvfParams;

typedef struct MrpcVvvfState {
    double shaft_angle;   // electrical angle [rad]
    double carrier_phase; // carrier phase [0,1)
    double current;       // load current state (A)
    double hp_state;      // DC blocker state
    uint64_t sample_index;
} MrpcVvvfState;

// Reset a state to silence.
void mrpc_vvvf_reset(MrpcVvvfState* state);

// Render `n` mono f32 samples at the commanded electrical fundamental
// `fundamental_hz` (>= 0) with traction effort `power` in [-1, 1]
// (positive accelerating, negative braking, 0 coasting).
void mrpc_vvvf_render(const MrpcVvvfParams* params,
                      MrpcVvvfState* state,
                      double fundamental_hz,
                      double power,
                      float* out,
                      int32_t n);

// ABI version of the DSP engine (incremented on any breaking change).
int32_t mrpc_vvvf_abi_version(void);

#ifdef __cplusplus
}
#endif
