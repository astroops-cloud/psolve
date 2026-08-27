# ASTAP vs psolve, head to head on the frames ASTAP has no answer for

**Date:** 2026-08-23. **psolve:** `ca89ced` (post binning-retry refetch),
release build. **ASTAP:** `astap_cli` CLI-2026.06.29 with the `d50` database,
at `~/astap` -- installed, just not on `PATH`, which is why an earlier note in
this repo wrongly recorded it as absent. **Hardware:** Apple M5 Max, 18 cores,
128 GB, macOS 25.5.0 arm64.

## Why not the usual corpus

The 10,378-frame agreement corpus is **the set of frames ASTAP solved** -- it
is built from ASTAP's own `measurement` rows. ASTAP scores 100% on it by
construction, so "psolve solves 97.74% of it" measures psolve's *coverage of
ASTAP*, not a contest. It cannot show ASTAP losing, ever.

So this runs on the population where neither tool is known to have succeeded:
**873 LIGHT frames with no ASTAP measurement row**, an intact location, and a
commanded pointing. 200 were sampled (seed 20260823).

**A missing row is not a proven failure.** `catalogue.db` records successes, so
"ASTAP failed" and "never attempted" are indistinguishable in it. The honest
description of this population is *frames ASTAP has no answer for*. What the
run below establishes is what each tool does when actually asked.

## Method

Both tools at their production invocations, each frame copied to scratch first
(`~/astroops` is read-only and ASTAP writes its `.ini` beside the input).

```sh
# ASTAP -- ingest/identify.py's order: computed -fov first, -fov 0 only on failure
astap_cli -f <frame> -r 15 -fov <computed> -d ~/astap -ra <hours> -spd <dec+90>
astap_cli -f <frame> -r 15 -fov 0        -d ~/astap -ra <hours> -spd <dec+90>

# psolve -- its own header-derived defaults
psolve solve <frame> --index gaia-dr3-g14-dec45-nside64.psidx --hint <ra>,<dec>
```

**One choice that counts against ASTAP, stated rather than buried:** the
`-fov 0` autodetect leg is capped at 60 s. Uncapped, it exceeded 180 s on the
first pilot frame. A solve that slow is a stall for an automated pipeline
whether or not it would eventually succeed. **2 of 200** hit the cap; they are
reported separately below and counted as failures.

## Result

| | ASTAP | psolve |
|---|---:|---:|
| solved | **21 / 200 (10.5%)** | **67 / 200 (33.5%)** |
| both solved | 12 | 12 |
| solved alone | 9 | **55** |
| neither | 124 | 124 |
| timed out | 2 | 0 |
| needed the `-fov 0` fallback | 194 / 200 | n/a |
| wall, median | 1,741 ms | **72 ms** |
| wall, mean | 4,804 ms | **61 ms** |
| wall, max | 160,480 ms | **119 ms** |
| wall, total | 960.7 s | **12.1 s** |

Where **both** solved (n=12) they agree to a median **0.602"**, max 2.536".
There is no accuracy difference between the tools on this population; the
difference is entirely in *whether* they answer.

## The headline is one rig, and saying otherwise would be wrong

| rig | n | ASTAP | psolve |
|---|---:|---:|---:|
| **ATR585M (= sv555, the PRIMARY rig)** | 64 | **11** | 6 |
| DWARFIII | 46 | 10 | 11 |
| SVBONY SV405CC | 90 | **0** | **50** |

**psolve's 3.2x advantage is entirely the SV405CC.** On the ATR585M, ASTAP
solves nearly twice as many as psolve. On the DWARFIII they are level.

**Naming, corrected 2026-08-24 by the astroops session: `sv555` and
`ATR585M` are the same instrument** -- `sv555` names the telescope/rig,
`ATR585M` the camera on it, and N.I.N.A. writes the path camera-over-scope
(`astro-outbox/ATR585M/SV555/...`). It is **the primary rig**: what shoots
most nights, what `plan_tonight.py --rig` defaults to, and what the depth
cutover gate was measured on.

That reframes this row rather than changing it. The rig where ASTAP leads is
not one camera of three -- it is the instrument everything else depends on.

The SV405CC column is this deployment's `XPIXSZ` ambiguity: the driver writes
the already-binned pixel size, so a formula multiplying by `XBINNING` doubles
the scale. psolve's binning retry -- merged today -- corrects the scale *and*
refetches the catalogue at the corrected radius. ASTAP is given the same
ambiguous header and its `-fov 0` autodetect does not recover on these frames.
That is one defect class on one camera, not a general superiority.

## Are psolve's 55 extra solves real?

Solving more is only a win if the answers are right, and there is no ASTAP
answer to check against -- that is the point of the population. Checked
instead against the mount's **commanded pointing**, re-solving all 55:

| | value |
|---|---|
| separation from commanded, median | **4.69'** |
| p90 / max | 22.48' / **33.44'** |
| beyond 1 deg (suspicious) | **0** |
| log-odds, median / min | 573.8 / **132.7** |

Commanded pointing is a weak reference -- the mount's own error measures
median 12.65', p90 24.32', max 27.60' -- so this cannot confirm a solve to
arcseconds. It **can** catch the failure that matters: a confident position
nowhere near where the telescope was aimed, which is the shape of the 87.77-deg
blind-solve incident. Nothing is near that. The worst is 33.44', consistent
with mount error, and the weakest acceptance is 132.7 decades against a gate of
12.0 -- these are not marginal accepts.

**The 55 extra solves are real.**

## So who is winning

- **On frames ASTAP already solves** (the 10,378-frame corpus): nobody. psolve
  reproduces ASTAP's answer on 97.74% of them to a 0.539" median. Two frames
  disagree by more than 30" and both were adjudged **ASTAP's** error, but two
  frames is not a verdict on a solver.
- **On frames ASTAP has no answer for:** psolve, 3.2x, and the extra solves
  verify as genuine. But the margin is one camera. Take the SV405CC out and
  ASTAP is ahead 21 to 17 -- and the rig ASTAP leads on is the primary one.
- **On speed, on this population:** psolve, by 79x on the mean. That number is
  about failure cost, not solve speed -- ASTAP spends seconds to minutes
  deciding it cannot solve a frame psolve refuses in 72 ms. On frames that do
  solve the gap is far smaller.
- **On capability:** level, as of today. Blind solving was ASTAP's last
  exclusive advantage and merged on 2026-08-23.

The fair summary is that psolve has closed the gap and now wins on this
deployment's specific hardware quirk, not that it is a better solver. ASTAP
beating it 11 to 6 on the ATR585M is in the same table and is the part worth
acting on.

## Reproduce

`.scratch/headtohead/` (gitignored): `run.py`, `verify_psolve_only.py`,
`unmeasured.psv` (the 873-frame population), `h2h-200.ndjson` (raw per-frame
results), `psolve-only-verified.json`. `~/astroops/` was never written --
every frame was copied to a temp dir before either tool touched it.

---

# Follow-up, 2026-08-24: the ATR585M (sv555) full population

The 64-frame result above is a random sample. The astroops session asked for
the full picture on this rig because it is **the primary instrument** -- so all
**287** ATR585M frames in the unmeasured population were run. No sampling.

| | ASTAP | psolve |
|---|---:|---:|
| solved | **60 / 287 (20.9%)** | 44 / 287 (15.3%) |
| both | 41 | 41 |
| **solved alone** | **19** | **3** |
| neither | 224 | 224 |
| timed out | 6 | 0 |
| wall, median | 654 ms | **73 ms** |
| wall, mean | 4,650 ms | **75 ms** |
| wall, max | 161,251 ms | **114 ms** |

**ASTAP wins on the primary rig at population scale: 60 to 44.** The 64-frame
sample said 11 to 6 (ratio 1.83); the full population says 1.36. The gap is
narrower than the sample suggested and it is unambiguously real.

The sharper number is **19 against 3**: nineteen frames ASTAP solves and psolve
cannot, against three the other way.

Where both solve, they agree to a median **0.354"** (max 14.013"). As on every
other population measured, there is no accuracy difference -- only a difference
in whether an answer is produced.

## The mechanism, and a correction to the earlier reading

psolve's 243 failures break down as **`TOO_FEW_STARS` 183 (75.3%)**,
`NO_QUAD_MATCH` 58, `LOW_CONFIDENCE` 2.

It would be easy to read that as "the star floor is what loses us the rig". It
is not, and the distinction matters:

- **The 224 frames NEITHER tool solves** are where `TOO_FEW_STARS` lives. Both
  solvers agree those frames are unsolvable. That is not a competitive loss.
- **The 19 frames psolve actually loses** report `NO_QUAD_MATCH` 18,
  `LOW_CONFIDENCE` 1 -- **not one `TOO_FEW_STARS`**.

So the losing frames are ones where psolve extracts enough stars to attempt
quads and then fails to match. Dissecting six of them individually
(2026-08-24) shows why, and it is still extraction: the fixed `min_pix = 4`
floor rejects **76-96%** of detections, leaving **8-22 usable stars** -- past
the `TOO_FEW_STARS` threshold, so a quad search is attempted, but far too few
to find a consistent transform.

```
frame   detected   used   too_small   elongated
14951        358      9         349           0
15154        244     22         198          24
13144        324      8         308           7
15186        266      8         254           4
```

**The reason code says `NO_QUAD_MATCH`; the cause is the star count.** That is
worth stating plainly because acting on the reason code alone would send
someone to the matcher, which is not where the defect is.

ATR585M is the finest-sampled rig in the fleet at **2.46"/px** against
DWARFIII's 2.75" and the SV405CC's 3.93"/7.86", so its stars span the fewest
pixels and a fixed pixel floor bites hardest there. Solve rate across the fleet
tracks plate scale: ATR585M 87.6%, DWARFIII 99.0%, SV405CC bin2 99.9%.

## What does NOT fix it

Relaxing the floor by hand rescues **2 of 6**:

| | default | `--min-pix 2` | `--min-pix 2 --max-ellipticity 0.9` |
|---|---|---|---|
| 14866 | no | no | **SOLVED** |
| 15154 | no | no | **SOLVED** |
| 14951, 13144, 15186, 15145 | no | no | no |

Lowering `--sigma` does not help either -- it multiplies detections 266 to
51,796 while `used` rises only 8 to 28, because the extra blobs are
single-pixel noise that `too_small` then rejects.

So there is a **second cause behind four of the six** that this diagnosis has
not identified. Anyone picking this up should not assume the floor is the whole
story.

**The fix is scale-aware extraction** -- deriving `min_pix` from the plate
scale rather than a constant -- and it needs its own corpus measurement, not
tuning against six frames. The last change to that floor *improved* fit RMS
while dropping solve rate **85.8% to 69.4%**, because `min_pix = 4` had been
implicitly co-tuned with the coarse double-binned plate scale. This is a
milestone, not a constant change.

## Standing conclusion

psolve wins on the SV405CC by a wide margin, loses on the ATR585M, and is level
on the DWARFIII. The rig it loses on is the primary one. **A switch-over
treating psolve as uniformly better would degrade the instrument that matters
most** -- which is why the deployment landing on 2026-08-24 is a ride-along
beside ASTAP with both filing rows independently, not a replacement.
