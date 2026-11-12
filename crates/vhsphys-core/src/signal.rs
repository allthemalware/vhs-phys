//! The contract of a signal that passes between decks: which raster it is scanned on, what
//! form it has on the wire, and where on the programme timeline it sits.
//!
//! The two engines carry their rasters as compile-time constants and their `Baseband` types
//! are distinct. Between engines nothing may be inferred from a buffer's length or from a
//! sample index: the 525-line grid is 910 samples per line at 4 x fsc (14.318 MS/s), the
//! 625-line grid is 910 samples per line at 14.21875 MS/s, so equal indices are not equal
//! times. A `SignalChunk` therefore states its `SignalPort` and its presentation time, and
//! every time on the programme timeline is an exact `Rational` number of seconds.
//!
//! Provenance tags follow `docs/RESEARCH.md`: `[std]` a standard, `[svc]` service data,
//! `[derived]` computed from those, `[meas]` measured, `[est]` estimated.
//!
//! See `docs/design/HETEROGENEOUS_BOUNCE.md`, section 4.

use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::fmt;

// ------------------------------------------------------------------ exact time

/// An exact fraction, kept reduced with a positive denominator. Field and frame schedules
/// are computed with it so that no clock is ever accumulated in binary floating point.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Rational {
    num: i64,
    den: i64,
}

fn gcd(a: i128, b: i128) -> i128 {
    let (mut a, mut b) = (a.abs(), b.abs());
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

impl Rational {
    pub const ZERO: Rational = Rational { num: 0, den: 1 };

    /// `num / den`, reduced. Panics on a zero denominator or a result outside 64 bits (a
    /// programme time needs neither).
    pub fn new(num: i64, den: i64) -> Self {
        Self::reduce(num as i128, den as i128)
    }

    pub fn int(n: i64) -> Self {
        Self { num: n, den: 1 }
    }

    fn reduce(num: i128, den: i128) -> Self {
        assert!(den != 0, "a fraction needs a denominator");
        let g = gcd(num, den).max(1);
        let s = if den < 0 { -1 } else { 1 };
        let (n, d) = (s * num / g, s * den / g);
        Self { num: i64::try_from(n).expect("fraction out of range"), den: i64::try_from(d).expect("fraction out of range") }
    }

    pub fn num(self) -> i64 {
        self.num
    }

    pub fn den(self) -> i64 {
        self.den
    }

    pub fn add(self, o: Rational) -> Rational {
        Self::reduce(self.num as i128 * o.den as i128 + o.num as i128 * self.den as i128, self.den as i128 * o.den as i128)
    }

    pub fn sub(self, o: Rational) -> Rational {
        Self::reduce(self.num as i128 * o.den as i128 - o.num as i128 * self.den as i128, self.den as i128 * o.den as i128)
    }

    pub fn mul(self, o: Rational) -> Rational {
        Self::reduce(self.num as i128 * o.num as i128, self.den as i128 * o.den as i128)
    }

    pub fn div(self, o: Rational) -> Rational {
        Self::reduce(self.num as i128 * o.den as i128, self.den as i128 * o.num as i128)
    }

    pub fn mul_int(self, n: i64) -> Rational {
        Self::reduce(self.num as i128 * n as i128, self.den as i128)
    }

    /// The largest whole number not above the fraction.
    pub fn floor(self) -> i64 {
        self.num.div_euclid(self.den)
    }

    /// The smallest whole number not below the fraction.
    pub fn ceil(self) -> i64 {
        -(-self.num).div_euclid(self.den)
    }

    pub fn to_f64(self) -> f64 {
        self.num as f64 / self.den as f64
    }

    /// The nearest fraction of whole microseconds to `seconds`: how a time typed by a person
    /// (a `--start`) enters the exact timeline.
    pub fn from_seconds_micros(seconds: f64) -> Rational {
        Rational::new((seconds * 1e6).round() as i64, 1_000_000)
    }
}

impl PartialOrd for Rational {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

impl Ord for Rational {
    fn cmp(&self, o: &Self) -> Ordering {
        (self.num as i128 * o.den as i128).cmp(&(o.num as i128 * self.den as i128))
    }
}

impl fmt::Display for Rational {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.den == 1 {
            write!(f, "{}", self.num)
        } else {
            write!(f, "{}/{}", self.num, self.den)
        }
    }
}

// ------------------------------------------------------------------ provenance

/// Where a figure comes from, as tagged throughout the project's research notes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provenance {
    /// A published standard.
    Std,
    /// Service literature.
    Svc,
    /// Computed from standard or service figures by a stated rule.
    Derived,
    /// Measured on named equipment.
    Meas,
    /// An estimate.
    Est,
}

impl Provenance {
    pub fn tag(self) -> &'static str {
        match self {
            Provenance::Std => "[std]",
            Provenance::Svc => "[svc]",
            Provenance::Derived => "[derived]",
            Provenance::Meas => "[meas]",
            Provenance::Est => "[est]",
        }
    }
}

// ------------------------------------------------------------------ raster

/// The three planes of one frame on an engine's baseband grid: luma with sync, and the two
/// colour-difference planes (U, V, unswitched), all in the engine's IRE units.
#[derive(Clone)]
pub struct Planes {
    pub y: Vec<f32>,
    pub cu: Vec<f32>,
    pub cv: Vec<f32>,
}

/// Scan timing: the line and field cadence of the electrical signal. It is not a colour
/// system and not a tape format; PAL-60 is PAL colour on the 525-line raster.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Raster {
    /// 525 lines, 60000/1001 fields per second, 2:1 interlace.
    #[serde(rename = "525/59.94")]
    L525,
    /// 625 lines, 50 fields per second, 2:1 interlace.
    #[serde(rename = "625/50")]
    L625,
}

/// Everything about a raster that code outside an engine needs, taken from that engine's
/// constants so the two cannot drift apart.
pub struct RasterSpec {
    pub raster: Raster,
    pub name: &'static str,
    pub lines: usize,
    /// Field period, seconds [std]: 1001/60000 or 1/50. Field `n` of the timeline starts at
    /// `n` field periods; frame `k` is fields `2k` and `2k + 1`.
    pub field_period: Rational,
    /// Line period, seconds [std]. Successive fields are offset by half a line (2:1 interlace).
    pub line_period: Rational,
    /// Baseband samples per line of the engine's line-locked grid.
    pub samples_per_line: usize,
    /// Samples of one frame on that grid.
    pub frame_samples: usize,
    /// First sample of the active picture within a line, and the active width, in samples.
    pub active_start: usize,
    pub active_w: usize,
    /// Picture lines of the active aperture (both fields).
    pub active_h: usize,
    /// First visible frame line of each field; display row `r` is line
    /// `field_first_line[r % 2] + r / 2` of field `r % 2`.
    pub field_first_line: [usize; 2],
    /// Black level and the black-to-white span, in the engine's IRE units.
    pub setup_ire: f32,
    pub span_ire: f32,
    /// Nominal luminance bandwidth of the broadcast system on this raster, Hz [std]
    /// (ITU-R BT.470: 4.2 MHz for System M, 5.0 MHz for Systems B/G).
    pub luma_bandwidth_hz: f64,
    /// A frame of sync, blanking and burst with no picture.
    pub blank: fn() -> Planes,
    /// Whether sample `x` of the active part of frame line `line` carries picture (the last
    /// visible line of the second 625-line field is a half line).
    pub carries_picture: fn(usize, usize) -> bool,
}

/// Colour-difference bandwidth a converter keeps, Hz [std]: 1.3 MHz is the wider of the NTSC
/// colour-difference bandwidths (I) and the PAL U/V bandwidth of BT.470.
pub const CHROMA_BANDWIDTH_HZ: f64 = 1.3e6;

macro_rules! raster_spec {
    ($engine:ident, $raster:expr, $name:literal, $field:expr, $line:expr, $bw:expr) => {{
        use crate::$engine::consts as c;
        use crate::$engine::video;
        fn blank() -> Planes {
            let mut b = video::Baseband::blank();
            video::write_standard_burst(&mut b);
            Planes { y: b.y, cu: b.cu, cv: b.cv }
        }
        RasterSpec {
            raster: $raster,
            name: $name,
            lines: c::LINES,
            field_period: $field,
            line_period: $line,
            samples_per_line: c::BB_PER_LINE,
            frame_samples: c::FRAME_BB,
            active_start: c::ACTIVE_START,
            active_w: c::ACTIVE_W,
            active_h: c::ACTIVE_H,
            field_first_line: c::FIELD_FIRST_LINE,
            setup_ire: c::SETUP_IRE,
            span_ire: c::VIDEO_SPAN_IRE,
            luma_bandwidth_hz: $bw,
            blank,
            carries_picture: video::carries_picture,
        }
    }};
}

impl Raster {
    pub const ALL: [Raster; 2] = [Raster::L525, Raster::L625];

    pub fn spec(self) -> RasterSpec {
        match self {
            // 525 lines in 1001/30000 s: a line is 1001/15 750 000 s (15 734.27 Hz)
            Raster::L525 => raster_spec!(ntsc, Raster::L525, "525/59.94", Rational::new(1001, 60_000), Rational::new(1001, 15_750_000), 4.2e6),
            Raster::L625 => raster_spec!(pal, Raster::L625, "625/50", Rational::new(1, 50), Rational::new(1, 15_625), 5.0e6),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Raster::L525 => "525/59.94",
            Raster::L625 => "625/50",
        }
    }

    pub fn field_period(self) -> Rational {
        match self {
            Raster::L525 => Rational::new(1001, 60_000),
            Raster::L625 => Rational::new(1, 50),
        }
    }

    /// Two fields.
    pub fn frame_period(self) -> Rational {
        self.field_period().mul_int(2)
    }

    /// Programme time at which frame `k` (its first field) starts.
    pub fn frame_time(self, k: i64) -> Rational {
        self.frame_period().mul_int(k)
    }

    /// The first frame that starts at or after programme time `t`.
    pub fn first_frame_at_or_after(self, t: Rational) -> i64 {
        t.div(self.frame_period()).ceil()
    }
}

// ------------------------------------------------------------------ ports

/// Colour encoding of a composite signal. Not a raster: PAL colour exists on both.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ColourSystem {
    Ntsc,
    Pal,
}

/// A complete composite signal system: raster, levels and colour encoding together.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompositeSystem {
    /// ITU System M with NTSC colour (525/59.94, 3.58 MHz subcarrier) [std].
    #[serde(rename = "M/NTSC")]
    MNtsc,
    /// ITU Systems B/G with PAL colour (625/50, 4.43 MHz subcarrier) [std].
    #[serde(rename = "B,G/PAL")]
    BgPal,
    /// PAL colour at 4.43 MHz on the 525/59.94 raster: the playback signal of a multi-standard
    /// deck. Not an ITU system; it exists only between such a deck and a set that locks to it.
    #[serde(rename = "PAL-60")]
    Pal60,
}

impl CompositeSystem {
    pub fn name(self) -> &'static str {
        match self {
            CompositeSystem::MNtsc => "M/NTSC",
            CompositeSystem::BgPal => "B,G/PAL",
            CompositeSystem::Pal60 => "PAL-60",
        }
    }

    pub fn raster(self) -> Raster {
        match self {
            CompositeSystem::MNtsc | CompositeSystem::Pal60 => Raster::L525,
            CompositeSystem::BgPal => Raster::L625,
        }
    }

    pub fn colour(self) -> ColourSystem {
        match self {
            CompositeSystem::MNtsc => ColourSystem::Ntsc,
            CompositeSystem::BgPal | CompositeSystem::Pal60 => ColourSystem::Pal,
        }
    }
}

/// What is on the wire.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignalForm {
    /// The numerical reference port: decoded luma and colour-difference samples handed over
    /// as they are. No connector of a real deck corresponds to it.
    DirectYc,
    /// Separate luma/sync and chroma/burst conductors (S-Video).
    SeparateYc,
    /// One composite waveform of the named system.
    Composite(CompositeSystem),
}

impl SignalForm {
    pub fn name(self) -> String {
        match self {
            SignalForm::DirectYc => "direct Y/C reference".into(),
            SignalForm::SeparateYc => "separate Y/C".into(),
            SignalForm::Composite(s) => format!("composite {}", s.name()),
        }
    }
}

/// One end of a connection: the raster the signal is scanned on and its form.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignalPort {
    pub raster: Raster,
    pub form: SignalForm,
}

impl SignalPort {
    pub fn reference(raster: Raster) -> Self {
        Self { raster, form: SignalForm::DirectYc }
    }

    pub fn describe(&self) -> String {
        format!("{} {}", self.raster.name(), self.form.name())
    }
}

/// One frame (two fields) of signal with what is needed to place it: its port, its frame
/// index on the raster's own clock and the programme time of its first field. Field parity
/// follows from the index: the first field of a frame is the even field of the timeline, the
/// second starts one field period later and half a line lower.
#[derive(Clone)]
pub struct SignalChunk {
    pub port: SignalPort,
    /// Frame index on the raster's clock: the frame starts at `frame` frame periods.
    pub frame: i64,
    /// Presentation time of the first sample of the frame's first field, seconds.
    pub pts: Rational,
    pub planes: Planes,
}

impl SignalChunk {
    pub fn new(raster: Raster, frame: i64, planes: Planes) -> Self {
        Self { port: SignalPort::reference(raster), frame, pts: raster.frame_time(frame), planes }
    }

    /// Check that the chunk is what its tag says: the planes have the raster's frame length
    /// and the time is the raster's time for the frame index.
    pub fn check(&self) -> Result<(), String> {
        let n = self.port.raster.spec().frame_samples;
        if self.planes.y.len() != n || self.planes.cu.len() != n || self.planes.cv.len() != n {
            return Err(format!("a {} frame has {n} samples per plane, this chunk has {}", self.port.raster.name(), self.planes.y.len()));
        }
        if self.pts != self.port.raster.frame_time(self.frame) {
            return Err(format!("frame {} of {} starts at {} s, the chunk says {} s", self.frame, self.port.raster.name(), self.port.raster.frame_time(self.frame), self.pts));
        }
        Ok(())
    }
}

/// How a link treats time. Signal delay is physics (the group delay of the device); lookahead
/// is only how much later input must have arrived before an output can be computed. A
/// symmetric interpolation aperture has lookahead and no signal delay.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct LinkTiming {
    /// Group delay the link adds to the picture, seconds.
    pub signal_delay: Rational,
    /// Input the link must have seen past an output instant before it can produce it, seconds.
    pub lookahead: Rational,
}

impl LinkTiming {
    pub const NONE: LinkTiming = LinkTiming { signal_delay: Rational::ZERO, lookahead: Rational::ZERO };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fractions_stay_reduced_and_exact() {
        let a = Rational::new(1001, 60_000);
        assert_eq!(a.mul_int(60_000), Rational::int(1001));
        assert_eq!(Rational::new(2, -4), Rational::new(-1, 2));
        assert_eq!(Rational::new(7, 2).floor(), 3);
        assert_eq!(Rational::new(7, 2).ceil(), 4);
        assert_eq!(Rational::new(-7, 2).floor(), -4);
        assert_eq!(Rational::new(-7, 2).ceil(), -3);
        assert_eq!(Rational::int(5).ceil(), 5);
        assert!(Rational::new(1, 50) > a);
        // a million 525-line fields, with no drift at all
        assert_eq!(a.mul_int(1_000_000), Rational::new(1001 * 50, 3));
        assert_eq!(a.add(a).sub(a).div(a), Rational::int(1));
    }

    #[test]
    fn the_rasters_have_their_standard_clocks() {
        let n = Raster::L525.spec();
        let p = Raster::L625.spec();
        // a frame is `lines` lines exactly
        assert_eq!(n.line_period.mul_int(525), Raster::L525.frame_period());
        assert_eq!(p.line_period.mul_int(625), Raster::L625.frame_period());
        // the exact clocks agree with the engines' own floating-point figures
        assert!((1.0 / Raster::L525.frame_period().to_f64() - crate::ntsc::consts::FRAME_RATE).abs() < 1e-9);
        assert!((1.0 / Raster::L625.frame_period().to_f64() - crate::pal::consts::FRAME_RATE).abs() < 1e-9);
        assert!((1.0 / n.line_period.to_f64() - crate::ntsc::consts::FH).abs() < 1e-6);
        // 30 000 frames of 525 last exactly as long as 25 025 of 625 (1001 s)
        assert_eq!(Raster::L525.frame_time(30_000), Raster::L625.frame_time(25_025));
        // equal sample indices are not equal times: the grids differ
        assert_ne!(n.line_period, p.line_period);
        assert_eq!((n.samples_per_line, p.samples_per_line), (910, 910));
    }

    #[test]
    fn frame_starts_round_up_to_the_grid() {
        let t = Raster::L525.frame_time(7);
        assert_eq!(Raster::L525.first_frame_at_or_after(t), 7);
        // 7 frames of 525 are 0.2336 s: the next 625-line frame is the sixth (0.24 s)
        assert_eq!(Raster::L625.first_frame_at_or_after(t), 6);
        assert_eq!(Raster::L625.first_frame_at_or_after(Rational::ZERO), 0);
    }

    #[test]
    fn a_chunk_is_checked_against_its_tag() {
        let planes = (Raster::L525.spec().blank)();
        let good = SignalChunk::new(Raster::L525, 3, planes.clone());
        assert!(good.check().is_ok());
        // the same samples relabelled as the other raster are refused, as is a wrong time
        let relabelled = SignalChunk { port: SignalPort::reference(Raster::L625), ..good.clone() };
        assert!(relabelled.check().is_err());
        let late = SignalChunk { pts: Rational::new(1, 3), ..good };
        assert!(late.check().is_err());
    }

    #[test]
    fn composite_systems_keep_raster_and_colour_apart() {
        assert_eq!((CompositeSystem::Pal60.raster(), CompositeSystem::Pal60.colour()), (Raster::L525, ColourSystem::Pal));
        assert_eq!((CompositeSystem::MNtsc.raster(), CompositeSystem::MNtsc.colour()), (Raster::L525, ColourSystem::Ntsc));
        assert_eq!((CompositeSystem::BgPal.raster(), CompositeSystem::BgPal.colour()), (Raster::L625, ColourSystem::Pal));
    }
}
