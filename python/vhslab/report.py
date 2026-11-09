"""Quantitative validation of the full chain, driven through the real CLI.

Builds multiburst test frames, runs `vhsphys`, and measures the end-to-end
response: luma bandwidth, chroma bandwidth, resolution loss versus head-to-tape
spacing (Wallace's law at work), and noise versus carrier-to-noise ratio.

usage: python -m vhslab.report <output.png> [path/to/vhsphys]
"""
from __future__ import annotations

import subprocess
import sys
import tempfile
from pathlib import Path

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt
import numpy as np
from PIL import Image

W, H = 754, 480
FS_PX = 4 * 315e6 / 88  # pixel rate = 4 x fsc (14.318 MHz)
FREQS = np.array([0.3, 0.6, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5, 4.0, 4.5])


def run_cli(cli, frame_png: Path, out_png: Path, *args):
    """Loop a still into a short lossless clip, run vhsphys, keep the last frame."""
    clip = frame_png.with_suffix(".mkv")
    subprocess.run(["ffmpeg", "-v", "error", "-y", "-loop", "1", "-i", str(frame_png), "-t", "0.6",
                    "-r", "30000/1001", "-c:v", "ffv1", str(clip)], check=True)
    subprocess.run([cli, "run", str(clip), str(out_png), "--seed", "3", *args], check=True,
                   stderr=subprocess.DEVNULL)
    return np.asarray(Image.open(out_png).convert("RGB")).astype(float) / 255.0


def luma(rgb):
    return 0.299 * rgb[..., 0] + 0.587 * rgb[..., 1] + 0.114 * rgb[..., 2]


def multiburst_frame(chroma=False):
    """Packets of a sine of rising frequency across the line, 5 us each."""
    x = np.arange(W)
    img = np.full((H, W, 3), 0.5)
    pk = W // len(FREQS)
    for i, f in enumerate(FREQS):
        sl = slice(i * pk, (i + 1) * pk)
        s = np.sin(2 * np.pi * f * 1e6 * x[sl] / FS_PX)
        if not chroma:
            img[:, sl, :] = (0.5 + 0.22 * s)[None, :, None]
        else:  # constant luma, sinusoidal R-Y (red/cyan), amplitude 0.18
            img[:, sl, 0] = (0.5 + 0.18 * s)[None, :]
            img[:, sl, 1] = (0.5 - 0.18 * s * 0.5)[None, :]
            img[:, sl, 2] = (0.5 - 0.18 * s * 0.5)[None, :]
    return img


def packet_amplitudes(sig_rows, ref_rows=None):
    """Amplitude of each packet by windowed least squares on the middle 60%."""
    s = sig_rows[40:440].mean(axis=0)
    pk = W // len(FREQS)
    out = []
    for i, f in enumerate(FREQS):
        a, b = int(i * pk + 0.2 * pk), int((i + 1) * pk - 0.2 * pk)
        x = np.arange(a, b)
        w = np.hanning(len(x))
        M = np.stack([np.sin(2 * np.pi * f * 1e6 * x / FS_PX), np.cos(2 * np.pi * f * 1e6 * x / FS_PX), np.ones_like(x, dtype=float)], 1)
        coef, *_ = np.linalg.lstsq(M * w[:, None], s[a:b] * w, rcond=None)
        out.append(np.hypot(coef[0], coef[1]))
    return np.array(out)


def response(cli, tmp, chroma, *args):
    img = multiburst_frame(chroma)
    p = tmp / f"mb_{int(chroma)}.png"
    Image.fromarray((img * 255).astype(np.uint8)).save(p)
    out = run_cli(cli, p, tmp / "o.png", "--preset", "pristine", *args)
    if chroma:
        ref = img[..., 0] - luma(img)
        got = out[..., 0] - luma(out)
    else:
        ref, got = luma(img), luma(out)
    r, g = packet_amplitudes(ref), packet_amplitudes(got)
    return g / r


def flat_noise(cli, tmp, *args):
    p = tmp / "flat.png"
    Image.fromarray(np.full((H, W, 3), 128, np.uint8)).save(p)
    out = run_cli(cli, p, tmp / "o.png", "--preset", "pristine", *args)
    y = luma(out)[60:420, 120:640]
    # remove slow structure (vertical/horizontal means) so only noise remains
    y = y - y.mean(axis=0, keepdims=True) - y.mean(axis=1, keepdims=True)
    return y.std()


def main(out_png, cli):
    tmp = Path(tempfile.mkdtemp(prefix="vhsphys-report-"))
    fig, ax = plt.subplots(2, 2, figsize=(12.5, 8.5))

    # ---- luma and chroma frequency response, pristine deck ----
    yl = response(cli, tmp, False)
    yl_n = yl / yl[0]
    ax[0, 0].plot(FREQS, 20 * np.log10(yl_n), "o-", label="luma, pristine")
    cr = response(cli, tmp, True)
    cr_n = cr / cr[0]
    ax[0, 0].plot(FREQS, 20 * np.log10(cr_n), "s-", label="chroma (R-Y), pristine")
    ax[0, 0].axhline(-3, color="gray", ls=":")
    ax[0, 0].set(xlabel="frequency (MHz)", ylabel="relative amplitude (dB)", title="End-to-end frequency response", ylim=(-30, 5))
    ax[0, 0].legend()
    f3 = np.interp(-3, 20 * np.log10(yl_n)[::-1], FREQS[::-1])
    ax[0, 0].annotate(f"luma -3 dB at ~{f3:.1f} MHz", (f3, -3), (f3 + 0.2, 2), arrowprops=dict(arrowstyle="->"))

    # ---- frequency response family vs head-to-tape spacing ----
    for d, c in zip([0.0, 0.15, 0.3, 0.5], ["C0", "C2", "C1", "C3"]):
        a = response(cli, tmp, False, "--set", f"head.extra_spacing_um={d}")
        ax[0, 1].plot(FREQS, 20 * np.log10(a / a[0]), "o-", color=c, label=f"+{d} um spacing")
    ax[0, 1].set(xlabel="frequency (MHz)", ylabel="relative amplitude (dB)", title="Luma response as the head lifts off the tape", ylim=(-45, 5))
    ax[0, 1].legend()

    # ---- noise vs CNR ----
    cn = [26, 30, 34, 38, 42, 46]
    nz = [flat_noise(cli, tmp, "--set", f"head.cnr_db={c}") for c in cn]
    ax[1, 0].semilogy(cn, np.array(nz) * 100, "o-")
    ax[1, 0].set(xlabel="RF carrier-to-noise (dB, 6 MHz)", ylabel="luma noise (% of full scale rms)", title="FM noise: picture noise vs CNR")
    ax[1, 0].grid(alpha=0.3, which="both")

    # ---- preset gallery strip: noise level per preset ----
    names = ["pristine", "good", "worn", "bad"]
    vals = []
    for n in names:
        p = tmp / "flat.png"
        out = run_cli(cli, p, tmp / "o.png", "--preset", n)
        y = luma(out)[60:420, 120:640]
        vals.append(np.median(np.abs(y - np.median(y))) * 1.4826)
    ax[1, 1].bar(names, np.array(vals) * 100, color=["C2", "C0", "C1", "C3"])
    ax[1, 1].set(ylabel="luma noise (% of full scale, robust rms)", title="Noise by condition preset (flat grey)")
    for a in ax.flat:
        a.grid(alpha=0.3)
    fig.tight_layout()
    fig.savefig(out_png, dpi=75)
    print(f"luma -3 dB point ~ {f3:.2f} MHz; chroma response at 0.6/1.0/1.5 MHz: "
          f"{20*np.log10(cr_n[1]):.1f} / {20*np.log10(cr_n[2]):.1f} / {20*np.log10(cr_n[3]):.1f} dB")
    print("noise vs CNR (rms %):", [f"{c}dB:{v*100:.2f}" for c, v in zip(cn, nz)])


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2] if len(sys.argv) > 2 else "target/release/vhsphys")
