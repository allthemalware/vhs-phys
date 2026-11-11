//! Vacuum camera pickup tube target physics (Vidicon, NewVicon, Saticon, Plumbicon)
//! and CRT display phosphor persistence.
//!
//! A pickup tube focuses the scene onto a thin photoconductive target sheet held at a bias
//! voltage. Incident light generates photo-carriers that raise the local target potential
//! `V_c(x, y)` per tube channel (`R`, `G`, `B`). When the scanning electron beam sweeps across
//! the target, it deposits electrons to discharge `V_c(x, y)` back toward cathode potential;
//! the discharged current forms the video signal fed to the recorder.
//!
//! Two physical mechanisms produce the characteristic "trailing lights" and comet-tails seen
//! on 1970s/80s PAL and NTSC magnetic tape broadcasts (such as BBC studio and OB recordings):
//!
//! 1. **Beam-current starvation on highlights (comet-tails):** The electron gun has a finite
//!    maximum beam current (`beam_ceiling`). Specular highlights (studio lamps, candles, car
//!    headlights, sequins) charge the target to several times nominal peak white
//!    (`highlight_gain` above `highlight_threshold`). A single scan can only neutralise up to
//!    `beam_ceiling` of that charge; the undischarged residual stays trapped on the target and
//!    is read out over subsequent frames as a flat-topped, decaying comet-tail. Excess charge
//!    also diffuses laterally across the semiconductor sheet (`bloom`), halating around hot
//!    highlights.
//! 2. **Dispersive beam-landing lag and per-channel photoconductor decay (`decay_rgb`):** At
//!    lower target potentials the thermal velocity spread of the electron beam increases
//!    landing resistance (`beam_gamma`), leaving a fraction of the stored charge behind each
//!    field. Because the R, G and B pickup tubes (or colour-under-stripe regions) have
//!    different photoconductive trap time constants (`decay_rgb`), a white moving highlight
//!    shifts hue along its tail (e.g. warm amber/red on Plumbicon or emerald/amber on NewVicon)
//!    before being FM-modulated onto the VHS tape.

use super::consts::{ACTIVE_H, ACTIVE_W};
use super::video::Frame;
use serde::{Deserialize, Serialize};

/// Pickup tube target chemistry.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TubeKind {
    #[default]
    Off,
    /// Lead-oxide (PbO) broadcast studio tube: crisp dark response, warm red/amber highlight comet-tails.
    Plumbicon,
    /// Selenium-arsenic-tellurium (Se-As-Te) 1980s OB/studio tube: long amber-green highlight trails.
    Saticon,
    /// ZnSe-ZnCdTe low-light / camcorder tube: pronounced green-amber comet-tails and highlight blooming.
    Newvicon,
    /// Antimony trisulfide (Sb2S3) tube: heavy photoconductive lag across all tones and long trails.
    Vidicon,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CameraParams {
    /// Enable the camera pickup tube target simulation ahead of the VHS recorder.
    pub enabled: bool,
    /// Pickup tube chemistry preset (`off`, `plumbicon`, `saticon`, `newvicon`, `vidicon`).
    pub tube: TubeKind,
    /// Per-channel (`[R, G, B]`) target charge retention factor per frame (`0.0 .. 0.98`).
    pub decay_rgb: [f32; 3],
    /// Maximum target charge discharged by the electron beam in a single scan (1.0 = nominal white).
    /// Charge above this ceiling cannot be neutralised in one pass and forms a comet-tail.
    pub beam_ceiling: f32,
    /// Low-light beam-landing acceptance non-linearity (`0.0 .. 1.0`). Higher values leave more
    /// residual charge at low target potentials (dispersive dark lag).
    pub beam_gamma: f32,
    /// Scene intensity (`0.0 .. 1.0`) above which highlights represent specular sources that
    /// over-charge the photoconductive target beyond nominal white.
    pub highlight_threshold: f32,
    /// Extra charge multiplier applied to scene intensity above `highlight_threshold`.
    pub highlight_gain: f32,
    /// Lateral charge diffusion across the target sheet when overloaded (`0.0 .. 1.0`).
    pub bloom: f32,
    /// Enable CRT P22 phosphor persistence on the decoded display output.
    pub phosphor_enabled: bool,
    /// Per-channel (`[R, G, B]`) CRT phosphor afterglow retention per frame (`0.0 .. 0.95`).
    pub phosphor_decay_rgb: [f32; 3],
}

impl Default for CameraParams {
    fn default() -> Self {
        Self {
            enabled: false,
            tube: TubeKind::Off,
            decay_rgb: [0.82, 0.78, 0.60],
            beam_ceiling: 1.10,
            beam_gamma: 0.32,
            highlight_threshold: 0.76,
            highlight_gain: 5.5,
            bloom: 0.35,
            phosphor_enabled: false,
            phosphor_decay_rgb: [0.42, 0.38, 0.22],
        }
    }
}

impl CameraParams {
    /// Physical parameter profile for a specific pickup tube chemistry.
    pub fn for_tube(tube: TubeKind) -> Self {
        let d = Self::default();
        match tube {
            TubeKind::Off => d,
            TubeKind::Plumbicon => Self {
                enabled: true,
                tube,
                decay_rgb: [0.80, 0.68, 0.52],
                beam_ceiling: 1.15,
                beam_gamma: 0.18,
                highlight_threshold: 0.80,
                highlight_gain: 4.5,
                bloom: 0.25,
                ..d
            },
            TubeKind::Saticon => Self {
                enabled: true,
                tube,
                decay_rgb: [0.84, 0.78, 0.58],
                beam_ceiling: 1.10,
                beam_gamma: 0.30,
                highlight_threshold: 0.76,
                highlight_gain: 5.5,
                bloom: 0.35,
                ..d
            },
            TubeKind::Newvicon => Self {
                enabled: true,
                tube,
                decay_rgb: [0.80, 0.88, 0.60],
                beam_ceiling: 1.05,
                beam_gamma: 0.42,
                highlight_threshold: 0.74,
                highlight_gain: 6.5,
                bloom: 0.45,
                ..d
            },
            TubeKind::Vidicon => Self {
                enabled: true,
                tube,
                decay_rgb: [0.88, 0.85, 0.78],
                beam_ceiling: 1.00,
                beam_gamma: 0.65,
                highlight_threshold: 0.70,
                highlight_gain: 6.0,
                bloom: 0.50,
                ..d
            },
        }
    }

    /// True when the pickup tube stage ahead of the recorder is active.
    pub fn tube_on(&self) -> bool {
        self.enabled || self.tube != TubeKind::Off
    }

    /// True when CRT phosphor persistence on the display side is active.
    pub fn phosphor_on(&self) -> bool {
        self.phosphor_enabled && self.phosphor_decay_rgb.iter().any(|&d| d > 0.0)
    }
}

/// Stateful photoconductive target of the camera pickup tube (754 x 480 x 3).
#[derive(Clone)]
pub struct CameraTube {
    params: CameraParams,
    target: Vec<[f32; 3]>,
    scratch: Vec<[f32; 3]>,
    initialized: bool,
}

impl CameraTube {
    pub fn new(params: CameraParams) -> Self {
        Self {
            params,
            target: Vec::new(),
            scratch: Vec::new(),
            initialized: false,
        }
    }

    /// Expose the photoconductive target to `frame`, diffuse overloaded highlight charge,
    /// scan the target with the electron beam, and return the resulting camera output frame.
    pub fn process_frame(&mut self, frame: &Frame) -> Frame {
        if !self.params.tube_on() {
            return frame.clone();
        }
        let n = frame.w * frame.h;
        if self.target.len() != n {
            self.target.resize(n, [0.0; 3]);
            self.scratch.resize(n, [0.0; 3]);
            self.initialized = false;
        }

        let p = &self.params;
        let thr = p.highlight_threshold.clamp(0.1, 0.99);
        let h_gain = p.highlight_gain.max(1.0);
        let ceiling = p.beam_ceiling.max(0.2);
        let gamma = p.beam_gamma.clamp(0.0, 1.5);
        let decay = [
            p.decay_rgb[0].clamp(0.0, 0.985),
            p.decay_rgb[1].clamp(0.0, 0.985),
            p.decay_rgb[2].clamp(0.0, 0.985),
        ];

        // 1. Optical exposure: incident light generates photo-carriers on the target.
        //    Specular highlights above `highlight_threshold` inject overloaded photo-charge.
        for i in 0..n {
            let r = frame.rgb[i * 3] as f32 / 255.0;
            let g = frame.rgb[i * 3 + 1] as f32 / 255.0;
            let b = frame.rgb[i * 3 + 2] as f32 / 255.0;
            let luma = 0.299 * r + 0.587 * g + 0.114 * b;
            let over = if luma > thr {
                1.0 + (h_gain - 1.0) * ((luma - thr) / (1.0 - thr)).powf(1.25)
            } else {
                1.0
            };
            let input = [r * over, g * over, b * over];
            if !self.initialized {
                self.target[i] = [r, g, b];
            } else {
                for c in 0..3 {
                    // Fresh photocurrent charges the target; existing trapped charge adds to it.
                    let base_charge = input[c] * (1.0 - 0.55 * decay[c]);
                    self.target[i][c] = (self.target[i][c] + base_charge).max(input[c]);
                }
            }
        }
        self.initialized = true;

        // 2. Lateral charge diffusion across the semiconductor target sheet (blooming on overload).
        let bloom = p.bloom.clamp(0.0, 1.0);
        if bloom > 0.001 && frame.w == ACTIVE_W && frame.h == ACTIVE_H {
            self.scratch.copy_from_slice(&self.target);
            let k_ortho = 0.12 * bloom;
            let k_diag = 0.06 * bloom;
            for y in 1..ACTIVE_H - 1 {
                let row = y * ACTIVE_W;
                for x in 1..ACTIVE_W - 1 {
                    let idx = row + x;
                    let q = self.scratch[idx];
                    // Excess potential above 0.85 diffuses into neighbouring target elements
                    for c in 0..3 {
                        let excess = (q[c] - 0.85).max(0.0);
                        if excess > 0.0 {
                            let d_o = excess * k_ortho;
                            let d_d = excess * k_diag;
                            self.target[idx - 1][c] += d_o;
                            self.target[idx + 1][c] += d_o;
                            self.target[idx - ACTIVE_W][c] += d_o;
                            self.target[idx + ACTIVE_W][c] += d_o;
                            self.target[idx - ACTIVE_W - 1][c] += d_d;
                            self.target[idx - ACTIVE_W + 1][c] += d_d;
                            self.target[idx + ACTIVE_W - 1][c] += d_d;
                            self.target[idx + ACTIVE_W + 1][c] += d_d;
                            self.target[idx][c] -= 2.0 * (d_o + d_d);
                        }
                    }
                }
            }
        }

        // 3. Electron beam scan: discharge up to `beam_ceiling`, with dispersive beam-landing
        //    resistance at low potentials and per-channel photoconductive trap decay.
        let mut out = Frame::new(frame.w, frame.h);
        for i in 0..n {
            for c in 0..3 {
                let v = self.target[i][c].max(0.0);
                // Beam acceptance efficiency rises with target potential: (v / (v + knee))^gamma
                let accept = if v > 1e-5 {
                    (v / (v + 0.12)).powf(gamma * 0.5)
                } else {
                    0.0
                };
                // Signal read out by the scanning beam (clamped by the gun's maximum beam current)
                let discharged = (v * accept).min(ceiling);
                // Soft-knee pre-amp compression near white (1.0) so overloaded highlights stay bright
                let sig = if discharged <= 0.90 {
                    discharged
                } else {
                    0.90 + 0.10 * ((discharged - 0.90) / (ceiling - 0.90).max(0.1)).tanh()
                };
                out.rgb[i * 3 + c] = (sig.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;

                // Residual charge left on the target after the beam passes:
                // - overloaded charge above `ceiling` is partially neutralised by secondary emission
                //   and beam wings, feeding the comet-tail reservoir with per-channel decay `decay[c]`
                // - normal charge decays according to the channel's trap time constant `decay[c]`
                let over_res = (v - ceiling).max(0.0) * 0.22 * decay[c].powi(2);
                let trap_res = v.min(ceiling) * 0.72 * decay[c] * (1.0 - 0.20 * accept);
                self.target[i][c] = (over_res + trap_res).min(ceiling * 2.5);
            }
        }
        out
    }
}

/// Stateful CRT P22 phosphor persistence filter on the display side.
#[derive(Clone)]
pub struct PhosphorState {
    decay_rgb: [f32; 3],
    enabled: bool,
    buf: Vec<[f32; 3]>,
}

impl PhosphorState {
    pub fn new(params: &CameraParams) -> Self {
        Self {
            decay_rgb: params.phosphor_decay_rgb,
            enabled: params.phosphor_on(),
            buf: Vec::new(),
        }
    }

    pub fn apply(&mut self, frame: &mut Frame) {
        if !self.enabled {
            return;
        }
        let n = frame.w * frame.h;
        if self.buf.len() != n {
            self.buf.resize(n, [0.0; 3]);
            for i in 0..n {
                for c in 0..3 {
                    self.buf[i][c] = frame.rgb[i * 3 + c] as f32 / 255.0;
                }
            }
            return;
        }
        let d = [
            self.decay_rgb[0].clamp(0.0, 0.96),
            self.decay_rgb[1].clamp(0.0, 0.96),
            self.decay_rgb[2].clamp(0.0, 0.96),
        ];
        for i in 0..n {
            for c in 0..3 {
                let x = frame.rgb[i * 3 + c] as f32 / 255.0;
                let y = x.max(self.buf[i][c] * d[c] + x * (1.0 - d[c]) * 0.35);
                self.buf[i][c] = y;
                frame.rgb[i * 3 + c] = (y.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spot_frame(cx: usize, cy: usize, radius: usize, val: u8) -> Frame {
        let mut f = Frame::new(ACTIVE_W, ACTIVE_H);
        for y in 0..ACTIVE_H {
            for x in 0..ACTIVE_W {
                let dx = x as isize - cx as isize;
                let dy = y as isize - cy as isize;
                if (dx * dx + dy * dy) <= (radius * radius) as isize {
                    let p = (y * ACTIVE_W + x) * 3;
                    f.rgb[p] = val;
                    f.rgb[p + 1] = val;
                    f.rgb[p + 2] = val;
                }
            }
        }
        f
    }

    #[test]
    fn default_camera_params_are_inactive() {
        let p = CameraParams::default();
        assert!(!p.tube_on());
        assert!(!p.phosphor_on());
    }

    #[test]
    fn moving_highlight_leaves_a_coloured_comet_tail() {
        let p = CameraParams::for_tube(TubeKind::Newvicon);
        assert!(p.tube_on());
        let mut tube = CameraTube::new(p);

        // Frame 0: dark baseline
        let dark = spot_frame(100, 240, 0, 0);
        let _ = tube.process_frame(&dark);

        // Frame 1: bright specular lamp at x = 200, y = 240
        let f1 = tube.process_frame(&spot_frame(200, 240, 8, 255));
        // Frame 2: lamp moves to x = 260, y = 240
        let f2 = tube.process_frame(&spot_frame(260, 240, 8, 255));
        // Frame 3: lamp moves to x = 320, y = 240
        let f3 = tube.process_frame(&spot_frame(320, 240, 8, 255));

        // At the old position (x = 200, y = 240), frames 2 and 3 still have a decaying trail
        let idx_old = (240 * ACTIVE_W + 200) * 3;
        let g1 = f1.rgb[idx_old + 1];
        let g2 = f2.rgb[idx_old + 1];
        let g3 = f3.rgb[idx_old + 1];
        assert!(g1 > 230, "lamp peak should be near white: {g1}");
        assert!(g2 > 80, "frame after lamp leaves should still have a strong comet tail: {g2}");
        assert!(g3 > 30 && g3 < g2, "tail decays over subsequent frames: {g2} -> {g3}");

        // NewVicon has slower green/red decay than blue, so the tail shifts toward green/amber
        let b2 = f2.rgb[idx_old + 2];
        assert!(g2 > b2 + 15, "NewVicon comet-tail shifts green/amber: G={g2} vs B={b2}");
    }
}
