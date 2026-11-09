"""Horizontal displacement per picture row, measured on a single sharp vertical edge."""
import sys
import numpy as np
from PIL import Image

def edge_positions(path):
    a = np.asarray(Image.open(path).convert("RGB")).astype(float).mean(axis=2)
    h, w = a.shape
    pos = np.full(h, np.nan)
    for r in range(h):
        seg = a[r, w // 2 - 120 : w // 2 + 120]
        lo, hi = seg[:30].mean(), seg[-30:].mean()
        level = 0.5 * (lo + hi)
        i = np.where((seg[:-1] - level) * (seg[1:] - level) <= 0)[0]
        if len(i) and hi - lo > 40:
            j = i[len(i) // 2]
            pos[r] = w // 2 - 120 + j + (level - seg[j]) / (seg[j + 1] - seg[j] + 1e-9)
    return pos

if __name__ == "__main__":
    for p in sys.argv[1:]:
        e = edge_positions(p)
        base = np.nanmedian(e[100:600])
        d = e - base
        rows = [0, 4, 8, 16, 32, 64, 128, 256, 400, 560, 640, 690, 705, 714]
        print(f"{p.split('/')[-1]:>14}: " + " ".join(f"{d[r]:+5.1f}" for r in rows) + f" | rms rows100-600 {np.nanstd(d[100:600]):.2f} px")
