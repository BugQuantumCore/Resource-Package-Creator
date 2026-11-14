//! BVE vehicle sound configuration parsing (motor noise data types 4 and 5),
//! following `org.mtr.mod.sound.BveVehicleSoundConfig` / `BveMotorData4` /
//! `BveMotorData5` from the MTR 4.0.x sources.
//!
//! Layout inside a sound set (`bveSoundBaseResource`, e.g.
//! `mtr:custom/my_train/sound`):
//! ```text
//! sounds/<namespace path>/sound.cfg
//! sounds/<namespace path>/train.dat            (type 4)
//! sounds/<namespace path>/powervol.csv         (type 5)
//! sounds/<namespace path>/powerfreq.csv        (type 5)
//! sounds/<namespace path>/brakevol.csv         (type 5)
//! sounds/<namespace path>/brakefreq.csv        (type 5)
//! ```
//! Motor sounds themselves are `<base>_<i>` sound events; this module only
//! models pitch/volume so the audition module can drive any audio backend.

/// Parsed `sound.cfg` (only the keys MTR consumes; the rest is ignored).
#[derive(Debug, Clone, Default)]
pub struct BveConfig {
    /// `MotorNoiseDataType` — 4 (BVE4/OpenBVE `train.dat`) or 5 (BVE5/6 CSVs).
    pub motor_noise_data_type: i32,
}

impl BveConfig {
    pub fn parse(text: &str) -> BveConfig {
        let mut cfg = BveConfig { motor_noise_data_type: 5 };
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
                continue;
            }
            if let Some((key, value)) = line.split_once('=') {
                let key = key.trim().to_ascii_lowercase();
                let value = value.trim();
                if key == "motornoisedatatype" {
                    cfg.motor_noise_data_type = value.parse().unwrap_or(5);
                }
            }
        }
        cfg
    }
}

/// Speed → value splines for one motor channel set (BVE 5).
#[derive(Debug, Clone, Default)]
pub struct FloatSplines {
    /// channel index → sorted `(speed key, value)` pairs.
    pub data: Vec<Vec<(f32, f32)>>,
}

impl FloatSplines {
    /// Parse a CSV where each line is `speed,v1,v2,…,vN` (values per channel).
    pub fn parse(text: &str) -> FloatSplines {
        let mut out = FloatSplines::default();
        for line in text.lines() {
            let line = line.trim().to_ascii_lowercase();
            if line.is_empty()
                || line.starts_with('#')
                || line.starts_with("//")
                || line.starts_with("bvets")
            {
                continue;
            }
            let tokens: Vec<&str> = line.split(',').collect();
            if tokens.is_empty() {
                continue;
            }
            let Ok(key) = tokens[0].trim().parse::<f32>() else { continue };
            for (i, tok) in tokens.iter().enumerate().skip(1) {
                let tok = tok.trim();
                if tok.is_empty() {
                    continue;
                }
                while out.data.len() < i {
                    out.data.push(Vec::new());
                }
                if let Ok(v) = tok.parse::<f32>() {
                    out.data[i - 1].push((key, v));
                }
            }
        }
        for channel in &mut out.data {
            channel.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        }
        out
    }

    /// Linear interpolation like `FloatSplines.getValue` in MTR.
    pub fn get_value(&self, index: usize, key: f32) -> f32 {
        let Some(spline) = self.data.get(index) else { return 0.0 };
        if spline.is_empty() {
            return 0.0;
        }
        // floor: last entry with key' <= key; ceil: first entry with key' >= key.
        let floor_pos = spline.partition_point(|(k, _)| *k <= key);
        let floor = if floor_pos > 0 { Some(spline[floor_pos - 1]) } else { None };
        let ceil = spline.get(floor_pos).copied();
        match (floor, ceil) {
            (None, Some((_, c))) => c,
            (Some((_, f)), None) => f,
            (Some((kf, f)), Some((kc, c))) => {
                if kf == kc {
                    f
                } else {
                    f + (c - f) * ((key - kf) / (kc - kf))
                }
            }
            (None, None) => 0.0,
        }
    }
}

/// BVE 5/6 motor data (`powervol/powerfreq/brakevol/brakefreq.csv`).
#[derive(Debug, Clone, Default)]
pub struct BveMotorData5 {
    pub power_volume: FloatSplines,
    pub power_frequency: FloatSplines,
    pub brake_volume: FloatSplines,
    pub brake_frequency: FloatSplines,
}

impl BveMotorData5 {
    pub fn sound_count(&self) -> usize {
        [
            self.power_volume.data.len(),
            self.power_frequency.data.len(),
            self.brake_volume.data.len(),
            self.brake_frequency.data.len(),
        ]
        .into_iter()
        .max()
        .unwrap_or(0)
    }

    /// Pitch multiplier for motor sound `index` at `speed` (m/s) with traction
    /// `power` (positive = accelerating, negative = braking, 0 = coasting).
    pub fn get_pitch(&self, index: usize, speed: f32, power: f32) -> f32 {
        if power == 0.0 {
            return 0.0;
        }
        if power > 0.0 {
            self.power_frequency.get_value(index, speed)
        } else {
            self.brake_frequency.get_value(index, speed)
        }
    }

    /// Volume (0..~1+) for motor sound `index`.
    pub fn get_volume(&self, index: usize, speed: f32, power: f32) -> f32 {
        if power == 0.0 {
            return 0.0;
        }
        if power > 0.0 {
            self.power_volume.get_value(index, speed) * power.abs()
        } else {
            self.brake_volume.get_value(index, speed) * power.abs()
        }
    }
}

/// BVE 4 / OpenBVE motor data (`train.dat`, sections `#Motor_P1/P2/B1/B2`).
#[derive(Debug, Clone, Default)]
pub struct BveMotorData4 {
    /// [P1, P2, B1, B2]; each entry: (soundId, pitch, volume) rows.
    pub channels: [Vec<(i32, f32, f32)>; 4],
}

impl BveMotorData4 {
    pub fn parse(text: &str) -> BveMotorData4 {
        let mut data = BveMotorData4::default();
        let mut section = String::new();
        for line in text.lines() {
            let line = line.trim().to_ascii_lowercase();
            if line.is_empty() {
                continue;
            }
            if let Some(rest) = line.strip_prefix('#') {
                section = rest.trim().to_string();
                continue;
            }
            let list_index = match section.as_str() {
                "motor_p1" => Some(0),
                "motor_p2" => Some(1),
                "motor_b1" => Some(2),
                "motor_b2" => Some(3),
                _ => None,
            };
            if let Some(idx) = list_index {
                let tokens: Vec<&str> = line.split(',').collect();
                if tokens.len() < 3 {
                    continue;
                }
                let (Ok(id), Ok(pitch), Ok(volume)) = (
                    tokens[0].trim().parse::<i32>(),
                    tokens[1].trim().parse::<f32>(),
                    tokens[2].trim().parse::<f32>(),
                ) else {
                    continue;
                };
                data.channels[idx].push((id, pitch / 100.0, volume / 128.0));
            }
        }
        data
    }

    pub fn sound_count(&self) -> usize {
        self.channels
            .iter()
            .flat_map(|c| c.iter().map(|(id, _, _)| *id))
            .max()
            .map(|m| (m + 1) as usize)
            .unwrap_or(0)
    }

    fn get_safe(list: &[(i32, f32, f32)], index: usize) -> (i32, f32, f32) {
        let i = index.min(list.len().saturating_sub(1));
        list.get(i).copied().unwrap_or((-1, 0.0, 0.0))
    }

    pub fn get_pitch(&self, index: usize, speed: f32, power: f32) -> f32 {
        if power == 0.0 {
            return 0.0;
        }
        let offset = if power > 0.0 { 0 } else { 2 };
        let entry_index = (speed / 0.2) as usize;
        let e0 = Self::get_safe(&self.channels[offset], entry_index);
        let e1 = Self::get_safe(&self.channels[offset + 1], entry_index);
        if index as i32 == e0.0 {
            e0.1
        } else if index as i32 == e1.0 {
            e1.1
        } else {
            0.0
        }
    }

    pub fn get_volume(&self, index: usize, speed: f32, power: f32) -> f32 {
        if power == 0.0 {
            return 0.0;
        }
        let offset = if power > 0.0 { 0 } else { 2 };
        let entry_index = (speed / 0.2) as usize;
        let e0 = Self::get_safe(&self.channels[offset], entry_index);
        let e1 = Self::get_safe(&self.channels[offset + 1], entry_index);
        if index as i32 == e0.0 {
            e0.2 * power.abs()
        } else if index as i32 == e1.0 {
            e1.2 * power.abs()
        } else {
            0.0
        }
    }
}

/// Either motor data flavour.
#[derive(Debug, Clone)]
pub enum BveMotorData {
    V4(BveMotorData4),
    V5(BveMotorData5),
}

impl BveMotorData {
    pub fn sound_count(&self) -> usize {
        match self {
            BveMotorData::V4(d) => d.sound_count(),
            BveMotorData::V5(d) => d.sound_count(),
        }
    }

    pub fn get_pitch(&self, index: usize, speed: f32, power: f32) -> f32 {
        match self {
            BveMotorData::V4(d) => d.get_pitch(index, speed, power),
            BveMotorData::V5(d) => d.get_pitch(index, speed, power),
        }
    }

    pub fn get_volume(&self, index: usize, speed: f32, power: f32) -> f32 {
        match self {
            BveMotorData::V4(d) => d.get_volume(index, speed, power),
            BveMotorData::V5(d) => d.get_volume(index, speed, power),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sound_cfg_parse() {
        let cfg = BveConfig::parse("BveTs Vehicle Sound Data 2.00\nMotorNoiseDataType=5\nVolume=1.0");
        assert_eq!(cfg.motor_noise_data_type, 5);
    }

    #[test]
    fn splines_interpolate() {
        let csv = "0,0.1,0.2\n5,1.0,0.4\n10,2.0,0.8";
        let s = FloatSplines::parse(csv);
        assert_eq!(s.data.len(), 2);
        assert!((s.get_value(0, 0.0) - 0.1).abs() < 1e-6);
        assert!((s.get_value(0, 2.5) - 0.55).abs() < 1e-6);
        assert!((s.get_value(1, 7.5) - 0.6).abs() < 1e-6);
        assert!((s.get_value(1, 99.0) - 0.8).abs() < 1e-6);
        assert_eq!(s.get_value(5, 0.0), 0.0);
    }

    #[test]
    fn motor_data5_power_selection() {
        let d = BveMotorData5 {
            power_frequency: FloatSplines::parse("0,1.0\n10,2.0"),
            power_volume: FloatSplines::parse("0,0.5\n10,1.0"),
            ..Default::default()
        };
        assert_eq!(d.get_pitch(0, 5.0, 0.0), 0.0);
        assert!((d.get_pitch(0, 5.0, 1.0) - 1.5).abs() < 1e-6);
        assert!((d.get_volume(0, 5.0, 1.0) - 0.75).abs() < 1e-6);
        assert_eq!(d.get_pitch(0, 5.0, -1.0), 0.0); // brake splines empty
    }

    #[test]
    fn motor_data4_channels() {
        let dat = "#Motor_P1\n0,100,64\n#Motor_P2\n1,110,64\n#Motor_B1\n0,90,32\n#Motor_B2\n1,95,32\n";
        let d = BveMotorData4::parse(dat);
        assert_eq!(d.sound_count(), 2);
        assert!((d.get_pitch(0, 0.0, 1.0) - 1.0).abs() < 1e-6);
        assert!((d.get_pitch(1, 0.0, 1.0) - 1.1).abs() < 1e-6);
        assert!((d.get_volume(0, 0.0, -1.0) - 0.25).abs() < 1e-6);
        assert_eq!(d.get_pitch(0, 1.0, 0.0), 0.0);
    }
}
