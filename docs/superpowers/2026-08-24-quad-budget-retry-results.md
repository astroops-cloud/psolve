# Quad-budget retry: acceptance measurement

**Date:** 2026-08-24. **Spec:** `specs/2026-08-24-quad-budget-retry-design.md`.
**Before:** `3ba1c32`. **After:** `f3b9974`. Release builds, Apple M5 Max.
**Corpus:** all 10,376 frames with an ASTAP measurement row and an intact
location, same index, same commanded-pointing hints, one invocation each.

`~/astroops/` was never written.

## Result

| | before | after |
|---|---:|---:|
| solved | 10,141 / 10,376 = **97.74%** | **10,211 / 10,376 = 98.41%** |
| **regressions** | -- | **0** |
| previously-solving frames whose centre moved **at all** | -- | **0** |
| gains | -- | **+70** |
| wall median | 70 ms | **69 ms** |

**Six of six criteria pass.**

| # | criterion | measured | verdict |
|---|---|---|---|
| 1 | >= 4 of the 7 star-rich ATR585M frames solve | 4, all within 5" of ASTAP | **PASS** |
| 2 | zero regressions, per frame | **0 of 10,376**, and no solved centre moved | **PASS** |
| 3 | corpus solve rate does not fall | 97.74% -> **98.41%** | **PASS** |
| 4 | median time unchanged for first-attempt solves | 70 ms -> 69 ms | **PASS** |
| 5 | `quad_budget` reported and correct | see below | **PASS** |
| 6 | both entry points | retry lives in `psolve-core`; both reach `solve_prepared` | **PASS** |

## The structural claim, proven rather than argued

The spec's central design choice was a retry rather than a raised default, so
that a frame solving at the caller's budget **cannot** take a different path.
That is checkable directly:

```
quad_budget across all solved-after frames:  {600: 10141, 1500: 70}
frames that solved BEFORE but report a retry budget after:  0
```

Every one of the 10,141 previously-solving frames reports the base budget, so
none entered the retry; and none of their centres moved. The regression-free
property is **structural, not statistical** -- it did not depend on this run
finding nothing.

## Where the 70 went

| rig | recovered |
|---|---:|
| **ATR585M (sv555, the primary rig)** | **41** |
| DWARFIII bin1 | 24 |
| SVBONY SV405CC bin1 | 5 |

The 19-frame ATR585M sample predicted 4. The corpus delivered **70**, of which
41 are on the primary rig -- the deficit this work was aimed at. The sample
understated the effect by an order of magnitude because it was drawn from
frames ASTAP had *no measurement row for*, a deliberately hard population;
most of the gain is on ordinary frames that were quietly failing.

**That is a bigger win than the spec projected, and the projection was not
wrong -- it was measuring a different population.** Worth stating plainly
rather than claiming foresight.

## What remains

165 failures, from 235: `NO_QUAD_MATCH` 149, `LOW_CONFIDENCE` 14,
`TOO_FEW_STARS` 2.

`LOW_CONFIDENCE` rose from 11 to 14. That is expected and is the safety
property working: a frame that previously found no transform at all can now
find one at the larger budget and have it **refused by the confidence gate**.
Three frames moved from "no answer" to "an answer that did not survive
scrutiny", which is the correct direction -- `LOW_CONFIDENCE` is a refusal,
not a wrong answer.

The star-poor group is untouched, as the spec said it would be. Those frames
detect a handful of the catalogue stars present (frame 15186: about 11 of 138)
and no quad budget helps when the stars are not there. That remains
undiagnosed; the lead is ASTAP's `find_stars`.

## Cost

Median 70 ms -> 69 ms: unchanged, within noise. Only the 165 frames that still
fail and the 70 that now succeed ever run a second attempt, and every one of
them was already failing.
