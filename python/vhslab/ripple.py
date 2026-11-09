"""Measure luma ripple in the grey bar of a dumped field (flat 76.8 IRE expected)."""
import json, sys
import numpy as np

def ripple(d):
    meta = json.load(open(d + "/meta.json"))
    luma = np.fromfile(d + "/luma.f32", "<f4")
    first = (-meta["bb_start"]) % 910
    rows = np.array([luma[first + ln * 910 + 135 + 15 : first + ln * 910 + 135 + 95] for ln in range(60, 110)])
    return rows.mean(), rows.std(axis=1).mean()

if __name__ == "__main__":
    for d in sys.argv[1:]:
        m, s = ripple(d)
        print(f"{d.split('/')[-1]:>14}: mean {m:6.2f} IRE  ripple rms {s:5.2f} IRE")
