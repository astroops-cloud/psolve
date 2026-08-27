# Spatially Stratified Star Selection — Design

**Date:** 2026-08-15
**Status:** proposed
**Prerequisite for:** the AstroOps integration case, and (indirectly) `2026-08-15-blind-solve-design.md`

## 1. The problem, as measured

psolve fails on 276 of the 9,495 frames ASTAP solved — 2.9%. The failures are
not scattered. Of the 276, **267 are `NO_QUAD_MATCH`**, and they concentrate in
dense fields:

| target | missed | of | rate |
|---|---|---|---|
| C_76 | 16 | 148 | **10.8%** |
| HD_93308 | 71 | 933 | **7.6%** |
| Eta_Carina | 14 | 221 | 6.3% |
| M_8 | 24 | 1945 | 1.2% |
| Eta_Carinae_Nebula | 7 | 979 | 0.7% |

The same root cause makes globular clusters fail outright. Omega Centauri
(NGC 5139) does not solve on defaults at all — 20,575 detections against 1,500
catalogue stars, `NO_QUAD_MATCH` — and solves only at
`--cat-limit 5000 --keep 2000 --radius 3.0`.

Every one of those flags is a **truncation default**, not a physical limit. A
cluster failing looks to the caller like "unsolvable frame" rather than "wrong
flags", which is the same misdirection class as the `FOV_MISMATCH`-for-missing-
hint defect fixed on 2026-08-14.

## 2. Root cause

Both sides of the match are truncated by **brightness**, and brightness is
spatially correlated.

- `extract.rs` sorts detections by flux descending and truncates to `keep`
  (default 500).
- `reader.rs::brightest_in_disc` returns the **N brightest** catalogue stars in
  the disc.

In a sparse field these coincide with a spatially uniform sample and everything
works. In a dense field they do not. Omega Centauri's brightest 500 detections
are all cluster-core members — blended, mutually contaminated, and occupying a
few arcminutes — while the catalogue's brightest 1,500 spread across the whole
disc. The two sets barely overlap geometrically, so no consistent transform
exists to be found.

This also degrades quad geometry generally: quads drawn from one corner of the
frame constrain the fit far more weakly than quads spread across it.

## 3. Design

**Select spatially, then by brightness within each cell.**

### 3.1 Image side

Partition the frame into a grid of `G x G` cells. Give each cell a budget of
`keep / G^2` detections and fill it with that cell's brightest.

`G` adapts to the crowding, since a uniform grid on a sparse frame is a no-op:

```
G = clamp(round(sqrt(detected / keep) * 2), 1, 16)
```

At `detected = 500` (sparse) this gives `G = 2`, near-identity. At
`detected = 20,575` (Omega Centauri) it gives `G = 12`, so 144 cells with a
budget of ~3 each — which is exactly the spread the matcher needs.

**Surplus redistribution is load-bearing.** A naive per-cell cap silently
*loses* stars: a frame with signal in half its cells would return `keep/2`.
After the first pass, redistribute unfilled budget to cells that still have
candidates, repeating until either `keep` is reached or no cell has anything
left. Without this, sparse and vignetted frames regress.

### 3.2 Catalogue side

The same principle, but the natural partition already exists: the index is
HEALPix-cellular. Rather than taking the globally brightest `cat_limit`, take
the brightest per HEALPix cell intersecting the disc, with the same
budget-and-redistribute rule.

This needs a new reader method alongside `brightest_in_disc` and
`stars_in_disc`; it must reuse `cells_in_disc` and `angsep_deg` rather than
reimplementing either.

### 3.3 What does not change

- `keep` and `cat_limit` keep their current defaults and meanings — this
  changes *which* stars are chosen, not how many.
- An explicit `--keep` or `--cat-limit` is still honoured exactly.
- `psolve-core` stays dependency-free with no filesystem access.
- The ASTAP-compatible path gets this for free, since both entry points share
  the extraction and catalogue-fetch code.

## 4. Acceptance criteria

Four independent measures already exist. All four must be reported.

1. **The 276 misses shrink**, and specifically the dense-field concentration
   flattens. C_76 at 10.8% and HD_93308 at 7.6% are the numbers to move.
2. **Omega Centauri solves at defaults** — no `--cat-limit`, `--keep` or
   `--radius` flags — and lands within 30" of an independent position check.
3. **No regression on the 9,495-frame agreement corpus.** Separation median is
   currently 0.531", p90 0.947", p99 3.128", with 0 scale outliers and 0 parity
   mismatches. Median must not worsen and the solve rate must not fall.
4. **The sham-rate floor drops or holds** on the crowded fields. The
   `astroops-ai` session measured 0.046 / 0.065 / 0.205 / 0.646 across
   IC 4592 / HD 37805 / Eta Carinae / Omega Centauri. It is an independent
   instrument built by another team, which is precisely what makes it useful
   here.

**A change that improves solve rate while worsening separation is a
regression**, not a trade. The CFA binning attempt on 2026-08-15 failed exactly
that way — solve rate 85.8% -> 69.4% while p50 fit RMS *improved* — and was
withdrawn.

## 5. Risks

**Star selection is upstream of everything.** Every downstream measurement in
this project was taken with brightest-N selection in place, so the full
agreement run is mandatory, not optional.

**Sparse fields are the regression risk, not dense ones.** The redistribution
rule is what protects them; it needs a test with a frame whose signal occupies
a minority of cells.

**The grid interacts with `min_pix`.** The CFA attempt showed that a fixed
extraction threshold can be silently co-tuned with something else. If
stratification changes which stars survive, re-check the rejection breakdown
rather than only the solve rate.

## 6. Open questions

- Should the image grid be in pixel space or projected sky space? Pixel space
  is simpler and adequate for these fields; sky space matters only for very
  wide fields with significant projection distortion.
- Should the catalogue side stratify by HEALPix cell (natural, free) or by a
  projected grid matching the image's (more symmetric, more work)? Start with
  HEALPix.
- Does `G`'s formula need a per-rig override? Prefer not — a tunable that
  nobody tunes is a default with extra steps.

## 7. Out of scope

The CFA double-binning defect (`fix-cfa-double-binning`, withdrawn at
`7ebda12`) is a separate problem with a separate cause. It should be revisited
**after** this lands, using the sham-rate floor as its measure, because
stratification may change the extraction picture it depends on.
