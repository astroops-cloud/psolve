# Binning-retry catalogue refetch -- acceptance measurement

Plan: `docs/superpowers/plans/2026-08-22-binning-retry-catalogue-refetch.md`
Spec: `docs/superpowers/specs/2026-08-22-binning-retry-catalogue-refetch-design.md`
Ledger: `.superpowers/sdd/2026-08-22-binning-retry-catalogue-refetch/progress.md`

Every number below was measured on 2026-08-23 on this machine. Nothing here
is carried over from the spec's projections; where a spec figure is
reproduced, it is reproduced by an independent run and said so.

## 0. Machine state and exact invocations

| | |
|---|---|
| host / OS | macos-arm64, Darwin 25.5.0 (Apple silicon) |
| binary under test | `cargo build --release`, build id **`3ba1c32`** (from the JSON `build` field, not from `git describe` by hand) |
| pre-fix comparator | **`ca1dc73`** -- the commit immediately before the fix, built in `git worktree add .claude/worktrees/prefix-ca1dc73 ca1dc73` |
| index | `~/astroops/data/gaia-dr3-g14-dec45-nside64.psidx` (G<=14) |
| catalogue database | `~/astroops/state/catalogue.db`, opened `sqlite3 -readonly` throughout |
| parallelism | `JOBS=8` |
| corpus, as of this run | 10,378 `astap/astap+d50` measurement rows; **10,376** with an intact `location` row (2 dropped, reported by `agreement.sh`); **9,585 binning=1** and **791 binning=2** |

`~/astroops/` was never written. No `-update` anywhere; the one frame used for
the two-entry-point check was **copied to `.scratch/`** before ASTAP mode
wrote its sidecars.

Per-frame invocation, identical in every run and identical to what
`scripts/agreement.sh` issues -- no flags beyond the index and the hint:

```sh
psolve solve <path> --index ~/astroops/data/gaia-dr3-g14-dec45-nside64.psidx \
      --hint <frame.ra_deg>,<frame.dec_deg>
```

The bin-2 frame list (791 rows):

```sh
sqlite3 -readonly ~/astroops/state/catalogue.db <<'SQL'
SELECT l.path, m.ra_deg, m.dec_deg, f.id, f.ra_deg, f.dec_deg, f.binning
FROM measurement m JOIN frame f ON f.id=m.frame_id
JOIN location l ON l.frame_id=f.id AND l.path=(
  SELECT l2.path FROM location l2 WHERE l2.frame_id=f.id AND l2.intact=1
  ORDER BY (l2.tree='library') DESC, l2.path ASC LIMIT 1)
WHERE m.tool_version='astap/astap+d50' AND m.ra_deg IS NOT NULL AND l.intact=1
  AND f.binning=2;
SQL
```

The full-corpus runs are `scripts/agreement.sh full <out.ndjson>`, once with
`PSOLVE_BIN` pointed at the `ca1dc73` worktree binary and once at the tree's
own `target/release/psolve`.

**Detecting the refetch.** The runner captures **stderr**, not just stdout,
and greps `refetched the catalogue` -- not the older `refetching the
catalogue at`, which no longer exists. ASTAP mode surfaces the refetch on
stderr only: the `.ini` format has no catalogue field, so no sidecar parse
can see it. Both facts were carried forward from Tasks 2 and 3 precisely
because a stale grep string would have produced a plausible-looking
"0 refetches over 791 frames".

The line, verbatim, from a real frame:

```
solving .../2025-05-20_21-47-39__-10.10_15.00s_0444.fits: refetched the
catalogue -- 1500 stars within 3.0096 deg (was 1500 within 6.0193) -- the
first disc was derived from the uncorrected scale
```

6.0193 / 3.0096 = 2.0000 = `XBINNING`, and the star budget is unchanged at
1500 in both discs. That is the whole fix, visible in one line: same budget,
half the radius, so the budget is spent on the field instead of on a ring of
stars that cannot appear in it.

## 1. The bin-2 population, before and after

791 frames, same list, same invocation, two binaries.

| | `ca1dc73` (before) | **`3ba1c32` (after)** |
|---|---|---|
| solved | **0 / 791 (0.0%)** | **790 / 791 (99.87%)** |
| separation median | -- | **0.707"** |
| separation p90 | -- | **1.378"** |
| separation p99 | -- | **2.278"** |
| separation max | -- | **38.217"** (arbitrated in section 4) |
| over 30" | -- | 1 |
| over 5" | -- | 3 |
| `scale_source` | `null` on all 791 | `header/binning-retry` on all 790 solved |
| frames emitting the refetch line | **0** | **791** (all of them, including the one that still fails) |
| unsolved reason codes | `NO_QUAD_MATCH` x 791 | `NO_QUAD_MATCH` x 1 |
| wall time, total / mean (see note) | 71.0 s / 89.8 ms | 74.1 s / 93.7 ms |

The spec projected 790/791 at a 0.707" median with p90 1.378". This run
reproduces those figures **exactly**, from the shipped code rather than the
hand-patched harness the spec's table came from. That is a confirmation, not
a copy: the spec's numbers were produced by forcing the radius externally,
these by the merged `CatalogRefetch` path through `cmd_solve.rs`.

**Correction, and it is the one number in this document that did not
reproduce.** Every other cell in the table above comes from
`bin2-{prefix,postfix}.ndjson`. The wall-time row does not -- it was taken from
the bin-2 subset of the *full-corpus* runs. The dedicated bin-2 runs give
**72.7 s / 91.9 ms (pre)** and **70.4 s / 89.0 ms (post)**: the post-fix run
measured *faster*. And "+4.3 ms per frame" was wrong arithmetic under either
source -- 3.1 s / 791 = **+3.9 ms**, which is also 93.7 - 89.8. The percentage
was right and got copied into the wrong unit.

**The two runs disagree in sign, so the honest statement is that the cost is
within noise on this hardware**, not the +4.3% first reported. What is
structurally true regardless: the change adds a second `select_catalog` call,
and only on the failure path, so no frame that already solved pays anything.

Recording this rather than quietly restating it, because the mistake is exactly
the one section 2 criticises -- a number tabled beside a run it did not come
from.

## 2. The full live corpus, and the per-frame regression check

`scripts/agreement.sh full`, 10,376 frames, both binaries.

| | `ca1dc73` (before) | **`3ba1c32` (after)** |
|---|---|---|
| solved | 9,351 / 10,376 = **90.12%** | 10,141 / 10,376 = **97.74%** |
| binning=1 | 9,351 / 9,585 | 9,351 / 9,585 |
| binning=2 | 0 / 791 | **790 / 791** |
| separation median | 0.531" | 0.539" |
| separation p90 | 0.967" | 1.011" |
| separation p99 | 3.202" | 3.132" |
| separation max | 30.688" | 38.217" |
| over 30" | 1 | 2 |
| unsolved | `NO_QUAD_MATCH` 1012, `LOW_CONFIDENCE` 11, `TOO_FEW_STARS` 2 | `NO_QUAD_MATCH` 222, `LOW_CONFIDENCE` 11, `TOO_FEW_STARS` 2 |
| wall total | 766 s | 772 s |

The corpus-wide separation percentiles move slightly **because the population
changed**, not because any frame's answer did: 790 bin-2 frames at a 0.707"
median joined a bin-1 population at 0.531". Restricted to binning=1 the
distributions are identical to the digit -- median 0.531", p90 0.967",
p99 3.202", max 30.688" in **both** runs.

### The per-frame comparison

```
shared frame_ids 10,376 (only-pre 0, only-post 0)
regressed 0    newly 790    net +790
newly solving, by binning: {2: 790}
```

**Zero regressions**, and a stronger statement than the criterion asks for:
comparing the whole solve record -- `wcs`, `field`, `stars`, `fit`,
`quality`, `confidence`, `catalog`, `solved`, `reason`, `detail` -- with only
the build id and timings excluded,

> **all 9,585 binning=1 frames have a byte-identical record before and after.**

Not "no frame flipped": no frame *moved*. The change is provably inert on
every non-bin-2 frame in the live corpus, which is what §7 of the spec argued
structurally and this measures directly.

### The baseline the plan named is a rejected design's artefact -- corrected here

The plan's Step 3 said to diff against `.scratch/agreement-full-current.ndjson`.
That comparison reports **33 regressions**. All 33 are false, and the reason
matters more than the count.

That file is dated 2026-08-15 12:42 and carries **no `build` field on any of
its 10,373 rows** -- it predates `22628a3`, the commit that added the build
identifier for exactly this reason. Identifying it therefore had to be done by
its numbers, and its numbers name it: over the published 9,495-frame corpus it
scores **9,261 solved, median 0.563"**, which is precisely the
*unconditional* stratified-selection run recorded as **FAIL -- regression** in
`docs/superpowers/2026-08-15-stratified-selection-results.md` (measure 3:
"9219->9261 (+42 net) but separation median/p90/p99 all worse") and superseded
by the conditional design. It was never a shipped binary.

Today's binaries score 9,268 / 0.530" on that same set -- matching what the
README and `docs/astap-compat.md` publish.

Confirmed by measurement rather than inference: all 33 frames fail on
`ca1dc73` too, and they also fail on `563982f`, on `3670bfe`, and on
`8365e60` -- the commit contemporaneous with the artefact's own timestamp.
Three repeat runs of the 33 through the post-fix binary give bit-identical
results, so this is not solver nondeterminism. Nothing between that artefact
and this change ever solved them in a released state.

**Correction to the plan:** the regression baseline for this change is
`ca1dc73`, the commit immediately before it, built and run here. A comparison
against an artefact whose binary cannot be identified is not a regression
test; it is the same defect class this project keeps paying for -- a plausible
number describing something other than what it names. `.scratch/` is
gitignored and regenerable; the file's name, `agreement-full-current`, is the
misleading part and it should not be cited as a baseline again.

### And the published corpus could not have seen this either

The 9,495-frame corpus behind the README's 97.61% headline contains
**zero binning=2 frames**. Both binaries score identically on it:

```
ca1dc73 : 9268/9495 = 97.61%  median 0.530"  p90 0.945"  p99 3.105"  max 30.688"  over-30" 1
3ba1c32 : 9268/9495 = 97.61%  median 0.530"  p90 0.945"  p99 3.105"  max 30.688"  over-30" 1
```

So the sampler was not the only instrument that excluded this population -- the
headline corpus did too. That is why section 6 updates both headline documents
with the full-corpus figure beside the restricted one rather than replacing it.

**But "blind" is the wrong word, and the record says so.**
`docs/superpowers/2026-08-15-stratified-selection-results.md` (committed
2026-08-15, eight days before this run) states plainly: "+791 `binning=2` rows
... none of them solve, 0/791, a pre-existing gap outside this milestone's
scope", and reports the full-population rate as "9342/10373 (90.06%) --
dragged down entirely by the 791 `binning=2` frames solving at 0%". The hole
was measured, named, published, and deliberately **deferred**.

That matters for what this fix generalises to. The sampler change stops a
stratum being silently omitted, which is worth having -- but it would not have
prevented this, because nothing here was silent. What actually happened is that
a correctly-reported finding was scoped out of one milestone and then not
picked up for eight days, during which the headline everyone quoted came from
a corpus that excluded it. The defect class left open is **deferred findings
with no owner**, not blind instruments, and no code change closes that one.

The artefact discussed in section 2 is the same story from the other side: its
9,261/9,495 at 0.563" are the numbers that committed doc published, so it is
not an unknown provenance -- it is a *recorded, rejected* run whose file name
outlived the decision.

## 3. Both entry points, on a real frame

Criterion 4 is asserted in the suite on synthetic fixtures
(`cli_solve_binning_retry_refetch.rs`, `astap_binning_retry.rs`). Repeated
here on real rig data, frame_id 8612,
`SVBONY_SV405CC/NGC_3372/2025/05/20/.../2025-05-20_19-49-28__-9.90_15.00s_0020.fits`,
**copied to `.scratch/` first** so ASTAP mode's sidecars are never written
beside the archive frame.

```sh
psolve solve frame.fits --index <g14 .psidx> --hint 161.079166666667,-59.8891666666667
psolve -f frame.fits -ra 10.738611111111133 -spd 30.110833333333296 -d ~/astroops/data
```

(`-ra` is hours = 161.0791667/15; `-spd` is south polar distance = dec + 90.)

| | native `psolve solve` | ASTAP `psolve -f` |
|---|---|---|
| exit code, `ca1dc73` | 1 (`NO_QUAD_MATCH`, "600 image quads vs 600 catalogue quads") | 1 |
| exit code, `3ba1c32` | **0** | **0** |
| refetch line on stderr | 1500 @ 3.0096 deg (was 1500 @ 6.0193) | *identical line* |
| result | JSON `scale_source: "header/binning-retry"`, centre 161.059752 -59.884318 | `.ini` `PLTSOLVD=T`, centre 161.059752 -59.884318 |

The two centres agree to **0.0000"** (the ASTAP-mode figure is recomputed from
the `.ini`'s `CRVAL`/`CRPIX`/`CD` at the image centre, not read from the JSON).
Both are 2.93" from ASTAP's recorded centre for that frame.

## 4. The 38.2" disagreement -- arbitrated

frame_id 9036, `SVBONY_SV405CC/NGC_3372/2025/05/20/.../2025-05-20_21-47-39__-10.10_15.00s_0444.fits`,
15 s sub-exposure, 7.707"/px, psolve centre 160.687251 -59.733264 against
ASTAP's recorded 160.666190 -59.733350: **38.217"**, over the 30" bar.

It is **not a regression** -- the frame does not solve at all before this
change -- and it gets the same treatment the NGC 3372 case got in
`docs/superpowers/2026-08-14-m3-first-real-frame.md`. Four independent
signals, all pointing the same way.

**Method caveat, stated not hidden.** `astap_cli` is not installed on this
machine and `catalogue.db`'s `measurement` table stores only ASTAP's centre
(`ra_deg`, `dec_deg`) -- no CD matrix. ASTAP's candidate WCS is therefore
constructed as psolve's own CD and rotation with `CRVAL` moved onto ASTAP's
recorded centre. That isolates exactly the quantity in dispute (where the
field centre is) and holds scale and rotation fixed; it cannot detect a shear
in ASTAP's solution the way the 2026-08-14 investigation could.

### 4.1 Session trend: ASTAP's answer leaves the track and comes back

**Strengthened by the whole-branch review, verified directly against `catalogue.db`.** Across frames 9007-9040 ASTAP's recorded RA tracks smoothly from 160.7075 to 160.6817. Frame 9036 departs by ~22 mdeg **while its declination stays exactly on the trend** (-59.73335, between 9035's -59.73370 and 9037's -59.73292). That is the discriminating detail: a mount excursion moves both axes and does not return within 15 s; a bad solve need not. This is the signal carrying the verdict -- the other three are consistency checks that depend on psolve's own WCS.

Consecutive 15 s frames from the same session, centres as each tool reports
them, and the step from the previous frame:

| frame | psolve centre | ASTAP centre | psolve step | ASTAP step |
|---|---|---|---:|---:|
| 0441 | 160.68927 -59.73482 | 160.68946 -59.73484 | -- | -- |
| 0442 | 160.68825 -59.73416 | 160.68850 -59.73421 | 2.99" | 2.84" |
| 0443 | 160.68764 -59.73378 | 160.68803 -59.73370 | 1.76" | 2.04" |
| **0444** | **160.68725 -59.73326** | **160.66619 -59.73335** | **1.99"** | **39.64"** |
| 0445 | 160.68600 -59.73284 | 160.68598 -59.73292 | 2.73" | 35.94" |
| 0446 | 160.68449 -59.73248 | 160.68439 -59.73277 | 3.03" | 2.93" |
| 0447 | 160.68290 -59.73192 | 160.68270 -59.73205 | 3.53" | 4.01" |

psolve traces a smooth 2-3.5"-per-frame tracking drift across all seven.
ASTAP's recorded centre jumps 39.6" off that track for one frame and 35.9"
back onto it 15 seconds later. A mount does not do that and return.

### 4.2 Reprojection flux, with two session-neighbour controls

Gaia G<=12 catalogue stars reprojected through each candidate WCS, peak pixel
value read from the raw file within a small box of the predicted position;
"on-light" is the fraction whose peak clears a local background threshold.
Controls are the frames immediately before and after, where the two tools
agree to 0.76" and 0.28".

Box = +-1 px (a 23" window, narrower than the 38" disagreement):

| frame | psolve<->ASTAP sep | psolve WCS: median peak / on-light | ASTAP WCS: median peak / on-light |
|---|---:|---|---|
| 0443 (control) | 0.76" | 2132 ADU / 100.0% | 2120 ADU / 100.0% |
| **0444 (disputed)** | **38.22"** | **2180 ADU / 100.0%** | **328 ADU / 38.7%** |
| 0445 (control) | 0.28" | 2200 ADU / 100.0% | 2196 ADU / 100.0% |

Background is ~210 ADU. At box = +-2 px (a 38" window, comparable to the
disagreement itself, so it deliberately blurs the distinction) the disputed
frame reads 2492 / 100.0% for psolve against 504 / 73.3% for ASTAP -- the same
verdict, weaker, exactly as a wider window should make it.

The metric agrees with both tools wherever the tools agree with each other,
which is what the controls establish. On 0444 psolve's WCS still lands every
reprojected catalogue star on real light while ASTAP's centre puts the
majority of them at background.

### 4.3 Re-hinting psolve at ASTAP's own answer

```sh
psolve solve <0444> --index <g14> --hint 160.666190002178,-59.7333499563742
```

solves to 160.68730 -59.73334: **0.302" from psolve's original answer** and
**38.30" from the hint it was given**, with log-odds 391.7, 374 matched stars,
rms 2.991". Given ASTAP's centre as its starting point, psolve walks away from
it and back to its own.

### 4.4 psolve's own fit on 0444 is unremarkable

log-odds 377 (neighbours: 374-386), rms 3.082" (neighbours: 2.97-3.39"),
364 matched stars. Nothing about this frame's solve is marginal; it is an
ordinary member of its session by every internal statistic.

**Verdict: the disagreement is ASTAP's error on this frame, not psolve's.**
Same conclusion, same method, and the same rig and target as the 2026-08-14
case. The agreement gate still reads this frame as a >30" disagreement, and on
its own terms it is right to -- the gate measures disagreement with ASTAP, not
correctness.

## 5. The one frame that still fails

frame_id 8753,
`SVBONY_SV405CC/NGC_3372/2025/05/20/.../2025-05-20_20-28-05__-10.00_15.00s_0161.fits`.

```
reason  : NO_QUAD_MATCH
detail  : 251 image quads vs 600 catalogue quads, no consistent transform
stars   : detected 3123, used 22, rejected {too_small: 3100, edge: 1}
catalog : concentration 2.712, stratified true
stderr  : refetched the catalogue -- 300 stars within 3.0096 deg
          (was 300 within 6.0193)
```

The refetch fired correctly and the corrected disc was fetched. The frame
simply has almost no signal: 3,100 of 3,123 detections are rejected as
`too_small`, leaving **22 usable stars** and only 251 image quads against the
catalogue's 600. This is a cloud/dew/focus frame, not a catalogue problem --
no radius would recover it. Recorded, not waved away.

## 6. Acceptance criteria, one by one

| # | criterion | measured | verdict |
|---|---|---|---|
| 1 | >= 785 of 791 bin-2 frames solve, median separation <= 1.0" | **790/791 (99.87%), median 0.707"** | **PASS** |
| 2 | no frame solving before this change stops solving, compared per frame | **0 regressed / 10,376 shared ids; all 9,585 bin-1 records byte-identical** | **PASS** |
| 3 | live corpus solve rate >= 97.5% | **97.74% (10,141/10,376)**, up from 90.12% | **PASS** |
| 4 | the same frame solves through native and ASTAP-compat entry points | real frame 8612: exit 0 both ways, centres agree to **0.0000"**, both emit the refetch line; suite tests cover the fixtures | **PASS** |
| 5 | the 38.2" disagreement is arbitrated and the finding reported either way | arbitrated in section 4 -- **one psolve-independent signal plus three consistency checks** (not four independent signals: 4.2 builds ASTAP's candidate from psolve's own CD and 4.4 is internal to psolve), verdict **ASTAP's error**, with the method's own caveat stated | **PASS** |
| 6 | the one remaining failure's reason code is recorded | `NO_QUAD_MATCH`, 22 usable stars of 3,123 detections | **PASS** |

Six for six. That is an unusual result in this repository and it is worth
saying why it is believable here rather than letting it read as a rubber
stamp: criterion 2 is the load-bearing one, and it is not a sampled or
aggregate claim -- it is a byte-for-byte identity over every one of the 9,585
frames the change could conceivably have touched.

### The risk that had to be confirmed rather than assumed

Task 2's review left one open question for this run. The corrected disc
carries only ~10% pointing margin where the uncorrected one carried ~110%, and
there is no fallback to the original catalogue when a refetched retry fails.
A bin-2 frame whose mount pointing error sat between the two would have solved
the old way and will not be tried that way now.

Confirmed empty, two ways:

- **Directly.** 0 of 791 solved before. There is no bin-2 frame in this corpus
  whose success the change could take away, and the 0-regression count over
  the whole corpus is the general statement of it.
- **With margin to spare, and more than expected.** The corrected disc is
  3.0096 deg against a 2.7360 deg frame half-diagonal, i.e. a 16.4' margin.
  Measured over the 790 solved frames, the hint-to-solution pointing error has
  median 12.65', p90 24.32', **max 27.60'** -- and **293 of the 790 exceed the
  16.4' margin outright yet still solve.** The margin is not a cliff: past it
  the disc stops covering the far corners of the field, and a frame with
  hundreds of matched stars does not need them. The frame at 27.60' solved
  with a 1.388" separation from ASTAP, 374 matched stars and log-odds 392.

That second number is the more useful one going forward. It says the failure
mode this risk describes needs a pointing error far beyond anything this rig
produces, not merely beyond 10%.

## 7. The CFA decode fix (`7ebda12`) -- status, so it is not re-litigated

Cherry-picked onto `3ba1c32` in a worktree (`1 file changed, 123 insertions,
6 deletions` -- clean, no conflict), built release, run over the same 791
frames with the same invocation:

| | solved | rate | median | p90 | max |
|---|---:|---:|---:|---:|---:|
| `3ba1c32` (shipped) | **790/791** | 99.87% | **0.707"** | 1.378" | 38.217" |
| `3ba1c32` + `7ebda12` | 776/791 | 98.10% | 0.919" | 1.759" | 38.039" |

Per frame: **15 regress, 1 newly solves, net -14** (14 `NO_QUAD_MATCH`,
1 `LOW_CONFIDENCE`). This reproduces the spec's §3 table exactly, now from the
shipped code rather than the spec's harness.

The fix is **correct in isolation** -- double-binning a frame the camera
already hardware-binned is simply wrong -- and **currently harmful**, because
`extract.rs`'s fixed `min_pix = 4` is implicitly tuned around the coarser
double-binned plate scale, so a correct decode halves the scale and stars span
fewer pixels than the floor admits. It neither closes the hole (0/791 either
way at the uncorrected radius) nor helps once the hole is closed.

**It stays unmerged, pending scale-aware extraction.** Correct-and-harmful is
the pairing that gets re-litigated if it is not written down.

## 8. The instrument, fixed

`scripts/agreement.sh`'s stratified sampler carried, until `df3d7e3`:

> Binning is NOT stratified: every ASTAP-solved frame in this database is
> binning=1 (verified separately; there are no 2x2 rows to draw from) ...

True when written, false by 791 rows now. Binning is a stratum, the stale
comment is replaced by a dated account of how it expired, and a populated
stratum missing from the sample is now a `SystemExit`, not a silent omission.

The check runs on the rows **actually emitted**, after the `[:n]` truncation
rather than before it -- with a floor of one frame per stratum, a stratum can
survive allocation and still be cut by the truncation, and would be exactly as
invisible. That is not hypothetical: `scripts/agreement.sh sample 5` reaches
the truncation path and now exits 1 with

```
agreement.sh: sample omits binning stratum/strata ['2'] present in the
population -- raise N or fix the sampler; a silently omitted stratum is how
the 791 bin-2 frames went unseen
```

which a pre-truncation check would have passed.

Verified working at the normal size: `scripts/agreement.sh sample 300` now
draws **278 bin-1 and 22 bin-2** frames, all 22 of which solve.

## 9. What this run changes about the headline numbers

`README.md` and `docs/astap-compat.md` publish 97.61% over 9,495 frames. That
figure is **unchanged and still correct on its own corpus** -- reproduced to
the digit by both binaries here -- but that corpus contains no binning=2
frames at all. Both documents now carry the full live corpus beside it:
**97.74% over 10,376 frames**, up from 90.12%, following the existing
convention of adding a column rather than overwriting a measurement.

## 10. Test suite state

`cargo test --workspace`: **601 passed, 0 failed, 2 ignored** (the two
`#[ignore]`d real-index measurements). `cargo clippy --workspace
--all-targets -- -D warnings`: clean. `cargo fmt` deliberately not run.
