# Conditional stratification: measurement and calibration

Branch `stratified-selection`, on top of the unconditional stratification
commits (`f59efef`..`2817017`, measured and found regressed in
`docs/superpowers/2026-08-15-stratified-selection-results.md`). This
document covers two rounds: the initial conditional design, and a fix
round after review found real defects in it. Both rounds' methodology and
numbers are recorded here because the fix round's corrections only make
sense against what they corrected.

**Safety, both rounds:** all measurement was read-only. No `psolve solve`
invocation in any script used here ever passes `-update`. All `sqlite3`
reads used `-readonly`. `~/astroops/state/catalogue.db`'s SHA-256 changed
between sessions (`249136aa...` -> `c20d8876...` -> `f2b58c30...`) with its
row count growing (10,373 -> 10,376 -> 10,378 `astap/astap+d50` rows) --
this is a live, externally-updated system (per this repo's own `CLAUDE.md`
context), not a write from this work: no command run here ever opens
`catalogue.db` outside `sqlite3 -readonly`, and no `psolve solve` call ever
carries `-update`. Spot-checked FITS frame mtimes (globular and
problem-corpus frames actually read) all show their original capture dates
(2025-05, 2025-06, 2026-08-11), none bumped to the measurement date.

---

## Round 1: the initial conditional design

### The statistic

```
concentration = max_cell_count / (n / n_cells)
```

on a **fixed 8x8 grid** (`n_cells = 64`), computed over the **brightest-`keep`
slice** of the post-rejection detections -- i.e. exactly the set legacy
`sort_by_flux_desc + truncate(keep)` would itself return, not the full
usable population. That distinction was found empirically: measuring over
the full population gave Omega Centauri a concentration near 1.0 (its full
~4,250-detection population is close to spatially uniform -- a globular's
outskirts genuinely fill the frame), even though the defect stratification
exists to fix is specifically that the *brightest* stars in a crowded field
are disproportionately core members.

### Round-1 finding: gating only the image side is not enough

Gating only `psolve-core`'s `extract::stratified_keep` (image-side
selection) while leaving `psolve-index`'s `stratified_in_disc` (catalogue
selection) unconditionally active made the full-corpus separation **worse**
than the original, already-regressed unconditional-everywhere design:
median separation 0.630" vs the fully-unconditional run's 0.563", against a
0.531" baseline. A quad matcher expects both point sets (image detections,
catalogue stars) to have been selected the same way; a legacy image against
a stratified catalogue is its own mismatch, worse than either side alone
being wrong.

The fix added a second gate, `psolve-cli`'s `cmd_solve::select_catalog`,
using an independently measured catalogue-side concentration (also on the
brightest-`limit` slice, over the disc's HEALPix cells rather than a pixel
grid -- the natural partition already exists there).

### Round-1 shipped state (later found to have defects -- see the fix round)

A single shared threshold (`5.0`) and a single shared decision function
(`should_stratify`) were used for BOTH sides, on the reasoning that both
statistics read ~1.0 for a uniform field and so a shared cutoff was
comparable. Full-corpus validation at that state: solve rate 9265/9495
(97.58%), separation median 0.531" (exactly the baseline), only 4
regressions (vs 46 under the unconditional design), 98.4% of both-solved
frames unperturbed. This looked clean and was reported as such -- **but the
calibration behind it was measured on the wrong statistic**, and the
"single shared constant" design was never actually exercised at its
disagreement cases. Both were caught in the fix round below.

---

## Round 2 (fix round): what review found, and the corrections

### 1. The image-side gate is inert on this corpus; the catalogue-side gate does all the work

`extract.rs` returns early whenever `stars.len() <= keep` (default 500), so
the image gate can only reach its own decision on frames with more than 500
usable detections. Measured: 7,385 of 9,495 real frames clear that bar
(most of this project's rigs produce many more than 500 usable detections
on a typical field -- the bar is not rare to clear), but of those, only
**3** actually cross the image-side threshold and stratify. Every one of
round 1's measured gains and regressions had `stars.used` well under 500 --
none of them ever reached the image-side gate's decision at all. The
catalogue-side gate is what decided all of them.

**Correction:** the round-1 doc comments characterised the image-side
statistic's distribution (rescued-target medians 6.7-12.0 vs not-helped
max 4.99) as the calibration evidence for the shared threshold. That
statistic has no causal role in round 1's own measured outcome. The
catalogue-side statistic is what actually needed calibrating -- see below.

### 2. One constant does not work for two differently-scaled statistics, even after normalisation

Measuring the CATALOGUE-side statistic's own distribution directly (not
inferred from the image side) found it reads ~2.4-3.3 for a uniform
catalogue under the round-1 formula, not ~1.0 -- because `cells_in_disc`
counts the PADDED candidate cell set (padded by `max_pixrad(nside)`, ~1.03
deg at nside 64, so a cell merely touching the disc is never missed by the
real fetch), while the numerator counts only genuinely in-disc stars. That
inflation meant round 1's shared threshold (5.0) was "5x uniform" on the
image side but only "~1.6x uniform" on the catalogue side -- two different
operating points wearing the same number, and the margin from the highest
ordinary-target frame measured (NGC_292, 4.78) to the gate (5.0) was 4.4%,
uncomfortably thin.

**Correction, part A -- renormalise the catalogue statistic.** Replaced the
padded HEALPix candidate-cell count with a pure-geometry effective cell
count: `disc solid angle / average HEALPix cell solid angle`
(`ncells_effective = 6 * nside^2 * (1 - cos(radius_rad))`, clamped to a
floor of 1). This depends only on the disc's own true area and `nside`, not
on which candidate cells happen to be touched by padding, so it scales
correctly with radius and stays comparable across every FOV (see finding 3
below for why that specifically matters). Measured uniform baseline after
this fix: **median 1.43** on the real corpus (down from ~2.4-3.3), close to
the image side's own ~1.0-2.7 range and no longer inflated by padding.

**Correction, part B -- stop sharing one constant.** Even after both
statistics are anchored near a ~1.0 uniform baseline, their REPORTED
distributions across the real corpus differ in scale throughout, not only
at that baseline:

| | image-side (`stars.concentration`) | catalogue-side (`catalog.concentration`) |
|---|---|---|
| median | 2.69 | 1.43 |
| p90 | 3.46 | 2.55 |
| p99 | 4.22 | 3.97 |
| max | 5.38 | 5.87 |

Applying the catalogue-calibrated value (2.0) to the image-side statistic
instead made the image gate fire on **6,931 of 9,495** real frames --
almost every ordinary frame -- rather than the 3 it fires on at its own,
separately calibrated value (5.0). The two statistics are now each
calibrated against their OWN measured distribution
(`psolve_core::extract::CONCENTRATION_THRESHOLD` = 5.0 for the image side;
`psolve-cli`'s `cmd_solve::CATALOG_CONCENTRATION_THRESHOLD` = 2.0 for the
catalogue side), compared only in the sense that both express "multiples of
a spatially uniform field", not as one literal shared number. Both call
through their own decision function so the two entry points
(`cmd_solve.rs`'s native path, `main.rs`'s ASTAP dispatch) can never drift
apart from EACH OTHER on the same side -- that guarantee is preserved; it
is the earlier claim that the two SIDES could never disagree that was
false and has been corrected.

The round-1 doc claimed "a shared constant means the two sides can never
disagree." That was checked and found false: on real data the two sides
disagreed on a nontrivial number of frames (each side firing independently
of the other), including cases where the image side stratified while the
catalogue side did not -- precisely the stratified-image/legacy-catalogue
mismatch round 1's own finding (section "Round-1 finding" above) showed is
harmful. Decoupling the thresholds does not reintroduce that mismatch
risk in practice: the image gate fires on essentially no real frames (3 of
9,495) regardless of its exact value, so in practice it is very rarely the
catalogue gate's partner in a query at all.

### 3. The catalogue statistic must be FOV-independent

`ncells` under the padded-candidate formula scales with `(radius + 1.03
deg)^2`, not `radius^2` -- for a narrow-field rig at radius <~0.5 deg, the
true disc sits inside 1-2 HEALPix cells while the padded candidate count is
7-9, so the OLD statistic would read 5-9 and fire UNCONDITIONALLY
regardless of any real clumping. This project's own corpus cannot exercise
that (every frame in it is 2.4-4.5 deg wide), which is exactly why it went
undetected until pointed out directly.

The pure-geometry renormalisation in finding 2 fixes this structurally: it
scales with the disc's own true area, so a narrow-field disc gets a small
`ncells_effective` reflecting its true tiny size, not an inflated padded
count. A disc narrower than a single HEALPix cell floors at
`ncells_effective = 1` -- there is nowhere else for the stars to be, so it
correctly reads as uniform and never fires.

### 4. `meaningful` must gate the decision, not only the report -- on BOTH sides

Caught while writing this fix and confirmed against real data: a small
candidate count relative to the (now correctly-scaled) cell count produces
a spuriously HIGH concentration from shot noise alone, on either side.

- **Image side:** a real corpus frame with 21 usable stars reported
  `concentration: 12.19` from nothing but which of 64 cells 4 of those 21
  happened to land in. 780 real frames read `>= 5.0` this way; 777 of them
  never reached the gate's own decision at all (`stars.len() <= keep`
  forces legacy regardless).
- **Catalogue side:** at a wide disc radius against a modest star count
  (`ncells_effective` in the thousands, candidate count in the hundreds), a
  genuinely uniform catalogue spreads to at most 1-2 stars per occupied
  cell purely because there are far more cells than stars, and reads as
  concentrated from that alone -- caught directly while writing
  `select_catalog`'s own test fixtures.

**Correction:** `CONCENTRATION_MIN_N` (image side, `= 64`, one candidate per
cell on average) and `catalog_concentration`'s `meaningful` flag (catalogue
side, `recs.len() >= ncells_effective`) now gate the DECISION itself on
both sides, not only the REPORTED number. Below the floor, the outcome is
forced to legacy regardless of what the raw statistic reads.

### 5. `stars.concentration` must not advertise stratification that did not happen

Fixed by making `Extraction::concentration` (and the equivalent
`catalog.concentration` in the JSON) `Option<f64>` -- `None` whenever the
gate could not have determined the outcome (image side: `stars.len() <=
keep`; catalogue side: `!meaningful`) OR the candidate count was too small
for the fixed/effective cell count to mean anything. A new `stars.stratified`
/ `catalog.stratified` boolean reports what actually happened,
independent of whether the underlying number is meaningful to show.

### 6. Source comments must describe what the code actually does

`extract.rs`'s `stratified_keep` doc used Omega Centauri as a worked
example. Fixed to say plainly what round 1's own measurement already
found: a real Omega Centauri frame reads `concentration: 2.048` on the
image-side statistic -- BELOW the gate -- so it is not rescued by
image-side stratification. This is consistent with (not contradicted by)
the established diagnosis that globular failures are upstream of
selection entirely (extraction floor / quad-matching against extreme
detection counts), which this milestone does not touch either way.

---

## Measured distributions (both statistics, corrected formulas)

All from real frames under `~/astroops`, read-only, current binary.

### Image-side (`stars.concentration`, brightest-`keep` slice, fixed 8x8 grid)

Reported only where meaningful (`stars.len() > keep` and `keep >= 64`).
Corpus-wide (8,256 of 10,376 frames meet that bar):

```
median 2.69   p90 3.46   p99 4.22   max 5.38
```

The 300-frame agreement-corpus sample specifically: median 2.59, p90 4.88.
The rescued targets (C 76, HD 93308, Eta Carina, M 8): medians 6.7-12.0.
The not-helped targets (Corona Australis, War and Peace, Centaurus A,
Caldwell 101): max 4.99. Threshold **5.0** sits in that gap. (These
per-target numbers are the ones that motivated round 1's choice; they
remain accurate DESCRIPTIONS of the image-side statistic, they are simply
not the reason the corpus outcome changed, per finding 1 above.)

### Catalogue-side (`catalog.concentration`, brightest-`limit` slice, disc-area/cell-area grid)

Corpus-wide (10,374 of 10,376 frames report a value):

```
median 1.43   p90 2.56   p99 3.97   max 5.87
```

The 300-frame agreement-corpus sample: median 1.40, p90 1.81, max 2.71.
The 276 baseline-failing frames: median 1.64. By target:

| group | targets | catalogue concentration |
|---|---|---|
| **rescued** | HD 93308 (2.46), C 76 (2.25), Eta Carina (2.71) | 2.25-2.71 |
| **rescued but missed by this threshold** | M 8 | 1.64 |
| **not helped** | Corona Australis (1.32), War and Peace (1.30), Centaurus A (1.10), Caldwell 101 (1.24) | 1.10-1.34 |

Centaurus A and Cats Paw Nebula (NGC 6334) -- the two targets whose
already-solving corpus frames regressed under the unconditional design --
measure 1.10 and 1.31 in the corpus sample specifically, comfortably below
threshold **2.0**. M 8's real, measured benefit (18/24 of its baseline
failures solved under the unconditional design) is NOT captured by this
threshold -- stated plainly, not hidden: M 8's catalogue concentration
(1.64) sits inside the "not helped" targets' range, and a threshold low
enough to catch it would also catch Corona Australis / War and Peace
(1.30-1.32), reintroducing exactly the false-trigger risk this gate exists
to prevent. This is the honest cost of choosing a threshold from the
data rather than one that rescues every previously-observed win.

### Globulars (30 frames: omegacen 23, ngc6681 3, ngc6809 4)

Image-side: median 2.11, mostly 1.9-2.4, overlapping the ordinary corpus
almost entirely -- does NOT separate from it, consistent with the
established diagnosis that globular failures are upstream of selection.
Catalogue-side was not separately re-measured for globulars in the fix
round (the image-side non-separation, plus the pre-existing
extraction/quad-matching diagnosis, already explains why they are not
rescued regardless of catalogue selection).

### Spec acceptance criterion 2 (Omega Centauri solves at defaults): NOT MET

A real Omega Centauri frame, current binary, defaults, read-only
(`omegacen` target, 300s frame, `--hint` only):

```
detected 21684, rejected.too_small 16216, used 500
stars.concentration 2.048, stars.stratified false
catalog.concentration 1.716, catalog.stratified false
NO_QUAD_MATCH
```

Both gates measure the frame correctly and both correctly decline to
stratify it: image-side concentration (2.048) sits below
`CONCENTRATION_THRESHOLD` (5.0), catalogue-side (1.716) sits below
`CATALOG_CONCENTRATION_THRESHOLD` (2.0) -- neither statistic reads this
frame as spatially concentrated on the slice each gate actually looks at,
which is consistent with (not contradicted by) round 1's own finding that
the *full* ~4,250-detection population is close to uniform. The failure is
upstream of both gates: 16,216 of the frame's 21,684 detections (75%) are
rejected below the `min_pix = 4` extraction floor before selection runs at
all, and the 500 stars that do survive to selection still lose at
quad-matching (`NO_QUAD_MATCH`) against a field this dense. Stratifying
*which* 500 stars get kept cannot fix a floor this milestone does not
touch.

**Verdict: criterion 2 is NOT MET**, in both the unconditional design
(`docs/superpowers/2026-08-15-stratified-selection-results.md`) and this
conditional one -- same upstream cause, unchanged by either round of this
fix. This corrects that document's own initial "correct negative, confirmed"
language, which conflated a diagnosis (globular failures are upstream of
selection, so this milestone was never going to fix them) with an
acceptance result (the criterion is still unmet, not passed).

### Spec acceptance criterion 4 (sham-rate floor): not evaluated

Unchanged from the unconditional design's own finding
(`docs/superpowers/2026-08-15-stratified-selection-results.md`, section 4):
the instrument does not exist anywhere on this machine (approved, not
built), so there is nothing to re-run against this round's commit. Not
re-checked this round; not assumed to pass.

---

## Full-corpus validation (final, both fixes applied)

`scripts/agreement.sh full`, read-only, 9,495 like-for-like frames against
the committed baseline (10,376 frames in the live corpus total; ~102s
wall).

| metric | baseline | round-1 shipped (later found miscalibrated) | **round-2 (final)** |
|---|---|---|---|
| solve rate | 9219/9495 (97.09%) | 9265/9495 (97.58%) | **9268/9495 (97.61%)** |
| separation median | 0.531" | 0.531" | **0.530"** |
| separation p90 | 0.947" | 0.948" | **0.945"** |
| separation p99 | 3.128" | -- | **3.098"** |
| separation max | 30.338" | -- | **30.688"** |
| gross errors (>30") | 1 | 1 (same frame) | **1 (same frame)** |
| previously-solving frames now failing | -- | 4 | **6** |
| previously-failing frames now solving | -- | 50 | **55** |
| net | -- | +46 | **+49** |
| image-side gate fires | -- (not measured) | -- (not measured) | **3/9,495** |
| catalogue-side gate fires | -- (not measured) | -- (not measured) | **496/9,495** |
| frames solved in both runs with a nonzero centre shift | -- | 145/9,215 (1.6%) | **386/9,213 (4.2%)** |
| median centre shift, whole both-solved population | -- | 0.0" | **0.000"** |

The round-1 and round-2 numbers are not directly comparable measurements of
the "same" change -- the corpus itself grew slightly between sessions (a
live, externally-updated system) and the catalogue statistic's formula
changed. Round 2's numbers are the current, correctly-calibrated state:
solve rate improves over baseline, separation median/p90/p99 all hold or
improve slightly, and the vast majority (95.8%) of previously-solving
frames are completely unperturbed.

---

## Tests

`crates/psolve-core/src/extract.rs`:
- `below_threshold_stratified_keep_is_bit_identical_to_legacy_sort_and_truncate`
  -- full-sequence equality against a manual sort+truncate.
- `the_gate_fires_on_a_clumped_fixture_and_not_on_a_uniform_one`.
- `the_gate_is_correct_right_at_its_own_boundary` -- finds the image-side
  gate's actual numeric boundary by search and checks both sides of it,
  rather than a fixture sitting at concentration 1.0 far from any
  realistic threshold value.
- `concentration_stat_is_near_one_for_a_spatially_uniform_field`,
  `concentration_stat_is_large_for_a_clump`,
  `concentration_stat_does_not_panic_on_degenerate_inputs`.
- `concentration_is_none_when_the_gate_was_never_reachable`,
  `concentration_and_stratified_agree_when_both_are_reported`,
  and the extended `results_are_sorted_brightest_first_and_capped_at_keep`
  -- the `Option<f64>`/`stratified` reporting contract.

`crates/psolve-cli/src/cmd_solve.rs`:
- `select_catalog_below_the_gate_returns_exactly_brightest_in_discs_result`
  -- the catalogue-side analogue of the image-side bit-identity test.
- `select_catalog_gate_is_correct_right_at_its_own_boundary` -- searches on
  `select_catalog`'s own `stratified` flag directly (not a hand-rolled
  reimplementation of its decision), which caught a real bug while writing
  this test: an earlier version compared `catalog_concentration` against
  the threshold alone, missing the `meaningful` gate `select_catalog` also
  applies.

`crates/psolve-cli/tests/cross_path_catalogue_selection.rs` (pre-existing,
Task 3): still green -- and is the fixture that originally caught finding 4
above indirectly (its catalogue-only clump, no image counterpart, only
passes when the catalogue-side gate is measured independently of the image
side).

447 tests total (437 original baseline + 10 across both rounds), `cargo
test --workspace` green, `cargo clippy --all-targets --workspace -- -D
warnings` clean.
