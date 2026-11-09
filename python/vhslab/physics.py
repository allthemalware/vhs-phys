"""Reference magnetic-recording physics, used to validate the Rust core.

Everything here is a direct numerical implementation of textbook relations
(Karlqvist head field, reciprocity readback, Wallace spacing loss, sinc gap and
azimuth losses) with no shortcuts, so the closed forms used in the simulator can
be checked against it.
"""
from __future__ import annotations

import numpy as np
from scipy.signal import fftconvolve

# 20*log10(e) * 2*pi: spacing loss in dB per (spacing / wavelength)
WALLACE_DB = 20.0 * np.log10(np.e) * 2.0 * np.pi  # = 54.575


def karlqvist_hx(x: np.ndarray, y: float, g: float) -> np.ndarray:
    """Longitudinal head field (per unit deep-gap field H0) at (x, y)."""
    return (np.arctan((g / 2 + x) / y) + np.arctan((g / 2 - x) / y)) / np.pi


def readback_pulse(x: np.ndarray, g: float, d: float, a: float, delta: float,
                   n_y: int = 40) -> np.ndarray:
    """Playback pulse of one arctangent transition (numerical reciprocity).

    e(x_t) = integral over medium depth y in [d, d+delta] of
             (transition derivative, a Lorentzian of half-width a)
             convolved with the Karlqvist field, evaluated at transition
             position x_t.  Units: arbitrary, returned unnormalised.
    """
    # Fine grid for the convolution; the Lorentzian tail is long so go wide.
    span = 60.0 * (g + d + a + delta + 1e-3)
    xs = np.linspace(-span, span, 16001)
    dx = xs[1] - xs[0]
    m_prime = (a / np.pi) / (xs**2 + a**2)  # d/dx of (1/pi) atan(x/a)
    out = np.zeros_like(x, dtype=float)
    ys = d + (np.arange(n_y) + 0.5) * delta / n_y
    for y in ys:
        h = karlqvist_hx(xs, y, g)
        # conv(m_prime, h)(x) sampled at the requested x
        full = fftconvolve(m_prime, h, mode="same") * dx
        out += np.interp(x, xs, full) * (delta / n_y)
    return out


def half_width(x: np.ndarray, e: np.ndarray) -> float:
    """Full width at half maximum of a single-peaked pulse sampled on x."""
    e = np.abs(e)
    pk = e.max()
    above = np.where(e >= pk / 2)[0]
    lo, hi = above[0], above[-1]
    # linear interpolation on both flanks
    def cross(i0, i1):
        y0, y1 = e[i0], e[i1]
        return x[i0] + (pk / 2 - y0) * (x[i1] - x[i0]) / (y1 - y0)
    left = cross(lo - 1, lo) if lo > 0 else x[lo]
    right = cross(hi, hi + 1) if hi < len(x) - 1 else x[hi]
    return right - left


def pw50_formula(g: float, d: float, a: float, delta: float) -> float:
    """Closed-form PW50 used by the simulator (Bertram/Williams-Comstock form)."""
    return float(np.sqrt(g**2 + 4.0 * (d + a) * (d + a + delta)))


def spacing_loss_db(d_over_lambda: float) -> float:
    return WALLACE_DB * d_over_lambda


def gap_loss(g_over_lambda: float) -> float:
    return float(np.sinc(g_over_lambda))  # numpy sinc is sin(pi x)/(pi x)


def azimuth_loss(width: float, theta_deg: float, wavelength: float) -> float:
    return float(np.sinc(width * np.tan(np.radians(theta_deg)) / wavelength))
