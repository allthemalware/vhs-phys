//! Tape speed (SP / LP / EP) and the track geometry that follows from it.
//!
//! The recorded track angle is the angle of the head path over a *stationary* tape
//! (ST 32: 5 deg 56' 7.4") plus the contribution of the tape's own motion: the head
//! moves relative to the tape by the vector difference of its drum velocity and the
//! tape velocity, so a slower tape leaves the track a little shallower. SP uses the
//! format's own figure (5 deg 58' 9.9"); LP and EP follow from the velocity vector.
//!
//! The track pitch is the lateral distance the head path advances between two fields,
//! `advance * sin(angle)`: 58 / 28.9 / 19.3 um, the figures of the format.

use crate::consts::*;
use serde::{Deserialize, Serialize};

/// Angle of the head path over a stationary tape (SMPTE ST 32), degrees.
pub const STATIONARY_ANGLE_DEG: f64 = 5.0 + 56.0 / 60.0 + 7.4 / 3600.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TapeSpeed {
    #[default]
    Sp,
    Lp,
    Ep,
}

impl TapeSpeed {
    pub const ALL: [TapeSpeed; 3] = [TapeSpeed::Sp, TapeSpeed::Lp, TapeSpeed::Ep];

    /// Fraction of the SP tape speed: 1, 1/2, 1/3.
    pub fn ratio(self) -> f64 {
        match self {
            TapeSpeed::Sp => 1.0,
            TapeSpeed::Lp => 0.5,
            TapeSpeed::Ep => 1.0 / 3.0,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            TapeSpeed::Sp => "sp",
            TapeSpeed::Lp => "lp",
            TapeSpeed::Ep => "ep",
        }
    }

    /// Tape speed, metres per second.
    pub fn tape_speed_m_s(self) -> f64 {
        TAPE_SPEED_SP * self.ratio()
    }

    /// Tape advance per field (one head sweep), micrometres.
    pub fn advance_um(self) -> f64 {
        self.tape_speed_m_s() / (2.0 * FRAME_RATE) * 1.0e6
    }

    /// Angle of the head path relative to the moving tape, degrees.
    fn path_angle_deg(self) -> f64 {
        let th = STATIONARY_ANGLE_DEG.to_radians();
        (th.sin() / (th.cos() - self.tape_speed_m_s() / HEAD_SPEED)).atan().to_degrees()
    }

    /// Recorded track angle, degrees. SP is exactly `TRACK_ANGLE_DEG`.
    pub fn track_angle_deg(self) -> f64 {
        TRACK_ANGLE_DEG + self.path_angle_deg() - TapeSpeed::Sp.path_angle_deg()
    }

    /// Track pitch, micrometres. SP is exactly `TRACK_PITCH_UM`.
    pub fn track_pitch_um(self) -> f64 {
        TRACK_PITCH_UM
            * (self.advance_um() / TapeSpeed::Sp.advance_um())
            * (self.track_angle_deg().to_radians().sin() / TRACK_ANGLE_DEG.to_radians().sin())
    }

    /// Along-track offset of adjacent tracks in RF samples. The tape advance per field is
    /// 1.5 H of head travel at SP (so neighbouring tracks are line aligned, see
    /// docs/RESEARCH.md 3.4); at LP and EP it is 0.75 H and 0.5 H.
    pub fn adjacent_track_offset_rf(self) -> f64 {
        ADJACENT_TRACK_OFFSET_RF as f64 * self.ratio()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sp_is_the_formats_own_figures() {
        assert_eq!(TapeSpeed::Sp.track_pitch_um(), TRACK_PITCH_UM);
        assert_eq!(TapeSpeed::Sp.track_angle_deg(), TRACK_ANGLE_DEG);
        assert_eq!(TapeSpeed::Sp.adjacent_track_offset_rf(), 4095.0);
        assert!((TapeSpeed::Sp.advance_um() - 556.4).abs() < 0.5);
    }

    #[test]
    fn slower_tapes_have_narrower_tracks() {
        let (lp, ep) = (TapeSpeed::Lp.track_pitch_um(), TapeSpeed::Ep.track_pitch_um());
        assert!((lp - 28.9).abs() < 0.3, "{lp}");
        assert!((ep - 19.3).abs() < 0.3, "{ep}");
        assert!(TapeSpeed::Lp.track_angle_deg() < TRACK_ANGLE_DEG);
        assert!(TapeSpeed::Ep.track_angle_deg() < TapeSpeed::Lp.track_angle_deg());
        assert!((TapeSpeed::Lp.adjacent_track_offset_rf() - 2047.5).abs() < 1e-9);
    }

    #[test]
    fn pitch_is_advance_times_sine_of_the_angle() {
        for s in TapeSpeed::ALL {
            let p = s.advance_um() * s.track_angle_deg().to_radians().sin();
            assert!((p - s.track_pitch_um()).abs() < 0.3, "{s:?}: {p} vs {}", s.track_pitch_um());
        }
    }
}
