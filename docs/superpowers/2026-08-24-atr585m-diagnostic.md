# Why psolve loses the primary rig: a completeness^4 problem

**Date:** 2026-08-24. **Binary:** `89c6e90` release, plus an out-of-tree
instrumented copy (never committed). **Frames:** the 19 ATR585M/sv555 frames
ASTAP solves and psolve does not, from the full 287-frame population measured
in `2026-08-23-astap-head-to-head.md`.

This is a **diagnostic spike**, not a fix. Its output is a mechanism.

## The finding

**A quad matches only if all four of its stars are present on both sides, so
the matchable fraction of quads goes as completeness to the FOURTH POWER.**
Completeness here means: of the catalogue stars that fall inside the frame,
what fraction did psolve detect?

| frame | detections | cat-in-frame | completeness | purity | code matches / 720,000 |
|---|---:|---:|---:|---:|---:|
| **15181 (fails)** | 500 | 736 | **42.4%** | 62.4% | **0** |
| **14971 (solves)** | 182 | 222 | **63.1%** | 75.8% | **27** |

```
0.424^4 = 3.2%      0.631^4 = 15.9%       a 5x difference in matchable quads
```

Completeness was measured by reprojecting the fetched catalogue through
**ASTAP's** solved WCS for each frame and cross-matching against psolve's kept
detections at 5 px. ASTAP's WCS is the independent reference; psolve's own
cannot be used, since on these frames it has none.

## This is ONE defect, not the two it looked like

The 19 losses split 12 "star-poor" and 7 "star-rich", and those looked like
different problems needing different fixes. They are the same problem seen at
two star counts. Both groups fail because completeness is too low; the
star-poor group additionally lacks the raw detections to recover.

That matters because it halves the work and it kills a plan: designing a
density-aware quad budget and a scale-aware extraction floor as separate
milestones would have been two fixes for one cause.

## What was ruled out, by measurement

Each of these was a plausible mechanism, and each is refuted. Recorded so
nobody re-derives them:

| hypothesis | test | result |
|---|---|---|
| pointing hint wrong | vs ASTAP's centre | **0.0 arcmin** -- exact |
| plate scale wrong | header 2.4616 vs ASTAP 2.4527 "/px | **0.36%** -- correct |
| too few stars | 500-2000 usable | ruled out |
| catalogue too shallow | G<=16 index, 14,719 stars available | still fails |
| catalogue limit too low | `--cat-limit` 500, 5000, 20000 | all fail |
| code tolerance too tight | `code_tol` 0.02 -> 0.05 (2.5x) | **rescues none** |
| catalogue/image density mismatch | matched the ratio to the solving frame (0.45 vs 0.49) | **still fails** |
| detections are not real stars | 42.4% land on catalogue stars | refuted -- they are real |
| quad baselines too short | `--keep` 500 -> 60 to widen them | best distance improves, **still fails** |

The density hypothesis is worth naming as refuted rather than merely dropped:
it was one step from being written into a spec as the mechanism, and the test
that killed it (`--cat-limit 500`, giving a density ratio of 0.45 against the
solving frame's 0.49) had already been run before the hypothesis was formed.

## Why completeness explains what the others could not

- **Raising `--cat-limit` makes it worse.** It adds catalogue stars that were
  never detected, which *lowers* completeness.
- **Thinning detections does not help.** It removes detections rather than
  supplying the missing ones; completeness cannot rise.
- **A wider code tolerance does not help.** The true quad pairs are not
  marginal -- for the most part they do not exist. `best_code_dist` was
  0.0210 on the failing frame against **0.00065** on the solving one, a factor
  of 32; that is not a tolerance away.
- **The star-rich frames have plenty of stars and still fail.** 500 detections
  at 42% completeness yields fewer matchable quads than 182 at 63%.

## Where the missing stars go

Purity is 62.4% on the failing frame: 38% of psolve's detections are not
G<=14 catalogue stars. On the ATR585M -- the finest-sampled rig at 2.46"/px --
extraction keeps roughly 5% of raw detections, and the `min_pix = 4` floor
rejects 76-96% of them. The survivors skew toward faint and blended sources
that have no catalogue counterpart, while genuine catalogue stars are lost to
the same floor.

So the extraction floor is implicated after all -- but as a **completeness**
problem, not a star-count one. That reframing matters: "too few stars" is
false (500-2000 are available); "too few of the RIGHT stars" is true.

## What this does not establish

**No fix is validated here.** Raising completeness is the indicated direction,
but no setting tested in this spike achieves it: relaxing `min_pix` and
`max_ellipticity` together rescues only **2 of 6** frames tried, and lowering
`--sigma` multiplies detections 266 -> 51,796 while `used` rises only 8 -> 28,
because the additions are single-pixel noise the floor then rejects.

So the next step is a design against completeness as the objective, measured
directly rather than inferred from solve rate -- and completeness is now a
quantity this project can measure, which it could not this morning.

## Reproducing

The instrumented binary is an out-of-tree copy under `.scratch/diag/` and is
deliberately **not committed**: it adds `eprintln!` counters to `match_.rs` and
an `std::env::var` star dump to `solve.rs`, and the latter would trip
`psolve-core`'s `no_filesystem` guard, which forbids `env` anywhere in that
crate including in comments. Rebuild it by copying the tree and re-applying
those two edits.
