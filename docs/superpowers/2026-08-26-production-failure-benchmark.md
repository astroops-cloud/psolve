# psolve against ASTAP's production failures: 72 of 184

**Date:** 2026-08-26. The strongest benchmark this project has had, because of
where the frames came from: the AstroOps session ran its **entire archive of
12,620 frames through ASTAP** via the live ingest path, and **1,088 were
parked** as unsolved. These are confirmed production failures at production
parameters -- not "frames with no recorded solve row", which is a weaker
claim this repo has leaned on before and which a memory already warns about.

214 frames were staged on the NAS by that session: **184** from the parked
set, stratified by instrument and exposure with a fixed seed, plus **30**
labelled as frames ASTAP solved, for use as controls.

## The controls came first, and 7 of the 30 were not controls

psolve failed 7 of the 30. Rather than treat that as a psolve result, ASTAP
was re-run on those 7 at production parameters (header `-fov`, real hint,
`-r 10`, then `-fov 0` on failure). **It solved none of them.**

```
2026-08-24_14-59-54    2 s   OBJECT=PROBE_az180_alt45
2026-08-24_15-06-05    2 s
2026-08-24_19-54-40   15 s   OBJECT=PROBE_az150_alt60
2026-08-24_19-54-55   15 s
2026-08-24_19-55-10   15 s
2026-08-24_19-57-24   15 s
2026-08-24_20-03-42   15 s
```

All seven are `PROBE_*` pointing-model frames.

**Correction, 2026-08-26.** This paragraph first guessed that "whatever
recorded them as solved appears to have stored the mount's commanded position
rather than a solve". That was wrong and the truth is simpler: **probe frames
are filed without ever being solved, by design.** `ingest/ingest.py` matches
`PROBE_az..._alt...` in the header and files straight to the `_probe` slug,
returning before the solve path runs -- correct behaviour, since a probe
frame's purpose is a star count at a known az/alt, not an identity.

Nothing recorded a fake solve. The controls were selected by **location** --
everything under `library/*/lights/*/*` -- on the assumption that being in the
library implies ASTAP solved it. It does not: probe frames file without
solving, and so does the error path that still has a target hint. The property
needed was *evidence of a solve*; the property used was *where the file
lives*.

**Excluding them, the harness is validated: 23 of 23 genuine controls solve**,
log-odds median 621, 8 of the 23 through the binning retry. The 184 were not
run until this was settled -- a control set that includes unsolvable frames
would have made every subsequent number unreadable.

## Result

**72 of 184 (39.1%).**

| instrument | solved | | exposure | solved |
|---|---|---|---|---|
| SVBONY SV405CC | **54 / 100 (54%)** | | 90 s | 44 / 70 (63%) |
| ATR585M | 18 / 84 (21%) | | 120 s | 12 / 25 (48%) |
| | | | 150 s | 2 / 2 |
| | | | 300 s | **13 / 85 (15%)** |

Which rung answered: **54 first attempt, 9 pair matching, 9 tight radius.**
A quarter of the recoveries came from rungs added on 2026-08-25 and
2026-08-26.

Remaining failures: `TOO_FEW_STARS` 86, `NO_QUAD_MATCH` 22, `CANNOT_READ` 4.

### Verified, not counted

- Every solve lands within **0.552 deg** of the frame's own header pointing
  (median 0.091, p90 0.330).
- Log-odds min 17.0, median 566.6, against a gate of 12.
- An independent photometric check on a random 14 of the 72 -- reprojecting
  the local `.psidx` through psolve's WCS and measuring core-minus-annulus
  against 400 random control positions -- corroborates **14 of 14**. That
  test was validated first on known-good frames, where it reads 56-1020 at
  star positions against 0-10 at controls.

### One result explicitly not explained

The AstroOps session **refuted** the obvious hypothesis for the SV405CC
column before this ran: that camera's already-binned `XPIXSZ` is handled on
their side by a `PREBINNED_INSTRUME` table, so ASTAP is being handed the
correct field. psolve still solves **54%** of them. **What it is doing there
is not diagnosed and is not guessed at here.**

## Six of the parked frames are truncated, not unsolvable

Four were found in this 184-frame sample. A full scan of all 1,104 parked
frames by the AstroOps session then found **six**:

```
    262,144 B  of 16,588,800   = 1 x 256 KB   2025-07-19_01-20-40_S
    262,144 B  of 16,588,800   = 1 x 256 KB   2025-07-19_02-53-08_H
    786,432 B  of 16,588,800   = 3 x 256 KB   2025-07-19_01-32-43_S
  1,048,576 B  of 16,588,800   = 4 x 256 KB   2025-07-19_03-16-23_O
  3,476,288 B  of 16,588,800                  2025-07-19_02-19-12_O
 13,617,984 B  of 16,813,440                  C 76_15s60_Astro_2025-05-22
```

**Four are exact multiples of 262,144 -- a 256 KB write buffer**, which is
sharper than "power-of-two boundary" and identifies the mechanism as an
interrupted write rather than a corrupt sensor read. **The sixth is a DWARF
frame from a different night and a different rig**, so this is not one bad
session; the corpus carries the failure mode.

Each passed the upload path's bounds by being non-zero and under the cap, and
each sat in the unsolved backlog counted as a solver failure for thirteen
months.

psolve reports `CANNOT_READ` with `fits truncated: need 16594560 bytes, have
262144`. **A distinct reason code from "I could not solve this" is the entire
argument for having distinguishable reason codes**, and this is the first time
it has separated broken data from hard data on real frames.

**Fixed upstream the same day** (`c19483d` in the AstroOps repo, deployed):
a truncation check now runs *before* the solve, so a short file no longer
burns a full ASTAP search to the 180 s cap first, and parks with its own
`truncated` status. It tests short only, never long -- extensions and padding
legitimately exceed the primary data unit.

## Timing

184 frames, **369.3 s total**: median 1542 ms, p90 3729 ms, max 7865 ms.
Failures cost median 1392 ms, max 4416 ms.

ASTAP parked all 184. At its 180 s cap that same set is **up to 9.2 hours**.

The failure cost is higher than the 61 ms median measured on the earlier
200-frame population, exactly as was predicted before this ran: these frames
are star-rich, so they reach `NO_QUAD_MATCH` through the full retry ladder
rather than exiting at `TOO_FEW_STARS`. It is still three orders of magnitude
short of the cap.

## What this does not claim

This is a **selected** population -- every frame in it is one ASTAP failed, so
39.1% is a recovery rate on a hard tail and not a solve rate. The complement
matters as much: **ASTAP solved 91.4% of the 12,620-frame import** (1,088
parked). The case for psolve here is the tail and the wall clock, not the
bulk.

> **Correction, 2026-08-26.** This paragraph first said ASTAP "solved 98.3% of
> 12,620 frames". **That is a windowed rate quoted as a whole-import one.**
> 98.3% was the last 42 minutes of the run — 739 filed against 13 parked —
> which the AstroOps session stated with that scope attached. Over the whole
> import the rate is **91.38%**, because 1,088 of 12,620 were parked; 98.3% of
> 12,620 would imply 215 failures, not 1,088. The error was mine in
> generalising a figure that arrived correctly qualified, and it was caught by
> the vault session doing arithmetic I had not: the two numbers I quoted in the
> same sentence could not both be true.

---

# Part two: 331 frames never offered to any solver

A second set staged the same day: the tail of a stopped import, **331
DWARFIII frames that had never been offered to any solver on this rig**. Not
a failure set -- no tool had had a chance at them -- which makes it the
cleanest A/B available and the only place the cost of an *empty* frame could
be measured.

## Result

**60 of 331 (18.1%)**, or **60 of 231 (26.0%)** excluding the 1 s frames.

| exposure | solved | wall median | wall max |
|---|---|---|---|
| **1 s** | **0 / 100** | 1,127 ms | 2,099 ms |
| 15 s | 45 / 176 (26%) | 2,541 ms | 6,061 ms |
| 30 s | 1 / 11 | 2,961 ms | 4,923 ms |
| 60 s | 14 / 34 (41%) | 2,674 ms | 4,476 ms |
| 120 s | 0 / 10 | 2,872 ms | 3,327 ms |

Rungs: 55 first attempt, 3 matched-filter, 2 pair matching. Log-odds median
780.6, min 15.6. All 60 solves land within **0.123 deg** of their own header
pointing (median 0.084).

## 26% is not a psolve deficiency -- the population is hard for both

DWARFIII frames solve at ~97% historically on this rig, so 26% looked alarming
until it was compared rather than assumed. On the **same 36 frames**, run with
production parameters:

```
psolve solved  7 / 36
ASTAP  solved  6 / 36      wall median 2,506 ms, max 8,823 ms
```

Both tools are near the floor. These 331 are the remainder of a **stopped**
import, and whatever stopped it, they are not a representative DWARFIII
sample. The right reading is that psolve is marginally ahead on a set neither
solver handles, not that psolve is 71 points worse than the historical rate.

## The empty-frame cost: 1,127 ms, not 52 ms

The 100 × 1 s frames are the measurement this set existed to provide, and they
**refute a number this project nearly let travel.**

An earlier population measured `TOO_FEW_STARS` at **52 ms median**, and that
figure was offered as an estimate of what a sub-second frame would cost. The
AstroOps session declined to accept it as transferable, on the grounds that a
frame with *zero* usable detections might behave differently from one with a
few. **That caution was correct: the real number is 1,127 ms median, max
2,099 ms -- 22x the estimate.**

The mechanism is visible in the counts: those frames yield a median **1,334
detections of which zero survive rejection**. The cost is not "finding
nothing quickly", it is finding 1,334 noise blobs, rejecting them all, and
then paying the matched-filter retry -- a full second decode, background and
extract -- before concluding the same thing again.

It remains a large win against a 180 s cap (~160x), and the honest figure for
411 such frames is about **7.5 minutes**, not the 21 seconds the 52 ms number
implied. Against ASTAP's cap the same set is up to 20.5 hours.

## Failure cost on this population

| reason | n | median | p90 | max |
|---|---|---|---|---|
| `NO_QUAD_MATCH` | 162 | 2,627 ms | 2,879 ms | 3,555 ms |
| `TOO_FEW_STARS` | 107 | 1,131 ms | 1,494 ms | 2,099 ms |
| `LOW_CONFIDENCE` | 2 | 5,492 ms | -- | 6,061 ms |

331 frames in **625 s** total.
