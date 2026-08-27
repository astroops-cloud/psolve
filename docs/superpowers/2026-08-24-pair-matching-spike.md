# Spike: pairwise-separation matching against the 122 corpus failures

**Date:** 2026-08-24. **Verdict: it works, and it is worth building properly.**

Throwaway code on branch `spike/pair-voting` (commit `a7e98de`), never merged.
The deliverable is this measurement.

## The question

Every improvement this month attacked **completeness**. The measured
constraint is that a quad matches only when all four of its stars survive on
both sides, so matchable quads go as completeness^4. The star-tracker
literature (`2026-08-24-beyond-plate-solvers.md`) attacks the **exponent**
instead, by matching on star pairs rather than four-star hash codes.

Does that see solutions on this rig's frames that quad matching cannot?

## What was actually built

The corpus run is **hinted** -- `agreement.sh` passes `--hint` from
`catalogue.db` -- so the catalogue disc is already resolved and both point
sets are already in a common tangent plane. No k-vector and no new index
format were needed. Pairwise interstar angle becomes pairwise distance, and
sorting each side's pairs by separation gives the same searchless merge-join
a k-vector buys.

**Two designs failed before the third worked, and both bound the result.**

1. **Geometric voting into a correspondence grid.** On a real frame the sweep
   cast **268 million votes across 157 thousand cells** -- a noise floor of
   ~1700 votes per cell against a true signal of tens. A pair carries one
   number where a quad code carries four, and at these star counts that is
   not enough information to accumulate.
2. **The same, restricted to the dominant rotation.** There is no rotation to
   find. The winning window held **0.38%** of the histogram's weight where a
   flat histogram holds 0.35%. The peak was noise.
3. **Hypothesise and verify** -- what works. Two correspondences plus the
   implied scale fix a similarity transform outright, so each separation
   agreement is *tested* against every other star in the frame instead of
   being accumulated. A true hypothesis places many stars on catalogue
   counterparts; a coincidental one places none.

The lesson generalises: verification is what makes a low-information
primitive usable. It is the same reason a quad solver reprojects rather than
trusting its code match.

Two supporting choices mattered. **Magnitude matching** -- the counterparts
of a frame's brightest N detections lie among the catalogue's brightest few
N, and the ~700 deeper stars the disc also returns can only cast wrong votes.
And **testing all four readings** of each agreement, because a separation
does not say which endpoint is which and distance is invariant under
reflection.

Downstream is untouched: the correspondences go to the existing `fit_tan`,
the existing reprojection count, and the existing `verify::accept` gate.

## Results

| | frames | solved | agree with ASTAP | disagree |
|---|---|---|---|---|
| **Quad failures** (`NO_QUAD_MATCH`) | 106 | **87** | **87** | **0** |
| Regression control (solve today) | 300 | 300 | 300 | 0 |
| **Null test** (hint a true 40 deg away) | 146 | **0** | -- | -- |

Agreement is not "within half a degree" -- it is **sub-arcsecond**:

- newly solved: median **0.8"**, p90 2.3", max 52", all 87 inside 60"
- regression control: median **0.4"**, p90 1.1", max 14.9"

Confidence on the newly solved frames: log-odds median 55, min 24, against a
gate of 12. Stars matched: median 22, min 10.

**Speed.** On the frames it newly solves, pair matching is *faster* than the
quad path that fails on them: median **0.03 s against 0.23 s**, max 1.67 s
against 0.28 s. It early-exits the moment a hypothesis explains enough stars,
where quad matching builds 1500 quads a side and compares them all before
giving up. A wrong hint is the slow case -- no hypothesis ever succeeds, so
the full budget is spent -- which is what the null test measures.

**Corpus impact if adopted as measured:** 87 of the 122 remaining failures,
taking the solve rate from **98.82% to 99.66%** (10,341 of 10,376).

## The null test was wrong the first time

The first null run reported one frame solving from a "wrong" hint. It had
solved to the **truth**, at dec -88.4: shifting RA by 40 degrees at that
declination moves the pointing **1.1 degrees**, so that frame was never given
a wrong hint at all. Replaced with a true great-circle offset, and every
null hint is now *verified* at >= 30 degrees from the answer before the run
counts (closest actual: 39.88 deg). The corrected result is 0 of 146.

Worth recording as its own fact: an RA offset is not an angular offset, and
near the pole it is barely an offset at all. The same mistake would silently
weaken any null test in this repo that shifts RA.

## What this does not show

- **Only the hinted path.** Blind solving is untouched and would need the
  actual k-vector index over whole-sky pair separations. Nothing here
  measures that.
- **300 control frames, not 10,254.** No regression was found, but the full
  corpus has not been run. That is the gate before adoption, not after.
- **The spike replaces quad matching outright.** A real implementation should
  almost certainly be a **retry** instead -- regression-free by construction,
  the way the binning, quad-budget and matched-filter retries already are. A
  frame that solves on its first attempt would then never reach it and its
  answer could not change.
- The 19 still-failing frames are mostly dense fields where the best
  hypothesis reached only 4-7 inliers with no margin over the runner-up.

## Incidental finding

Two frames were rejected as `LOW_CONFIDENCE` reading
`9 matched, 27.1 decades (need 12.0), rms 0.26 px (need <= 3.00)` -- both
printed criteria pass. The one that failed is `min_matched: 10`, which the
message does not mention. The detail string reports the thresholds that were
met and omits the one that was not, so it cannot be used to diagnose the
rejection it is describing. Small, real, and independent of this spike.

## Recommendation

Build it, as a **retry** behind the existing quad path, hinted only:

1. Port `pairvote` into `psolve-core` properly, with the failed voting
   designs kept in the module doc -- they are why the working design is
   shaped as it is.
2. Wire it as a retry after `QUAD_RETRY_BUDGET` fails, through **both** entry
   points (native and ASTAP dispatch).
3. Full corpus run as the acceptance gate: the 122 must improve and the
   10,254 must not move.
4. Leave blind alone for now. It is a separate piece of work needing a real
   pair-separation index, and its acceptance gate is multiplicity-corrected.
