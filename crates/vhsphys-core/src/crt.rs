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

use super::consts::{ACTIVE_H, ACTIVE_START, ACTIVE_W, BB_PER_LINE, FH, LINES};
use crate::dsp::sample_cubic;
use rayon::prelude::*;
use super::interference::{approach_scale, dipole_field, electron_momentum, mains_waveform, E_CHARGE};
use crate::interference_params::*;
use super::video::{decode_ideal, row_to_line, ycc_to_rgb, Baseband, Frame};

/// A magnet nearer the axis than this sits on the neck of the tube and is taken to point along it; further out it points at the axis.
const ON_AXIS_RHO_M: f64 = 0.05;

/// Simpson intervals per segment of the beam's path. The field of a magnet a few centimetres from the beam changes
/// over millimetres, and the neck is 0.27 m long: 256 intervals keep the integral within a fraction of a percent.
const PATH_INTERVALS: usize = 256;

/// Nodes of the coarse grid over the screen on which the displacement is integrated once per frame; between the nodes
/// it is interpolated bilinearly. 48 x 36 is 5.6 mm per cell on a 0.27 m screen, well inside the scale on which the
/// dipole field of a magnet outside the tube varies.
const GRID_W: usize = 48;
const GRID_H: usize = 36;

/// Fixed-point steps that invert `s + d(s) = q` for the source point `s` of an output point `q`.
const INVERSION_STEPS: usize = 6;

/// The inversion stops once a step moves the aim point by less than this many pixels along both axes: far from a magnet the
/// field is nearly uniform and the first step already lands there, so the remaining steps would only reshuffle the last
/// bits of the aim point.
const INVERSION_TOLERANCE_PX: f32 = 1.0e-3;

/// Damping factor on later fixed-point inversion steps so steep dipole gradients near a close magnet
/// converge smoothly instead of oscillating ("crinkling").
const INVERSION_DAMPING: f32 = 0.65;

/// Bounds of the brightness gain `1 / |det J|`: beams that crowd (det < 1) brighten, beams that spread darken.
const GAIN_MIN: f32 = 0.25;
const GAIN_MAX: f32 = 4.0;

/// Sub-sample positions across one pixel (in pixel widths from its centre) over which the stripe mixing is averaged.
const PURITY_SUBSAMPLES: [f32; 4] = [-0.375, -0.125, 0.125, 0.375];

/// Below this change of the shift across one pixel, in stripes, the four sub-samples would all give the same weights.
const PURITY_FLAT_STRIPES: f32 = 1.0e-3;

/// Displacements beyond this many screen widths are clamped: they only arise with a magnet inside the beam's path, where
/// the point-dipole field is singular, and the interpolation must stay finite.
const MAX_SHIFT_SCREENS: f64 = 4.0;

/// Charge over momentum of a beam electron, per tesla metre: the bending `dv/dl = -(e/p) (v x B)`.
fn e_over_p(anode_kv: f64) -> f64 {
    E_CHARGE / electron_momentum(anode_kv)
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn add3(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn scale3(a: [f64; 3], s: f64) -> [f64; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}

/// Weight of sample `i` of `n` intervals in composite Simpson's rule (times `h / 3`).
fn simpson_weight(i: usize, n: usize) -> f64 {
    if i == 0 || i == n {
        1.0
    } else if i % 2 == 1 {
        4.0
    } else {
        2.0
    }
}

/// Where the central beam lands when a uniform field `(b_x, b_y)` (tesla, across the tube) fills the tube:
/// `[+(e/p) (D^2/2) b_y, -(e/p) (D^2/2) b_x]` metres. `p` from `interference::electron_momentum`.
pub fn uniform_displacement_m(depth_m: f64, anode_kv: f64, b_x_t: f64, b_y_t: f64) -> [f64; 2] {
    // The kick per metre is -(e/p) (z x B) = (e/p) (b_y, -b_x); its lever arm (D - z) integrates to D^2 / 2.
    let lever = e_over_p(anode_kv) * depth_m * depth_m / 2.0;
    [lever * b_y_t, -lever * b_x_t]
}

/// Unit vector of the magnet's axis: pointing from the magnet at the tube axis (radially inwards) when it is 0.05 m or more
/// from the axis, along +z when it is nearer.
pub fn magnet_axis(m: &CrtMagnet) -> [f64; 3] {
    if m.rho_m < ON_AXIS_RHO_M {
        return [0.0, 0.0, 1.0];
    }
    let (sin, cos) = m.angle_deg.to_radians().sin_cos();
    [-cos, -sin, 0.0]
}

/// Field of the picture-tube magnet at `point` (tube coordinates, metres) at time `t_s`, tesla; the magnet sits at
/// `(rho cos a, rho sin a, z)` with `rho = rho_m * approach_scale(approach_s, t_s)`. Zero without a magnet.
pub fn magnet_field_t(p: &InterferenceParams, point: [f64; 3], t_s: f64) -> [f64; 3] {
    if !p.crt_magnet_on() {
        return [0.0; 3];
    }
    PlacedMagnet::new(p, t_s).field(point)
}

/// Uniform stray field across the tube at time `t_s`, tesla: `stray_field_ut * 1e-6 * mains_waveform(t, f, phase)`
/// along `(cos a, sin a)` with `a = stray_angle_deg`, `[bx, by, 0]`.
pub fn stray_field_t(p: &InterferenceParams, t_s: f64) -> [f64; 3] {
    if !p.stray_on() {
        return [0.0; 3];
    }
    let m = &p.mains;
    let b = m.stray_field_ut * 1.0e-6 * mains_waveform(t_s, m.freq_hz, m.phase);
    let (sin, cos) = m.stray_angle_deg.to_radians().sin_cos();
    [b * cos, b * sin, 0.0]
}

/// Where the beam aimed at screen point `(x_m, y_m)` (metres from the centre, +x right, +y up) lands at time `t_s`, relative to
/// the aim, metres: the effect of the magnet's dipole field and of the stray field together, found by integrating
/// the bending along the beam's path (as described at the top of the file).
pub fn displacement_m(p: &InterferenceParams, x_m: f64, y_m: f64, t_s: f64) -> [f64; 2] {
    if !p.crt_on() {
        return [0.0; 2];
    }
    let magnet = PlacedMagnet::new(p, t_s);
    let stray = stray_field_t(p, t_s);
    let field = |point: [f64; 3]| add3(magnet.field(point), stray);
    let tube = Tube::new(&p.crt);
    let neck = tube.neck_bending(&field);
    tube.landing_shift(x_m, y_m, &neck, &field)
}

/// How the colours of a beam are mixed when it lands `shift_in_stripes` stripe widths away from where it was aimed on an
/// aperture grille of repeating red, green, blue stripes: the beam covers two neighbouring stripes with weights
/// `1 - f` and `f` where `f` is the fractional part of the shift, so the matrix is `(1 - f) P^n + f P^(n+1)` with
/// `P` the cyclic permutation red -> green -> blue -> red and `n` the integer part, mod 3. `out = M * in` on (r, g, b);
/// every column sums to 1 (no energy is lost).
pub fn purity_matrix(shift_in_stripes: f64) -> [[f32; 3]; 3] {
    permutation_matrix(permutation_weights(shift_in_stripes as f32))
}

/// Time, seconds, at which output row `row` of frame `k` is scanned: `(k * 525 + line) / FH` with the line from `video::row_to_line`.
pub fn line_time_s(k: i64, row: usize) -> f64 {
    let (line, _) = row_to_line(row);
    (k as f64 * LINES as f64 + line as f64) / FH
}

/// Scan continuous baseband video `(y, cu, cv)` directly through the picture tube's electron beam
/// and phosphor aperture grille, writing the final `u8` video `Frame` once at the very end.
/// When `!p.crt_on()`, this is identical to `decode_ideal(bb)`.
pub fn scan_to_frame(bb: &Baseband, k: i64, p: &InterferenceParams) -> Frame {
    if !p.crt_on() {
        return decode_ideal(bb);
    }
    let mut frame = Frame::new(ACTIVE_W, ACTIVE_H);
    let screen = Screen::new(ACTIVE_W, ACTIVE_H, &p.crt);
    let grids = DisplacementGrids::new(p, k, &screen);
    let limit_px = MAX_SHIFT_SCREENS * ACTIVE_W as f64;
    // Every row is scanned at its own time and needs nothing from its neighbours, so they can be rendered side by side.
    frame.rgb.par_chunks_mut(ACTIVE_W * 3).enumerate().for_each_init(
        || vec![[0.0f32; 2]; GRID_W * GRID_H],
        |combined, (row, out)| {
            let stray = stray_field_t(p, line_time_s(k, row));
            grids.combine(stray, limit_px, combined);
            let field = GridField::new(combined, &screen);
            render_row_baseband(out, bb, &screen, &field, row);
        },
    );
    frame
}

// ------------------------------------------------------------------------- the beam's path

/// The magnet at its position at one instant: its moment, axis and place in tube coordinates.
struct PlacedMagnet {
    moment_am2: f64,
    axis: [f64; 3],
    position: [f64; 3],
}

impl PlacedMagnet {
    fn new(p: &InterferenceParams, t_s: f64) -> Self {
        let m = &p.crt.magnet;
        let rho = m.rho_m * approach_scale(p.approach_s, t_s);
        let (sin, cos) = m.angle_deg.to_radians().sin_cos();
        Self { moment_am2: m.kind.moment_am2(), axis: magnet_axis(m), position: [rho * cos, rho * sin, m.z_m] }
    }

    /// Dipole field at `point`, tesla; zero for an absent magnet (its moment is zero).
    fn field(&self, point: [f64; 3]) -> [f64; 3] {
        let r = [point[0] - self.position[0], point[1] - self.position[1], point[2] - self.position[2]];
        dipole_field(self.moment_am2, self.axis, r)
    }
}

/// The bending picked up along the neck (gun to deflection point), which every beam shares: the integral of the kick
/// `-(e/p) (z x B)` weighted by the distance left to the deflection point, and unweighted. A beam that still has `l` metres
/// to fly after the deflection point lands `weighted + l * plain` away from its aim.
struct NeckBending {
    weighted: [f64; 3],
    plain: [f64; 3],
}

/// Geometry of the tube and the beam's momentum.
struct Tube {
    depth_m: f64,
    deflection_z_m: f64,
    e_over_p: f64,
}

impl Tube {
    fn new(c: &CrtParams) -> Self {
        Self { depth_m: c.depth_m, deflection_z_m: c.deflection_z_m, e_over_p: e_over_p(c.anode_kv) }
    }

    /// Change of direction per metre of path at `point` for a beam travelling along the unit vector `v`.
    fn kick(&self, v: [f64; 3], b: [f64; 3]) -> [f64; 3] {
        scale3(cross(v, b), -self.e_over_p)
    }

    /// Simpson integral along the neck, computed once per field configuration because the neck is common to all beams.
    fn neck_bending(&self, field: &impl Fn([f64; 3]) -> [f64; 3]) -> NeckBending {
        let n = PATH_INTERVALS;
        let len = self.deflection_z_m;
        let along_z = [0.0, 0.0, 1.0];
        let (mut weighted, mut plain) = ([0.0; 3], [0.0; 3]);
        for i in 0..=n {
            let z = len * i as f64 / n as f64;
            let kick = scale3(self.kick(along_z, field([0.0, 0.0, z])), simpson_weight(i, n));
            weighted = add3(weighted, scale3(kick, len - z));
            plain = add3(plain, kick);
        }
        let h = len / n as f64 / 3.0;
        NeckBending { weighted: scale3(weighted, h), plain: scale3(plain, h) }
    }

    /// Where the beam aimed at `(x_m, y_m)` lands relative to its aim, metres: the neck's bending carried over the rest of
    /// the flight plus the Simpson integral of the kick along the straight run from the deflection point to the screen,
    /// each kick weighted by the path still to fly.
    fn landing_shift(&self, x_m: f64, y_m: f64, neck: &NeckBending, field: &impl Fn([f64; 3]) -> [f64; 3]) -> [f64; 2] {
        let run = [x_m, y_m, self.depth_m - self.deflection_z_m];
        let len = (run[0] * run[0] + run[1] * run[1] + run[2] * run[2]).sqrt();
        let mut total = add3(neck.weighted, scale3(neck.plain, len));
        if len > 0.0 {
            let v = scale3(run, 1.0 / len);
            let n = PATH_INTERVALS;
            let mut acc = [0.0; 3];
            for i in 0..=n {
                let l = len * i as f64 / n as f64;
                let point = [v[0] * l, v[1] * l, self.deflection_z_m + v[2] * l];
                let kick = self.kick(v, field(point));
                acc = add3(acc, scale3(kick, simpson_weight(i, n) * (len - l)));
            }
            total = add3(total, scale3(acc, len / n as f64 / 3.0));
        }
        // Small angles: the part of the deviation across the beam that lies in the screen plane is what moves the spot.
        [total[0], total[1]]
    }
}

// ------------------------------------------------------------------------- the stripes

/// Weights of the three cyclic permutations `P^0, P^1, P^2` for a beam landing `shift` stripes off its aim: the beam
/// straddles stripes `n` and `n + 1` with weights `1 - f` and `f`.
fn permutation_weights(shift_in_stripes: f32) -> [f32; 3] {
    let whole = shift_in_stripes.floor();
    let f = shift_in_stripes - whole;
    // `whole` is an integer-valued float; its residue mod 3 picks the permutation (negative shifts wrap the other way).
    let n = ((whole as i32 % 3) + 3) as usize % 3;
    let mut w = [0.0f32; 3];
    w[n] += 1.0 - f;
    w[(n + 1) % 3] += f;
    w
}

/// `sum_n w[n] P^n` with `P` the cyclic shift red -> green -> blue -> red: `(P^n)[i][j] = 1` iff `i = j + n (mod 3)`.
fn permutation_matrix(w: [f32; 3]) -> [[f32; 3]; 3] {
    let mut m = [[0.0f32; 3]; 3];
    for (n, &weight) in w.iter().enumerate() {
        for j in 0..3 {
            m[(j + n) % 3][j] += weight;
        }
    }
    m
}

/// `M * rgb` for the matrix of the permutation weights `w`, without building the matrix.
fn mix_colours(w: [f32; 3], rgb: [f32; 3]) -> [f32; 3] {
    [
        w[0] * rgb[0] + w[1] * rgb[2] + w[2] * rgb[1],
        w[0] * rgb[1] + w[1] * rgb[0] + w[2] * rgb[2],
        w[0] * rgb[2] + w[1] * rgb[1] + w[2] * rgb[0],
    ]
}

/// Mean of the stripe-permutation weights over the width of a pixel whose shift is `shift` stripes at its centre and changes
/// by `slope` stripes per pixel: when that change is more than a stripe the beam's colour is the even mix rather than one of
/// the permutations.
fn mean_permutation_weights(shift: f32, slope: f32) -> [f32; 3] {
    if slope.abs() < PURITY_FLAT_STRIPES {
        return permutation_weights(shift);
    }
    let mut mean = [0.0f32; 3];
    for offset in PURITY_SUBSAMPLES {
        let w = permutation_weights(shift + slope * offset);
        for c in 0..3 {
            mean[c] += w[c] / PURITY_SUBSAMPLES.len() as f32;
        }
    }
    mean
}

// ------------------------------------------------------------------------- the screen

/// The screen as seen by the frame: the conversion between pixel positions and tube coordinates, and the stripe pitch.
struct Screen {
    w: usize,
    h: usize,
    /// Metres per pixel, horizontally and vertically.
    m_per_px: [f64; 2],
    /// Pixel widths per stripe width.
    px_per_stripe: f64,
}

impl Screen {
    fn new(w: usize, h: usize, c: &CrtParams) -> Self {
        let m_per_px = [c.screen_w_m / w as f64, c.screen_h_m / h as f64];
        Self { w, h, m_per_px, px_per_stripe: c.stripe_mm * 1.0e-3 / m_per_px[0] }
    }

    /// Tube coordinates of a point given in pixel units (column, row; the centre of pixel `(c, r)` is `(c, r)`): x to the
    /// right, y up, measured from the middle of the screen.
    fn pixel_to_m(&self, col: f64, row: f64) -> [f64; 2] {
        [(col + 0.5 - self.w as f64 / 2.0) * self.m_per_px[0], (self.h as f64 / 2.0 - row - 0.5) * self.m_per_px[1]]
    }

    /// A displacement in metres as a displacement in pixel units (rows grow downwards, so y flips).
    fn shift_to_px(&self, d: [f64; 2]) -> [f64; 2] {
        [d[0] / self.m_per_px[0], -d[1] / self.m_per_px[1]]
    }

    /// Size of a cell of the coarse grid in pixels; the grid's corner nodes sit on the corner pixel centres.
    fn cell_px(&self) -> [f64; 2] {
        [(self.w - 1) as f64 / (GRID_W - 1) as f64, (self.h - 1) as f64 / (GRID_H - 1) as f64]
    }

    /// Pixel point of a grid node.
    fn node_pixel(&self, i: usize, j: usize) -> [f64; 2] {
        let cell = self.cell_px();
        [i as f64 * cell[0], j as f64 * cell[1]]
    }
}

/// The displacement field of one frame on the coarse grid, in pixel units: the static part from the magnet at the time
/// of the middle row, and the displacement per tesla of a unit stray field along x and along y. The stray field is uniform
/// and the bending linear in it, so a row's field is `magnet + bx * unit_x + by * unit_y`.
struct DisplacementGrids {
    magnet: Vec<[f64; 2]>,
    unit_x: Vec<[f64; 2]>,
    unit_y: Vec<[f64; 2]>,
}

impl DisplacementGrids {
    fn new(p: &InterferenceParams, k: i64, screen: &Screen) -> Self {
        let tube = Tube::new(&p.crt);
        let magnet = PlacedMagnet::new(p, line_time_s(k, screen.h / 2));
        let magnet_field = |point: [f64; 3]| magnet.field(point);
        let unit_x_field = |_: [f64; 3]| [1.0, 0.0, 0.0];
        let unit_y_field = |_: [f64; 3]| [0.0, 1.0, 0.0];
        Self {
            magnet: if p.crt_magnet_on() { grid_of(&tube, screen, &magnet_field) } else { vec![[0.0; 2]; GRID_W * GRID_H] },
            unit_x: if p.stray_on() { grid_of(&tube, screen, &unit_x_field) } else { vec![[0.0; 2]; GRID_W * GRID_H] },
            unit_y: if p.stray_on() { grid_of(&tube, screen, &unit_y_field) } else { vec![[0.0; 2]; GRID_W * GRID_H] },
        }
    }

    /// The grid for one row's stray field `(bx, by)`, tesla. The sum is clamped, not the parts: the unit grids are
    /// displacements per tesla and so are large numbers that only become pixels once multiplied by microtesla.
    fn combine(&self, stray: [f64; 3], limit_px: f64, out: &mut [[f32; 2]]) {
        for (n, o) in out.iter_mut().enumerate() {
            let (m, ux, uy) = (self.magnet[n], self.unit_x[n], self.unit_y[n]);
            for ((o, m), (ux, uy)) in o.iter_mut().zip(m).zip(ux.into_iter().zip(uy)) {
                let d = m + stray[0] * ux + stray[1] * uy;
                *o = d.clamp(-limit_px, limit_px) as f32;
            }
        }
    }
}

/// The displacement (pixel units) at every grid node for one field configuration.
fn grid_of(tube: &Tube, screen: &Screen, field: &impl Fn([f64; 3]) -> [f64; 3]) -> Vec<[f64; 2]> {
    let neck = tube.neck_bending(field);
    let mut grid = Vec::with_capacity(GRID_W * GRID_H);
    for j in 0..GRID_H {
        for i in 0..GRID_W {
            let [col, row] = screen.node_pixel(i, j);
            let [x, y] = screen.pixel_to_m(col, row);
            grid.push(screen.shift_to_px(tube.landing_shift(x, y, &neck, field)));
        }
    }
    grid
}

/// The displacement at one source point, pixel units, and its Jacobian `d(displacement) / d(source)` (per pixel).
struct LocalShift {
    d: [f32; 2],
    /// `[[ddx/dx, ddx/dy], [ddy/dx, ddy/dy]]`.
    jac: [[f32; 2]; 2],
}

impl LocalShift {
    /// `1 / |det(I + J)|`, the brightness of beams that crowd together or spread apart, bounded.
    fn gain(&self) -> f32 {
        let det = (1.0 + self.jac[0][0]) * (1.0 + self.jac[1][1]) - self.jac[0][1] * self.jac[1][0];
        if det.abs() < 1.0 / GAIN_MAX {
            GAIN_MAX
        } else {
            (1.0 / det.abs()).clamp(GAIN_MIN, GAIN_MAX)
        }
    }
}

/// The combined displacement grid of one row, sampled bilinearly anywhere on the screen (clamped to the grid outside it).
struct GridField<'a> {
    nodes: &'a [[f32; 2]],
    /// Grid cells per pixel.
    cells_per_px: [f32; 2],
}

impl<'a> GridField<'a> {
    fn new(nodes: &'a [[f32; 2]], screen: &Screen) -> Self {
        let cell = screen.cell_px();
        Self { nodes, cells_per_px: [1.0 / cell[0] as f32, 1.0 / cell[1] as f32] }
    }

    #[inline]
    fn node(&self, i: isize, j: isize) -> [f32; 2] {
        let ic = i.clamp(0, (GRID_W - 1) as isize) as usize;
        let jc = j.clamp(0, (GRID_H - 1) as isize) as usize;
        self.nodes[jc * GRID_W + ic]
    }

    /// The grid cell holding grid-space point `(col, row)` pixels (clamped to the grid): the indices of its first node and
    /// the fractions across it.
    #[inline]
    fn locate(&self, col: f32, row: f32) -> (isize, isize, f32, f32) {
        let gx = (col * self.cells_per_px[0]).clamp(0.0, (GRID_W - 1) as f32);
        let gy = (row * self.cells_per_px[1]).clamp(0.0, (GRID_H - 1) as f32);
        let i = (gx.floor() as isize).min((GRID_W - 2) as isize);
        let j = (gy.floor() as isize).min((GRID_H - 2) as isize);
        (i, j, gx - i as f32, gy - j as f32)
    }

    /// Catmull-Rom weights of the four nodes around a point `t` of the way across the middle cell.
    #[inline]
    fn weights(t: f32) -> [f32; 4] {
        [
            t * (t * (-0.5 * t + 1.0) - 0.5),
            t * t * (1.5 * t - 2.5) + 1.0,
            t * (t * (-1.5 * t + 2.0) + 0.5),
            t * t * (0.5 * t - 0.5),
        ]
    }

    /// Derivatives of `weights` with respect to `t`.
    #[inline]
    fn weight_slopes(t: f32) -> [f32; 4] {
        [
            -1.5 * t * t + 2.0 * t - 0.5,
            4.5 * t * t - 5.0 * t,
            -4.5 * t * t + 4.0 * t + 0.5,
            1.5 * t * t - t,
        ]
    }

    /// C1-continuous Catmull-Rom bicubic interpolation of the displacement `d` and its exact
    /// analytic Jacobian `jac` so cell boundaries never create step discontinuities in
    /// beam-crowding brightness gain or phosphor stripe slope.
    fn sample(&self, col: f32, row: f32) -> LocalShift {
        let (i, j, tx, ty) = self.locate(col, row);
        let wx = Self::weights(tx);
        let dwx = Self::weight_slopes(tx);
        let wy = Self::weights(ty);
        let dwy = Self::weight_slopes(ty);

        let mut d = [0.0f32; 2];
        let mut jac = [[0.0f32; 2]; 2];
        for m in 0..4 {
            let mut row_val = [0.0f32; 2];
            let mut row_dx = [0.0f32; 2];
            for n in 0..4 {
                let p = self.node(i + n as isize - 1, j + m as isize - 1);
                for c in 0..2 {
                    row_val[c] += wx[n] * p[c];
                    row_dx[c] += dwx[n] * p[c];
                }
            }
            for c in 0..2 {
                d[c] += wy[m] * row_val[c];
                jac[c][0] += wy[m] * row_dx[c] * self.cells_per_px[0];
                jac[c][1] += dwy[m] * row_val[c] * self.cells_per_px[1];
            }
        }
        LocalShift { d, jac }
    }

    /// Only the displacement of `sample`, with the same arithmetic in the same order so that the value is bit for bit
    /// the same: the fixed-point inversion needs nothing else until its last point, and the Jacobian is most of the work.
    fn sample_displacement(&self, col: f32, row: f32) -> [f32; 2] {
        let (i, j, tx, ty) = self.locate(col, row);
        let wx = Self::weights(tx);
        let wy = Self::weights(ty);

        let mut d = [0.0f32; 2];
        for m in 0..4 {
            let mut row_val = [0.0f32; 2];
            for n in 0..4 {
                let p = self.node(i + n as isize - 1, j + m as isize - 1);
                for c in 0..2 {
                    row_val[c] += wx[n] * p[c];
                }
            }
            for c in 0..2 {
                d[c] += wy[m] * row_val[c];
            }
        }
        d
    }
}

/// Invert `s + d(s) = q` for the aim point `s` of a beam landing at `q`, using damped fixed-point
/// iteration so steep magnetic field gradients converge without oscillating. Only the displacement is
/// needed on the way; the full sample with its Jacobian is taken once, at the aim point found.
#[inline]
fn invert_beam_aim(field: &GridField, q: [f32; 2]) -> ([f32; 2], LocalShift) {
    let d = field.sample_displacement(q[0], q[1]);
    let mut s = [q[0] - d[0], q[1] - d[1]];
    for step in 1..INVERSION_STEPS {
        let d = field.sample_displacement(s[0], s[1]);
        let target = [q[0] - d[0], q[1] - d[1]];
        let alpha = if step == 1 { 1.0 } else { INVERSION_DAMPING };
        let moved = [alpha * (target[0] - s[0]), alpha * (target[1] - s[1])];
        s[0] += moved[0];
        s[1] += moved[1];
        if moved[0].abs() < INVERSION_TOLERANCE_PX && moved[1].abs() < INVERSION_TOLERANCE_PX {
            break;
        }
    }
    (s, field.sample(s[0], s[1]))
}

/// Fill output row `row` directly from the continuous `Baseband` scanlines `(y, cu, cv)` at
/// `14.32 MHz` (`BB_PER_LINE` samples per line), writing the quantised `u8` video pixel only once
/// at the very end.
fn render_row_baseband(out: &mut [u8], bb: &Baseband, screen: &Screen, field: &GridField, row: usize) {
    let stripes_per_px = 1.0 / screen.px_per_stripe as f32;
    for (col, px) in out.chunks_exact_mut(3).enumerate() {
        let q = [col as f32, row as f32];
        let (s, local) = invert_beam_aim(field, q);
        let rgb = sample_baseband_gun(bb, screen, s);
        let weights = mean_permutation_weights(local.d[0] * stripes_per_px, local.jac[0][0] * stripes_per_px);
        let mixed = mix_colours(weights, rgb);
        let gain = local.gain();
        for c in 0..3 {
            px[c] = (mixed[c] * gain).round().clamp(0.0, 255.0) as u8;
        }
    }
}

/// Continuous sub-sample readout of the electron gun drives `(R, G, B)` (on `0..255` scale)
/// directly from the `Baseband` scanlines at continuous position `s = (col, row)`.
/// Horizontal sampling uses `dsp::sample_cubic` along the continuous baseband line; vertical
/// interpolation blends between adjacent scanlines. Outside the active window there is no beam (black).
fn sample_baseband_gun(bb: &Baseband, screen: &Screen, s: [f32; 2]) -> [f32; 3] {
    let (w, h) = (screen.w as f32, screen.h as f32);
    if s[0] <= -1.0 || s[1] <= -1.0 || s[0] >= w || s[1] >= h {
        return [0.0; 3];
    }
    let y0f = s[1].floor();
    let fy = s[1] - y0f;
    let y0 = y0f as isize;

    let sample_line = |r: isize, x: f32| -> [f32; 3] {
        if r < 0 || r >= screen.h as isize || x < 0.0 || x >= w {
            return [0.0; 3];
        }
        let (line, _) = row_to_line(r as usize);
        let pos = (line * BB_PER_LINE + ACTIVE_START) as f64 + (x as f64).clamp(0.0, (screen.w - 1) as f64);
        let y_ire = sample_cubic(&bb.y, pos);
        let cu_ire = sample_cubic(&bb.cu, pos);
        let cv_ire = sample_cubic(&bb.cv, pos);
        let rgb = ycc_to_rgb(y_ire, cu_ire, cv_ire);
        [rgb[0] * 255.0, rgb[1] * 255.0, rgb[2] * 255.0]
    };

    // Account for horizontal border fade when -1.0 < s[0] < 0.0 or w - 1.0 < s[0] < w
    let x_clamped = s[0].clamp(0.0, w - 1.0);
    let x_weight = if s[0] < 0.0 {
        1.0 + s[0]
    } else if s[0] > w - 1.0 {
        w - s[0]
    } else {
        1.0
    };

    let top = sample_line(y0, x_clamped);
    let bottom = sample_line(y0 + 1, x_clamped);
    [
        (top[0] + fy * (bottom[0] - top[0])) * x_weight,
        (top[1] + fy * (bottom[1] - top[1])) * x_weight,
        (top[2] + fy * (bottom[2] - top[2])) * x_weight,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Peak displacement of the test grid, pixels, and the wavelength of its ripple across the picture: gentle enough that the
    /// inversion converges, steep enough that a point does not land where it was aimed.
    const RIPPLE_PX: f32 = 6.0;
    const RIPPLE_WAVELENGTH_PX: f32 = 300.0;

    /// Probe points (columns and rows) spread over the picture and a little beyond it, where the grid is clamped.
    const PROBE_COLS: usize = 37;
    const PROBE_ROWS: usize = 29;
    const PROBE_MARGIN_PX: f32 = 20.0;

    fn ripple_grid(screen: &Screen) -> Vec<[f32; 2]> {
        let k = std::f32::consts::TAU / RIPPLE_WAVELENGTH_PX;
        let mut nodes = Vec::with_capacity(GRID_W * GRID_H);
        for j in 0..GRID_H {
            for i in 0..GRID_W {
                let [col, row] = screen.node_pixel(i, j);
                nodes.push([RIPPLE_PX * (k * (col as f32 + 0.7 * row as f32)).sin(), RIPPLE_PX * (k * (row as f32 - 0.4 * col as f32)).cos()]);
            }
        }
        nodes
    }

    fn probes(screen: &Screen) -> Vec<[f32; 2]> {
        let (w, h) = (screen.w as f32, screen.h as f32);
        let mut points = Vec::new();
        for r in 0..PROBE_ROWS {
            for c in 0..PROBE_COLS {
                let col = -PROBE_MARGIN_PX + (w + 2.0 * PROBE_MARGIN_PX) * c as f32 / (PROBE_COLS - 1) as f32;
                let row = -PROBE_MARGIN_PX + (h + 2.0 * PROBE_MARGIN_PX) * r as f32 / (PROBE_ROWS - 1) as f32;
                points.push([col, row]);
            }
        }
        points
    }

    #[test]
    fn the_displacement_alone_is_bit_for_bit_the_displacement_of_the_full_sample() {
        let screen = Screen::new(ACTIVE_W, ACTIVE_H, &CrtParams::default());
        let nodes = ripple_grid(&screen);
        let field = GridField::new(&nodes, &screen);
        for [col, row] in probes(&screen) {
            let full = field.sample(col, row).d;
            let alone = field.sample_displacement(col, row);
            assert_eq!([full[0].to_bits(), full[1].to_bits()], [alone[0].to_bits(), alone[1].to_bits()], "at ({col}, {row})");
        }
    }

    #[test]
    fn stopping_when_converged_finds_the_aim_point_of_the_full_iteration() {
        let screen = Screen::new(ACTIVE_W, ACTIVE_H, &CrtParams::default());
        let nodes = ripple_grid(&screen);
        let field = GridField::new(&nodes, &screen);
        for q in probes(&screen) {
            // all the steps, each with the full sample
            let mut local = field.sample(q[0], q[1]);
            let mut s = [q[0] - local.d[0], q[1] - local.d[1]];
            for step in 1..INVERSION_STEPS {
                local = field.sample(s[0], s[1]);
                let alpha = if step == 1 { 1.0 } else { INVERSION_DAMPING };
                s[0] += alpha * (q[0] - local.d[0] - s[0]);
                s[1] += alpha * (q[1] - local.d[1] - s[1]);
            }
            let (found, at) = invert_beam_aim(&field, q);
            assert!((found[0] - s[0]).abs() < INVERSION_TOLERANCE_PX && (found[1] - s[1]).abs() < INVERSION_TOLERANCE_PX, "{q:?}: {found:?} against {s:?}");
            // the local shift is the full sample at the point found
            let again = field.sample(found[0], found[1]);
            assert_eq!(at.d, again.d);
            assert_eq!(at.jac, again.jac);
        }
    }
}
