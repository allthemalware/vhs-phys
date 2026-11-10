//! What a magnet and a stray mains field do to the electron beam of the TV: the Lorentz force
//! integrated along the beam, the mixing of the colours when it lands on the wrong stripe, and
//! the displacement and brightness of a decoded picture.

use vhsphys_core::consts::*;
use vhsphys_core::crt::*;
use vhsphys_core::interference::*;
use vhsphys_core::interference_params::*;
use vhsphys_core::video::{row_to_line, Frame};

fn near(a: f64, b: f64, rel: f64) -> bool {
    (a - b).abs() <= rel * b.abs().max(1e-300)
}

fn magnet(kind: MagnetKind, z: f64, rho: f64, angle: f64) -> InterferenceParams {
    let mut p = InterferenceParams::default();
    p.crt.magnet = CrtMagnet { kind, z_m: z, rho_m: rho, angle_deg: angle };
    p
}

fn stray(ut: f64, angle: f64) -> InterferenceParams {
    let mut p = InterferenceParams::default();
    p.mains.stray_field_ut = ut;
    p.mains.stray_angle_deg = angle;
    p
}

fn e_over_p(kv: f64) -> f64 {
    E_CHARGE / electron_momentum(kv)
}

#[test]
fn a_uniform_field_bends_the_central_beam_by_the_lorentz_force() {
    // e/p = 1853 per tesla metre at 25 kV and the lever of a field that fills the tube is D^2/2 = 0.06125 m^2:
    // 113.5 m per tesla, 5.67 mm for 50 uT
    let d = uniform_displacement_m(0.35, 25.0, 0.0, 50.0e-6);
    assert!(near(d[0], 5.6748e-3, 5e-3), "{d:?}");
    assert!(d[1].abs() < 1e-15);
    // the other component bends the other way: +bx moves the beam down
    let d = uniform_displacement_m(0.35, 25.0, 50.0e-6, 0.0);
    assert!(d[0].abs() < 1e-15 && near(d[1], -5.6748e-3, 5e-3));
    // linear in the field, quadratic in the length of the tube, smaller for faster electrons
    let a = uniform_displacement_m(0.35, 25.0, 0.0, 10.0e-6)[0];
    assert!(near(uniform_displacement_m(0.35, 25.0, 0.0, 30.0e-6)[0], 3.0 * a, 1e-9));
    assert!(near(uniform_displacement_m(0.70, 25.0, 0.0, 10.0e-6)[0], 4.0 * a, 1e-9));
    assert!(uniform_displacement_m(0.35, 35.0, 0.0, 10.0e-6)[0] < a);
    // the earth's field (about 50 uT) moves the raster by several millimetres, as every set shows
    assert!((3.0e-3..9.0e-3).contains(&uniform_displacement_m(0.35, 25.0, 0.0, 50.0e-6)[0].abs()));
}

#[test]
fn the_stray_field_is_the_mains_waveform_along_its_direction() {
    let mut p = stray(40.0, 30.0);
    p.mains.freq_hz = 50.0;
    p.mains.phase = 0.4;
    for t in [0.0, 0.0031, 0.0177] {
        let b = stray_field_t(&p, t);
        let w = 40.0e-6 * mains_waveform(t, 50.0, 0.4);
        assert!(near(b[0], w * 30.0f64.to_radians().cos(), 1e-9) || b[0].abs() < 1e-15);
        assert!(near(b[1], w * 30.0f64.to_radians().sin(), 1e-9) || b[1].abs() < 1e-15);
        assert_eq!(b[2], 0.0);
    }
    assert_eq!(stray_field_t(&InterferenceParams::default(), 0.1), [0.0, 0.0, 0.0]);
}

#[test]
fn a_magnet_beside_the_tube_points_at_the_axis_and_one_on_it_points_along_it() {
    let side = magnet_axis(&CrtMagnet { kind: MagnetKind::Neodymium, z_m: 0.2, rho_m: 0.15, angle_deg: 0.0 });
    assert!((side[0] + 1.0).abs() < 1e-12 && side[1].abs() < 1e-12 && side[2].abs() < 1e-12);
    let top = magnet_axis(&CrtMagnet { kind: MagnetKind::Neodymium, z_m: 0.2, rho_m: 0.15, angle_deg: 90.0 });
    assert!(top[0].abs() < 1e-12 && (top[1] + 1.0).abs() < 1e-12);
    let on_axis = magnet_axis(&CrtMagnet { kind: MagnetKind::Neodymium, z_m: 0.2, rho_m: 0.01, angle_deg: 40.0 });
    assert_eq!(on_axis, [0.0, 0.0, 1.0]);
}

#[test]
fn the_field_of_a_placed_magnet_is_its_dipole_field() {
    let p = magnet(MagnetKind::Neodymium, 0.10, 0.20, 90.0);
    let point = [0.0, 0.0, 0.30];
    let b = magnet_field_t(&p, point, 0.0);
    // the magnet is at (0, 0.2, 0.1) with its axis along -y; the point is at (0, -0.2, 0.2) from it
    let want = dipole_field(1.6, [0.0, -1.0, 0.0], [0.0, -0.2, 0.2]);
    for k in 0..3 {
        assert!((b[k] - want[k]).abs() < 1e-12 + 1e-9 * want[k].abs(), "{k}: {} vs {}", b[k], want[k]);
    }
    // closing in: at the start the magnet is four times as far from the axis
    let mut q = p.clone();
    q.approach_s = 10.0;
    let far = magnet_field_t(&q, point, 0.0);
    let want = dipole_field(1.6, [0.0, -1.0, 0.0], [0.0, -0.8, 0.2]);
    for k in 0..3 {
        assert!((far[k] - want[k]).abs() < 1e-12 + 1e-9 * want[k].abs());
    }
    assert_eq!(magnet_field_t(&InterferenceParams::default(), point, 0.0), [0.0, 0.0, 0.0]);
}

/// The central beam runs along the axis: its deflection is `e/p` times the integral of the lever arm
/// times the transverse field along the tube. Composite Simpson over `n` intervals.
fn central_beam_integral(p: &InterferenceParams, t: f64) -> [f64; 2] {
    let (d, n) = (p.crt.depth_m, 2000usize);
    let k = e_over_p(p.crt.anode_kv);
    let mut acc = [0.0f64; 2];
    for i in 0..=n {
        let z = d * i as f64 / n as f64;
        let w = if i == 0 || i == n { 1.0 } else if i % 2 == 1 { 4.0 } else { 2.0 };
        let b = magnet_field_t(p, [0.0, 0.0, z], t);
        acc[0] += w * (d - z) * b[1];
        acc[1] -= w * (d - z) * b[0];
    }
    let h = d / n as f64 / 3.0;
    [k * acc[0] * h, k * acc[1] * h]
}

#[test]
fn the_central_beam_of_a_magnet_is_bent_by_the_integral_of_the_field() {
    for (kind, z, rho, ang) in [(MagnetKind::Neodymium, 0.30, 0.12, 0.0), (MagnetKind::Neodymium, 0.05, 0.08, 135.0), (MagnetKind::Speaker, 0.20, 0.45, 270.0), (MagnetKind::Fridge, 0.35, 0.10, 20.0)] {
        let p = magnet(kind, z, rho, ang);
        let want = central_beam_integral(&p, 0.0);
        let got = displacement_m(&p, 0.0, 0.0, 0.0);
        let scale = want[0].abs().max(want[1].abs());
        assert!(scale > 1.0e-6, "{kind:?}: the test magnet does nothing");
        assert!((got[0] - want[0]).abs() < 0.02 * scale && (got[1] - want[1]).abs() < 0.02 * scale, "{kind:?}: {got:?} vs {want:?}");
    }
}

#[test]
fn a_distant_magnet_is_a_uniform_field_that_falls_as_the_inverse_cube() {
    let at = |rho: f64| {
        let p = magnet(MagnetKind::Speaker, 0.175, rho, 0.0);
        let d = displacement_m(&p, 0.0, 0.0, 0.0);
        (d[0] * d[0] + d[1] * d[1]).sqrt()
    };
    let (a, b) = (at(1.0), at(2.0));
    assert!(a > 1.0e-4, "{a}");
    assert!((6.4..=9.6).contains(&(a / b)), "ratio {}", a / b);
    // and it agrees with a uniform field equal to its field at the middle of the tube
    let p = magnet(MagnetKind::Speaker, 0.175, 2.0, 0.0);
    let mid = magnet_field_t(&p, [0.0, 0.0, 0.175], 0.0);
    let uni = uniform_displacement_m(0.35, 25.0, mid[0], mid[1]);
    let got = displacement_m(&p, 0.0, 0.0, 0.0);
    assert!((got[0] - uni[0]).abs() < 0.08 * uni[0].abs().max(uni[1].abs()) && (got[1] - uni[1]).abs() < 0.08 * uni[0].abs().max(uni[1].abs()), "{got:?} vs {uni:?}");
}

#[test]
fn without_a_source_nothing_moves_and_the_stray_field_reverses_every_half_cycle() {
    assert_eq!(displacement_m(&InterferenceParams::default(), 0.05, -0.03, 0.123), [0.0, 0.0]);
    let mut p = stray(80.0, 70.0);
    p.mains.freq_hz = 60.0;
    for (x, y, t) in [(0.0, 0.0, 0.0021), (0.09, 0.06, 0.0043), (-0.12, -0.08, 0.0107)] {
        let a = displacement_m(&p, x, y, t);
        let b = displacement_m(&p, x, y, t + 0.5 / 60.0);
        assert!((a[0] + b[0]).abs() < 1e-9 * (1.0 + a[0].abs()) && (a[1] + b[1]).abs() < 1e-9 * (1.0 + a[1].abs()), "{a:?} {b:?}");
        assert!(a[0].abs() + a[1].abs() > 1.0e-5);
    }
    // the central beam in the stray field is the closed form
    let t = 0.0031;
    let b = stray_field_t(&p, t);
    let want = uniform_displacement_m(0.35, 25.0, b[0], b[1]);
    let got = displacement_m(&p, 0.0, 0.0, t);
    assert!((got[0] - want[0]).abs() < 0.01 * want[0].abs().max(want[1].abs()) && (got[1] - want[1]).abs() < 0.01 * want[0].abs().max(want[1].abs()));
}

#[test]
fn a_beam_that_lands_between_stripes_lights_both_of_them() {
    let id = purity_matrix(0.0);
    let perm = purity_matrix(1.0);
    for i in 0..3 {
        for j in 0..3 {
            assert!((id[i][j] - if i == j { 1.0 } else { 0.0 }).abs() < 1e-6, "{i}{j}");
            // one stripe to the right: red lands on green, green on blue, blue on red
            assert!((perm[i][j] - if i == (j + 1) % 3 { 1.0 } else { 0.0 }).abs() < 1e-6, "{i}{j} {perm:?}");
        }
    }
    let half = purity_matrix(0.5);
    for i in 0..3 {
        for j in 0..3 {
            assert!((half[i][j] - 0.5 * (id[i][j] + perm[i][j])).abs() < 1e-6);
        }
    }
    // three stripes is a whole triad: nothing changes; a shift to the left is the inverse permutation
    let triad = purity_matrix(3.0);
    for i in 0..3 {
        for j in 0..3 {
            assert!((triad[i][j] - id[i][j]).abs() < 1e-6);
            assert!((purity_matrix(-1.0)[i][j] - perm[j][i]).abs() < 1e-6);
        }
    }
    // no light is lost anywhere, and averaged over a whole triad every colour is an even mix
    for s in [-4.3, -0.2, 0.37, 1.9, 2.5, 7.77] {
        let m = purity_matrix(s);
        for j in 0..3 {
            assert!((m[0][j] + m[1][j] + m[2][j] - 1.0).abs() < 1e-5, "s {s}");
        }
    }
    let n = 3000;
    let mut mean = [[0.0f64; 3]; 3];
    for k in 0..n {
        let m = purity_matrix(3.0 * (k as f64 + 0.5) / n as f64);
        for i in 0..3 {
            for j in 0..3 {
                mean[i][j] += m[i][j] as f64 / n as f64;
            }
        }
    }
    for i in 0..3 {
        for j in 0..3 {
            assert!((mean[i][j] - 1.0 / 3.0).abs() < 2e-3);
        }
    }
}

#[test]
fn a_row_is_scanned_at_its_line_time() {
    let (line0, _) = row_to_line(0);
    assert!(near(line_time_s(0, 0), line0 as f64 / FH, 1e-12));
    let (line1, field1) = row_to_line(1);
    assert_eq!(field1, 1);
    assert!(near(line_time_s(0, 1), line1 as f64 / FH, 1e-12));
    assert!(near(line_time_s(7, 100) - line_time_s(0, 100), 7.0 * 525.0 / FH, 1e-9));
    // consecutive rows of one field are consecutive lines of that field
    assert!(near(line_time_s(0, 12) - line_time_s(0, 10), 1.0 / FH, 1e-9));
}

fn bar_frame() -> Frame {
    // a white vertical bar, 40 pixels wide, in the middle of a black picture
    let mut f = Frame::new(ACTIVE_W, ACTIVE_H);
    for y in 0..ACTIVE_H {
        for x in ACTIVE_W / 2 - 20..ACTIVE_W / 2 + 20 {
            for k in 0..3 {
                f.rgb[(y * ACTIVE_W + x) * 3 + k] = 255;
            }
        }
    }
    f
}

/// A black and white checkerboard of 24-pixel squares over the whole picture.
fn checker_frame() -> Frame {
    let mut f = Frame::new(ACTIVE_W, ACTIVE_H);
    for y in 0..ACTIVE_H {
        for x in 0..ACTIVE_W {
            let v = if (x / 24 + y / 24) % 2 == 0 { 255 } else { 0 };
            for k in 0..3 {
                f.rgb[(y * ACTIVE_W + x) * 3 + k] = v;
            }
        }
    }
    f
}

/// Centroid column and total brightness of the bar in a row.
fn row_stats(f: &Frame, y: usize) -> (f64, f64) {
    let (mut s, mut sx) = (0.0, 0.0);
    for x in 0..f.w {
        let v = f.rgb[(y * f.w + x) * 3 + 1] as f64;
        s += v;
        sx += v * x as f64;
    }
    (sx / s.max(1e-9), s)
}

#[test]
fn nothing_acting_on_the_tube_leaves_the_picture_bit_for_bit() {
    let mut f = bar_frame();
    for (i, v) in f.rgb.iter_mut().enumerate() {
        *v = v.wrapping_add((i * 7 % 13) as u8);
    }
    let before = f.rgb.clone();
    apply(&mut f, 5, &InterferenceParams::default());
    assert_eq!(f.rgb, before);
    // a dimmer or a phone does not touch the tube
    let mut p = InterferenceParams::default();
    p.phone.mode = PhoneMode::Gsm;
    p.mains.dimmer_angle_deg = 60.0;
    apply(&mut f, 5, &p);
    assert_eq!(f.rgb, before);
}

#[test]
fn a_stray_mains_field_makes_each_line_of_the_picture_shift_by_the_beams_displacement() {
    // along +y the field moves the beam sideways: the bar wobbles from line to line
    let mut p = stray(100.0, 90.0);
    p.mains.freq_hz = 60.0;
    let k = 3i64;
    let mut f = bar_frame();
    apply(&mut f, k, &p);
    let mut moved = 0;
    for y in (40..440).step_by(13) {
        let t = line_time_s(k, y);
        let yy = (ACTIVE_H as f64 / 2.0 - y as f64 - 0.5) / ACTIVE_H as f64 * p.crt.screen_h_m;
        let d = displacement_m(&p, 0.0, yy, t);
        let want = ACTIVE_W as f64 / 2.0 + d[0] / p.crt.screen_w_m * ACTIVE_W as f64 - 0.5;
        let (centroid, light) = row_stats(&f, y);
        assert!((centroid - want).abs() < 1.2, "row {y}: bar at {centroid:.2}, beam says {want:.2}");
        // a displacement without a change of area keeps the light of the bar
        assert!((light / (40.0 * 255.0) - 1.0).abs() < 0.03, "row {y}: {light}");
        if (centroid - (ACTIVE_W as f64 / 2.0 - 0.5)).abs() > 3.0 {
            moved += 1;
        }
    }
    assert!(moved > 10, "the bar should wobble by tens of pixels at 100 uT");
}

#[test]
fn a_magnet_beside_the_screen_warps_the_picture_near_it_and_only_shifts_the_far_side() {
    let mut p = magnet(MagnetKind::Neodymium, 0.34, 0.16, 0.0);
    p.crt.magnet.angle_deg = 0.0; // on the right-hand side
    let k = 2;
    let mut f = checker_frame();
    let original = f.rgb.clone();
    apply(&mut f, k, &p);
    let diff = |x0: usize, x1: usize| -> f64 {
        let mut s = 0.0;
        for y in 0..ACTIVE_H {
            for x in x0..x1 {
                for c in 0..3 {
                    s += (f.rgb[(y * ACTIVE_W + x) * 3 + c] as f64 - original[(y * ACTIVE_W + x) * 3 + c] as f64).abs();
                }
            }
        }
        s / ((x1 - x0) * ACTIVE_H * 3) as f64
    };
    let near_side = diff(ACTIVE_W - 200, ACTIVE_W);
    let far_side = diff(0, 120);
    // The magnet sits 17 cm from the neck, where a neodymium disc still gives about 46 uT (the earth's field), so the
    // whole raster moves a little (0.8 to 1.3 mm across the picture) and the far side is not untouched: it is shifted.
    // What the magnet next to the screen adds is a warp, and the picture is damaged more on its side.
    assert!(far_side > 0.0 && near_side > 1.25 * far_side, "near {near_side}, far {far_side}");
    // the warp: the displacement changes across the near side (0.95 mm to 3.8 mm) several times more than across the
    // far side, where it is a nearly rigid shift (1.3 mm to 0.8 mm)
    let spread = |xs: [f64; 9]| -> f64 {
        let d: Vec<[f64; 2]> = xs.iter().map(|&x| displacement_m(&p, x, 0.0, 0.0)).collect();
        let mut s = 0.0f64;
        for a in &d {
            for b in &d {
                s = s.max(((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt());
            }
        }
        s
    };
    let line = |from: f64, to: f64| -> [f64; 9] { std::array::from_fn(|i| from + (to - from) * i as f64 / 8.0) };
    let (warp_near, warp_far) = (spread(line(0.05, 0.135)), spread(line(-0.135, -0.05)));
    assert!(warp_far > 0.0 && warp_near > 4.0 * warp_far, "warp near {warp_near}, far {warp_far}");
    // the same magnet on the left puts the damage on the left
    let mut q = p.clone();
    q.crt.magnet.angle_deg = 180.0;
    let mut g = checker_frame();
    apply(&mut g, k, &q);
    assert_ne!(f.rgb, g.rgb);
    let diff_left = {
        let mut s = 0.0;
        for y in 0..ACTIVE_H {
            for x in 0..200 {
                s += (g.rgb[(y * ACTIVE_W + x) * 3 + 1] as f64 - original[(y * ACTIVE_W + x) * 3 + 1] as f64).abs();
            }
        }
        s / (200 * ACTIVE_H) as f64
    };
    assert!(diff_left > 5.0, "{diff_left}");
}

#[test]
fn a_smooth_distortion_keeps_the_light_of_the_picture() {
    // a flat grey picture seen through a far magnet: beams crowd and spread, but the total stays
    let mut p = magnet(MagnetKind::Speaker, 0.2, 1.0, 45.0);
    p.approach_s = 0.0;
    let mut f = Frame::new(ACTIVE_W, ACTIVE_H);
    f.rgb.fill(110);
    apply(&mut f, 1, &p);
    // the middle of the picture (the borders lose or gain what moves in or out of the frame)
    let (mut sum, mut n) = (0.0, 0.0);
    for y in 60..ACTIVE_H - 60 {
        for x in 60..ACTIVE_W - 60 {
            for c in 0..3 {
                sum += f.rgb[(y * ACTIVE_W + x) * 3 + c] as f64;
                n += 1.0;
            }
        }
    }
    assert!((sum / n / 110.0 - 1.0).abs() < 0.03, "{}", sum / n);
    // and a field that bends the beams by more than a stripe changes the colours but still loses no light overall
    let mut strong = magnet(MagnetKind::Neodymium, 0.34, 0.14, 270.0);
    strong.approach_s = 0.0;
    let mut g = Frame::new(ACTIVE_W, ACTIVE_H);
    g.rgb.fill(0);
    for y in 0..ACTIVE_H {
        for x in 0..ACTIVE_W {
            g.rgb[(y * ACTIVE_W + x) * 3] = 200; // pure red
        }
    }
    apply(&mut g, 1, &strong);
    let (mut r, mut gr, mut b) = (0.0, 0.0, 0.0);
    for px in g.rgb.chunks(3) {
        r += px[0] as f64;
        gr += px[1] as f64;
        b += px[2] as f64;
    }
    assert!(gr > 0.02 * r || b > 0.02 * r, "no colour mixing near a strong magnet");
}
