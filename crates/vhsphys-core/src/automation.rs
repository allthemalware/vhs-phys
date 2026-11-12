//! Parameter automation: time-varying parameter curves for tape, deck, mechanics, and camera.
//!
//! Real-world analog video exhibits time-varying degradation:
//! - Tracking offset drift as tape wanders across the drum or manual knobs are turned.
//! - Progressive head clogging as tape sheds oxide into the head gap, causing Wallace spacing loss.
//! - Mechanical tension fluctuations and capstan wow/flutter.
//! - Dynamic camera exposure (iris changes) causing highlight comet tails to bloom and decay.
//!
//! Automation tracks support Hold (step), Linear, Smooth (smoothstep S-curve), and Cosine easing.
//!
//! The tracking, head and iris tracks are offsets added to the configured values (um, degrees or stops;
//! `head_gain` multiplies the configured gain), so a track at zero changes nothing. The three `crt_magnet_*`
//! tracks (degrees, cm, cm) are absolute positions that replace the configured ones.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// The parameters the engine reads from a schedule: each canonical name with the short names a file may use instead.
/// Tracks are stored under the canonical name, so a track under any other name is never read.
const PARAMETERS: &[(&str, &[&str])] = &[
    ("tracking_offset_um", &["tracking", "tracking_offset"]),
    ("tracking_wander_um", &["wander"]),
    ("tracking_slope_um", &["tracking_slope", "slope"]),
    ("tracking_bow_um", &["tracking_bow", "bow"]),
    ("head_spacing_um", &["clog", "clogging", "spacing"]),
    ("head_gain", &[]),
    ("head_wear_um", &["wear"]),
    ("h0_um", &[]),
    ("azimuth_error_deg", &[]),
    ("tension_offset", &["tension"]),
    ("head_magnet_cm", &["head_magnet", "magnet_head", "magnet"]),
    ("iris_stops", &["iris", "exposure"]),
    ("crt_magnet_angle_deg", &["crt_magnet_angle", "magnet_angle"]),
    ("crt_magnet_cm", &["crt_magnet_distance", "magnet_distance", "crt_magnet_dist"]),
    ("crt_magnet_z_cm", &["crt_magnet_z", "magnet_z"]),
];

/// The canonical name for a canonical name or an alias, or `None` when the engine reads no such parameter.
pub fn canonical_name(name: &str) -> Option<&'static str> {
    PARAMETERS.iter().find(|(canonical, aliases)| *canonical == name || aliases.contains(&name)).map(|(canonical, _)| *canonical)
}

/// Every known name for an error message: each canonical name followed by its aliases in brackets.
fn known_names() -> String {
    let entries: Vec<String> = PARAMETERS
        .iter()
        .map(|(canonical, aliases)| if aliases.is_empty() { canonical.to_string() } else { format!("{canonical} [{}]", aliases.join(", ")) })
        .collect();
    entries.join(", ")
}

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
    pub fn new(keyframes: Vec<Keyframe>) -> Self {
        let mut track = Self { keyframes };
        track.sort_keyframes();
        track
    }

    /// Put the keyframes in frame order, which `eval` relies on. Stable, so keyframes at the same frame keep their order.
    pub fn sort_keyframes(&mut self) {
        self.keyframes.sort_by(|a, b| a.frame.partial_cmp(&b.frame).unwrap_or(std::cmp::Ordering::Equal));
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

    /// Store a track under the canonical name when `name` is a known name or alias, and under `name` as given otherwise.
    /// The keyframes are put in frame order.
    pub fn set_track(&mut self, name: impl Into<String>, mut track: AutomationTrack) {
        let name = name.into();
        let key = canonical_name(&name).map_or(name, str::to_string);
        track.sort_keyframes();
        self.tracks.insert(key, track);
    }

    pub fn get_track(&self, name: &str) -> Option<&AutomationTrack> {
        self.tracks.get(canonical_name(name).unwrap_or(name))
    }

    /// A copy holding only the named tracks (by canonical name or alias); names the schedule has no track for are skipped.
    pub fn subset(&self, names: &[&str]) -> Self {
        let tracks = names
            .iter()
            .filter_map(|&name| {
                let key = canonical_name(name).unwrap_or(name);
                self.tracks.get(key).map(|track| (key.to_string(), track.clone()))
            })
            .collect();
        Self { tracks }
    }

    /// Evaluate a parameter track by canonical name or alias.
    pub fn eval(&self, name: &str, frame: f64) -> Option<f64> {
        self.get_track(name).and_then(|track| track.eval(frame))
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

    /// Dynamic mechanical tension offset.
    pub fn tension(&self, frame: f64) -> Option<f64> {
        self.eval("tension_offset", frame)
    }

    /// Dynamic head magnet distance in cm.
    pub fn head_magnet_cm(&self, frame: f64) -> Option<f64> {
        self.eval("head_magnet_cm", frame)
    }

    /// Dynamic camera iris exposure offset in stops.
    pub fn iris_stops(&self, frame: f64) -> Option<f64> {
        self.eval("iris_stops", frame)
    }

    /// The last step of every loader: refuse names the engine does not read and keyframes that are not finite, rename each
    /// track to its canonical name (merging the tracks of two names for one parameter) and put the keyframes in frame order.
    fn into_checked(self) -> Result<Self, String> {
        // sorted so that keyframes at the same frame merge in the same order every time
        let mut named: Vec<(String, AutomationTrack)> = self.tracks.into_iter().collect();
        named.sort_by(|a, b| a.0.cmp(&b.0));

        let mut tracks: HashMap<String, AutomationTrack> = HashMap::new();
        for (name, mut track) in named {
            let Some(canonical) = canonical_name(&name) else {
                return Err(format!("unknown automation track '{name}'; known names: {}", known_names()));
            };
            if let Some(kf) = track.keyframes.iter().find(|k| !k.frame.is_finite() || !k.value.is_finite()) {
                return Err(format!("track '{name}': keyframe at frame {} with value {} is not finite", kf.frame, kf.value));
            }
            tracks.entry(canonical.to_string()).or_default().keyframes.append(&mut track.keyframes);
        }
        for track in tracks.values_mut() {
            track.sort_keyframes();
        }
        Ok(Self { tracks })
    }

    /// Parse schedule from JSON string.
    pub fn from_json_str(s: &str) -> Result<Self, String> {
        Self::parse_json(s)?.into_checked()
    }

    /// Parse schedule from TOML string.
    pub fn from_toml_str(s: &str) -> Result<Self, String> {
        Self::parse_toml(s)?.into_checked()
    }

    /// Parse schedule from CSV string.
    ///
    /// Accepts two common CSV formats:
    /// 1. Event list: `frame,param_name,value[,interp]`
    /// 2. Wide table: `frame,param1,param2,...`
    pub fn from_csv_str(s: &str) -> Result<Self, String> {
        Self::parse_csv(s)?.into_checked()
    }

    // The three parsers only read the syntax of their format; `into_checked` then judges what was read.

    fn parse_json(s: &str) -> Result<Self, String> {
        serde_json::from_str(s).map_err(|e| format!("JSON parse error: {e}"))
    }

    fn parse_toml(s: &str) -> Result<Self, String> {
        toml::from_str(s).map_err(|e| format!("TOML parse error: {e}"))
    }

    fn parse_csv(s: &str) -> Result<Self, String> {
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
                // The first format whose syntax fits wins, and its content is then checked. Falling on to the next
                // format after a refused name would let the CSV parser read a one-line JSON file as an empty table.
                let json_err = match Self::parse_json(&content) {
                    Ok(schedule) => return schedule.into_checked(),
                    Err(e) => e,
                };
                let toml_err = match Self::parse_toml(&content) {
                    Ok(schedule) => return schedule.into_checked(),
                    Err(e) => e,
                };
                match Self::parse_csv(&content) {
                    Ok(schedule) => schedule.into_checked(),
                    Err(csv_err) => Err(format!("not a schedule as JSON ({json_err}), TOML ({toml_err}) or CSV ({csv_err})")),
                }
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

    #[test]
    fn canonical_name_knows_names_and_aliases_only() {
        assert_eq!(canonical_name("iris_stops"), Some("iris_stops"));
        assert_eq!(canonical_name("iris"), Some("iris_stops"));
        assert_eq!(canonical_name("exposure"), Some("iris_stops"));
        assert_eq!(canonical_name("clog"), Some("head_spacing_um"));
        assert_eq!(canonical_name("magnet_z"), Some("crt_magnet_z_cm"));
        assert_eq!(canonical_name("h0_um"), Some("h0_um"));
        assert_eq!(canonical_name("wear"), Some("head_wear_um"));
        assert_eq!(canonical_name("tension"), Some("tension_offset"));
        assert_eq!(canonical_name("head_magnet"), Some("head_magnet_cm"));
        assert_eq!(canonical_name("slope"), Some("tracking_slope_um"));
        assert_eq!(canonical_name("bow"), Some("tracking_bow_um"));
        for unread in ["crinkle_density", "crinkle", "typo", ""] {
            assert_eq!(canonical_name(unread), None, "{unread}");
        }
    }

    #[test]
    fn every_name_belongs_to_one_parameter() {
        let mut seen = std::collections::HashSet::new();
        for (canonical, aliases) in PARAMETERS {
            for name in std::iter::once(canonical).chain(aliases.iter()) {
                assert!(seen.insert(*name), "{name} is listed twice");
            }
        }
    }

    #[test]
    fn set_track_stores_known_names_canonically_and_keeps_other_names() {
        let mut schedule = AutomationSchedule::new();
        schedule.set_track("clog", AutomationTrack::constant(0.3));
        schedule.set_track("my_own_curve", AutomationTrack::constant(9.0));
        assert!(schedule.tracks.contains_key("head_spacing_um"));
        assert!(!schedule.tracks.contains_key("clog"));
        assert!(schedule.tracks.contains_key("my_own_curve"));
        assert_eq!(schedule.head_spacing(0.0), Some(0.3));
        assert_eq!(schedule.eval("spacing", 0.0), Some(0.3));
        assert_eq!(schedule.eval("my_own_curve", 0.0), Some(9.0));
        assert!(schedule.get_track("clogging").is_some());
    }

    #[test]
    fn sort_keyframes_is_stable_and_used_by_new_and_set_track() {
        let unsorted = vec![Keyframe::linear(10.0, 1.0), Keyframe::hold(0.0, 2.0), Keyframe::linear(10.0, 3.0)];
        let track = AutomationTrack::new(unsorted.clone());
        let frames: Vec<(f64, f64)> = track.keyframes.iter().map(|k| (k.frame, k.value)).collect();
        assert_eq!(frames, vec![(0.0, 2.0), (10.0, 1.0), (10.0, 3.0)]);

        let mut schedule = AutomationSchedule::new();
        schedule.set_track("h0_um", AutomationTrack { keyframes: unsorted });
        assert_eq!(schedule.get_track("h0_um").unwrap(), &track);
    }

    #[test]
    fn loaders_merge_the_tracks_of_two_names_for_one_parameter() {
        let csv = "\
frame, param, value
0, tracking, 0.0
20, tracking_offset, 10.0
10, tracking_offset_um, 4.0
";
        let schedule = AutomationSchedule::from_csv_str(csv).unwrap();
        assert_eq!(schedule.tracks.len(), 1);
        let frames: Vec<f64> = schedule.get_track("tracking_offset_um").unwrap().keyframes.iter().map(|k| k.frame).collect();
        assert_eq!(frames, vec![0.0, 10.0, 20.0]);
        assert_eq!(schedule.tracking_offset(15.0), Some(7.0));
    }

    #[test]
    fn loaders_refuse_unknown_names_and_non_finite_keyframes() {
        let err = AutomationSchedule::from_csv_str("frame, crinkle\n0, 1.0\n").unwrap_err();
        assert!(err.contains("crinkle") && err.contains("head_spacing_um"), "{err}");

        let err = AutomationSchedule::from_csv_str("frame, iris\n0, 0.0\n5, inf\n").unwrap_err();
        assert!(err.contains("iris"), "{err}");
        assert!(AutomationSchedule::from_csv_str("frame, param, value\nnan, iris, 1.0\n").is_err());

        let toml = "[tracks.head_gain]\nkeyframes = [{ frame = 0.0, value = nan }]\n";
        assert!(AutomationSchedule::from_toml_str(toml).is_err());
    }

    #[test]
    fn json_keyframes_out_of_order_are_sorted_on_load() {
        let json = r#"{"tracks": {"tracking": {"keyframes": [{"frame": 60, "value": 10}, {"frame": 0, "value": 0}]}}}"#;
        let schedule = AutomationSchedule::from_json_str(json).unwrap();
        assert_eq!(schedule.tracking_offset(30.0), Some(5.0));
    }

    #[test]
    fn subset_keeps_only_the_named_tracks() {
        let mut schedule = AutomationSchedule::new();
        schedule.set_track("iris_stops", AutomationTrack::constant(1.0));
        schedule.set_track("head_gain", AutomationTrack::constant(0.5));
        let camera = schedule.subset(&["iris", "h0_um"]);
        assert_eq!(camera.tracks.len(), 1);
        assert_eq!(camera.iris_stops(0.0), Some(1.0));
        assert_eq!(camera.eval("head_gain", 0.0), None);
        assert_eq!(schedule.tracks.len(), 2, "the original is left alone");
    }
}
