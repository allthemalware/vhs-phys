//! Parameter automation: time-varying parameter curves for tape, deck, mechanics, and camera.
//!
//! Real-world analog video exhibits time-varying degradation:
//! - Tracking offset drift as tape wanders across the drum or manual knobs are turned.
//! - Progressive head clogging as tape sheds oxide into the head gap, causing Wallace spacing loss.
//! - Mechanical tension fluctuations and capstan wow/flutter.
//! - Dynamic camera exposure (iris changes) causing highlight comet tails to bloom and decay.
//!
//! Automation tracks support Hold (step), Linear, Smooth (smoothstep S-curve), and Cosine easing.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// Interpolation mode between two keyframes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Interpolation {
    /// Hold previous value until the next keyframe.
    Hold,
    /// Piecewise linear transition.
    Linear,
    /// Hermite smoothstep easing (zero derivative at endpoints).
    Smooth,
    /// Half-period cosine easing.
    Cosine,
}

impl Default for Interpolation {
    fn default() -> Self {
        Self::Linear
    }
}

/// A single parameter keyframe positioned at an exact frame index or fractional time.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Keyframe {
    /// Frame index (fractional to allow field-level or sub-frame positioning).
    pub frame: f64,
    /// Parameter value at this keyframe.
    pub value: f64,
    /// Transition interpolation to the subsequent keyframe.
    #[serde(default)]
    pub interp: Interpolation,
}

impl Keyframe {
    pub fn new(frame: f64, value: f64, interp: Interpolation) -> Self {
        Self { frame, value, interp }
    }

    pub fn linear(frame: f64, value: f64) -> Self {
        Self::new(frame, value, Interpolation::Linear)
    }

    pub fn hold(frame: f64, value: f64) -> Self {
        Self::new(frame, value, Interpolation::Hold)
    }

    pub fn smooth(frame: f64, value: f64) -> Self {
        Self::new(frame, value, Interpolation::Smooth)
    }
}

/// A time-ordered sequence of keyframes defining a parameter curve.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AutomationTrack {
    pub keyframes: Vec<Keyframe>,
}

impl AutomationTrack {
    pub fn new(mut keyframes: Vec<Keyframe>) -> Self {
        keyframes.sort_by(|a, b| a.frame.partial_cmp(&b.frame).unwrap_or(std::cmp::Ordering::Equal));
        Self { keyframes }
    }

    pub fn is_empty(&self) -> bool {
        self.keyframes.is_empty()
    }

    pub fn len(&self) -> usize {
        self.keyframes.len()
    }

    pub fn add(&mut self, kf: Keyframe) {
        let idx = self.keyframes.partition_point(|k| k.frame <= kf.frame);
        self.keyframes.insert(idx, kf);
    }

    /// Constant value across all frames.
    pub fn constant(value: f64) -> Self {
        Self {
            keyframes: vec![Keyframe::hold(0.0, value)],
        }
    }

    /// Linear ramp between two points.
    pub fn linear_ramp(start_frame: f64, start_val: f64, end_frame: f64, end_val: f64) -> Self {
        Self {
            keyframes: vec![
                Keyframe::linear(start_frame, start_val),
                Keyframe::hold(end_frame, end_val),
            ],
        }
    }

    /// Smooth S-curve transition between two points.
    pub fn smooth_ramp(start_frame: f64, start_val: f64, end_frame: f64, end_val: f64) -> Self {
        Self {
            keyframes: vec![
                Keyframe::smooth(start_frame, start_val),
                Keyframe::hold(end_frame, end_val),
            ],
        }
    }

    /// Evaluate the parameter curve at `frame`.
    pub fn eval(&self, frame: f64) -> Option<f64> {
        if self.keyframes.is_empty() {
            return None;
        }
        if self.keyframes.len() == 1 || frame <= self.keyframes[0].frame {
            return Some(self.keyframes[0].value);
        }
        if frame >= self.keyframes.last().unwrap().frame {
            return Some(self.keyframes.last().unwrap().value);
        }

        let idx = self.keyframes.partition_point(|k| k.frame <= frame);
        let k0 = &self.keyframes[idx - 1];
        let k1 = &self.keyframes[idx];
        let span = k1.frame - k0.frame;
        if span.abs() < 1e-9 {
            return Some(k1.value);
        }

        let alpha = ((frame - k0.frame) / span).clamp(0.0, 1.0);
        let factor = match k0.interp {
            Interpolation::Hold => 0.0,
            Interpolation::Linear => alpha,
            Interpolation::Smooth => alpha * alpha * (3.0 - 2.0 * alpha),
            Interpolation::Cosine => 0.5 * (1.0 - (std::f64::consts::PI * alpha).cos()),
        };

        Some(k0.value + (k1.value - k0.value) * factor)
    }
}

/// Complete parameter automation schedule containing multiple parameter tracks.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AutomationSchedule {
    #[serde(default)]
    pub tracks: HashMap<String, AutomationTrack>,
}

impl AutomationSchedule {
    pub fn new() -> Self {
        Self {
            tracks: HashMap::new(),
        }
    }

    pub fn set_track(&mut self, name: impl Into<String>, track: AutomationTrack) {
        self.tracks.insert(name.into(), track);
    }

    pub fn get_track(&self, name: &str) -> Option<&AutomationTrack> {
        self.tracks.get(name)
    }

    /// Evaluate a parameter track by name, checking canonical aliases.
    pub fn eval(&self, name: &str, frame: f64) -> Option<f64> {
        if let Some(track) = self.tracks.get(name) {
            return track.eval(frame);
        }
        let alias = match name {
            "tracking" | "tracking_offset" => "tracking_offset_um",
            "wander" => "tracking_wander_um",
            "clog" | "clogging" | "spacing" => "head_spacing_um",
            "wear" => "head_wear_um",
            "iris" | "exposure" => "iris_stops",
            "tension" => "tension_offset",
            "crinkle" => "crinkle_density",
            "magnet" => "head_magnet",
            _ => return None,
        };
        self.tracks.get(alias).and_then(|t| t.eval(frame))
    }

    /// Dynamic head tracking offset in um.
    pub fn tracking_offset(&self, frame: f64) -> Option<f64> {
        self.eval("tracking_offset_um", frame)
    }

    /// Dynamic additional head-tape spacing (oxide clogging) in um.
    pub fn head_spacing(&self, frame: f64) -> Option<f64> {
        self.eval("head_spacing_um", frame)
    }

    /// Dynamic head wear in um.
    pub fn head_wear(&self, frame: f64) -> Option<f64> {
        self.eval("head_wear_um", frame)
    }

    /// Dynamic camera iris exposure offset in stops.
    pub fn iris_stops(&self, frame: f64) -> Option<f64> {
        self.eval("iris_stops", frame)
    }

    /// Dynamic mechanical tension offset.
    pub fn tension(&self, frame: f64) -> Option<f64> {
        self.eval("tension_offset", frame)
    }

    /// Parse schedule from JSON string.
    pub fn from_json_str(s: &str) -> Result<Self, String> {
        serde_json::from_str(s).map_err(|e| format!("JSON parse error: {e}"))
    }

    /// Parse schedule from TOML string.
    pub fn from_toml_str(s: &str) -> Result<Self, String> {
        toml::from_str(s).map_err(|e| format!("TOML parse error: {e}"))
    }

    /// Parse schedule from CSV string.
    ///
    /// Accepts two common CSV formats:
    /// 1. Event list: `frame,param_name,value[,interp]`
    /// 2. Wide table: `frame,param1,param2,...`
    pub fn from_csv_str(s: &str) -> Result<Self, String> {
        let lines: Vec<&str> = s.lines().map(|l| l.trim()).filter(|l| !l.is_empty() && !l.starts_with('#')).collect();
        if lines.is_empty() {
            return Ok(Self::new());
        }

        let header: Vec<&str> = lines[0].split(',').map(|c| c.trim()).collect();
        let mut schedule = Self::new();

        if header.len() >= 3 && (header[1].eq_ignore_ascii_case("param") || header[1].eq_ignore_ascii_case("parameter") || header[1].eq_ignore_ascii_case("name")) {
            // Event list format: frame, param, value, [interp]
            for (line_idx, line) in lines.iter().enumerate().skip(1) {
                let parts: Vec<&str> = line.split(',').map(|c| c.trim()).collect();
                if parts.len() < 3 {
                    return Err(format!("line {}: expected at least 3 columns (frame, parameter, value)", line_idx + 1));
                }
                let frame: f64 = parts[0].parse().map_err(|e| format!("line {}: invalid frame: {e}", line_idx + 1))?;
                let param = parts[1];
                let value: f64 = parts[2].parse().map_err(|e| format!("line {}: invalid value: {e}", line_idx + 1))?;
                let interp = if parts.len() > 3 {
                    match parts[3].to_ascii_lowercase().as_str() {
                        "hold" | "step" => Interpolation::Hold,
                        "smooth" => Interpolation::Smooth,
                        "cosine" => Interpolation::Cosine,
                        _ => Interpolation::Linear,
                    }
                } else {
                    Interpolation::Linear
                };
                let track = schedule.tracks.entry(param.to_string()).or_default();
                track.add(Keyframe::new(frame, value, interp));
            }
        } else {
            // Wide table format: frame, param1, param2, ...
            let params = &header[1..];
            for (line_idx, line) in lines.iter().enumerate().skip(1) {
                let parts: Vec<&str> = line.split(',').map(|c| c.trim()).collect();
                if parts.is_empty() {
                    continue;
                }
                let frame: f64 = parts[0].parse().map_err(|e| format!("line {}: invalid frame: {e}", line_idx + 1))?;
                for (col_idx, &param_name) in params.iter().enumerate() {
                    if col_idx + 1 < parts.len() && !parts[col_idx + 1].is_empty() {
                        let value: f64 = parts[col_idx + 1].parse().map_err(|e| {
                            format!("line {}, col {}: invalid value: {e}", line_idx + 1, col_idx + 2)
                        })?;
                        let track = schedule.tracks.entry(param_name.to_string()).or_default();
                        track.add(Keyframe::linear(frame, value));
                    }
                }
            }
        }

        Ok(schedule)
    }

    /// Load an automation schedule from a file, auto-detecting format.
    pub fn from_file(path: &Path) -> Result<Self, String> {
        let content = std::fs::read_to_string(path).map_err(|e| format!("failed to read {}: {e}", path.display()))?;
        let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
        match ext.to_ascii_lowercase().as_str() {
            "json" => Self::from_json_str(&content),
            "toml" => Self::from_toml_str(&content),
            "csv" => Self::from_csv_str(&content),
            _ => {
                // Try json, then toml, then csv
                Self::from_json_str(&content)
                    .or_else(|_| Self::from_toml_str(&content))
                    .or_else(|_| Self::from_csv_str(&content))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn track_evaluation_constant_and_empty() {
        let empty = AutomationTrack::default();
        assert_eq!(empty.eval(0.0), None);

        let c = AutomationTrack::constant(4.2);
        assert_eq!(c.eval(-10.0), Some(4.2));
        assert_eq!(c.eval(0.0), Some(4.2));
        assert_eq!(c.eval(100.0), Some(4.2));
    }

    #[test]
    fn linear_interpolation() {
        let track = AutomationTrack::new(vec![
            Keyframe::linear(0.0, 10.0),
            Keyframe::linear(10.0, 20.0),
        ]);
        assert_eq!(track.eval(-5.0), Some(10.0));
        assert_eq!(track.eval(0.0), Some(10.0));
        assert_eq!(track.eval(5.0), Some(15.0));
        assert_eq!(track.eval(10.0), Some(20.0));
        assert_eq!(track.eval(20.0), Some(20.0));
    }

    #[test]
    fn hold_interpolation() {
        let track = AutomationTrack::new(vec![
            Keyframe::hold(0.0, 5.0),
            Keyframe::hold(10.0, 15.0),
        ]);
        assert_eq!(track.eval(0.0), Some(5.0));
        assert_eq!(track.eval(9.99), Some(5.0));
        assert_eq!(track.eval(10.0), Some(15.0));
        assert_eq!(track.eval(20.0), Some(15.0));
    }

    #[test]
    fn smooth_interpolation() {
        let track = AutomationTrack::new(vec![
            Keyframe::smooth(0.0, 0.0),
            Keyframe::smooth(10.0, 100.0),
        ]);
        // Midpoint of smoothstep 3*(0.5)^2 - 2*(0.5)^3 = 0.75 - 0.25 = 0.5
        let mid = track.eval(5.0).unwrap();
        assert!((mid - 50.0).abs() < 1e-6);

        // Near start, slope is shallower than linear (10 * 2.5 = 25 vs 0.25^2*(3-0.5) = 0.0625*2.5 = 0.15625 * 100 = 15.625)
        let early = track.eval(2.5).unwrap();
        assert!(early < 25.0);
    }

    #[test]
    fn schedule_json_roundtrip() {
        let mut schedule = AutomationSchedule::new();
        schedule.set_track("tracking_offset_um", AutomationTrack::linear_ramp(0.0, 0.0, 60.0, 15.0));
        schedule.set_track("head_spacing_um", AutomationTrack::smooth_ramp(0.0, 0.0, 120.0, 0.5));

        let json = serde_json::to_string(&schedule).unwrap();
        let loaded = AutomationSchedule::from_json_str(&json).unwrap();
        assert_eq!(loaded.tracking_offset(30.0), Some(7.5));
        assert_eq!(loaded.head_spacing(0.0), Some(0.0));
        assert_eq!(loaded.head_spacing(120.0), Some(0.5));
    }

    #[test]
    fn schedule_csv_event_parsing() {
        let csv = "\
# Event-based keyframes
frame, param, value, interp
0, tracking_offset_um, 0.0, linear
60, tracking_offset_um, 10.0, linear
0, iris_stops, 0.0, smooth
30, iris_stops, 2.5, smooth
";
        let schedule = AutomationSchedule::from_csv_str(csv).unwrap();
        assert_eq!(schedule.tracking_offset(30.0), Some(5.0));
        assert_eq!(schedule.iris_stops(0.0), Some(0.0));
        assert_eq!(schedule.iris_stops(30.0), Some(2.5));
    }

    #[test]
    fn schedule_csv_table_parsing() {
        let csv = "\
frame, tracking_offset_um, head_spacing_um
0, 0.0, 0.0
50, 10.0, 0.2
100, 20.0, 0.8
";
        let schedule = AutomationSchedule::from_csv_str(csv).unwrap();
        assert_eq!(schedule.tracking_offset(25.0), Some(5.0));
        assert_eq!(schedule.head_spacing(50.0), Some(0.2));
    }
}
