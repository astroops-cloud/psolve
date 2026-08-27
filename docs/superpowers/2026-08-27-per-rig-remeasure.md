# Re-measuring the per-rig split: does psolve still lose the primary rig?

**Date:** 2026-08-27. **Answer: yes, and for the reason already diagnosed.**

## Why this was run

The standing conclusion of `2026-08-23-astap-head-to-head.md` was that psolve
"wins on the SV405CC by a wide margin, loses on the ATR585M, and is level on
the DWARFIII", and that a switch-over treating psolve as uniformly better
"would degrade the instrument that matters most". That is why the 2026-08-24
deployment is a ride-along rather than a replacement.

**Three solver improvements landed on 2026-08-25, after that measurement** --
pair matching, the tight-radius retry, and the 99.93% corpus figure. The
per-rig split was never re-run, so the regression's status was *unknown*, not
*fixed*. A measurement expires when the thing it measured changes.

The corpus number cannot answer this. **The DWARFIII contributes 8,105 of the
10,378 ASTAP-solved frames -- 78%** -- so a corpus-wide rate is mostly a
statement about one camera, and the primary rig contributes barely a tenth of
it. `scripts/agreement.sh` gained a `RIG` axis for exactly this.

## Two populations, and they answer different questions

The distinction matters, because measuring the wrong one looks like an answer.

### 1. Frames ASTAP SOLVED -- the agreement population

`RIG='ATR585M' scripts/agreement.sh full`, all 1,102 frames:

| | |
|---|---|
| psolve solved | **1,099 / 1,102 (99.73%)** |
| median separation vs ASTAP | **0.132"** |
| p90 / p99 | 0.490" / 3.941" |
| gross errors >30" | 1 (a `_probe` frame, 52.57") |
| scale outliers >5% | 0 |

**psolve is not broken on the primary rig.** On the frames a ride-along
actually compares, it reproduces ASTAP at four times better than the
corpus-wide 0.54" median.

This is *not* a refutation of the 2026-08-23 claim, which was measured on the
other population entirely.

### 2. Frames ASTAP has NO answer for -- the hard population

Every such LIGHT frame with an intact file, all three rigs, header hint where
present:

| rig | n | solved | rate | dominant failure |
|---|---:|---:|---:|---|
| SVBONY SV405CC | 376 | 251 | 66.8% | `TOO_FEW_STARS` 103 (27%) |
| DWARFIII | 210 | 141 | 67.1% | `NO_QUAD_MATCH` 34 (16%) |
| **ATR585M** | **287** | **98** | **34.1%** | **`TOO_FEW_STARS` 183 (64%)** |

**The regression stands.** The primary rig solves at roughly half the rate of
the other two on comparable populations.

## The mechanism, and why 2026-08-25 did not help

**183 of 287 ATR585M failures are `TOO_FEW_STARS` -- 64% of the whole
population**, against 27% and 15% on the other rigs. That is a **detection**
shortfall, not a matching one, and it is precisely what
`2026-08-24-atr585m-diagnostic.md` diagnosed as a completeness⁴ problem.

Pair matching and the tight-radius retry are both **matching** rungs. They
operate on the star list after extraction. A frame that never yields enough
stars cannot be rescued by a better matcher, so the improvements that produced
the 99.93% corpus figure were structurally incapable of moving this number.
That is not a disappointment; it is the two measurements agreeing.

## What this does NOT establish

- **"No ASTAP answer" is not "ASTAP failed."** `catalogue.db` records only
  successes, so "never attempted" and "attempted and failed" are
  indistinguishable in it. This measures psolve's absolute capability per rig,
  not a head-to-head. The stronger provenance is
  `2026-08-26-production-failure-benchmark.md`, which used frames ASTAP
  actually parked.
- **The separation column is not a correctness check.** It is measured against
  *commanded pointing*, and on DWARFIII **141 of 141** solved frames exceed
  30" from it -- which says the commanded pointing on these frames is
  untrustworthy, not that there are 141 wrong answers. Only ATR585M's 4.0"
  median suggests a dependable hint. Without ground truth these cannot
  distinguish a bad solve from a bad hint.
- **Correctness on the hard population is unmeasured.** Establishing it needs
  either ASTAP re-run over the same frames or the reproject-and-measure-flux
  check applied at scale.

## Operational conclusion

**Keep the ride-along. Do not cut the ATR585M over to psolve.** The primary rig
is where psolve is weakest on marginal frames, and the cause is a detection
shortfall that no rung of the current retry ladder addresses.

Two things follow for anyone picking up the next piece of work:

1. **The ATR585M gap is a detection problem.** `extract.rs`, not `match_.rs`.
   `2026-08-24-detection-experiments.md` and `2026-08-24-matched-filter-results.md`
   are the prior art.
2. **Distortion will not help it.** The distortion signal on this rig is the
   weakest of the three (`2026-08-27-distortion-signal.md`), and distortion
   concerns where detected stars land, not whether they are detected at all.
