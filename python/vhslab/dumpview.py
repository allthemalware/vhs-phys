"""Plot intermediate signals dumped by the Rust `dump` example."""
from __future__ import annotations

import json
import sys
from pathlib import Path

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt
import numpy as np
from scipy import signal

FSC = 315e6 / 88
FH = FSC * 2 / 455


def load(d):
    d = Path(d)
    meta = json.loads((d / "meta.json").read_text())
    arr = lambda n: np.fromfile(d / n, dtype="<f4")
    return meta, arr("rf.f32"), arr("luma.f32"), arr("cunder.f32")


def psd(x, fs):
    f, p = signal.welch(x, fs, nperseg=8192)
    return f / 1e6, 10 * np.log10(p + 1e-30)


def main(dirs, out):
    fig, ax = plt.subplots(2, 2, figsize=(13, 8))
    for d, c in zip(dirs, ["C0", "C3", "C2"]):
        meta, rf, luma, cu = load(d)
        f, p = psd(rf, meta["fs_rf"])
        ax[0, 0].plot(f, p, c, lw=0.8, label=Path(d).name)
        f, p = psd(cu, meta["fs_bb"])
        ax[0, 1].plot(f, p, c, lw=0.8)
        # one visible line of luma (line 100 of the field window)
        n_line = 910
        start = (60 + 21) * n_line + 0
        ax[1, 0].plot(luma[start + 135 : start + 135 + 754], c, lw=0.8)
        ax[1, 1].plot(luma[start : start + 2 * n_line], c, lw=0.8)
    ax[0, 0].set_title("RF spectrum at the preamp (dB/Hz)")
    ax[0, 0].set_xlim(0, 21)
    ax[0, 0].legend()
    ax[0, 1].set_title("colour-under signal spectrum")
    ax[0, 1].set_xlim(0, 7)
    ax[1, 0].set_title("luma, one active line (IRE)")
    ax[1, 1].set_title("luma, two lines incl. sync")
    for a in ax.flat:
        a.grid(alpha=0.3)
    fig.tight_layout()
    fig.savefig(out, dpi=70)


if __name__ == "__main__":
    main(sys.argv[2:], sys.argv[1])
