"""Compare an output frame to the input colour bars: per-bar mean colour and noise."""
import sys
import numpy as np
from PIL import Image

def bars(path):
    a = np.asarray(Image.open(path).convert("RGB")).astype(float)
    out = []
    for b in range(7):
        x0, x1 = int((b + 0.25) * 754 / 7), int((b + 0.75) * 754 / 7)
        reg = a[20:300, x0:x1]
        out.append((reg.mean(axis=(0, 1)), reg.std(axis=(0, 1))))
    return out

if __name__ == "__main__":
    ref, got = bars(sys.argv[1]), bars(sys.argv[2])
    names = ["grey", "yellow", "cyan", "green", "magenta", "red", "blue"]
    for n, (m0, _), (m1, s1) in zip(names, ref, got):
        print(f"{n:8s} in {np.round(m0).astype(int)}  out {np.round(m1).astype(int)}  noise std {np.round(s1,1)}")
