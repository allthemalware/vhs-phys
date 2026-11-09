//! The picture tube in the outside world: what a magnet and a stray mains field do to the electron
//! beam of the TV. Derivations in `docs/INTERFERENCE.md`.
//!
//! Tube coordinates: z along the axis from the electron gun (z = 0) to the screen (z = `depth_m`), x to
//! the right and y up as seen from the front. An electron of momentum `p` in a field `B` is bent by
//! `dv/dl = -(e/p) (v x B)` (v the unit direction of travel); a kick at a point of the path moves the
//! place where the beam lands on the screen by the remaining path length times the change of direction.
//! The beam aimed at the screen point `(x, y)` runs from the gun along z to the deflection point
//! `(0, 0, deflection_z_m)` and then straight to `(x, y, depth_m)`.
//!
//! Frozen contract: names, signatures and the semantics written here.

use crate::interference_params::*;
use crate::video::Frame;

/// Where the central beam lands when a uniform field `(b_x, b_y)` (tesla, across the tube) fills the tube:
/// `[+(e/p) (D^2/2) b_y, -(e/p) (D^2/2) b_x]` metres. `p` from `interference::electron_momentum`.
pub fn uniform_displacement_m(depth_m: f64, anode_kv: f64, b_x_t: f64, b_y_t: f64) -> [f64; 2] {
    todo!("uniform_displacement_m")
}

/// Unit vector of the magnet's axis: pointing from the magnet at the tube axis (radially inwards) when it is 0.05 m or more
/// from the axis, along +z when it is nearer.
pub fn magnet_axis(m: &CrtMagnet) -> [f64; 3] {
    todo!("magnet_axis")
}

/// Field of the picture-tube magnet at `point` (tube coordinates, metres) at time `t_s`, tesla; the magnet sits at
/// `(rho cos a, rho sin a, z)` with `rho = rho_m * approach_scale(approach_s, t_s)`. Zero without a magnet.
pub fn magnet_field_t(p: &InterferenceParams, point: [f64; 3], t_s: f64) -> [f64; 3] {
    todo!("magnet_field_t")
}

/// Uniform stray field across the tube at time `t_s`, tesla: `stray_field_ut * 1e-6 * mains_waveform(t, f, phase)`
/// along `(cos a, sin a)` with `a = stray_angle_deg`, `[bx, by, 0]`.
pub fn stray_field_t(p: &InterferenceParams, t_s: f64) -> [f64; 3] {
    todo!("stray_field_t")
}

/// Where the beam aimed at screen point `(x_m, y_m)` (metres from the centre, +x right, +y up) lands at time `t_s`, relative to
/// the aim, metres: the effect of the magnet's dipole field and of the stray field together, found by integrating
/// the bending along the beam's path (as described at the top of the file).
pub fn displacement_m(p: &InterferenceParams, x_m: f64, y_m: f64, t_s: f64) -> [f64; 2] {
    todo!("displacement_m")
}

/// How the colours of a beam are mixed when it lands `shift_in_stripes` stripe widths away from where it was aimed on an
/// aperture grille of repeating red, green, blue stripes: the beam covers two neighbouring stripes with weights
/// `1 - f` and `f` where `f` is the fractional part of the shift, so the matrix is `(1 - f) P^n + f P^(n+1)` with
/// `P` the cyclic permutation red -> green -> blue -> red and `n` the integer part, mod 3. `out = M * in` on (r, g, b);
/// every column sums to 1 (no energy is lost).
pub fn purity_matrix(shift_in_stripes: f64) -> [[f32; 3]; 3] {
    todo!("purity_matrix")
}

/// Time, seconds, at which output row `row` of frame `k` is scanned: `(k * 525 + line) / FH` with the line from `video::row_to_line`.
pub fn line_time_s(k: i64, row: usize) -> f64 {
    todo!("line_time_s")
}

/// Apply the tube's response to a decoded frame: every pixel shows what the beam aimed at it would show where it really
/// lands (a displacement field `displacement_m` per row time, converted from metres to pixels with the screen size),
/// scaled in brightness by the inverse of the local area change (beams that crowd together are brighter), and the colours mixed
/// by `purity_matrix` of the horizontal shift (averaged over the pixel when the shift changes by more than a stripe within it).
/// Rows of pixels that no beam reaches are black. Does nothing, bit for bit, when `!p.crt_on()`.
pub fn apply(frame: &mut Frame, k: i64, p: &InterferenceParams) {
    todo!("apply")
}
