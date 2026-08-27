# M3 spike: the first real frame

**Date:** 2026-08-14
**Frame:** `~/astroops/library/eagle/lights/H/2026-07-29_22-47-02_H_120.00s_100g_1x1_0001_-10.00.fits`
**Index:** `gaia-dr3-g14-dec45-nside64.psidx` (14,631,960 stars, 0.22 GB)
**Machine:** macos-arm64 (M-series, 14 cores)

Measured before planning M3, because the milestone exists to settle exactly
one question: does this work on real sky, and is it faster than ASTAP?

## It solves, and it agrees with ASTAP

At `--radius 1.55 --cat-limit 1500`:

| Quantity | psolve | ASTAP | Delta |
|---|---|---|---|
| Centre at pixel (1920.5, 1080.5) | 274.689608, -13.811199 | 274.689087, -13.810971 | **2.0 arcsec (0.81 px)** |
| CD determinant | +4.644e-07 | +4.643e-07 | parity agrees |
| Plate scale | 2.4533 "/px | - | 0.33% off header optics |
| Fit RMS | 0.969" (0.4 px) | - | - |
| Reprojected matches | 447 of 500 used | - | - |
| Confidence | 968 decades log-odds | - | - |

3 of 4 real frames solve. The physics is correct.

## But the speed premise does not hold

**230 ms against ASTAP's 180 ms** on the same frame and machine, consistent
across runs, versus the design's projected 12-14 ms.

Per-stage breakdown (avg of 3 release runs):

| decode | background | extract | quads | catalogue | match | fit | verify | total |
|---|---|---|---|---|---|---|---|---|
| 4.4 | 32.6 | 34.1 | **145.4** | 0.04 | 10.7 | 0.006 | 0.15 | 229 ms |

**Quad building is 63% of the solve.** The catalogue fetch is 0.04 ms —
so the "smaller custom index means less to sort through" premise was never
where the time went. That argument is dead; the index size advantage is real
for memory and portability, not for speed.

Two causes, both fixable:

1. **Massive over-generation.** `build_quads` clamps `k` to 12 and emits the
   full C(12,3) = 220 combinations per seed star: 500 image seeds -> 110k
   quads, 1500 catalogue seeds -> 330k. About **440,000 quads built to keep
   1,200** (`max_quads` 600 each side). Full combinations were a deliberate
   M2 ruling (overlap under noise beats sliding-window: 92-97% vs 80-85%
   recall) - so the fix is to stop generating what will be discarded, not to
   go back to windows.
2. **Single-threaded.** `psolve-core` has no rayon; quad building is
   embarrassingly parallel across seed stars and currently uses one core of 14.

## One default prevents a real frame from solving

At defaults the frame returns `NO_QUAD_MATCH`. Isolating the two defaults
shows `--cat-limit` was never at fault: it already resolves to 1500, which is
the value that works. Only `--radius` is wrong.

Sweep with cat-limit left at its default:

| radius | catalogue stars | result |
|---|---|---|
| 1.40 | 1500 | solved, **492 matched** |
| 1.55 | 1500 | solved, 447 matched |
| 1.70 | 1500 | solved, 347 matched |
| 1.85 | 1500 | solved, 266 matched |
| **1.977 (default)** | 1500 | **NO_QUAD_MATCH** |

Monotonic dilution, with the shipped default sitting just past the cliff.

`default_radius_deg` is `field_height_deg + 0.5`. For this 3840x2160 frame at
2.4533 "/px the field is 2.617 x 1.472 deg, so the default is 1.972 deg and the
disc is 12.2 deg^2 against a 3.85 deg^2 frame. Most catalogue quads are then
built from stars the frame cannot see - the set-mismatch regime the M2 closed
loop already measured as failing.

The geometrically correct radius is **half the field diagonal** - the disc has
to reach the frame's corners and no further. Here that is
sqrt(2.617^2 + 1.472^2)/2 = 1.50 deg, which lands in the region that solves
with the most matches.

## Fixed in 6255958

- `field.center` reported `pix_to_radec(crpix)`, which **is** CRVAL by
  definition, and `fit_tan` pins CRVAL to the caller's hint - so every solve
  echoed the hint back. It made this 2.0" solve look 40" wrong. Now
  `pix_to_radec(nx/2, ny/2)`.
- Per-stage timings added (spec 7.2 required them; T13 omitted them).
  `std::time::Instant` needs no dependency and does not trip the
  `no_filesystem` guard, so no clock injection was needed.

## Carried into the M3 plan

- Fix `default_radius_deg` to half the field diagonal; add a real-frame
  regression test so "solves at defaults" cannot silently regress.
- Expose the extraction knobs. `min_pix`, `k_sigma` and `keep` are not
  reachable from the CLI, so a frame rejected by the sensitivity floor cannot
  be rescued by the user at all.
- Quad generation is the optimisation target, not the catalogue.
- `.gitignore` lines 10-11 ignore `*.ini` and `*.wcs` - the exact sidecar
  formats M3 must write and test. Needs a negation for the fixtures dir
  before any reference sidecar is committed, or it will vanish silently.
- The fourth frame's failure is diagnosed: same rig, same 2.453 "/px scale,
  same parity, and ASTAP solved it, so the frame is solvable. psolve gets only
  **95 usable stars** from 1491 detections because `min_pix: 4` rejects 1373 as
  `too_small`. It is a sparse-field sensitivity failure (thin cloud or moon),
  not a geometry bug. Fix before the 9,495-solve agreement run, or the run
  measures the wrong thing.

## Sparse frame (Task 4)

**Frame:** `~/astroops/library/eagle/lights/H/2026-08-11_22-26-00_H_120.00s_100g_1x1_0001_-9.90.fits`
**Ground truth (ASTAP):** CRVAL1 274.7273080441, CRVAL2 -13.84718397251

At the extraction defaults (`min_pix=4`, `k_sigma=5.0`) this frame detects
1491 stars and keeps only 95 -- 1373 rejected `too_small` -- against 3058
detected / 500 used on the sibling frame that solves cleanly. The
task-brief premise, checked by hand before this task, was that no radius
(1.2-1.55 deg) or cat-limit (1500-4000) rescues it.

### Step 1: the min-pix x sigma sweep, cat-limit pinned at 1500

Pinning `--cat-limit 1500` isolates the extraction knobs from the
catalogue-depth knob so the grid below varies one thing at a time (per this
task's own instructions -- 1500 is the value that already works on the
sibling frame). Extended afterward to 5x6 (min-pix 2-6, sigma 2.5-5.0) for a
clearer picture of the floor's shape:

| min-pix | sigma | detected | used | solved |
|---|---|---|---|---|
| 2 | 2.5 | 91346 | 349 | false (NO_QUAD_MATCH) |
| 2 | 3.0 | 31012 | 190 | false |
| 2 | 3.5 | 10142 | 174 | false |
| 2 | 4.0 | 3941 | 146 | false |
| 2 | 4.5 | 2122 | 128 | false |
| 2 | 5.0 | 1491 | 111 | false |
| 3 | 2.5 | 91346 | **359** | false |
| 3 | 3.0 | 31012 | 197 | false |
| 3 | 3.5 | 10142 | 174 | false |
| 3 | 4.0 | 3941 | 146 | false |
| 3 | 4.5 | 2122 | 128 | false |
| 3 | 5.0 | 1491 | 111 | false |
| 4 | 2.5 | 91346 | 230 | false |
| 4 | 3.0 | 31012 | 156 | false |
| 4 | 3.5 | 10142 | 144 | false |
| 4 | 4.0 | 3941 | 122 | false |
| 4 | 4.5 | 2122 | 110 | false |
| 4 | 5.0 (default) | 1491 | 95 | false |
| 5 | 2.5 | 91346 | 178 | false |
| 5 | 3.0 | 31012 | 136 | false |
| 5 | 3.5 | 10142 | 120 | false |
| 5 | 4.0 | 3941 | 102 | false |
| 5 | 4.5 | 2122 | 92 | false |
| 5 | 5.0 | 1491 | 77 | false |
| 6 | 2.5 | 91346 | 155 | false |
| 6 | 3.0 | 31012 | 119 | false |
| 6 | 3.5 | 10142 | 94 | false |
| 6 | 4.0 | 3941 | 86 | false |
| 6 | 4.5 | 2122 | 80 | false |
| 6 | 5.0 | 1491 | 65 | false |

`used` rises to **359** (min-pix=3, sigma=2.5), well above 95 -- the
sensitivity floor is not the whole story. Below sigma=2.5 (checked at 2.0
and 1.5) detection count explodes to 233k-463k, `used` saturates at the
`keep` cap of 500, and it is pure noise, not stars: at that point the
extractor is no longer measuring the sky. The best *real* combination
(min-pix=3, sigma=2.5, used=359) was then swept across radius (1.2-1.8 deg)
x cat-limit (1500-4000) -- **24 additional runs, all NO_QUAD_MATCH.**
Pinning cat-limit at 1500 (or higher) never solves this frame at any
extraction setting tried.

### Step 2: the actual fix -- don't pin cat-limit at all

Running the *unmodified* CLI defaults (`psolve solve <file> --index <idx>`,
no flags) solves it:

```
solving ...: 300 catalogue stars within 1.6569 deg of 274.7333,-13.8453
"solved":true, "used":95, "matched":53, "log_odds":141.98
field.center = (274.7267524166, -13.8478337193)
```

Separation from ASTAP's CRVAL: **3.04 arcsec** -- well inside the 30" failure
bar and the regression test's 10" bar. Deterministic across repeated runs.

The difference from every failing row above is `--cat-limit`: this run used
the *auto-sized* value, not a pinned one. `default_cat_limit` sizes itself
from this frame's own detected-star count under default extraction params
(95), giving `(95*3).clamp(300, 5000) = 300`. Sweeping cat-limit alone at
default extraction params (min-pix=4, sigma=5.0) finds a sharp boundary:

| cat-limit | solved | matched |
|---|---|---|
| 300 | true | 53 |
| 350 | true | 55 |
| 400 | true | 53 |
| 450 | false | - |
| 500 | false | - |
| 600-4000 | false | - |

More catalogue stars make this frame's already-sparse quad set *worse*, not
better -- exactly the "true quads drown in false ones" reasoning that
`default_cat_limit`'s own doc comment gives for sizing the catalogue depth to
the frame rather than maximising it. The frame was never unsolvable; earlier
manual checks (including this task's own brief) pinned or swept cat-limit at
values (1500-4000) that all sit past the cliff.

**Outcome: 1** (a knob combination solves it correctly) -- but the knob is
the *existing*, unmodified `default_cat_limit` auto-sizing, not a new
`ExtractParams` default. No code change was needed. `ExtractParams::default()`
is untouched; `min_pix=4`/`k_sigma=5.0` (the shipped defaults) are exactly
what solved it. The frame is added to
`crates/psolve-cli/tests/real_frames.rs`'s `real_frames_solve_at_defaults`
regression alongside the sibling frame, both solving at true CLI defaults.

### Step 4: regression across the eagle library

All 5 frames in `~/astroops/library/eagle/lights/*/*.fits` solve at CLI
defaults (no flags beyond `--index`): **5/5 (100%)**, unchanged before and
after this task since no default was modified.

## Task 11: the agreement run — GATE FAILED (one gross error — and it is ASTAP's, not psolve's)

**Current status: GATE FAILED, on its own criterion, on a frame where
direct measurement shows ASTAP's own solution is wrong.** The original run
below (GATE PASSED) measured `field.center` through a half-pixel bug;
corrected, one frame's separation is 30.34″, 0.34″ over the
`MAX_GROSS_ERRORS = 0` bar. A further check (reprojecting catalogue stars
through each WCS and reading real pixel flux) found that on that specific
frame, ASTAP's solution — not psolve's — is the one that does not describe
where the stars actually are. The gate's threshold is unchanged and its
FAIL stands on its own terms (it measures disagreement-with-ASTAP, and the
disagreement is real); what changes is the record no longer implies psolve
is the one that erred. The original (superseded) run, the fix round's
corrected run, and this further finding are all recorded here in full —
**including the 300-frame sample's own gate result at each stage** — so a
reader sees the sample outcome as well as the full run's, not only the
number that ended up passing.

**9495 of 9495 ASTAP-solved frames attempted** (full run, no subset), CFA
weighted (8396 CFA / 1099 mono by `BAYERPAT` header presence — corrected
from an initial 8190/1305 split by filter name; see below). Full detail:
`.superpowers/sdd/2026-08-14-m3-astap-compat/task-11-report.md`.

### Original run (superseded — `field.center` had a half-pixel bug)

**300-frame sample: GATE FAILED at this stage too**, on solve rate alone
(0.9467 < 0.95; every other check passed) — judged sane per the brief's own
bar (no crashes, no gross errors, no scale outliers) and the full run
proceeded per the brief's instruction, but the sample's own gate result was
FAIL, not PASS, and is recorded here rather than only in the task report.

| Quantity | Sample (300) | Full (9495) | Gate | Full pass? |
|---|---|---|---|---|
| Solve rate | 284/300 = 94.67% (FAIL) | 9219/9495 = 97.09% | ≥ 95% | yes |
| Median separation vs ASTAP | 1.679″ | **1.68″** | ≤ 5.0″ | yes |
| p99 separation | 3.383″ | **3.49″** | ≤ 30″ | yes |
| Max separation | 28.599″ | 28.60″ | — | (below 30″) |
| Gross errors (>30″) | 0 | **0** | ≤ 0 | yes |
| Parity mismatches | 0 | **0** | ≤ 0 | yes |

Full-run gate at this stage: **PASS** (later found to rest on a biased
measurement — see below).

### The bug, and the fix

A fix-round review found `field.center` (`cmd_solve.rs`) evaluating
`pix_to_radec(nx/2, ny/2)` — FITS's 1-based image centre — when psolve's
internal pixel coordinates are 0-based (`extract.rs` centroids over array
indices `0..nx`). Off by half a pixel on each axis: at this corpus's
~2.5″/px typical scale, roughly a **1.6″ systematic bias** — almost exactly
the 1.68″ median the original run measured. The signature was in the
original numbers already: the header-WCS subset's separation distribution
had p90/median = 1.06, the fingerprint of a constant offset, not scatter.
The same bug's sibling — `sidecar.rs`'s `.ini`/`.wcs`/`-update` writers
taking the internal 0-based `crpix` and writing it raw into 1-based FITS
`CRPIX1`/`CRPIX2` cards, no `+ 1.0` — meant every sidecar and every
`-update`'d header this crate ever wrote had CRPIX off by exactly one
pixel, invisible to byte-exact tests built from hand-transcribed
(already-1-based) fixture literals rather than a real solve's output.

Fixed: `field.center` now evaluates at `((nx-1)/2, (ny-1)/2)`; the two
sidecar-writing functions now add `+ 1.0` to `crpix` at the point of
formatting (documented as the CRPIX convention in `sidecar.rs`'s module
doc). Pinned by a **new test driven by a real solve** (not a transcribed
fixture): `real_frames.rs`'s
`sidecar_crpix_is_one_based_and_agrees_with_astap_on_a_real_solve` solves a
real frame through both native and ASTAP mode and asserts the sidecar's
CRPIX equals native mode's raw `wcs.crpix` plus exactly 1.0, corroborated
against that same frame's real ASTAP header CRPIX. Verified load-bearing by
reverting the fix and confirming both this test and the pre-existing
byte-exact fixture tests fail, then pass again restored.
`cargo test --workspace`: 362 passed (361 baseline + 1), clippy clean.

### Re-derived, corrected numbers

**300-frame sample (corrected sampler too — see below): GATE PASSED.**

| Quantity | Result | Gate | Pass? |
|---|---|---|---|
| Solve rate | 289/300 = 96.33% | ≥ 95% | yes |
| Median separation | 0.582″ | ≤ 5.0″ | yes |
| p99 separation | 2.831″ | ≤ 30″ | yes |
| Max separation | 7.683″ | — | (below 30″) |
| Gross errors | 0 | ≤ 0 | yes |
| Parity mismatches | 0 | ≤ 0 | yes |

**Full run (9495 frames): GATE FAILED**, on `gross_errors` alone.

| Quantity | Result | Gate | Pass? |
|---|---|---|---|
| Solve rate | 9219/9495 = **97.09%** | ≥ 95% | yes |
| Median separation vs ASTAP | **0.53″** (was 1.68″) | ≤ 5.0″ | yes |
| p99 separation | **3.13″** | ≤ 30″ | yes |
| Max separation | **30.34″** (was 28.60″) | — | over 30″ |
| Gross errors (>30″) | **1** | ≤ 0 | **no** |
| Scale outliers (>5% vs header optics) | **0** | — | — |
| Parity mismatches | **0** | ≤ 0 | yes |

Median separation dropped 3.2x (1.68″ → 0.53″), confirming the fix. But one
frame — `SVBONY_SV405CC/NGC_3372/.../2025-05-21_21-21-32__-10.00_90.00s_0050.fits`
— crosses the 30″ gate at 30.338″. It was at 27.8″ before the fix, already
the closest call in that run; the correct, bug-free centre calculation
reveals it sits just over the line, not under it.

**A second fix round settled which tool is actually wrong on this frame, by
direct measurement: it is ASTAP, not psolve.** Reprojecting Gaia catalogue
stars through each candidate WCS and reading pixel flux at the predicted
positions, checked against two session-neighbour control frames so the
metric's own reliability is visible rather than assumed:

| frame | psolve↔ASTAP sep | psolve WCS: peak / on-light | ASTAP WCS: peak / on-light |
|---|---|---|---|
| 0048 (control) | 1.70″ | 4090 ADU / 99.7% | 4012 ADU / 99.6% |
| **0050 (failing)** | **30.33″** | **4616 ADU / 99.8%** | **124 ADU / 30.5%** |
| 0055 (control) | 1.40″ | 4032 ADU / 99.7% | 3548 ADU / 99.5% |

Background is ~60 ADU. Both WCS solutions score equally well on the two
controls (the metric agrees with both tools when they agree with each
other); on frame 0050, psolve's WCS still lands 99.8% of reprojected stars
on real light while **ASTAP's WCS puts 69.5% of them at background**.
Corroborated four ways: re-hinting psolve at ASTAP's own centre still
converges to psolve's answer; a cubic fit through this session's other 147
solved frames predicts the true pointing to within 8.0″ of psolve and
32.6″ of ASTAP; ASTAP's own `.ini` for this frame is internally sheared
(`CDELT` axis mismatch 0.41% vs 0.02-0.05% on neighbouring frames, `CROTA`
split -0.131° vs -0.056°); and a fresh `astap_cli` run today reproduces the
same wrong answer, ruling out a stale database row. The field is NGC 3372
(Carina) on the galactic plane, 43,951 detected sources — plausibly where a
coarse `d50`-resolution catalogue mis-associates a star, though that
mechanism is not confirmed beyond the evidence above. The next-largest
separations (27.25″, 25.59″, 22.28″, 15.39″, all 15s DWARFIII sub-exposures
of the same Carina/M8 region) have too little signal for this reprojection
check to discriminate reliably and are recorded as **undetermined**, not as
further confirmed ASTAP errors.

`MAX_GROSS_ERRORS = 0` was set before any run and is unchanged: the gate
measures *disagreement with ASTAP*, not correctness, and on its own terms
it correctly reads **FAIL** — psolve's answer for frame 0050 really is
30.3″ from what `catalogue.db` records as ASTAP's answer. What the
reprojection measurement adds is the fact the gate's criterion cannot see:
here, the disagreement is ASTAP's error. Whether agreement-with-ASTAP is
still the right gate criterion, given ASTAP is demonstrably not always
ground truth, is this milestone's decision to make, not this fix round's —
stated here for the reader rather than decided.

The M2-review hazard this milestone was built to catch — a CFA frame
reporting 2× its true FOV while claiming success — **did not reproduce**:
scale ratio sat at 1.01-1.03× across all 9219 solved frames, OSC and mono
alike (reclassified by `BAYERPAT`, not filter name — see below), zero
frames over the 5% flag threshold.

### Other corrections made in the same fix round

- **Header WCS (`CRVAL`/`CD`) present on only 301/9495 frames (3.2%)** — the
  `library` tree; unchanged finding, still holds.
- **OSC/mono reclassified by `BAYERPAT` header presence, not filter name.**
  206 SVBONY SV405CC frames shot through its Duo-Band filter carry
  `filt_eff='Duo-Band'` but ARE a CFA sensor — miscounted as mono
  originally. Corrected split: 8396 CFA / 1099 mono (was 8190/1305). Mono
  solve rate on the corrected 1099 is 87.7% (was 88.6% on the contaminated
  1305) — the reclassification does not overturn the OSC-vs-mono
  conclusion, but is now measuring the right population.
- **Hint provenance stated accurately.** 375 of 9495 frames (371
  `library`-tree) have `pointing_src='solve'` in `catalogue.db` —
  `frame.ra_deg`/`dec_deg` for these IS ASTAP's own answer copied back, not
  independent mount pointing. Does not materially bias the separation
  numbers (the solved centre comes from star matching, not the hint), but
  the report no longer implies universal hint independence.
- **Header-WCS/parity coverage disclosed as mono-only.** 299 of 301
  header-WCS frames are mono; the header-CRVAL separation and parity checks
  say essentially nothing about the 86% CFA majority.
- **300-sample's declination-extreme bias found and fixed.** The original
  sampler's within-stratum selection (`round(j*(len-1)/(k-1))`) always
  landed on declination rank 0 and rank n-1, over-sampling the dec range's
  edges on every draw. Fixed: k equal-count declination bins, one random
  draw per bin.
- **Scale-ratio, separation-percentile, and parity breakdowns added by
  OSC/mono** (previously aggregate-only for scale ratio and parity) — the
  scale-ratio-by-OSC split is the one the CFA hazard specifically
  motivated.
- **Raw NDJSON retained**, gzip-compressed, in the repo:
  `docs/superpowers/data/task-11-agreement-sample-300.ndjson.gz` and
  `docs/superpowers/data/task-11-agreement-full-9495.ndjson.gz` — every
  number above is reproducible from these files.

`cargo test --workspace`: 362 passed. `cargo clippy --workspace --all-targets
-- -D warnings`: clean. `~/astroops/` verified unwritten both times (see
task-11 report for the `find -newer` + `lsof` verification).

## Task 12: the timing comparison, run fairly

The 180 ms ASTAP figure this project had been quoting was measured without
recording its flags. AstroOps invokes ASTAP **blind** (`-r 180`), with a
hinted narrow-radius retry as the second form — comparing ASTAP's blind
solve against psolve's hinted solve would flatter psolve, so both tools are
measured **both ways**, on the same frame, serially (never in parallel — a
previous measurement of 114.7 ms/solve was single-solve latency *under
12-way contention*, not what one invocation costs alone), 5 runs each,
median reported, exact flags shown in every row.

**Frame:** `library/ic4604/lights/S/2026-07-29_20-39-12_S_120.00s_100g_1x1_0001_-9.90.fits`
(3840×2160, `FOCALLEN=243mm`, `XPIXSZ=2.9µm` — the same rig as this
document's own reference `eagle` frame). Copied into scratch before any
measurement; **never `-update`d in the timing loop itself** (a separate,
explicit `-update` demonstration below used its own fresh scratch copies).
**Index:** `gaia-dr3-g14-dec45-nside64.psidx` (234 MB, on disk at
`~/astroops/data/`, opened read-only).

Picking a frame for this comparison was not trivial: the eagle reference
frame this document opens with (and library frames sampled at random from
several other rigs) frequently **fail to solve at all** under the real
AstroOps hinted-retry flags (`-ra`/`-spd`/`-r`/`-fov`), because `-fov
1.4770` is fixed system-wide across every rig this deployment runs, while
this rig's true field is 2.626°×1.477° (diagonal ≈3.0°) — `-fov 1.4770`
matches only the *short* axis, not the diagonal ASTAP's flag semantics call
for. psolve's own `(fov/2)*1.10` search-radius formula (Task 6) then comes
out to 0.81°, well under half the frame's true angular footprint, and
whether that narrower disc still contains enough matched stars is close to
a coin flip: of 25 real ASTAP-solved library-tree frames sampled at random
from this same rig, only 4 solved under the literal hinted-retry flags —
including the eagle frame itself failing (`NO_QUAD_MATCH`) at exactly this
narrow radius, despite solving cleanly at psolve's own wider,
optics-derived default. **This is a real, measured fragility in the
hinted-retry radius formula, not a chosen-frame artifact** — recorded here
because it surfaced directly out of trying to run this comparison fairly,
and it is the reason the benchmarked frame below is not the eagle frame
this document otherwise centres on. `ic4604`'s frame was the first
randomly-sampled candidate where all four measurements below actually
solve, which is what a fair timing comparison requires.

### The table

> **SUPERSEDED by "Task 12, re-measured (M3 final review)" below.** The
> table and per-stage breakdown in this subsection are kept verbatim as the
> historical record of what was measured on 2026-08-14 before the final
> review. Two things about them are now known to be wrong: the diagnosis of
> the ~74 ms gap (it was not the index disc query), and the conclusion that
> psolve is slower than ASTAP (it is not, once the real cause is removed).

| Mode | Tool | Flags | Median (5 runs, ms) |
|---|---|---|---:|
| Blind | ASTAP | `astap_cli -f <F> -r 180 -fov 1.4770 -d /home/user/astap` | **131.71** |
| Blind | psolve | `psolve solve <F> --index <psidx>` (no `--hint` → header `OBJCTRA '16 25 31'`/`OBJCTDEC '-23 26 12'`; radius auto = 1.6569°, half-diagonal + 10% margin from `FOCALLEN`/`XPIXSZ`) | **159.90** |
| Hinted | ASTAP | `astap_cli -f <F> -ra 16.425176 -spd 66.561173 -r 15 -fov 1.4770 -d /home/user/astap` | **132.68** |
| Hinted | psolve | `psolve solve <F> --index <psidx> --hint 246.377634,-23.438827 --radius 0.81235` (`0.81235 = (1.4770/2)*1.10`, exactly what ASTAP mode's own `search_radius_deg` resolves `-r 15 -fov 1.4770` to for this frame) | **157.81** |

Runs (ms), for the record — process wall clock via a `perf_counter`-timed
`subprocess.run`, serial, no other load on the machine:

- ASTAP blind: `131.75, 126.37, 131.30, 131.71, 131.87`
- ASTAP hinted: `130.84, 129.40, 133.76, 132.68, 133.28`
- psolve blind: `159.48, 159.40, 160.41, 159.90, 160.63`
- psolve hinted: `160.73, 157.68, 157.81, 157.69, 157.99`

All four solved (not marginally): ASTAP `PLTSOLVD=T` both ways; psolve
blind `log_odds=452.2, matched=171/179`; psolve hinted `log_odds=226.6,
matched=83/179`.

**Verdict, stated plainly: psolve is slower than ASTAP in both modes on
this measurement.** Blind: 159.90 vs 131.71 ms, psolve **21.4% slower**
(+28.2 ms). Hinted: 157.81 vs 132.68 ms, psolve **18.9% slower** (+25.1
ms). This is worse than the ~175 ms this milestone's pre-work (quad-build
optimisation) left the design at on the eagle reference frame, and far
short of the design's original 12–14 ms projection. The gap between a
projection and a measurement is the finding this task exists to record,
and it does not favour psolve.

### Where the remaining time goes — psolve's per-stage breakdown

From the same runs' JSON `timings_ms` (median of 5, ms):

| Stage | Blind | Hinted |
|---|---:|---:|
| decode | 3.208 | 3.151 |
| background | 30.693 | 30.635 |
| extract | 34.540 | 34.425 |
| quads | 3.755 | 1.346 |
| catalogue (in-solve) | 0.019 | 0.006 |
| match | 11.214 | 11.197 |
| fit | 0.016 | 0.003 |
| verify | 0.021 | 0.019 |
| **sum of stages** | **83.47** | **80.78** |
| **CLI-reported total** | **157.07** | **154.45** |
| **unaccounted gap** | **73.6** | **73.7** |

Quad building — 145.4 ms and 63% of the solve before this milestone's
optimisation work, per this document's own opening measurement — is now
**1.3–3.8 ms**, under 3% of the total. That fix held. But a **~74 ms gap**
between the sum of psolve-core's own instrumented stages and the CLI's
reported `total` shows up in both modes, at nearly the same size regardless
of how many catalogue stars were fetched (537 for blind's 1.66°-radius
disc, 218 for hinted's 0.81°-radius disc) — this is the catalogue **index
disc query** (`Index::brightest_in_disc` against the 234 MB on-disk
`.psidx`), which happens in `cmd_solve.rs` *before* `solve()`'s own timer
starts and so is invisible to the 8 instrumented stages
(`cmd_solve.rs`'s own comment: *"The gap between this and the sum of the
stages above IS the index fetch cost"*). It was not visible in this
document's original 229 ms measurement (stage sum 227.4 ms, matching the
total almost exactly) because quad building's 145 ms dwarfed it then;
optimising quads down to a few milliseconds has made this previously
invisible cost the single largest line item in the solve. **This is new
information this task surfaced, not something Tasks 1–5 fixed or
budgeted for.**

> **The paragraph above is wrong about the cause, and the next section
> replaces it.** The ~74 ms gap was real; it was not the disc query. It was
> `default_cat_limit` running a second, complete decode + background +
> extract over the frame purely to count stars, before `solve()` did the
> identical work again — visible in `detected_star_count`'s own doc comment
> at the time, which described the duplication and priced it as acceptable.
> The disc query itself measures ~1 ms.

## Task 12, re-measured (M3 final review)

The final whole-branch review challenged the attribution above, and the
challenge was correct. Two measurements settle it, both on the same frame,
release, serial:

- The **pre-fix** binary run two ways, differing only in whether the
  star-count probe runs. `--cat-limit 537` is exactly what the auto path
  computes for this frame (3 × 179 usable stars), so the catalogue fetched,
  the disc query and the solve are all identical between the two rows:

  | pre-fix binary, blind, same frame | wall clock | stage sum | gap |
  |---|---:|---:|---:|
  | auto `--cat-limit` | 147.7 ms | 76.6 ms | **66.1 ms** |
  | explicit `--cat-limit 537` | 83.8 ms | 77.8 ms | **0.96 ms** |

  The stage sums agree to 1.2 ms; the wall clock differs by 64 ms. The gap
  is the probe, not the lookup.
- After the fix (`psolve-core` split into `prepare()` +
  `solve_prepared()`, so decode/background/extract happen once and the star
  count comes from the extraction the solve itself uses): the gap is
  **1.00 ms blind, 0.89 ms hinted**, median of 5 — that is the disc query,
  measured directly.

### The re-measured table

Release builds, one discarded warm-up round, then **9 interleaved rounds**
(every row runs once per round, in the same order, so all rows see the same
machine state), medians:

| Mode | Tool | Flags | Median (9 runs, ms) |
|---|---|---|---:|
| Blind | ASTAP | `astap_cli -f <F> -r 180 -fov 1.4770 -d /home/user/astap` | **100.86** |
| Blind | psolve | `psolve solve <F> --index <psidx>` | **77.34** |
| Blind | psolve, pre-fix binary | same flags | **147.68** |
| Blind | psolve, ASTAP mode | `psolve -f <F> -r 180 -fov 1.4770 -d <psidx dir>` | **75.63** |
| Blind | psolve, ASTAP mode, pre-fix | same flags | **145.69** |
| Hinted | ASTAP | `astap_cli -f <F> -ra 16.425176 -spd 66.561173 -r 15 -fov 1.4770 -d /home/user/astap` | **101.05** |
| Hinted | psolve | `psolve solve <F> --index <psidx> --hint 246.377634,-23.438827 --radius 0.81235` | **75.16** |
| Hinted | psolve, pre-fix binary | same flags | **145.59** |
| Hinted | psolve, ASTAP mode | `psolve -f <F> -ra 16.425176 -spd 66.561173 -r 15 -fov 1.4770 -d <psidx dir>` | **75.45** |
| Hinted | psolve, ASTAP mode, pre-fix | same flags | **145.77** |

All ten solved. A second independent 9-round measurement a minute later
reproduced every median to within 0.8 ms.

**Verdict: psolve is now faster than ASTAP in both modes on this
measurement** — 23.3% faster blind (77.34 vs 100.86), 25.6% faster hinted
(75.16 vs 101.05).

**Two caveats, stated because the result is the flattering one.**

1. **ASTAP's own figure moved between sessions and psolve's did not, by
   nearly as much.** ASTAP, entirely unchanged, measured 131.71 ms blind on
   2026-08-14 and 100.86 ms here; the pre-fix psolve binary measured 159.90
   ms then and 147.68 ms here. Both are faster now, ASTAP
   disproportionately. That earlier session's machine state is not
   recoverable, so the two tables are not comparable to each other. This is
   exactly why the pre-fix binary was re-measured *inside* the new table:
   the ~70 ms credited to the fix is a within-table difference measured
   under identical conditions, while the ~30 ms shift in ASTAP's figure is
   not attributable to anything psolve did.
2. **One frame, one machine, one rig.** Nothing here says psolve is faster
   in general.

### Where the time goes now — post-fix per-stage breakdown

From the same runs' JSON `timings_ms` (median of 5, ms):

| Stage | Blind | Hinted |
|---|---:|---:|
| decode | 4.91 | 4.81 |
| background | 27.93 | 28.17 |
| extract | 24.20 | 24.42 |
| quads | 3.30 | 1.24 |
| catalogue (in-solve) | 0.02 | 0.01 |
| match | 10.08 | 10.35 |
| fit | 0.02 | 0.00 |
| verify | 0.02 | 0.01 |
| **sum of stages** | **70.47** | **69.01** |
| **CLI-reported total** | **71.52** | **69.84** |
| **gap (= the index disc query)** | **1.00** | **0.89** |

Background estimation and star extraction are now the two dominant costs,
~74% of the solve between them. That is where an M4 optimisation would go.

### The fix

`psolve-core` gains `prepare()` → `PreparedFrame` → `solve_prepared()`.
`solve()` is now literally those two calls, so nothing that does not need
the star count up front changes at all. `psolve-cli` (both native mode and
ASTAP mode) calls `prepare()`, sizes `--cat-limit` from
`PreparedFrame::usable_star_count()`, fetches the catalogue, then calls
`solve_prepared()`. `psolve-core` gains no dependency and no filesystem
access; its guard test still passes.

Verified identical, not assumed: the pre-fix and post-fix release binaries
produce **byte-identical JSON** (every field but `timings_ms`) on the
benchmark frame in both blind and hinted form — same 537 and 218 catalogue
stars, same WCS, same confidence — and on 21 further real library frames
sampled across rigs, **21/21 identical**, solved and unsolved alike. The
agreement numbers cannot move: the agreement run passes only `--index` and
`--hint`, so its extraction parameters are the defaults the old probe also
used.

One deliberate behavioural difference, in a case nothing measured here
exercises: the auto catalogue limit is now sized from the extraction the
solve actually runs, so a non-default `--sigma`/`--min-pix`/`--keep`/
`--max-ellipticity`/`--saturation` now influences it (it previously came
from a defaults-only probe that ignored those flags). ASTAP mode has no
such flags, and every default invocation is unchanged.

### The `-update` safety model, demonstrated on a real invocation

Both real AstroOps invocations, `astap_cli` swapped for `psolve`, run
against fresh scratch copies of the `ic4604` frame **with `-update`**
(never against `~/astroops` itself):

```
$ psolve -f demo-blind.fits  -r 180 -fov 1.4770 -d <dbdir> -update
$ echo $?
0
$ psolve -f demo-hinted.fits -ra 16.425176 -spd 66.561173 -r 15 -fov 1.4770 -d <dbdir> -update
$ echo $?
0
```

Both exit `0`; both `.ini` sidecars read `PLTSOLVD=T`; both input files'
headers now carry `CRVAL`/`CD`/`PLTSOLVD=T`. Verified directly rather than
assumed: the data unit's byte offset is unchanged (8640 both before and
after), and the pixel bytes after `-update` are byte-for-byte identical to
the pixel bytes of the untouched source file in `~/astroops` (compared
directly, not merely asserted) — exactly the guarantee `fits_update.rs`'s
module doc describes.

### Read-only verification

An absolute-timestamp sentinel (`touch -t 202608141810`; this machine's
`find` is `bfs`, which silently accepts and does nothing useful with
`-newermt`, so a relative timestamp would prove nothing) placed before any
work in this task, then `find ~/astroops -newer <sentinel>` after every
measurement above (including the `-update` demonstration): the only paths
that changed are the same pre-existing, independently-running live-imaging
pipeline (`~/astroops/code/livestack/*.py`, `astroops-cockpit`'s uvicorn
backend) already identified in Task 11's report — `state/skyline.log`,
`state/catalogue.db` (held open by that backend, opened here only via
`sqlite3 -readonly`, which never opens for writing), `work/power/*.csv`,
and files under `code/core`/`code/bin` the live pipeline itself writes.
**None of the three trees this task ever reads or writes —
`~/astroops/library/`, `~/astroops/archive/`, `~/astroops/data/` — appear
in that list.** The `ic4604` source frame's MD5
(`5b2881f2209bc14b8f908fd16fa9013a`) was checked directly before and after
every step in this task, including after the `-update` demonstration
above, and never changed. `-update` was never passed to any invocation
whose target was a path under `~/astroops`.

`cargo test --workspace`: 363 passed, 0 failed (baseline unchanged — Task
12 added no Rust code). `cargo clippy --workspace --all-targets -- -D
warnings`: clean.
