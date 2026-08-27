# Stratified selection: acceptance measurement

> **Amendment (conditional-stratification fix round, same day):** this
> document records the UNCONDITIONAL stratification measured at `93cff70`.
> That design was superseded by a gated version (see
> `docs/superpowers/2026-08-15-conditional-stratification-results.md`) after
> measure 3 below failed. Two corrections to this record, made after a
> review round on the gated design:
>
> - **Section 1's "measure 1: pass (correct negative, confirmed)" verdict
>   is corrected.** Against the design spec's own acceptance criterion 2 --
>   *"Omega Centauri solves at defaults"* -- the honest verdict is **NOT
>   MET**. The upstream-cause reasoning in section 1 below (globular
>   failures are at extraction/quad-matching, not selection, so this
>   milestone was never going to fix them) is sound and still stands as
>   *context* for why the criterion was not met -- but it justifies
>   DEFERRING the criterion, not converting a failed one into a passed one.
>   Reusing "correct negative" language to mean "pass" conflated a
>   diagnosis with an acceptance result. Criterion 2 is not met by either
>   the unconditional design measured here or the conditional one that
>   replaced it (same upstream cause; the gate does not touch it -- see the
>   conditional results doc's own globular section).
> - **Measure 4 (the sham-rate floor) was not evaluated in the conditional
>   fix round either** -- the reasoning in section 4 below (the instrument
>   does not exist anywhere on this machine, approved-but-not-built)
>   still applies unchanged. Not re-stated as newly true; carried forward.
>
> Measure 3's regression finding below is what triggered the conditional
> redesign and remains accurate as a record of the unconditional design;
> the conditional results doc reports whether the redesign actually fixed
> it.

**Date:** 2026-08-15. **Machine:** macos-arm64. **Commit measured:** `93cff70`
(`stratified-selection` branch, `fix(cli): wire stratified catalogue
selection into both solve entry points`). Build: `cargo build --release`,
fresh (`target/release/psolve`, mtime matches commit).

**Verdict up front: mixed, and net negative on the corpus that matters
most.** Measure 2 (the 276 misses) shows real, honest improvement in some
dense fields. Measure 3 (the full corpus) shows that improvement was bought
with a measurable precision regression spread across the *entire* solved
population, plus 46 frames that used to solve now failing outright. Per this
project's own standard — *"a change that improves solve rate while
worsening separation is a regression, not a trade"* — **measure 3 fails.**

Safety: `~/astroops/` and `~/mnt/astro/` were never written to. `psolve
solve` was run with no `-update` anywhere. All `sqlite3` reads used
`-readonly`. The 30 globular frames were hashed (`shasum -a 256`)
before and after — identical. The 276 re-solved frames were not
pre-hashed (an oversight caught after the fact); mtime was checked
post-hoc against today's date for all 276 and none show it, consistent
with `psolve solve`'s documented no-write behaviour (only `-update`,
never passed here, touches a frame). `scripts/agreement.sh` performed the
10,373-frame run; it is a pre-existing, previously-trusted read-only
script in this repo and was not additionally hashed given its scale.

---

## 1. Omega Centauri and other globulars, at defaults

`psolve solve <frame> --index ~/astroops/data/gaia-dr3-g14-dec45-nside64.psidx`,
no `--cat-limit`/`--keep`/`--radius`. All frames for `omegacen` (target id 10,
RA/Dec 201.696/-47.479 — the actual Omega Centauri; not to be confused with
`cena` = Centaurus A, which shares three digits of RA), `ngc6681` and
`ngc6809`, every intact location in `catalogue.db`.

Hours on target: Omega Centauri 1.92 h (23 frames), NGC 6681 0.10 h
(3 frames), NGC 6809 0.13 h (4 frames).

| target | frames | solved | stage each failure hits |
|---|---|---|---|
| omegacen | 23 | **0/23** | catalogue selection reached every time (`used`=500, the `--keep` default cap), then `NO_QUAD_MATCH` |
| ngc6681 | 3 | **0/3** | never reaches catalogue selection — `TOO_FEW_STARS`, 0-1 of 246-363 detections survive `min_pix=4` |
| ngc6809 | 4 | **1/4** | 2 reach selection (`used`=96, `used`=9) then `NO_QUAD_MATCH`; 1 `TOO_FEW_STARS` (0 used); 1 **solves** |

**The established finding still holds, confirmed at this commit:** globular
failures are upstream of this milestone. ngc6681's 0/3 is the extraction
floor (`min_pix=4`) — catalogue stratification cannot help a frame that
never reaches catalogue selection. ngc6809 and Omega Centauri reach
selection and then lose at quad matching against extreme detection counts
(15,000–22,700 detections in Omega Centauri's case, `used` capped at the
500-star `--keep` default) — a matching/density problem, not a selection
concentration problem, so stratifying *which* 500 stars get kept does not
change the outcome here.

**The one ngc6809 solve is not evidence stratification fixes globulars.**
That frame (`.../ngc6809/lights/L/..._0000.fits`) is the one with
`pointing_src='solve'` in `catalogue.db` — its header/DB pointing is
already ASTAP's own solved centre, not the nominal target position, so the
search started almost exactly on the answer (log_odds 137.5, 47/48 matched,
1.27" fit RMS). Independent check: psolve's field centre
(294.87931, -31.02448) is 1.8" from the DB's `pointing_src='solve'` centre
(294.87984, -31.02465) — well within 30", a confident correct solve, not a
false one. It is a favorable-hint artifact unrelated to catalogue-side
stratification.

**Acceptance for measure 1: confirmed as specified — a correct negative.**
No globular solves at defaults except the one pre-conditioned frame; no
confident wrong solve was produced.

---

## 2. The 276 previously-failing frames

Baseline: `docs/superpowers/data/task-11-agreement-full-9495.ndjson.gz`,
276 records with `psolve.solved=false` (267 `NO_QUAD_MATCH`, 7
`LOW_CONFIDENCE`, 2 `TOO_FEW_STARS` — reconfirmed exactly against the gzipped
file before re-solving). Hours on target across these 276 frames: **7.89 h**.
Re-solved with each record's own baseline `cmd` (identical `--hint`) against
the current binary.

**88 / 276 (31.9%) now solve.** Remaining 188: 176 `NO_QUAD_MATCH`, 10
`LOW_CONFIDENCE`, 2 `TOO_FEW_STARS`.

By target (object_name), original-corpus failure rate → now-solving count,
targets from the brief plus every other target represented in the 276:

| target | corpus hours | baseline fail-rate | now solve (of failing) |
|---|---|---|---|
| C 76 | 0.62 h | **10.81%** (16/148) | **15/16** — rate drops to 0.68% |
| HD 93308 | 7.90 h | **7.61%** (71/933) | **35/71** — rate drops to 3.86% |
| Eta Carina | 5.50 h | **6.33%** (14/221) | 4/14 — rate drops to 4.52% |
| M 8 | 16.28 h | 1.23% (24/1945) | 18/24 |
| Eta Carinae Nebula | 6.45 h | 0.72% (7/979) | 1/7 |
| Corona Australis - ESO 396-N14 | 8.33 h | 18.00% (18/100) | **0/18 — unchanged** |
| War and Peace - NGC?6357 | 7.42 h | 17.98% (16/89) | **0/16 — unchanged** |
| Centaurus A | 4.75 h | 21.05% (12/57) | **0/12 — unchanged** |
| Caldwell 101 | 9.42 h | 7.08% (8/113) | **0/8 — unchanged** |
| (44 smaller targets, 1-9 frames each) | — | — | 30/54 |

The brief's three named figures (10.8/7.6/6.3%) are reproduced exactly
against the baseline data. The dense-field concentration **partly
flattens**: C 76 and HD 93308 improve sharply, Eta Carina improves less
than half. But four other dense fields present in the same 276 —
Corona Australis, War and Peace, Centaurus A, Caldwell 101 — show **zero**
improvement; every one of their baseline failures is still a failure.
Stratification helps some crowded fields and not others; it is not a
general fix for density.

**Acceptance for measure 2: partial pass, honestly mixed** — real
improvement, unevenly distributed, several dense fields untouched.

---

## 3. The full agreement corpus — regression found

`scripts/agreement.sh full` re-run over the live corpus (`.scratch/` output,
not committed — reproducible by re-running the script). Full command
completed 10,373/10,373 frames, 102 s wall.

**The corpus has grown.** `measurement` (tool_version astap/astap+d50) now
has 10,373 rows (154.58 h) vs the baseline's 9,495 (146.87 h): +791
`binning=2` rows (5.12 h, all from another team's sweep — none of them
solve, 0/791, a pre-existing gap outside this milestone's scope) plus 87
additional `binning=1` rows not in the original baseline set.

### Like-for-like: the original 9,495 frames only

All 9,495 baseline `frame_id`s matched exactly in the current run (no
frame lost from the set).

| metric | baseline (committed) | current (this commit) | delta |
|---|---|---|---|
| solve rate | 9219/9495 (97.09%) | **9261/9495 (97.54%)** | +42 net, **improved** |
| separation median | 0.531″ | **0.563″** | **+0.032″, worse** |
| separation p90 | 0.947″ | **1.007″** | **+0.060″, worse** |
| separation p99 | 3.128″ | **3.208″** | **+0.080″, worse** |
| scale outliers | 0 | 0 | unchanged, pass |
| parity mismatches | 0 | 0 | unchanged, pass |
| gross errors (>30″) | 1 | 1 | same pre-existing frame (SVBONY NGC 3372, 30.34″→30.87″), not new |

**The +42 net hides real churn.** 88 of the baseline's 276 failures now
solve (measure 2, above) — but **46 frames that solved in the baseline now
fail**, all newly `NO_QUAD_MATCH`. These are concentrated in two dense
fields that were *not* helped by stratification: Centaurus A (7 of its
57 frames, on top of the 12 already failing) and Cats Paw Nebula / NGC 6334
(10 frames), plus 8 DWARFIII frames, 2 SVBONY, 1 Caldwell 101.
88 − 46 = 42, which reconciles the net exactly.

**The precision regression is not confined to the churned frames.**
Restricting to the 9,173 frames that solved in *both* runs (the identical
frame set, isolating any mix-shift effect): separation still worsens
(median 0.532″ → 0.563″, p90 0.946″ → 1.004″, p99 3.111″ → 3.199″), and
9,098 of those 9,173 frames (99.2%) have their fitted WCS centre shift by
a nonzero amount versus baseline — median shift 0.30″, p90 0.66″, p99
1.37″, max 17.56″. This is systematic: stratified catalogue selection is
changing which stars get matched even on frames that already worked, and
on average it is matching to a slightly worse fit. Frames with separation
>1″ grew from 803 to 951 (+18%); the >5″ tail held flat at 27.

**Acceptance for measure 3: FAIL.** Solve rate improved, but median, p90 and
p99 separation all regressed on the identical baseline population, and 46
previously-correct solves broke. This is exactly the pattern the project
has already named as unacceptable: *an improvement in solve rate bought
with worse separation is a regression, not a trade.*

### Full current population (10,373 frames, 154.58 h), for completeness

Solve rate 9342/10373 (90.06%) — dragged down entirely by the 791
`binning=2` frames solving at 0%, which is a separate, pre-existing gap and
not attributable to this milestone. Restricted to `binning=1` (9,582
frames): 9342/9582 = 97.5%, consistent with the like-for-like figure above.
This population is **not** a valid comparison against the committed
baseline on its own — it is reported only so the "new total" is on record,
per the brief's requirement to report it.

---

## 4. The sham-rate floor — not reproduced, and here is why

The `astroops-ai` session's figures (`docs/superpowers/specs/2026-08-15-absolute-transparency-design.md`
in that repo) are IC 4592 0.046, HD 37805 0.065, Eta Carinae 0.182–0.205
(two figures appear in that doc: 0.182 in the validation table, 0.205 in
prose — both are theirs, not reconciled here), Omega Centauri 0.646
(refused, floor > 0.30 gate).

**Not reproduced here, deliberately, rather than reimplemented cheaply:**

- The design's own status line reads *"approved, not built."* Checked
  directly: `astroops-ai`'s working tree is clean, its `axes-transparency-sky`
  branch (and every other branch) contains no transparency/photometry
  module, and no untracked script exists anywhere under `~/` or in that
  repo's `.scratch`. There is no runnable artifact to re-run.
- The method is not a thin wrapper — forced aperture photometry at
  proper-motion-propagated catalogue positions on a CFA-vs-mono surface
  chosen by an empirical 200-star margin test, asymmetric sigma-clipped
  background, a G≤16 (or G≤18) reference index separate from the G≤14
  solving index, plus the mandatory sham-position control itself. Rebuilding
  that from the spec, un-reviewed by its authors, to produce a number that
  is then compared against their own published figures, would defeat the
  reason this instrument is valuable: *it is independent because another
  team built and validated it, not because the number is easy to
  regenerate.*
- The `psolve solve` change under test here is confined to which stars are
  *selected* for quad-building, not to WCS fitting or photometry — so the
  sham-rate instrument, once it exists, is exactly the right tool to check
  this milestone's crowded-field claims from a second angle. That value is
  undiminished by waiting.

**Recommendation: ask the `astroops-ai` session to re-run their measurement
against this commit (`93cff70`) on IC 4592, HD 37805, Eta Carinae and Omega
Centauri**, the same four fields, so the floor figures are comparable
directly. Until then, measure 4 is **not evaluated**, not assumed to pass.

---

## Summary

| # | measure | headline | acceptance |
|---|---|---|---|
| 1 | Omega Centauri / globulars at defaults | 0/23, 0/3, 1/4 solve; failures upstream of this milestone | **pass** (correct negative, confirmed) |
| 2 | 276 previously-failing frames | 88/276 (31.9%) now solve; concentration flattens for C 76 and HD 93308, only partly for Eta Carina, not at all for Corona Australis / War and Peace / Centaurus A / Caldwell 101 | **partial pass** |
| 3 | Full agreement corpus, like-for-like (9,495) | solve rate 9219→9261 (+42 net) but separation median/p90/p99 all worse, 46 previously-solving frames now fail | **FAIL — regression** |
| 4 | Sham-rate floor | not reproduced; instrument not built anywhere on this machine; recommend astroops-ai re-run against `93cff70` | **not evaluated** |

**This milestone should not be accepted as a clean win.** It trades a subset
of dense-field solve failures for a corpus-wide precision regression and a
new set of failures in different dense fields (Centaurus A, Cats Paw
Nebula). Per the project's own standing rule, that is a regression, not a
trade, regardless of the net solve-rate delta being positive.
