# Blind-solve milestone: acceptance measurement (Task 8)

Branch `blind-solve`, commit `5c2e73b`, 585 tests + 2 ignored, clippy clean
(re-verified at the top of this task — 0 code changes made in this task, this
is a pure measurement exercise). Hardware: Apple M5 Max, 18 cores, 128 GB RAM,
macOS Darwin 25.5.0 (arm64), `cargo build --release`.

**Result up front: 4 of 5 criteria pass. The null test -- the one that
matters most -- returned zero false positives; read section 2 for what that
does and does not establish, because it validates the pipeline rather than the
gate. Criterion 5 (the sentinel-pointing frame) does not solve, and its
premise turns out to rest on a single frame that no solver is known to
handle.**

*Corrected 2026-08-16 after review: three statements in the original draft were
false or overstated -- the null run's refusal reasons, the split of the 38
hinted failures, and the null index's declination clearance. Each is corrected
in place below with the error named, per the project's "measured, not
projected" convention.*

---

## 1. Solves without a hint

**Method.** 109 real frames, spanning all 3 rigs in the archive (ATR585M 42,
DWARFIII 41, SVBONY SV405CC 26), exposures 0.1s–14,895s (≈4.1h single stacked
integration), **35.6 hours on target**. Each frame's `RA`/`DEC`/`OBJCTRA`/
`OBJCTDEC` header cards were blanked in a **scratch copy** (source files never
touched — verified below); the stripped copy was then solved twice: once with
an explicit `--hint` (DB commanded pointing) as ground truth, once with
`--quad-index` and no hint at all (blind).

| | value |
|---|---|
| Frames attempted | 109 |
| Solved hinted (ground truth exists) | 71/109 |
| Of those, also solved blind | **68/71 (95.8%)** |
| Blind solved but hinted did not | 0 (no spurious blind convergence) |
| Hinted solved but blind refused | 3 — see below, all `NO_QUAD_MATCH` |
| Separations solved-both (arcsec) | min 0.0001, median 0.0293, p90 0.150, p99 0.545, max 0.558 |
| Frames > 30″ from hinted answer | **0** |

The 38 frames where hinted itself failed to solve split **20 `NO_QUAD_MATCH`
/ 18 `TOO_FEW_STARS`** -- not "mostly `TOO_FEW_STARS`", as an earlier draft of
this document claimed; the counts come from `accept-results-combined.json`.
They are a pre-existing data-quality limit shared by both paths, not a
blind-specific gap — blind never solved where
hinted also failed, and never disagreed with a real solve by more than 0.56″
(the spec's bar is 30″).

**The 3 refusals**, investigated rather than waved off: all three are the
*same* long-integration DWARFIII field (RA 97.65, dec 4.98) at 2,775s /
5,970s / 11,160s exposure — heavy star elongation (96–173 detections flagged
`elongated`, consistent with guiding drift over a very long single sub) that
the hinted path's targeted disc search tolerates but the blind quad search's
tighter shape/scale gate (Task 7's re-measured `SHAPE_TOL`/
`SCALE_CONSISTENCY_FRAC`) does not. **Refuses cleanly (`NO_QUAD_MATCH`),
does not mismatch.** This is the correct failure mode, and it is the same
principle the null test below exists to enforce.

**PASS** — solves within the 30″ bar on every frame it attempts, and prefers
refusal to disagreement on the frames it doesn't.

### How the 109 frames were chosen: **the rule is lost**

*Added 2026-08-23.* The sample was carried in two scratch files,
`blind-sample.tsv` (45 rows) and `blind-sample-big.tsv` (64 rows), 45 + 64 =
109. Neither the query that produced them nor the files themselves survive:
the session scratchpad was wiped whole on 2026-08-23 11:13, taking the TSVs,
the header-stripped frame copies and `accept-results-combined.json` with it.
No task report, brief or ledger entry in
`.superpowers/sdd/2026-08-15-blind-solve/` records a selection rule, and
nothing under `scripts/` produces such a file.

**Reconstruction was attempted and failed.** Against the live
`~/astroops/state/catalogue.db` (opened `-readonly`), the reported fingerprint
-- 109 frames split ATR585M 42 / DWARFIII 41 / SVBONY SV405CC 26, exposures
0.1s-14,895s, 35.6 h on target -- was tested against every rule family that
could plausibly have produced it:

- one frame per group, for all 1-, 2- and 3-column groupings of
  `rig`/`target_id`/`object_name`/`exposure_s`/`gain`/`binning`/`filt_eff`/
  `naxis1`/`naxis2`/`focal_len`/`pixel_um`/`date(captured_at)`, under four
  population filters (LIGHT; LIGHT + `pointing_src='commanded'`; LIGHT + has a
  `measurement` row; both). Nearest results were 90 and 132 frames; **no
  grouping yields 42 / 41 / 26**;
- every stride sample (take every *k*-th, k = 60..220) over LIGHT frames
  ordered by `id`, by `captured_at` and by `rig, captured_at`, filtered to
  those yielding exactly 109 rows. **None reproduces the rig split or the
  35.6 h total.**

So the rule is **not recoverable, and this document says so rather than
inventing one.** The measurement in this section is a real measurement of 109
real frames; it is simply not re-drawable, and a re-run would be a different
sample.

**What IS recoverable, and worth having, is the sample's shape -- because it
is checkable against the archive and it qualifies the headline number.** Both
statements below were verified against `catalogue.db` on 2026-08-23:

| | sample | archive (all LIGHT frames) |
|---|---|---|
| ATR585M | 42 (38.5%) | 1,394 (12.4%) |
| DWARFIII | 41 (37.6%) | 8,316 (73.9%) |
| SVBONY SV405CC | 26 (23.9%) | 1,545 (13.7%) |

1. **The sample is rig-balanced, not population-proportional.** DWARFIII is
   74% of the archive's LIGHT frames and 38% of the sample. So "68/71 (95.8%)
   also solved blind" is a **per-rig-balanced** figure, and is not the rate
   AstroOps would see over its actual frame mix -- a fleet-weighted rate would
   be dominated by DWARFIII, the rig that contributed all 3 of the
   hinted-but-not-blind refusals. Deliberately spreading a sample across rigs
   is the right instinct for a capability measurement; reading the result as a
   fleet rate is not.
2. **The sample touches both extremes of the archive's exposure range.** Its
   0.1s–14,895s span is exactly the full LIGHT range in `catalogue.db`; the
   0.1s end exists only on ATR585M (6 frames archive-wide) and the 14,895s end
   only on DWARFIII (3 frames). Whatever the rule was, it was not confined to
   a comfortable middle.

---

## 2. The null test — zero false positives (THE CRITERION THAT MATTERS)

**Null index build.** A second `.psqidx` built from the *same* paired G≤16
`.psidx` (required for the fingerprint check) but swept only declination
**32°–45°**, full RA: `psolve quad-index build --star-index
gaia-dr3-g16-dec45-nside64.psidx --out null-sky-dec32-45.psqidx --min-dec 32
--max-dec 45` → 1,947,237 quads, 78,048 tiles, 10.8s, 0 clamped.

**Confirmed by inspection that the test frames genuinely fall outside it**:
every non-sentinel frame in the archive has declination in
[-89.85 deg, +24.12 deg] (checked against `catalogue.db`); the largest field of
view in the fleet (SVBONY 4x binning, 243mm/9.26um) has a half-diagonal of
~2.74 deg, so no real frame's footprint can reach past ~26.9 deg declination.

The clearance is **~3.1 deg, not the ">=5 deg" an earlier draft of this
document claimed.** `--min-dec 32` does not put the index floor at 32 deg:
`tiles_for_band` filters tiles by their *centre* (`bounds.contains(ra_c,
dec_c)`, `cmd_quadindex.rs:174-177`) and each tile carries a disc of
`radius_deg = scale_deg / 2` (`:218`), so a tile admitted on its centre
reaches half a band-scale further south. Worked over all six entries of
`BAND_SCALES_DEG`, the binding case is the 4 deg band, whose centre grid lands
exactly on 32.0 deg and so reaches **30.0 deg**:

| band scale | lowest centre >= 32 | disc radius | southern reach |
|---|---|---|---|
| 0.25 / 0.5 / 1.0 / 2.0 | 32.125 / 32.25 / 32.5 / 33.0 | 0.125 / 0.25 / 0.5 / 1.0 | 32.00 |
| **4.0** | **32.00** | **2.0** | **30.00** |
| 8.0 | 36.82 | 4.0 | 32.82 |

So the index floor is 30.0 deg against a maximum frame reach of ~26.9 deg.
**The conclusion is unchanged -- comfortably disjoint, no overlap possible
regardless of RA -- but the margin is 3.1 deg, not 5 deg.**

**Result: every one of 110 frames (the same 109-frame sample above, plus the
sentinel frame in section 5) refused against the null index. Zero accepted.**

| | value |
|---|---|
| Frames tested against the null-sky index | 110 |
| Accepted (must be 0) | **0** |
| Refusal reason | **91 `NO_QUAD_MATCH`, 18 `TOO_FEW_STARS`** |
| Frames that actually entered the blind search | **91** |
| Null-index wall clock | min 0.032s, median 0.712s, p90 0.883s, max 1.039s |

An earlier draft of this document recorded the refusal reason as
"(all) `NO_QUAD_MATCH`". That was false, and the error mattered: the 18
`TOO_FEW_STARS` frames never built a quad, never consulted the index, and
could not have produced a false positive against *any* index. They are the
sub-0.1s wall clocks that produce the reported "min 0.032s". **The honest null
denominator is 91, not 110.**

### What this criterion does and does not establish

The two claims below are easy to conflate. Only the first is measured.

**Measured, and holding.** Against a disjoint index, the pipeline returned no
wrong position on any frame -- over the 91 frames that entered the search,
spanning ~30 independent sky pointings (2 deg linkage; the 109 frames are not
109 independent trials), all 3 rigs, and exposures from 0.1s to 4+ hours. This
is the property the milestone needs, and it is the property AstroOps depends
on: blind solving does not invent positions.

**Not measured.** That `AcceptParams::blind(M)` -- the multiplicity-corrected
gate built in Task 6, and the reason blind solving is safe *in principle* --
rejects coincidences end to end. **Zero null frames reached it.** Its own
refusal code, `LOW_CONFIDENCE`, appears nowhere in the 109-frame null run;
every refusal was produced upstream, by star extraction or the quad matcher.
Across the entire acceptance measurement the gate was evaluated only on the 68
frames that legitimately solved, and it accepted all 68. Outside unit tests it
has never been observed rejecting anything.

That is not a defect -- earlier stages refusing the wrong sky first is the
pipeline working, and a wrong position is prevented either way. But it means
this criterion validates **the pipeline**, not **the gate**, and the record
should not imply otherwise. A negative test proves nothing about a specific
component unless that component can be shown to have run.

### The gate, covered synthetically instead (2026-08-23)

The follow-up planned here was a deeper null index, chosen to raise `M` until
a wrong-sky coincidence survived the upstream stages. It was dropped: the
inputs it was scoped against no longer exist (see §1's note on the wiped
scratchpad), so it became a full re-measurement -- and it would still have
probed the gate only by *hoping* a coincidence got that far.

Two deterministic tests in `crates/psolve-core/tests/synthetic.rs` drive it
directly instead. One frame -- 310 scattered stars plus a compact 14-star
group -- and a wrong catalogue holding a congruent copy of that group
(rotated 140°, moved across the field) among 113 decoys. The copy's quad codes
match exactly, so the matcher finds a consistent transform and the fit
converges tightly, on a WCS **6.89′ from the truth**: this milestone's
motivating incident (the 87.77° miss) in miniature.

| | measured |
|---|---|
| Candidate's evidence | **13.68 decades**, 12 matched |
| At the hinted gate (12.0) | **ACCEPTED** -- a confidently wrong answer |
| At the blind gate, `M` = 12,600 (16.10) | **REFUSED, `LOW_CONFIDENCE`** |
| Same frame, its own true catalogue, blind gate | **ACCEPTED** |
| Crossover `M`, swept 1..`usize::MAX` | accepted at 47, refused at 48 -- `12.0 + log10(M)` predicts 47.9 |

An outcome that flips on `min_log_odds` alone cannot be produced by any stage
upstream of `verify::accept`, which is what establishes the candidate reached
the gate. Both tests were confirmed to FAIL when the behaviour they pin is
broken (zeroing `multiplicity_decades`; disabling the `verify::accept` branch
in `solve.rs`).

**This does not upgrade criterion 2.** It is synthetic, and it drives the
*hinted* pipeline with the blind gate substituted into `SolveOptions::accept`
-- so the candidate does not arrive through a `.psqidx` code-space lookup, and
it is scored by `verify::confidence` rather than `blind_confidence` (the four
fitted correspondences are still counted as evidence; the real blind path
deducts them, which makes it stricter than what is measured here). What can
now be said that could not be said before: the gate has been observed refusing
a real candidate, and the multiplicity correction has been observed moving the
outcome. **The end-to-end real-sky path still never reached it.**

**PASS** on the claim that matters, with the scope above stated rather than
implied.

---

## 3. Speed

Blind-solve wall clock, release binary, the same 109-frame run (measured with
6 concurrent worker processes — realistic pipeline load, not a distorting
factor: the single highest time recorded anywhere in this task, 3.6s, was a
**solo** run of the sentinel frame in §5, higher than every parallel-batch
number below).

| | value |
|---|---|
| n | 109 |
| min | 0.039s |
| median | 1.243s |
| p90 | 2.523s |
| p99 | 2.631s |
| **max** | **2.668s** |
| count > 5s | **0** |

**PASS** against the 5s bar (1.87× headroom at the worst case measured) and
well inside "within 50× of ASTAP's 0.10s solving case" (2.668/0.10 ≈ 27×).
Solved-only frames cluster tightly (median 1.248s, max 1.734s); the higher
end of the full distribution is refusals that searched every band/cluster
before giving up (up to 40 cluster attempts), not solves running long.

---

## 4. The hinted path is unchanged

Re-ran `scripts/agreement.sh full` on this branch's own release binary
(commit `5c2e73b`) against the identical G≤14 index the original corpus used,
then filtered to the **exact 9,495 frame_ids** in the committed baseline
(`docs/superpowers/data/task-11-agreement-full-9495.ndjson.gz` — all 9,495
IDs present in this run's larger 10,376-frame extraction, confirming no
frame was silently dropped) and ran the same `agreement-report.py`.
**146.9 hours on target.**

| | this run | brief's "current main" | baseline `.gz` file's own stats |
|---|---|---|---|
| Solved | **9268/9495** | 9268/9495 | 9219/9495 (older snapshot — see note) |
| median | **0.530″** | 0.530″ | 0.531″ |
| p90 | **0.946″** | 0.946″ | 0.947″ |
| p99 | **3.107″** | 3.111″ | 3.128″ |

Solve count, median, and p90 match the brief's stated current-`main` numbers
**exactly**. The p99 gap (3.107″ vs 3.111″) is the same percentile-estimator
convention difference Task 6's report already identified and resolved (not a
regression — recomputing the brief's own 9268-frame set by nearest-rank
reproduces 3.107″ exactly). The static `.gz` file predates a subsequent
solve-rate fix on `main` (9219 vs 9268), which is why it's used here only as
the frame-id list, not as the literal number to match — the brief's own
restated figures are the target, and they match.

One pre-existing gross error (>30″) survives unchanged in both runs — same
file (`SVBONY_SV405CC/NGC_3372/.../0050.fits`), 30.34″ in the baseline vs
30.69″ now — a single known outlier, not a blind-solve regression.

**PASS** — no `--quad-index` was passed anywhere in this run; the hinted path
is bit-for-bit the same code path Task 6 already proved untouched (0 of
10,376 frames differed pre/post-fix), and this independent re-run reproduces
its numbers.

---

## 5. Sentinel-pointing frames now solve

**Searched the entire archive for the literal sentinel**: `grep -arlF "DEC
= -90." ~/astroops/archive ~/astroops/library` (29s, all FITS files) plus
`SELECT * FROM frame WHERE dec_deg = -90.0` against `catalogue.db` — both
methods agree on exactly **one** frame in the whole ~15,000-frame archive:

```
frame_id=2, rig=SVBONY CCD SV405CC, exposure=120s, gain=0
path: archive/fits/SVBONY_CCD_SV405CC/UnknownOBJECT/2025/04/15/.../
      Light_20250415_174205_120.0s_Gain0_Bin2_Temp18.2_2.fits
header: RA = 15.35417, DEC = -90.   (literal sentinel, unparsed, un-laundered by ingestion)
```

Confirmed `psolve` currently returns `NO_HINT` for it without `--quad-index`
(the motivating case). With `--quad-index` (real G≤16 index): **it does not
solve.** `NO_QUAD_MATCH` after 3.6s (within budget), having offered 7,661
hypotheses, kept 161 candidate transforms, and tried the maximum 40 clusters.
Against the null-sky index it correctly refuses too (0.82s) — consistent
with §2.

**Honest reason, not a workaround**: of 15,630 raw star detections, only 500
survived as usable and 10,410 were rejected as `too_small` — a
noise-dominated capture at gain 0, consistent with `UnknownOBJECT`/an aborted
or misconfigured acquisition, not a blind-search deficiency. There is no
ASTAP measurement recorded for this specific frame in `catalogue.db` (neither
`measurement` nor `rejected` has a row for `frame_id=2`), so the design
doc's motivating "ASTAP solves it" claim cannot be confirmed or denied for
*this exact specimen* — the anecdote that motivated the milestone (an 87.77°
ASTAP miss) was a different frame (NGC 6380), not this one. The broader
109-frame sample above shows the blind path converges reliably (68/71,
95.8%) whenever a frame is independently known-solvable, so this is read as
a data-quality limit of the one located sentinel specimen, not a defect in
the blind solver — but it should be stated plainly: **on the only true
sentinel frame this archive contains, blind solving does not succeed.**

**FAIL on the located specimen** (n=1). Criterion 5 is not demonstrated
positively; it is explained, not passed.

---

## Summary

| # | Criterion | Headline | Result |
|---|---|---|---|
| 1 | Solve without hint | 68/71 hinted-solvable frames also solve blind, 0/68 over 30″ (max 0.558″). **The sample's selection rule is lost and the sample is not re-drawable** (§1); it is rig-balanced, not fleet-weighted, so the 95.8% is not an archive-wide rate. | **PASS** |
| 2 | Null test | **0/110 accepted against the wrong sky** — a result about the **pipeline**, not the gate: no null frame reached `verify::accept`. The gate is now covered synthetically instead (§2), refusing a 13.68-decade candidate 6.89′ from the truth that the hinted threshold accepts. | **PASS (the one that matters)** |
| 3 | Speed | max 2.668s / 109 frames, all < 5s | **PASS** |
| 4 | Hinted path unchanged | 9268/9495, median 0.530″, p90 0.946″, p99 3.107″ — matches brief exactly | **PASS** |
| 5 | Sentinel frames solve | the sole sentinel frame found (n=1) does **not** solve blind. **The criterion's motivating premise -- that such frames are ASTAP-solvable and psolve-unsolvable -- is unverified in both directions**: ASTAP is not installed on this machine and `catalogue.db` holds no solve row for `frame_id=2`, so there is no evidence any solver handles it. n=1, and that one is unmeasured on the other side. | **FAIL** |

**Read-only verification.** `~/astroops/data/{gaia-dr3-g14-dec45-nside64.psidx,
gaia-dr3-g16-dec45-nside64.psidx,gaia-dr3-g16-dec45-nside64.psqidx}`: SHA-256
identical before and after this entire task. None of the 110 source frame
paths read in this task appear in a `find ~/astroops -newer <absolute
touch -t stamp>` listing; the ~889 files that do appear are attributable to
the live capture daemon's own concurrent activity (`SEQ_*.json`, `live/`
stack outputs, `catalogue.db` growth from live ingestion) — none are inputs
this task read or outputs it wrote. The null-sky `.psqidx` and all run
artifacts live under this session's scratch directory only, never under
`~/astroops`.
