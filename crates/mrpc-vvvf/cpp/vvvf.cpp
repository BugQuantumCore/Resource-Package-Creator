// vvvf.cpp — implementation of the mrpc-vvvf synthesiser. See vvvf.h.

#include "vvvf.h"

#include <cmath>
#include <cstdlib>

namespace {

constexpr double kPi = 3.14159265358979323846;

double clampd(double v, double lo, double hi) { return v < lo ? lo : (v > hi ? hi : v); }

// Deterministic xorshift64* PRNG so renders are reproducible.
struct Rng {
    uint64_t s;
    explicit Rng(uint64_t seed) : s(seed ? seed : 0x9E3779B97F4A7C15ull) {}
    double next_pm1() {
        s ^= s >> 12; s ^= s << 25; s ^= s >> 27;
        uint64_t x = s * 0x2545F4914F6CDD1Dull;
        // Map to [-1, 1) with decent uniformity.
        return ((int64_t)(x >> 11)) * (1.0 / 9007199254740992.0);
    }
};

// Natural-sampled triangular carrier in [0, 1]; `phase` in [0, 1).
inline double triangle(double phase) {
    double p = phase - std::floor(phase);
    return p < 0.5 ? (4.0 * p) : (4.0 * (1.0 - p));
}

// ---- SHE (selected harmonic elimination) angle tables ----
// Quarter-wave symmetric bipolar waveform with k switchings per quarter at
// angles a[0..k-1]. Fundamental b1 and eliminated harmonics h are controlled by
//   b_n = (4 / (n*pi)) * (1 + 2 * sum_i (-1)^i * cos(n * a_i))
// We solve for b1 = target with harmonics 5, 7, 11, ... (k-1 of them) set to
// zero via damped Newton iteration. Cached per pulse count.
struct SheAngles {
    bool valid = false;
    double target_b1 = 0.0;
    double angles[14] = {0}; // supports up to k = 14 (sync_max_ratio <= 15)
};

SheAngles g_she_cache[16];

void solve_she(int k, double target_b1, SheAngles& out) {
    if (k < 2 || k > 14) return;
    out.target_b1 = target_b1;
    // Initial guess: angles evenly spread across the quarter.
    double a[14];
    for (int i = 0; i < k; i++) a[i] = (i + 1) * (kPi / 2.0) / (k + 1);

    auto b_n = [&](int n, const double* ang) {
        double s = 1.0;
        for (int i = 0; i < k; i++) s += 2.0 * ((i % 2 == 0) ? -1.0 : 1.0) * std::cos(n * ang[i]);
        return (4.0 / (n * kPi)) * s;
    };

    for (int iter = 0; iter < 64; iter++) {
        // Residuals: fundamental at target, harmonics 5,7,11,... at 0 (k-1 of them).
        double r[14];
        r[0] = b_n(1, a) - target_b1;
        int m = 1;
        for (int h = 5; m < k && h <= 61; h += (h == 5 ? 2 : (h == 7 ? 4 : (h == 11 ? 6 : 10)))) {
            r[m++] = b_n(h, a);
        }
        double norm = 0.0;
        for (int i = 0; i < k; i++) norm += r[i] * r[i];
        if (norm < 1e-18) break;

        // Numeric Jacobian.
        const double eps = 1e-6;
        double J[14][14];
        for (int c = 0; c < k; c++) {
            double ap[14];
            for (int i = 0; i < k; i++) ap[i] = a[i];
            ap[c] += eps;
            double rp[14];
            rp[0] = b_n(1, ap) - target_b1;
            m = 1;
            for (int h = 5; m < k && h <= 61; h += (h == 5 ? 2 : (h == 7 ? 4 : (h == 11 ? 6 : 10)))) {
                rp[m++] = b_n(h, ap);
            }
            for (int rw = 0; rw < k; rw++) J[rw][c] = (rp[rw] - r[rw]) / eps;
        }
        // Gauss elimination with partial pivoting for J dx = -r.
        double A[14][15];
        for (int rw = 0; rw < k; rw++) {
            for (int c = 0; c < k; c++) A[rw][c] = J[rw][c];
            A[rw][k] = -r[rw];
        }
        for (int col = 0; col < k; col++) {
            int piv = col;
            for (int rw = col + 1; rw < k; rw++) {
                if (std::fabs(A[rw][col]) > std::fabs(A[piv][col])) piv = rw;
            }
            if (std::fabs(A[piv][col]) < 1e-14) return; // singular — mark invalid
            if (piv != col) for (int c2 = col; c2 <= k; c2++) { double t = A[piv][c2]; A[piv][c2] = A[col][c2]; A[col][c2] = t; }
            for (int rw = col + 1; rw < k; rw++) {
                double f = A[rw][col] / A[col][col];
                if (f == 0.0) continue;
                for (int c2 = col; c2 <= k; c2++) A[rw][c2] -= f * A[col][c2];
            }
        }
        double dx[14];
        for (int i = k - 1; i >= 0; i--) {
            double s = A[i][k];
            for (int c = i + 1; c < k; c++) s -= A[i][c] * dx[c];
            dx[i] = s / A[i][i];
        }
        // Damped update; keep angles ordered in (0, pi/2).
        double lambda = 1.0;
        for (int sub = 0; sub < 6; sub++) {
            double an[14];
            for (int i = 0; i < k; i++) an[i] = a[i] + lambda * dx[i];
            // Sort and clamp.
            for (int i = 1; i < k; i++) { double t = an[i]; int j = i - 1; while (j >= 0 && an[j] > t) { an[j + 1] = an[j]; j--; } an[j + 1] = t; }
            for (int i = 0; i < k; i++) an[i] = clampd(an[i], 1e-3, kPi / 2 - 1e-3);
            double rn = 0.0;
            rn += (b_n(1, an) - target_b1) * (b_n(1, an) - target_b1);
            int mm = 1;
            for (int h = 5; mm < k && h <= 61; h += (h == 5 ? 2 : (h == 7 ? 4 : (h == 11 ? 6 : 10)))) {
                double v = b_n(h, an); rn += v * v; mm++;
            }
            if (rn < norm) {
                for (int i = 0; i < k; i++) a[i] = an[i];
                break;
            }
            lambda *= 0.5;
        }
    }

    // Accept if the residual is small enough.
    double r0 = b_n(1, a) - target_b1;
    if (std::fabs(r0) < 1e-4) {
        for (int i = 0; i < k; i++) out.angles[i] = a[i];
        out.valid = true;
    }
}

const SheAngles& she_for(int k) {
    static bool init = false;
    if (!init) {
        for (int kk = 2; kk <= 14; kk++) {
            solve_she(kk, 0.8, g_she_cache[kk]);
        }
        init = true;
    }
    if (k < 0) k = 0;
    if (k > 15) k = 15;
    return g_she_cache[k];
}

// SHE bipolar output for electrical angle `theta` (0..2pi) with `k` quarter
// switchings. Starts positive; toggles at each angle in each quarter with the
// correct parity.
double she_voltage(double theta, const SheAngles& she, int k) {
    if (!she.valid) return 0.0;
    double h = std::fmod(theta, kPi); // half wave
    if (h < 0) h += kPi;
    double q = std::fmod(h, kPi / 2.0); // quarter wave
    int sign_half = (h < kPi / 2.0) ? 1 : -1;
    // Count switchings below q.
    int toggles = 0;
    for (int i = 0; i < k; i++) {
        if (q >= she.angles[i]) toggles++;
    }
    int level = (toggles % 2 == 0) ? 1 : -1; // starts at +1
    return sign_half * level;
}

} // namespace

extern "C" {

void mrpc_vvvf_reset(MrpcVvvfState* state) {
    if (!state) return;
    state->shaft_angle = 0.0;
    state->carrier_phase = 0.0;
    state->current = 0.0;
    state->hp_state = 0.0;
    state->sample_index = 0;
}

int32_t mrpc_vvvf_abi_version(void) { return 1; }

void mrpc_vvvf_render(const MrpcVvvfParams* p, MrpcVvvfState* st,
                      double fundamental_hz, double power,
                      float* out, int32_t n) {
    if (!p || !st || !out || n <= 0) return;

    const double sr = p->sample_rate > 0.0 ? p->sample_rate : 48000.0;
    const double dt = 1.0 / sr;
    const double f = fundamental_hz > 0.0 ? fundamental_hz : 0.0;
    const double pw = clampd(power, -1.0, 1.0);

    // Modulation index: constant flux below base frequency, constant voltage
    // (field weakening) above; effort scales the amplitude.
    double vf = p->base_freq_hz > 0.0 ? clampd(f / p->base_freq_hz, 0.0, 1.0) : 1.0;
    double m = p->max_mod_depth * vf * std::fabs(pw);
    m = clampd(m, 0.0, 1.15); // slight overmodulation allowed

    // Synchronous band: integer pulses per period between min and max ratio.
    int sync_pulses = 0;
    if (f > 0.0) {
        double ratio = p->carrier_freq / f;
        if (ratio <= p->sync_max_ratio && ratio >= p->sync_min_ratio - 0.5) {
            sync_pulses = (int)clampd(std::lround(ratio), p->sync_min_ratio, p->sync_max_ratio);
        }
    }

    Rng rng(st->sample_index * 2654435761ull + 12345);

    const double vdc = p->dc_bus > 0.0 ? p->dc_bus : 1500.0;
    const double r = p->load_r > 0.0 ? p->load_r : 0.8;
    const double l = p->load_l > 0.0 ? p->load_l : 0.004;
    // Reference impedance at the base frequency for normalisation.
    const double w_base = 2.0 * kPi * (p->base_freq_hz > 0.0 ? p->base_freq_hz : 60.0);
    const double z_ref = std::sqrt(r * r + (w_base * l) * (w_base * l));
    const double i_ref = vdc / (z_ref > 1e-9 ? z_ref : 1.0);

    const double noise = clampd(p->noise_gain, 0.0, 1.0) * clampd(p->speed_fraction, 0.0, 1.0);

    for (int32_t i = 0; i < n; i++) {
        // Advance phases.
        st->shaft_angle += 2.0 * kPi * f * dt;
        if (st->shaft_angle >= 2.0 * kPi * 1e6) st->shaft_angle = std::fmod(st->shaft_angle, 2.0 * kPi);
        st->carrier_phase += (sync_pulses > 0 ? (f * sync_pulses) : p->carrier_freq) * dt;
        st->carrier_phase -= std::floor(st->carrier_phase);

        // Inverter pole voltage (bipolar two-level), volts.
        double v;
        if (m <= 1e-6) {
            v = 0.0;
        } else if (p->scheme == 1 && sync_pulses > 0) {
            const SheAngles& she = she_for(sync_pulses);
            v = she_voltage(st->shaft_angle, she, sync_pulses) * vdc * 0.5 * clampd(m / 0.8, 0.0, 1.2);
        } else {
            // SPWM: compare duty (0.5 + 0.5*m*sin) with the triangle carrier.
            double duty = 0.5 + 0.5 * m * std::sin(st->shaft_angle);
            double carrier = triangle(st->carrier_phase);
            v = (carrier < duty ? 0.5 : -0.5) * vdc;
        }

        // Motor phase current through the RL load (explicit Euler; tau = L/R
        // is orders of magnitude above one sample).
        st->current += (v - r * st->current) / l * dt;
        if (st->current > 1e4) st->current = 1e4;
        if (st->current < -1e4) st->current = -1e4;

        // Audible mix: current (dominant magnetic noise) + voltage edges +
        // speed-scaled broadband rolling/fan noise. DC-blocked and soft-clipped.
        double audible = 0.68 * (st->current / (i_ref > 1e-9 ? i_ref : 1.0))
                       + 0.32 * (v / vdc)
                       + noise * 0.12 * rng.next_pm1();
        // One-pole DC blocker (high-pass): slow-moving average removed.
        st->hp_state += 0.0005 * (audible - st->hp_state);
        double sample = audible - st->hp_state;
        // Soft clip.
        double a = std::fabs(sample);
        if (a > 1.0) sample = (sample / a) * (1.0 - std::exp(-(a - 1.0)));
        out[i] = (float)clampd(sample * 1.9, -1.0, 1.0);
        st->sample_index++;
    }
}

} // namespace
