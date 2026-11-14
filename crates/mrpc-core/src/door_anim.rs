//! Door animation math — a faithful Rust port of
//! `org.mtr.mod.resource.DoorAnimationType` (MTR 4.0.x), plus the TSC door
//! cycle timing (`DOOR_MOVE_TIME = 3200 ms`, `DOOR_DELAY = 1000 ms` from
//! `org.mtr.core.data.Vehicle`).

use serde::{Deserialize, Serialize};

/// Door animation curves; names serialise exactly like the MTR enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum DoorAnimationType {
    #[default]
    #[serde(rename = "STANDARD")]
    Standard,
    #[serde(rename = "STANDARD_SLOW")]
    StandardSlow,
    #[serde(rename = "CONSTANT")]
    Constant,
    #[serde(rename = "PLUG_FAST")]
    PlugFast,
    #[serde(rename = "PLUG_SLOW")]
    PlugSlow,
    #[serde(rename = "PLUG_SLOW_2")]
    PlugSlow2,
    #[serde(rename = "PLUG_SLOW_3")]
    PlugSlow3,
    #[serde(rename = "BOUNCY_1")]
    Bouncy1,
    #[serde(rename = "BOUNCY_2")]
    Bouncy2,
    #[serde(rename = "MLR")]
    Mlr,
    #[serde(rename = "R179")]
    R179,
    #[serde(rename = "R211")]
    R211,
    /// Tolerant fallback; behaves like STANDARD.
    #[serde(other, rename = "UNKNOWN")]
    Unknown,
}

impl DoorAnimationType {
    pub const ALL: [DoorAnimationType; 12] = [
        DoorAnimationType::Standard,
        DoorAnimationType::StandardSlow,
        DoorAnimationType::Constant,
        DoorAnimationType::PlugFast,
        DoorAnimationType::PlugSlow,
        DoorAnimationType::PlugSlow2,
        DoorAnimationType::PlugSlow3,
        DoorAnimationType::Bouncy1,
        DoorAnimationType::Bouncy2,
        DoorAnimationType::Mlr,
        DoorAnimationType::R179,
        DoorAnimationType::R211,
    ];

    /// Cosine-eased ramp used throughout the MTR implementation.
    fn smooth_ends(start_value: f64, end_value: f64, start_time: f64, end_time: f64, time: f64) -> f64 {
        if time < start_time {
            return start_value;
        }
        if time > end_time {
            return end_value;
        }
        let time_change = end_time - start_time;
        let value_change = end_value - start_value;
        value_change * (1.0 - (std::f64::consts::PI * (time - start_time) / time_change).cos()) / 2.0
            + start_value
    }

    /// X displacement of the door (across the car / outward for plug doors).
    pub fn door_animation_x(&self, multiplier: f64, flipped: bool, time: f64) -> f64 {
        if multiplier == 0.0 {
            return 0.0;
        }
        let abs = self.door_animation_x_absolute(multiplier.abs(), time);
        let signed = (if flipped { -1.0 } else { 1.0 }) * multiplier;
        if signed == 0.0 {
            0.0
        } else {
            abs * signed.signum()
        }
    }

    /// Z displacement of the door (along the car axis, the sliding direction).
    pub fn door_animation_z(&self, multiplier: f64, flipped: bool, time: f64, opening: bool) -> f64 {
        if multiplier == 0.0 {
            return 0.0;
        }
        let abs = self.door_animation_z_absolute(multiplier.abs(), time, opening);
        let signed = (if flipped { -1.0 } else { 1.0 }) * multiplier;
        if signed == 0.0 {
            0.0
        } else {
            abs * signed.signum()
        }
    }

    fn door_animation_x_absolute(&self, multiplier: f64, time: f64) -> f64 {
        match self {
            DoorAnimationType::PlugFast | DoorAnimationType::PlugSlow2 | DoorAnimationType::PlugSlow3 => {
                if time < 0.05 {
                    time * 20.0 * multiplier + 0.05
                } else {
                    multiplier + 0.05
                }
            }
            DoorAnimationType::PlugSlow => Self::smooth_ends(0.05, multiplier + 0.05, 0.0, 0.1, time),
            _ => 0.0,
        }
    }

    fn door_animation_z_absolute(&self, door_max: f64, time: f64, opening: bool) -> f64 {
        match self {
            DoorAnimationType::Constant => door_max * clamp(time / 0.5, 0.0, 1.0),
            DoorAnimationType::PlugFast => Self::smooth_ends(-door_max, door_max, -0.5, 0.5, time),
            DoorAnimationType::PlugSlow => {
                if opening {
                    Self::smooth_ends(0.0, door_max, 0.05, 0.5, time)
                } else if time > 0.5 {
                    Self::smooth_ends(2.0, door_max, 0.5, 1.0, time)
                } else if time < 0.3 {
                    Self::smooth_ends(0.0, 2.0, 0.05, 0.3, time)
                } else {
                    2.0
                }
            }
            DoorAnimationType::PlugSlow2 => Self::smooth_ends(-door_max, door_max, -0.75, 0.75, time),
            DoorAnimationType::PlugSlow3 => Self::smooth_ends(-door_max, door_max, -1.0, 1.0, time),
            DoorAnimationType::Bouncy1 => {
                if opening {
                    if time > 0.4 {
                        Self::smooth_ends(door_max - 1.0, door_max - 0.5, 0.4, 0.5, time)
                    } else {
                        Self::smooth_ends(-door_max + 1.0, door_max - 1.0, -0.4, 0.4, time)
                    }
                } else if time > 0.2 {
                    Self::smooth_ends(1.0, door_max - 0.5, 0.2, 0.5, time)
                } else if time > 0.1 {
                    Self::smooth_ends(1.5, 1.0, 0.1, 0.2, time)
                } else {
                    Self::smooth_ends(-1.5, 1.5, -0.1, 0.1, time)
                }
            }
            DoorAnimationType::Bouncy2 => {
                if opening {
                    if time > 0.4 {
                        Self::smooth_ends(door_max - 0.5, door_max, 0.4, 0.5, time)
                    } else {
                        Self::smooth_ends(-door_max + 0.5, door_max - 0.5, -0.4, 0.4, time)
                    }
                } else if time > 0.3 {
                    Self::smooth_ends(1.0, door_max, 0.3, 0.5, time)
                } else if time > 0.1 {
                    Self::smooth_ends(3.0, 1.0, 0.1, 0.3, time)
                } else {
                    Self::smooth_ends(-3.0, 3.0, -0.1, 0.1, time)
                }
            }
            DoorAnimationType::Mlr => {
                if opening {
                    if time < 0.2 {
                        0.0
                    } else if time > 0.7 {
                        door_max
                    } else {
                        (time - 0.2) * 2.0 * door_max
                    }
                } else {
                    let stopping_point = 1.5;
                    if time > 0.25 {
                        ((time - 0.25) * 2.0 * door_max + stopping_point + 1.0).min(door_max)
                    } else if time > 0.2 {
                        Self::smooth_ends(stopping_point, stopping_point + 2.0, 0.2, 0.3, time)
                    } else if time < 0.1 {
                        time / 0.1 * stopping_point
                    } else {
                        stopping_point
                    }
                }
            }
            DoorAnimationType::R179 => {
                if opening {
                    if time > 0.4 {
                        Self::smooth_ends(door_max - 1.0, door_max - 0.5, 0.4, 0.6, time)
                    } else {
                        Self::smooth_ends(-door_max + 1.0, door_max - 1.0, -0.4, 0.4, time)
                    }
                } else if time > 0.2 {
                    Self::smooth_ends(1.0, door_max - 0.5, 0.2, 0.6, time)
                } else {
                    Self::smooth_ends(-1.5, 1.5, -0.4, 0.4, time)
                }
            }
            DoorAnimationType::R211 => {
                if time < 0.2 {
                    0.0
                } else if time > 0.7 {
                    door_max
                } else {
                    (time - 0.2) * 2.0 * door_max
                }
            }
            DoorAnimationType::StandardSlow => {
                if !opening {
                    if time > 0.2 {
                        Self::smooth_ends(1.0, door_max, 0.2, 0.5, time)
                    } else if time > 0.1 {
                        1.0
                    } else {
                        Self::smooth_ends(0.0, 1.0, 0.0, 0.1, time)
                    }
                } else {
                    Self::smooth_ends(0.0, door_max, 0.0, 0.5, time)
                }
            }
            _ => Self::smooth_ends(0.0, door_max, 0.0, 0.5, time),
        }
    }
}

fn clamp(v: f64, lo: f64, hi: f64) -> f64 {
    v.max(lo).min(hi)
}

/// TSC door-cycle constants (`org.mtr.core.data.Vehicle`).
pub const DOOR_MOVE_TIME: i64 = 3200;
pub const DOOR_DELAY: i64 = 1000;

/// A full stop-to-stop door cycle for previewing open/close animations.
///
/// Timeline (milliseconds):
/// ```text
/// 0 .. delay            closed (value 0)
/// delay .. delay+move   opening (value 0 → 1)
/// .. +dwell             fully open (value 1)
/// .. +move              closing (value 1 → 0)
/// .. +close_hold        closed again
/// ```
#[derive(Debug, Clone, Copy)]
pub struct DoorCycle {
    pub delay_ms: i64,
    pub move_ms: i64,
    pub dwell_ms: i64,
    pub close_hold_ms: i64,
}

impl Default for DoorCycle {
    fn default() -> Self {
        Self { delay_ms: DOOR_DELAY, move_ms: DOOR_MOVE_TIME, dwell_ms: 4000, close_hold_ms: 1000 }
    }
}

/// The phase of the door cycle at a given time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DoorPhase {
    ClosedBeforeOpen,
    Opening,
    Open,
    Closing,
    ClosedAfterClose,
}

impl DoorCycle {
    pub fn total_ms(&self) -> i64 {
        self.delay_ms + self.move_ms + self.dwell_ms + self.move_ms + self.close_hold_ms
    }

    /// Door value (0 = closed, 1 = fully open) and phase at time `t_ms`.
    pub fn sample(&self, t_ms: i64) -> (f64, DoorPhase) {
        let t = t_ms.max(0);
        if t < self.delay_ms {
            return (0.0, DoorPhase::ClosedBeforeOpen);
        }
        let t = t - self.delay_ms;
        if t < self.move_ms {
            return (t as f64 / self.move_ms as f64, DoorPhase::Opening);
        }
        let t = t - self.move_ms;
        if t < self.dwell_ms {
            return (1.0, DoorPhase::Open);
        }
        let t = t - self.dwell_ms;
        if t < self.move_ms {
            return (1.0 - t as f64 / self.move_ms as f64, DoorPhase::Closing);
        }
        (0.0, DoorPhase::ClosedAfterClose)
    }

    /// Milliseconds elapsed since the doors *started* opening/closing — used by
    /// the `renderFrom/UntilOpeningDoorTime` part visibility windows.
    pub fn since_door_start(&self, t_ms: i64, opening: bool) -> i64 {
        if opening {
            (t_ms - self.delay_ms).max(0)
        } else {
            (t_ms - self.delay_ms - self.move_ms - self.dwell_ms).max(0)
        }
    }

    /// Whether a part with the given visibility window renders at `t_ms`.
    pub fn part_visible(
        &self,
        t_ms: i64,
        from_open: i64,
        until_open: i64,
        from_close: i64,
        until_close: i64,
    ) -> bool {
        let (_, phase) = self.sample(t_ms);
        let opening = matches!(phase, DoorPhase::Opening | DoorPhase::Open | DoorPhase::ClosedBeforeOpen);
        let elapsed = self.since_door_start(t_ms, opening);
        let (from, until) = if opening { (from_open, until_open) } else { (from_close, until_close) };
        if from == 0 && until == 0 {
            return true; // no window configured
        }
        elapsed >= from && (until == 0 || elapsed <= until)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn smooth_ends_endspoints() {
        assert!(close(DoorAnimationType::smooth_ends(0.0, 10.0, 0.0, 0.5, -1.0), 0.0));
        assert!(close(DoorAnimationType::smooth_ends(0.0, 10.0, 0.0, 0.5, 0.25), 5.0));
        assert!(close(DoorAnimationType::smooth_ends(0.0, 10.0, 0.0, 0.5, 0.5), 10.0));
    }

    #[test]
    fn standard_matches_mtr() {
        // STANDARD: smoothEnds(0, doorMax, 0, 0.5, time)
        let t = DoorAnimationType::Standard;
        assert!(close(t.door_animation_z(14.0, false, 0.0, true), 0.0));
        assert!(close(t.door_animation_z(14.0, false, 0.25, true), 7.0));
        assert!(close(t.door_animation_z(14.0, false, 0.5, true), 14.0));
        assert!(close(t.door_animation_z(14.0, false, 1.0, true), 14.0));
        // Flipped mirrors the sign.
        assert!(close(t.door_animation_z(-14.0, false, 0.5, true), -14.0));
        assert!(close(t.door_animation_z(14.0, true, 0.5, true), -14.0));
    }

    #[test]
    fn r211_step_shape() {
        let t = DoorAnimationType::R211;
        assert!(close(t.door_animation_z(10.0, false, 0.1, true), 0.0));
        assert!(close(t.door_animation_z(10.0, false, 0.45, true), (0.45 - 0.2) * 2.0 * 10.0));
        assert!(close(t.door_animation_z(10.0, false, 0.8, true), 10.0));
    }

    #[test]
    fn door_cycle_phases() {
        let c = DoorCycle::default();
        assert_eq!(c.sample(0).1, DoorPhase::ClosedBeforeOpen);
        assert_eq!(c.sample(999).1, DoorPhase::ClosedBeforeOpen);
        assert_eq!(c.sample(1000).1, DoorPhase::Opening);
        assert!(close(c.sample(1000 + 1600).0, 0.5));
        assert_eq!(c.sample(4200).1, DoorPhase::Open);
        assert!(close(c.sample(4200).0, 1.0));
        assert_eq!(c.sample(8200).1, DoorPhase::Closing);
        assert!(close(c.sample(8200 + 1600).0, 0.5));
        assert_eq!(c.sample(11400).1, DoorPhase::ClosedAfterClose);
    }
}
