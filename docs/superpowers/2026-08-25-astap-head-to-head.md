# ASTAP vs psolve, re-run after the pair-matching retry

**Date:** 2026-08-25. **psolve:** `efd7b3e` (pair-match retry, grid neighbour
search, `--max-mag`), release build. **ASTAP:** `astap_cli` CLI-2026.06.29,
`d50` database at `~/astap`. **Hardware:** Apple M5 Max, 18 cores, 128 GB,
macOS 25.5.0 arm64.

**Identical 200-frame sample to the 2026-08-23 run** -- reproduced from
`unmeasured.psv` with the same seed and verified frame-id for frame-id
(200/200). Both tools were re-run; ASTAP's numbers are not carried over.

Population and method unchanged from `2026-08-23-astap-head-to-head.md`: the
873 LIGHT frames with **no ASTAP measurement row**, an intact location and a
commanded pointing. The usual corpus cannot host a contest -- it is built from
ASTAP's own successes, so ASTAP scores 100% on it by construction.

```sh
# ASTAP -- ingest/identify.py's order: computed -fov first, -fov 0 on failure
astap_cli -f <frame> -r 15 -fov <computed> -d ~/astap -ra <hours> -spd <dec+90>
astap_cli -f <frame> -r 15 -fov 0          -d ~/astap -ra <hours> -spd <dec+90>

# psolve -- its own header-derived defaults
psolve solve <frame> --index gaia-dr3-g14-dec45-nside64.psidx --hint <ra>,<dec>
```

The `-fov 0` leg is capped at 60 s, as before; 2 frames hit the cap and are
counted as failures.

## Every solve was checked, not just counted

A tool that reports `PLTSOLVD=T` has not necessarily solved the frame. Both
tools' answers were checked against the commanded pointing -- a weak reference,
good to a fraction of a degree on these mounts, but more than sufficient to
catch an answer that is degrees away.

| | reported | **wrong (>1 deg from commanded)** | correct |
|---|---:|---:|---:|
| ASTAP | 21 | **2** | 19 (9.5%) |
| psolve | 96 | **0** | 96 (48.0%) |

**Both of ASTAP's wrong answers came from the `-fov 0` autodetect leg, and
both reported `PLTSOLVD=T`.** Frame 6538 is M 8, commanded at
RA 271.069 Dec -24.390:

    psolve : RA 270.996  Dec -24.379   0.067 deg from commanded   (M 8)
    ASTAP  : RA 279.935  Dec -17.889  10.513 deg from commanded

Frame 6502 is the same failure, 10.494 deg out -- and it is **one of ASTAP's
two solo wins**, so on inspection ASTAP wins exactly **one** frame psolve does
not.

The failure is confined to that leg. Splitting ASTAP's 21 solves by which one
answered:

| leg | solves | wrong |
|---|---:|---:|
| computed `-fov` (tried first) | 6 | **0** |
| `-fov 0` autodetect (only on failure) | 15 | **2** |

The 6 are exactly the 6 frames of 200 that never needed the fallback. So the
right statement is narrower than "ASTAP is unreliable": its reference value is
good where the first leg answers, and the two wrong answers are confined to
the autodetect leg the production invocation reaches only after the computed
one has already failed.

**Six is a small denominator, though**, and this document will not lean on it.
"All 6 correct" is consistent with the computed leg being sound but is not
strong evidence of it -- this population was selected as frames ASTAP has no
recorded answer for, which is precisely where the computed leg would be
expected to struggle, and 194 of 200 falling through to `-fov 0` is that
selection showing. On an ordinary population the split would not look like
this.

This is the failure this project designs against, observed in the other tool:
not a crash, a confident wrong answer that an automated pipeline would file as
truth. psolve's own equivalent gate is `verify`, and on the same 96 solves it
let nothing through: median **0.079 deg** from commanded, max 0.913 deg.

## Result

| | ASTAP | psolve (now) | psolve (Aug 23) |
|---|---:|---:|---:|
| correct solves | **19 / 200 (9.5%)** | **96 / 200 (48.0%)** | 67 / 200 (33.5%) |
| solved alone | **1** | **77** | 55 |
| both | 18 | 18 | 12 |
| neither | 102 | 102 | 124 |
| wrong answers | **2** | **0** | 0 |
| timed out | 2 | 0 | 0 |
| needed `-fov 0` | 194 / 200 | n/a | n/a |
| wall, median | 1,826 ms | **62 ms** | 72 ms |
| wall, mean | 4,876 ms | 747 ms | 61 ms |
| wall, max | 160,632 ms | 5,439 ms | 119 ms |
| wall, total | 975.3 s | **149.4 s** | 12.1 s |

**Since 2026-08-23 psolve gained 29 frames and lost none.**

Where both solved and ASTAP was correct (n=18) they agree to a median
**1.194"**, p90 14.4", max 54.2"; 15 of 18 inside 5".

## The cost, stated plainly

psolve's failure path got much more expensive, and the mean and max above are
where it shows.

| psolve outcome | n | median | p90 | max | total |
|---|---:|---:|---:|---:|---:|
| solved | 96 | 62 ms | 2,868 ms | 5,439 ms | 51.5 s |
| `TOO_FEW_STARS` | 72 | 51 ms | 72 ms | 74 ms | 3.4 s |
| **`NO_QUAD_MATCH`** | 29 | **3,286 ms** | 3,799 ms | 4,130 ms | **94.1 s** |
| `LOW_CONFIDENCE` | 3 | 93 ms | 214 ms | 214 ms | 0.4 s |

A `NO_QUAD_MATCH` used to cost ~70 ms and now costs **3.3 seconds**: pair
matching has no early abort, so a hopeless frame is charged the full
hypothesis sweep before giving up. Those 29 frames are **63% of psolve's
entire wall time** for the run.

The median is unchanged at 62 ms because a frame that solves through quads
never reaches the retry. But "psolve fails fast" is no longer true, and for a
pipeline that is a real property to have lost. The fix is an early abort --
the sweep already knows its best inlier count and its runner-up, so a frame
whose best hypothesis is not pulling away is one to give up on rather than
grind to the ceiling. Not attempted here; it needs its own measurement, since
an abort that fires too early takes back some of the 29 frames gained.

Against that, ASTAP's own tail is worse: its slowest frame took **160.6 s**,
and it needed the `-fov 0` fallback on 194 of 200 frames, which is where both
its wrong answers and its worst latencies come from.

---

## Re-run at `0579c33`, after the tight-radius rung

The measurement above was taken at `c91cd0a`. The tight-radius retry landed
afterwards in `6335179`, and 29 of these 200 frames had failed with
`NO_QUAD_MATCH` -- exactly what that rung targets -- so the 96 was expected to
be a floor. Re-run on the **same 200 frames**, both tools again:

| | ASTAP | psolve @ `c91cd0a` | psolve @ `0579c33` |
|---|---:|---:|---:|
| reported | 21 | 96 | **113** |
| **wrong (>1 deg from commanded)** | **2** | 0 | **0** |
| correct | 19 (9.5%) | 96 (48.0%) | **113 (56.5%)** |

**psolve gained 17 frames and lost none.** All 113 verified against the
commanded pointing: median **0.076 deg**, p90 0.213, max 0.913, **none beyond
1 deg**. `LOW_CONFIDENCE` has disappeared from the failure reasons entirely.

ASTAP is unchanged at 21 reported / 19 correct -- same binary, same database,
re-run rather than carried over.

### Which rung answered

| rung | solves |
|---|---:|
| first attempt (`header`) | 70 |
| matched-filter re-extraction | 5 |
| pair matching | 21 |
| **tight search radius** | **17** |

The two rungs added on 2026-08-25 -- pair matching and tight radius -- account
for **38 solves, twice ASTAP's entire correct total of 19**. That is 33.6% of
psolve's 113, 40.4% of its 94-solve margin over ASTAP, and **88.4% of
everything beyond the first attempt** (38 of 43).

Every rung is reached only after the ones above it have failed, so none of the
70 first-attempt solves could have been affected by adding them. The
architectural property is measured here rather than asserted.

> **Correction.** This paragraph first read "two thirds of psolve's margin
> over ASTAP comes from rungs that did not exist a day earlier", and the
> commit that introduced it (`d78e50f`) carries that phrasing in its message.
> **No reading of the numbers gives two thirds**: 38/94 is 40.4%, 43/94 is
> 45.7%, 38/113 is 33.6%, 38/43 is 88.4%. Two thirds of a 94-solve margin
> would be 62.7 solves and the new rungs produced 38. It was a ratio asserted
> without being computed -- in a document whose subject is a tool reporting
> confident wrong answers -- and it was caught by the peer session
> maintaining the vault notes, not by me. The figures above are each stated
> with their denominator so the reading cannot drift again.

### The cost is still real

| | @ `c91cd0a` | @ `0579c33` |
|---|---:|---:|
| wall median | 62 ms | 64 ms |
| wall mean | 747 ms | 804 ms |
| wall max | 5,439 ms | 6,404 ms |
| wall total | 149.4 s | 160.8 s |

11 seconds more across 200 frames, for 17 more solves. The remaining 87
failures are **72 `TOO_FEW_STARS`** (which fail in ~50 ms) and 15
`NO_QUAD_MATCH` (which pay the full ladder). ASTAP over the same run: median
1,836 ms, max 163.1 s, total 985.4 s.

> **Which run is which.** These are NOT the figures in the summary table above,
> and the difference is a separate re-run rather than a correction. The table
> reports `c91cd0a` (psolve 62 ms, ASTAP 1,826 ms / 975.3 s); this section
> reports `0579c33` (psolve 64 ms, ASTAP 1,836 ms / 985.4 s). Neither
> supersedes the other. Flagged 2026-08-27 after a review found the README
> quoting one pair in one section and the other pair in another, with nothing
> saying they came from different runs.
>
> **And neither pair is a speed comparison.** psolve's median is over the
> frames it SOLVED; ASTAP's is over all 200, of which ~180 were not correct
> solves, so it is dominated by the cost of a search giving up -- its 160.6 s
> max is an exhausted search. Right numbers, different populations. The
> like-for-like basis is the 18 frames both tools solved, which is too small to
> carry a claim.
