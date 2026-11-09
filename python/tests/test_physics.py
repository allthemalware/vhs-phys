import numpy as np
import pytest
from vhslab import physics as P


def test_wallace_constant():
    assert P.WALLACE_DB == pytest.approx(54.575, abs=0.01)  # the "54.6"


@pytest.mark.parametrize("g,d,a,delta", [
    (0.001, 0.05, 0.1, 0.3),   # near-zero gap: formula is exact there
    (0.3, 0.05, 0.1, 0.3),     # VHS-like head
    (0.3, 0.30, 0.1, 0.3),     # worn / dirty head
    (0.3, 0.05, 0.2, 0.15),
])
def test_pw50_formula_matches_numeric_karlqvist(g, d, a, delta):
    x = np.linspace(-6, 6, 6001)
    e = P.readback_pulse(x, g, d, a, delta)
    numeric = P.half_width(x, e)
    closed = P.pw50_formula(g, d, a, delta)
    assert numeric == pytest.approx(closed, rel=0.06)


def test_pulse_area_independent_of_spacing():
    x = np.linspace(-200, 200, 40001)
    areas = []
    for d in (0.05, 0.3):
        e = P.readback_pulse(x, 0.3, d, 0.1, 0.3)
        areas.append(np.trapezoid(e, x))
    assert areas[1] == pytest.approx(areas[0], rel=0.05)


def test_azimuth_numbers_in_research_notes():
    # 58 um track, 12 deg azimuth difference
    assert 20 * np.log10(abs(P.azimuth_loss(58, 12, 1.46))) == pytest.approx(-28, abs=4)
    assert 20 * np.log10(abs(P.azimuth_loss(58, 12, 9.3))) == pytest.approx(-14, abs=1.5)
    assert 20 * np.log10(abs(P.azimuth_loss(29, 12, 9.3))) == pytest.approx(-7.6, abs=1.5)
