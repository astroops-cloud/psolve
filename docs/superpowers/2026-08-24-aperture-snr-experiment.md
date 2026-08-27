# Aperture-SNR acceptance: tested, and the SNR is not the part that works

**Date:** 2026-08-24. **Baseline:** `291fe3e` -- psolve with the binning
refetch, the quad-budget retry and the matched-filter retry all landed
(corpus 98.82%). **Frames:** 20 ATR585M frames that still fail after all
three, plus 12 controls. Implemented in an out-of-tree copy; nothing committed
to the solver.

## What was tested

ASTAP's acceptance shape, reproduced faithfully: a cheap **cross-shaped
hot-pixel test** (at least 2 of the 4 orthogonal neighbours above
`background + 4*noise`), then flux summed over a **fixed circular aperture**
around the centroid, accepted on **SNR**. That replaces psolve's
`npix >= min_pix` rejection, which discards a candidate before measuring
anything.

The premise was that a faint star whose above-threshold footprint is two
pixels is still measured over a real aperture and can pass.

## Result

| variant | still-failing (20) | controls (12) |
|---|---:|---:|
| shipped (`min_pix 4`) | **0/20** | 12/12 |
| out-of-tree copy, no change (sanity) | 0/20 | -- |
| cross test + aperture, `snr > 10` | **2/20** | **12/12** |
| cross test + aperture, `snr > 3` | 2/20 | 12/12 |
| cross test + aperture, **`snr > 0`** | **2/20** | 12/12 |
| cross test + aperture, `snr > 50` | 3/20 | -- |
| cross test + aperture, `snr > 200` | 2/20 | 12/12 |

Control median time 81 ms -> 84 ms.

## The finding: the SNR cut does nothing; the cross test does the work

**Disabling the SNR cut entirely (`snr > 0`) gives the same 2/20 as
`snr > 10`.** Across a 200x range of thresholds the result moves only between
2 and 3, which is noise at n=20.

So the mechanism is not "acceptance on SNR measured over an aperture", which
is how this candidate has been described in this project's docs since the
ASTAP comparison. It is **replacing a pixel-count floor with a shape test**.
`min_pix >= 4` asks "is this blob big enough"; the cross test asks "does this
look like a point source", and a faint real star passes the second where it
fails the first.

That matters for what to build. Aperture photometry is a substantial addition
-- a real measurement path psolve does not have. **A cross-shaped neighbour
test is a dozen lines and four comparisons per candidate**, and on this
evidence captures the entire gain.

## Is it worth doing?

**Marginal, and much smaller than the three changes that landed today.**

2 of 20 is 10% of the remaining hard tail. Extrapolated to the 122 corpus
failures that would be a dozen or so frames -- roughly 98.82% -> 98.9%. Real,
regression-free on the controls tested, and nearly free in time.

Against that: it changes the extraction acceptance path, which has the worst
regression history in this repository (a change there once improved fit RMS
while dropping solve rate 85.8% -> 69.4%). A dozen frames does not obviously
justify touching it, and unlike the three retries landed today it cannot be
made regression-free by construction -- it alters what extraction returns on
**every** frame, not only on frames that have already failed.

Unless it is made a retry too. That is the shape this project keeps arriving
at, and it would apply here: keep `min_pix` for the first attempt, and re-run
extraction with the cross test only after a failure. Untested.

## Caveats

- **n = 20 and n = 12.** The difference between 2/20 and 3/20 is not
  meaningful.
- The completeness harness was **not** usable here: with the matched-filter
  retry landed, `solve_prepared` runs twice on a failing frame and the star
  dump mixes both extractions. Solve outcomes are the measurement above;
  completeness figures from that harness on a retrying binary should not be
  trusted until the dump is made attempt-aware.
- The aperture noise model is the simple one -- per-pixel sigma times the root
  of the pixel count, no Poisson term from the source itself. ASTAP's is
  richer. Given the SNR cut turned out not to matter, this is unlikely to
  change the conclusion.
