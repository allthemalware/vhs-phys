# Performance

Measured on 8 cores with the `worn` preset: about 12-14 fps on the 754x480 core raster
(0.5 CPU-seconds per frame, ffmpeg included). Generations multiply the cost: 2 are about 8 fps,
3 about 6 fps. Real time (29.97 fps) is not reached; the picture model is the cost, not the I/O.

## Where the time goes

Wall time of the four stages for 120 frames (8.8 s in all, the rest is ffmpeg and start-up):

| stage | wall | share |
|---|---|---|
| record (relay, transition times) | 1.45 s | 17 % |
| playback head (pulse rendering, noise) | 2.56 s | 30 % |
| deck electronics (equaliser to de-emphasis) | 2.55 s | 30 % |
| decode (sync, colour) | 0.62 s | 7 % |

CPU-seconds summed over all threads, inside the two big stages:

| head (21.7 s) | | deck (17.9 s) | |
|---|---|---|---|
| neighbouring tracks | 7.8 | FM discriminator | 5.4 |
| own-track pulses | 5.8 | supply hum/ripple (per sample) | 2.5 |
| noise and head resonance | 3.8 | luma FIR | 1.8 |
| transport warp (`Mech::fill_warp`) | 3.4 | equaliser | 1.7 |
| defects, weights | 0.8 | the rest (nine small filters) | 6.5 |

## What did not help

- `--batch 16` instead of 8: 13.9 against 13.7 fps. The four stages already fill the 8 cores.
- AVX2 (`-C target-cpu=x86-64-v3`) and fat LTO with one codegen unit: the video is bit-identical
  and the clip costs the same 60 CPU-seconds. The work is filter recurrences, random numbers
  and trigonometry, which do not vectorise. (A first wall-clock comparison seemed to show
  +20 %; it was machine load. Compare CPU-seconds, not frames per second, on a busy machine.)

## Candidates, not done

1. RF rate 12 x fsc to 8 x fsc: every stage that scales with the sample count gets a third
   cheaper, about 1.25x overall. It changes the output, so the golden hash would be
   re-baselined after a fidelity check (discriminator exactness, 9 MHz FM sidebands).
2. Render the opposite-azimuth neighbour tracks at reduced fidelity: their high frequencies are
   removed by the azimuth loss anyway; only the colour-under band matters. About 1.1x.
3. A faster Gaussian generator for the head noise, a coarser grid for the slow terms of
   `fill_warp`, incremental oscillators for the supply hum. A few percent each.

Together these are perhaps 1.5x. Real time at standard definition needs a different structure
(f32 filters vectorised across fields, or the GPU), and is a project of its own.
