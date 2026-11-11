//! The camera stage: the video that goes into the recorder is what a three-tube Plumbicon
//! camera made of the scene, with the lag and the comet tails of its pickup tubes (`tube`).
//!
//! The input frame is taken as the camera's gamma-corrected output for the scene it was
//! shot of: it is run back through the BT.709 transfer to scene light, scaled by the iris,
//! integrated on the tube targets (one charge per picture element and colour channel, which
//! the stage keeps from frame to frame), read out by the beam and gamma-corrected again. With
//! a tube that had no lag and no beam limit the stage would give the input back (apart from
//! the tubes' transfer exponent of 0.95); what it adds is what the beam leaves behind.
//!
//! The stage is off by default and then costs nothing: the session does not even build it.

use super::consts::{ACTIVE_H, ACTIVE_W, BB_PER_LINE, FRAME_RATE, LINES};
use super::video::Frame;
use crate::tube::{dark_equilibrium, frame_step, oetf, oetf_inverse, photo_current, TubeChannel, TubeKind, BEAM_RATIO_DEFAULT};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

/// The camera: which pickup tubes it has and how it is set up.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CameraParams {
    /// The pickup tube (`off`, `plumbicon`, `plumbicon-bias`, `plumbicon-act`).
    pub tube: TubeKind,
    /// Exposure of the scene on the tube targets, in stops (powers of two of the light).
    /// Positive values overexpose: highlights then carry more than the beam can take off.
    pub iris_stops: f64,
    /// Beam current as a multiple of what peak white needs (the data sheet sets 2.0).
    pub beam_ratio: f64,
}

impl Default for CameraParams {
    fn default() -> Self {
        Self { tube: TubeKind::Off, iris_stops: 0.0, beam_ratio: BEAM_RATIO_DEFAULT }
    }
}

/// Time per frame the beam spends on the picture, seconds: the active part of the line times
/// the active part of the lines times the frame period, so that one code serves 525/60 and
/// 625/50.
fn active_scan_s() -> f64 {
    (ACTIVE_W as f64 / BB_PER_LINE as f64) * (ACTIVE_H as f64 / LINES as f64) / FRAME_RATE
}

/// The pickup tubes of the camera with the charge on every element of their targets.
#[derive(Clone)]
pub struct CameraTube {
    channels: [TubeChannel; 3],
    /// Signal of an element in dark equilibrium, nA: the level the camera clamps to black.
    black: [f64; 3],
    /// Photo current for each input level, nA, per channel: the iris, the inverse gamma
    /// corrector and the target's transfer exponent, evaluated once for the 256 levels.
    photo: [[f64; 256]; 3],
    /// Charge per element and channel (`ACTIVE_W x ACTIVE_H x 3`), nA.
    charge: Vec<f64>,
}

impl CameraTube {
    /// The stage for `params`, or `None` when the camera has no tube. Every element starts
    /// in dark equilibrium, as a camera does that has been capped for a long time.
    pub fn new(params: &CameraParams) -> Option<Self> {
        let channels = params.tube.channels(params.beam_ratio, active_scan_s())?;
        let exposure = params.iris_stops.exp2();
        let mut black = [0.0; 3];
        let mut photo = [[0.0; 256]; 3];
        let mut start = [0.0; 3];
        for k in 0..3 {
            start[k] = dark_equilibrium(&channels[k]);
            let mut q = start[k];
            black[k] = frame_step(&channels[k], &mut q, 0.0);
            for (v, p) in photo[k].iter_mut().enumerate() {
                *p = photo_current(&channels[k], oetf_inverse(v as f64 / 255.0) * exposure);
            }
        }
        let charge = (0..ACTIVE_W * ACTIVE_H * 3).map(|i| start[i % 3]).collect();
        Some(Self { channels, black, photo, charge })
    }

    /// Update the iris exposure setting for subsequent frames.
    pub fn set_iris_stops(&mut self, iris_stops: f64) {
        let exposure = iris_stops.exp2();
        for k in 0..3 {
            for (v, p) in self.photo[k].iter_mut().enumerate() {
                *p = photo_current(&self.channels[k], oetf_inverse(v as f64 / 255.0) * exposure);
            }
        }
    }

    /// Expose the targets to `frame` for one frame time, scan them, and return what the
    /// camera outputs. The charge left on the targets is kept for the next frame.
    pub fn process_frame(&mut self, frame: &Frame) -> Frame {
        assert_eq!(frame.rgb.len(), self.charge.len(), "the camera stage takes frames of the engine's active size");
        let mut out = Frame::new(frame.w, frame.h);
        let (channels, black, photo) = (&self.channels, &self.black, &self.photo);
        let row = ACTIVE_W * 3;
        out.rgb
            .par_chunks_mut(row)
            .zip(self.charge.par_chunks_mut(row))
            .zip(frame.rgb.par_chunks(row))
            .for_each(|((out, charge), input)| {
                for (i, ((o, q), &v)) in out.iter_mut().zip(charge.iter_mut()).zip(input.iter()).enumerate() {
                    let k = i % 3;
                    let s = frame_step(&channels[k], q, photo[k][v as usize]);
                    let lin = (s - black[k]) / channels[k].peak_signal_na;
                    *o = (255.0 * oetf(lin)).round() as u8;
                }
            });
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(v: u8) -> Frame {
        let mut f = Frame::new(ACTIVE_W, ACTIVE_H);
        f.rgb.fill(v);
        f
    }

    #[test]
    fn the_default_camera_has_no_tube() {
        let p = CameraParams::default();
        assert_eq!(p.tube, TubeKind::Off);
        assert_eq!(p.iris_stops, 0.0);
        assert_eq!(p.beam_ratio, BEAM_RATIO_DEFAULT);
        assert!(CameraTube::new(&p).is_none());
    }

    #[test]
    fn the_tube_is_named_in_the_config_as_the_data_sheet_types_are() {
        let p: CameraParams = toml::from_str("tube = \"plumbicon-act\"\niris_stops = 1.5").unwrap();
        assert_eq!(p.tube, TubeKind::PlumbiconAct);
        assert_eq!(p.iris_stops, 1.5);
        assert_eq!(p.beam_ratio, BEAM_RATIO_DEFAULT);
    }

    #[test]
    fn a_capped_camera_reads_black() {
        let p = CameraParams { tube: TubeKind::Plumbicon, ..CameraParams::default() };
        let mut cam = CameraTube::new(&p).unwrap();
        for _ in 0..3 {
            assert!(cam.process_frame(&flat(0)).rgb.iter().all(|&v| v == 0));
        }
    }

    #[test]
    fn a_light_that_goes_out_leaves_a_fading_trace_and_the_charge_is_kept_between_frames() {
        let p = CameraParams { tube: TubeKind::Plumbicon, ..CameraParams::default() };
        let mut cam = CameraTube::new(&p).unwrap();
        for _ in 0..40 {
            cam.process_frame(&flat(128));
        }
        let first = cam.process_frame(&flat(0)).rgb[1];
        let second = cam.process_frame(&flat(0)).rgb[1];
        let third = cam.process_frame(&flat(0)).rgb[1];
        assert!(first > second && second >= third, "{first} {second} {third}");
        assert!(first > 0 && first < 64, "{first}");
    }
}
