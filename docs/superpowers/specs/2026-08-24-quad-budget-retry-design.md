# Quad-Budget Retry -- Design

**Date:** 2026-08-24
**Status:** proposed
**Size:** one focused fix plus its measurement. Not a milestone.
**Depends on:** nothing. Applies to `main` at `d577307`.
**Diagnostic:** `docs/superpowers/2026-08-24-atr585m-diagnostic.md`

## 1. The problem

On the primary rig (sv555 / ATR585M) ASTAP solves **19** frames psolve cannot,
against **3** the other way, over the full 287-frame unmeasured population
(`docs/superpowers/2026-08-23-astap-head-to-head.md`).

Those 19 split into two groups that share a mechanism but need different
remedies:

| group | n | usable stars | what limits them |
|---|---:|---|---|
| **star-rich** | 7 | 274-500 | **quad sampling** -- this spec |
| star-poor | 12 | 5-75 | too few usable stars -- **out of scope** |

**This spec addresses the star-rich group only.**

## 2. Mechanism

A quad matches only if all four of its stars exist on both sides, so the
matchable fraction of quads goes as **completeness^4** -- completeness being
the fraction of in-frame catalogue stars psolve detects. Measured by
reprojecting the catalogue through ASTAP's solved WCS and cross-matching at
5 px:

| frame | completeness | completeness^4 | code matches / 720,000 |
|---|---:|---:|---:|
| 15181 (fails) | 42.4% | 3.2% | **0** |
| 14971 (solves) | 63.1% | 15.9% | **27** |

`solve.rs` builds **600** quads per side, fixed:

```rust
let iq = quad::build_quads(&image_pts, 6, opts.max_quads);   // max_quads = 600
let cq = quad::build_quads(&cat_pts,   6, opts.max_quads);
```

In a dense field the two 600-quad samples are drawn from populations of very
different size (500 image stars against 1,500 catalogue stars) and do not
overlap. Raising the budget makes them overlap:

| `max_quads` | code matches on 15181 | outcome |
|---:|---:|---|
| 600 | 0 | `NO_QUAD_MATCH` |
| 1500 | -- | **SOLVED**, log-odds 625.1, **0.161" from ASTAP** |
| 3000 | -- | **SOLVED**, log-odds 625.1, 0.439" from ASTAP |

### Refuted alternatives

Each was plausible and each is refuted by measurement; recorded so they are
not re-derived. Hint error (**0.0 arcmin**), plate scale (**0.36%**),
catalogue depth (G<=16, 14,719 stars), catalogue limit (500 / 5,000 /
20,000), code tolerance (0.02 -> 0.05, **2.5x, rescues none**),
catalogue/image density mismatch (matched to the solving frame's ratio,
0.45 vs 0.49, **still fails**), detections not being real stars (42.4% land
on catalogue stars), and quad baseline length (`--keep` 500 -> 60).

## 3. Measured effect, and its limits

All 19 lost frames plus the 41 controls that already solve, same index, same
hints:

| `max_quads` | lost recovered | control frames | control mean time |
|---:|---:|---:|---:|
| **600** (today) | **0 / 19** | 41/41 | **95 ms** |
| **1500** | **5 / 19** | 41/41 | 152 ms |
| **3000** | 6 / 19 | 41/41 | 358 ms |

At 1500 the recoveries are **4 of the 7 star-rich frames** (14822, 15158,
15163, 15181) plus one star-poor frame (14980). Going to 3000 buys **one more
frame for 2.4x the time**, which is not worth it.

**Be clear about the size of this win: 5 of 19, not 19 of 19.** The 12
star-poor frames are unaffected and remain lost. This does not close the
ATR585M gap; it closes a quarter of it.

## 4. Design

A **retry**, not a raised constant.

Matching is `O(image_quads x catalogue_quads)`, so 600 -> 1500 is 6.25x the
comparisons. Raising the constant makes all 10,141 currently-solving frames
pay it -- the control mean above rises 95 ms -> 152 ms -- for no benefit, and
puts every one of them at risk of a changed answer.

Instead: solve at 600 as today; **on `NO_QUAD_MATCH` only**, retry once at
1500.

```
attempt 1: max_quads = 600      (unchanged)
  solved            -> return, bit-identical to today
  NO_QUAD_MATCH     -> attempt 2: max_quads = 1500
  any other failure -> return, no retry
```

**Regression-free by construction, not by measurement.** A frame that solves
at 600 never enters the retry, so its answer cannot change. That satisfies the
per-frame acceptance bar structurally rather than by hoping a corpus run finds
nothing.

`TOO_FEW_STARS` and `LOW_CONFIDENCE` are deliberately **not** triggers.
`TOO_FEW_STARS` means extraction produced too little to work with and more
quads cannot exist. `LOW_CONFIDENCE` means a transform was found and the
evidence gate refused it -- retrying with more quads to get past a
multiplicity-corrected confidence gate is precisely the shape that produced a
confident solve 87.77 degrees from the truth (`verify.rs`).

This mirrors `solve_with_binning_retry`, the established pattern in this file:
attempt, detect one specific failure, retry once with one parameter changed,
and report which attempt answered.

### Reporting

The JSON must say which attempt produced the answer, for the same reason
`scale_source` exists: a caller comparing runs needs to know the budget was
raised. Emit `quad_budget` alongside `scale_source` -- `600` normally,
`1500` when the retry answered.

### Both entry points

Native and ASTAP-compat both reach `solve_prepared`. **Any behaviour change
must be wired through both** -- a fix of exactly this shape reached
`cmd_solve.rs` alone on 2026-08-14 and left ASTAP dispatch stale.

## 5. Acceptance

**Net-positive AND regression-free, compared per frame, not in aggregate.**

1. The 7 star-rich frames: at least 4 solve, each within 5" of ASTAP's centre.
2. **Zero regressions** across the full 10,376-frame corpus: no frame that
   solves today may stop solving, and no solved frame's centre may move.
   Byte-identical solve records for every frame that does not enter the retry
   is the expected result, and anything else is a finding.
3. Corpus solve rate does not fall. It is expected to rise slightly.
4. Median solve time unchanged for frames that solve on attempt 1.
5. `quad_budget` is reported and correct.

## 6. Out of scope

- **The 12 star-poor frames.** They are limited by usable star count (5-75),
  and no quad budget helps when there are not enough stars to form quads from.
  They need extraction work, which is its own design against completeness as
  the measured objective.
- Scaling the budget from star count or field density rather than a fixed
  retry value. A second fixed value is simpler, measurable, and sufficient for
  the frames in evidence; a density heuristic is a tuning surface with no
  measurement behind it yet.
- `max_quads` as a CLI flag. Nothing in evidence needs per-frame control.

## 7. Risks

- **The 1500 value is fitted to 19 frames.** It is a measured choice, not a
  derived one, and the corpus run is what tests whether it generalises.
- **Retry cost on failures.** 235 frames currently fail corpus-wide; each
  would now cost roughly double. Acceptable -- they are already failing, and
  the median is untouched.
- **A fourth proposed mechanism.** Three earlier explanations for these frames
  were refuted by measurement. This one differs in kind -- it is a
  demonstrated fix rather than an explanation -- but it has been true for
  hours rather than days, and the corpus run is the check.
