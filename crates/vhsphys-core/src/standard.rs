//! Television standards: which raster the deck records (the engine) and which colour system
//! the television decodes.
//!
//! * `ntsc`: 525 lines / 59.94 fields, NTSC colour-under, NTSC television.
//! * `pal`: 625 lines / 50 fields, PAL colour-under, PAL television (delay-line decoder).
//! * `pal60`: a 525-line NTSC tape played on a multi-standard PAL deck into a PAL
//!   television. The tape and its mechanics are NTSC; the deck moves the chroma to
//!   4.43 MHz with the V axis switched line by line, so the television's PAL decoder
//!   turns the tape's phase errors into saturation errors instead of hue errors.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TvStandard {
    #[default]
    Ntsc,
    Pal,
    Pal60,
}

impl TvStandard {
    pub const ALL: [TvStandard; 3] = [TvStandard::Ntsc, TvStandard::Pal, TvStandard::Pal60];

    pub fn name(self) -> &'static str {
        match self {
            TvStandard::Ntsc => "ntsc",
            TvStandard::Pal => "pal",
            TvStandard::Pal60 => "pal60",
        }
    }

    /// True when the tape carries a 625-line recording (the `pal` engine).
    pub fn is_625(self) -> bool {
        self == TvStandard::Pal
    }

    /// The colour system of the television at the end of the chain.
    pub fn colour_tv(self) -> ColourTv {
        match self {
            TvStandard::Ntsc => ColourTv::Ntsc,
            TvStandard::Pal | TvStandard::Pal60 => ColourTv::Pal,
        }
    }
}

/// Colour decoder of the television.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ColourTv {
    #[default]
    Ntsc,
    Pal,
}

/// PAL delay-line decoding of one line.
///
/// `cur` and `prev` are the chroma phasors `[re, im]` of this line and of the line before it in
/// the same field, as the television demodulates them against its own reference (V not yet
/// un-switched). `cur_switched` is true when the V axis of this line is inverted. The
/// decoder adds the two lines to separate U and subtracts them to separate V:
///
///     U = Re(cur + prev) / 2,   V = s * Im(cur - prev) / 2,   s = -1 if cur_switched else 1
///
/// A phase error `phi` common to both lines then scales U and V by `cos(phi)` and leaves
/// the hue alone. Returns `[U, V]`.
pub fn pal_delay_line(cur: [f32; 2], prev: [f32; 2], cur_switched: bool) -> [f32; 2] {
    let s = if cur_switched { -1.0 } else { 1.0 };
    [0.5 * (cur[0] + prev[0]), s * 0.5 * (cur[1] - prev[1])]
}

/// Simple PAL (no delay line): un-switch V on this line only. A phase error then shows as
/// a hue error of opposite sign on alternate lines (Hanover bars).
pub fn pal_simple(cur: [f32; 2], cur_switched: bool) -> [f32; 2] {
    [cur[0], if cur_switched { -cur[1] } else { cur[1] }]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rot(p: [f32; 2], a: f32) -> [f32; 2] {
        let (c, s) = (a.cos(), a.sin());
        [p[0] * c - p[1] * s, p[0] * s + p[1] * c]
    }

    #[test]
    fn delay_line_turns_a_phase_error_into_a_saturation_error() {
        let (u, v, phi) = (0.3f32, -0.2f32, 0.5f32);
        for sw in [false, true] {
            let sc = if sw { -1.0 } else { 1.0 };
            let cur = rot([u, sc * v], phi);
            let prev = rot([u, -sc * v], phi);
            let out = pal_delay_line(cur, prev, sw);
            assert!((out[0] - u * phi.cos()).abs() < 1e-6 && (out[1] - v * phi.cos()).abs() < 1e-6, "{out:?}");
        }
    }

    #[test]
    fn names_round_trip() {
        for s in TvStandard::ALL {
            let t: TvStandard = serde_json_like(s.name());
            assert_eq!(s, t);
        }
    }

    fn serde_json_like(name: &str) -> TvStandard {
        #[derive(Deserialize)]
        struct W {
            s: TvStandard,
        }
        toml::from_str::<W>(&format!("s = \"{name}\"")).unwrap().s
    }
}
