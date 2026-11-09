"""Try parameter variants against the multiburst response: tune.py <cli> name:set,set ..."""
import sys, tempfile
from pathlib import Path
import numpy as np
from vhslab import report as R

def row(cli, tmp, chroma, sets):
    args = []
    for s in sets:
        args += ["--set", s]
    return R.response(cli, tmp, chroma, *args)

def main(cli, variants):
    tmp = Path(tempfile.mkdtemp(prefix="vhs-tune-"))
    idx = {f: i for i, f in enumerate(R.FREQS.tolist())}
    ly = [0.3, 1.0, 2.0, 2.5, 3.0, 3.5]
    print("luma dB rel 0.3 MHz at", ly, "| chroma dB at 0.3 0.6 1.0")
    for v in variants:
        name, _, sets = v.partition(":")
        sets = [s for s in sets.split(";") if s]
        y = row(cli, tmp, False, sets); c = row(cli, tmp, True, sets)
        yd = 20*np.log10(y/y[0]); cd = 20*np.log10(c/c[0])
        print(f"{name:16s} luma " + " ".join(f"{yd[idx[f]]:6.1f}" for f in ly) + "  | chroma " + " ".join(f"{cd[idx[f]]:6.1f}" for f in (0.3, 0.6, 1.0)))

if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2:])
