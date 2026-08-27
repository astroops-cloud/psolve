# Which detector improvements are worth doing: measured

**Date:** 2026-08-24. **Method:** each variant implemented in an out-of-tree
copy of `87961ac` and scored on **completeness** -- the fraction of in-frame
catalogue stars psolve detects and keeps, obtained by reprojecting the
catalogue through **ASTAP's** solved WCS and cross-matching at 5 px. Solve
rate is reported as the downstream check, not the objective.

**Frames:** the **15** ATR585M frames ASTAP still solves and psolve does not
after the quad-budget retry, and **12 controls** that both solve. ASTAP truth
was obtainable for **6 of the 15** and **11 of the 12**; those are the
denominators below, and they are small.

## Result

| variant | lost: completeness | lost: solved | **controls: solved** |
|---|---:|---:|---:|
| **baseline** (`sigma 5`, `min_pix 4`) | 11.0% | 0/6 | **11/11** |
| **matched filter, sigma 1.5** | **28.0%** | 2/6 | **11/11** |
| matched filter, sigma 1.0 | 27.9% | **3/6** | 9/11 |
| lower threshold (`--sigma 3.0`) | 21.9% | 2/6 | 10/11 |
| `--sigma 3.0 --min-pix 2` | 23.0% | 2/6 | 7/11 |
| `--sigma 2.0` | 17.4% | 1/6 | 8/11 |
| `--sigma 2.0 --min-pix 2` | 9.3% | 0/6 | 1/11 |
| `--keep 2000` | 11.0% | 0/6 | 9/11 |
| rank by SNR instead of flux | 11.0% | 0/6 | 11/11 |
| matched filter 1.5 + `--sigma 3.0` | 13.8% | 1/6 | 7/11 |

## Verdict on each

### 1. Matched filtering -- WORTH DOING, and it is the only clear win

Convolve the background-subtracted image with a Gaussian matched to the PSF,
threshold the **filtered** image, measure on the **original** -- SExtractor's
separation of detect-from-measure.

**At sigma 1.5 it lifts completeness on the failing frames from 11.0% to
28.0% -- 2.5x -- with no control regression at all (11/11).** That is the only
variant tested that improves the hard cases without costing the easy ones.

Sigma 1.0 recovers one more frame (3/6 against 2/6) but drops two controls.
On this evidence 1.5 is the better default, and the difference between them is
within noise at n=6 -- a real implementation should tune the kernel against the
frame's own measured FWHM rather than fix it.

### 2. Adaptive detection ladder -- worth doing ONLY as a retry, never globally

Lowering the threshold to `sigma 3.0` helps the failing frames (11.0% ->
21.9%, two recovered) and **costs a control** (11/11 -> 10/11). `sigma 2.0` is
worse on both -- there is an optimum and going past it admits more noise than
signal.

That is precisely the case for a ladder: apply the lower threshold **only when
the first pass has already failed**, exactly as ASTAP's `retries := 4` does and
as psolve's binning and quad-budget retries already do. As a global change it
is a net loss.

**It is also the weaker of the two.** Matched filtering beats it on the failing
frames (28.0% vs 21.9%) and does not regress controls.

### 3. Relaxing `min_pix` -- HARMFUL, confirming the earlier finding

`--sigma 3.0 --min-pix 2` scores 23.0% on the failing frames, marginally above
`sigma 3.0` alone, and collapses the controls from 10/11 to **7/11**.
`--sigma 2.0 --min-pix 2` is catastrophic: **1/11** controls.

Consistent with the earlier per-blob measurement -- the blobs `min_pix` rejects
are overwhelmingly single-pixel noise. Lowering the floor admits noise, and
without an aperture measurement there is nothing to distinguish it from signal.

### 4. Rank by SNR instead of flux -- NO-OP here

Identical to baseline on both sets, to a tenth of a percent. The reason is
mundane: `keep = 500` never binds on these frames, so the ranking never
truncates anything and cannot change what is kept. It may matter on dense
fields where the cap does bind; nothing here measures that.

### 5. Combining matched filtering with a lower threshold -- WORSE than either

28.0% alone, 21.9% alone, **13.8% together**, and controls drop to 7/11.
Lowering the threshold on an already-filtered image re-admits exactly the noise
the filter suppressed. Worth recording because "both good things together"
is the obvious next thing to try.

### 6. `--keep 2000` -- HARMFUL

No completeness change and controls fall 11/11 -> 9/11. More stars is not
better; it dilutes the quad set with faint detections that have no catalogue
counterpart.

## What none of this fixes

**Completeness on the failing frames reaches 28% at best, against 62-69% on
the frames that solve.** Matched filtering closes about a third of that gap.
These frames remain materially harder than the ones psolve handles, and no
variant tested makes them ordinary.

The untested candidate is the one ASTAP actually relies on: **acceptance on
SNR measured over an aperture**, rather than on connected-pixel count. ASTAP
measures HFD over a 14-pixel annulus and accepts on `snr > 10`, so a faint star
whose above-threshold footprint is two pixels is still measured and can pass.
psolve discards it before measuring anything. That needs real aperture
photometry, which is a larger change than anything here, and it is plausibly
what makes ASTAP's threshold ladder work where psolve's proxy does not.

## Caveats

- **n = 6 and n = 11.** The solved counts especially are noisy; treat
  completeness as the signal and solve rate as corroboration.
- Completeness is measured against **ASTAP's** WCS. Where ASTAP is wrong the
  reference is wrong, and two frames in the corpus are known to be.
- The matched filter here uses a fixed kernel sigma. A real implementation
  should derive it from the frame's measured FWHM.
- All variants were run in an out-of-tree copy; **nothing here is committed to
  the solver**, and the `std::env::var` switches used to A/B them would trip
  `psolve-core`'s `no_filesystem` guard if they ever were.
