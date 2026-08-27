# Matched-filter retry: acceptance measurement

**Date:** 2026-08-24. **Before:** `ab2e4a8` (quad-budget retry).
**After:** `6b8feb4`. Release builds, Apple M5 Max. **Corpus:** all 10,376
frames with an ASTAP measurement row and an intact location, same index, same
commanded-pointing hints. `~/astroops/` never written.

## Result

| | before | after |
|---|---:|---:|
| solved | 10,211 / 10,376 = **98.41%** | **10,254 / 10,376 = 98.82%** |
| **regressions** | -- | **0** |
| previously-solving frames whose centre moved **at all** | -- | **0** |
| gains | -- | **+43**, every one via the retry |
| wall median | 69 ms | 70 ms |

Recovered by rig: **ATR585M (sv555, the primary rig) 32**, DWARFIII 11.

## The structural property held again

The retry is unreachable for a frame that solves on the first attempt, so no
solved answer can change. The corpus confirms it: **zero regressions, and zero
solved centres moved.**

One check in the comparison script fired and is a **false positive worth
recording**: it flagged "70 frames that solved before report a retry budget
after". All 70 are exactly the frames the *quad-budget* retry rescued in the
previous run -- they legitimately carry budget 1500 in both. The check was
written to validate the quad retry against a pre-quad baseline and does not
transfer to a comparison whose baseline already contains it. Zero unexplained.

## Cost

Median 69 ms -> 70 ms. The matched filter costs 25.3 ms -> 211.3 ms of
extraction when it runs, but it runs only on frames that have already failed
everything else: **122 of 10,376**, plus the 43 it rescues. The other 98.8%
never touch it.

## The day's cumulative effect

| change | corpus solve rate |
|---|---:|
| start of 2026-08-24 | 90.12% |
| binning-retry catalogue refetch | 97.74% |
| quad-budget retry | 98.41% |
| **matched-filter retry** | **98.82%** |

**90.12% -> 98.82%, with zero regressions at every step.** All three are
retries reached only after a first attempt has failed, so at no point did a
frame that already solved take a different path.

## What remains

122 failures: `NO_QUAD_MATCH` 106, `LOW_CONFIDENCE` 14, `TOO_FEW_STARS` 2.

`LOW_CONFIDENCE` is unchanged at 14 -- the matched filter is deliberately not
triggered by it, since re-extracting to get past a confidence gate is the
shape that once produced a confident solve 87.77 degrees from the truth.

The remaining `NO_QUAD_MATCH` frames are the hard tail. Measured earlier,
completeness on them reaches about 28% with the filter against 62-69% on
frames that solve, so the filter closes roughly a third of the detection gap
and no variant tested closes the rest. The untested candidate remains
acceptance on **SNR measured over an aperture** rather than connected-pixel
count -- what ASTAP actually relies on, and a larger change than anything
landed today.
