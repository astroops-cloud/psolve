# Improving star detection: what the literature and ASTAP both say

**Date:** 2026-08-24. Written after the quad-budget retry closed the star-rich
half of the ATR585M gap (`2026-08-24-quad-budget-retry-results.md`) and left
the star-poor half untouched: **15 frames ASTAP solves and psolve does not**,
where psolve detects a handful of the catalogue stars present -- about **11 of
138** on frame 15186.

That is a detector problem. This records what a detector is supposed to do.

## psolve's detector, stated plainly

One pass. Threshold every pixel at `local_background + 5.0 * local_sigma`,
take connected components, reject any with fewer than 4 pixels, reject on
ellipticity and extent, keep the brightest 500 **by flux**.

Every element of that is a defensible first implementation. Together they are
the weakest detector of the three approaches below.

## 1. ASTAP escalates its detection threshold; psolve has one shot

**This is the single clearest difference and it maps directly onto the failing
population.** From `unit_star_align.pas`:

```
retries := 4;  { try up to four times to get enough stars from the image }
repeat
  if retries = 4 then  detection_level := head.star_level    ...
  if retries = 3 then  detection_level := head.star_level2   ...
  if retries = 2 then  ...
```

ASTAP runs the whole detection pass at a high threshold, and if it does not
have enough stars, lowers the threshold and runs again -- up to four times.
psolve thresholds once at `k_sigma = 5.0` and reports `TOO_FEW_STARS` or
proceeds with what it has.

A star-poor frame is exactly the case this ladder exists for.

## 2. ASTAP accepts on SNR measured over an aperture; psolve on connected-pixel count

ASTAP's acceptance, same file:

- find a pixel above `background + detection_level`;
- check the four orthogonal neighbours against `background + 4 * noise`, and
  require **at least 2** -- a cheap PSF-shape test, commented "At least 3
  illuminated pixels. Not a hot pixel";
- measure **HFD over a 14-pixel annulus**;
- accept if `hfd <= 30` **and `snr > 10`** and `hfd > hfd_min`;
- mark a star area of radius `3 * hfd` so it is not re-detected.

The consequence: **a faint star whose above-threshold footprint is two or
three pixels is still measured over a 14-pixel annulus and can have a good
SNR.** psolve discards it at `min_pix = 4` before measuring anything.

This is why relaxing `min_pix` did not help when tested -- psolve has no
aperture measurement to fall back on, so lowering the floor admits noise
rather than recovering stars. The floor is not the problem; the absence of a
measurement is.

## 3. Neither tool matched-filters. The literature says both should

The standard technique for detecting faint point sources is to **convolve with
a kernel matched to the PSF before thresholding**. SExtractor's own paper and
documentation put it directly: for detecting faint point sources the PSF gives
optimum results as a convolution mask, and detection is normally performed on
an image convolved with a Gaussian of about the PSF's FWHM, "acting as a
matched filter". astrometry.net does its own image-processing pass before
extraction for the same reason.

psolve thresholds **raw pixels**. So does ASTAP, as far as its detection
routine shows.

Two consequences worth separating:

- **It would raise faint-star S/N**, which is the star-poor case directly.
- **It would suppress single-pixel noise**, which is what `min_pix = 4`
  currently exists to fight. Measured on frame 15181: dropping `--sigma` 5.0
  to 2.5 took detections from 266 to **51,796** while usable stars rose only
  8 to 28, because the additions were single-pixel noise the floor then
  rejected. A matched filter attacks that at the source.

**This is the one improvement available to psolve that ASTAP does not already
have**, which makes it the most interesting and the least certain.

## 4. ASTAP selects the brightest by SNR; psolve by flux

`QuickSort_starlist_onSNR` -- "sort on SNR DESCENDING, highest SNR first" --
feeds `get_brightest_stars`. psolve sorts by flux.

On a crowded or blended field the brightest-by-flux detections include merged
pairs and saturated cores, which have no single catalogue counterpart. Ranking
by SNR prefers well-measured stars. Cheap to change; effect unmeasured.

## Suggested order

1. **Adaptive detection ladder.** Largest expected effect, targets the exact
   failing population, and mirrors two retry patterns psolve already has
   (`solve_with_binning_retry`, and the quad-budget retry added today). Fits
   this codebase's established shape.
2. **SNR over an aperture as the acceptance test**, replacing connected-pixel
   count. Bigger change -- it needs an aperture measurement psolve does not
   have -- but it is what makes 1 worth having: lowering a threshold without
   it just admits noise.
3. **Matched filtering.** Textbook-optimal, absent from both tools, and the
   only item here that could put psolve ahead rather than level.
4. **Rank by SNR rather than flux.** Small, cheap, unmeasured.

## How any of it gets tested

**Completeness, measured directly** -- the fraction of in-frame catalogue
stars psolve detects, obtained by reprojecting the catalogue through ASTAP's
solved WCS and cross-matching. Frame 15186 sits at about 8%; the frames that
solve sit at 63%. That harness exists
(`2026-08-24-atr585m-diagnostic.md`) and it is fast: no corpus run is
needed to iterate, and three hypotheses were killed with it in twenty minutes.

Tune against completeness; verify on solve rate afterwards. Inferring detector
quality from solve rate alone is how a change once improved fit RMS while
dropping solve rate 85.8% to 69.4%.

## Sources

- SExtractor: Bertin & Arnouts 1996, <https://aas.aanda.org/articles/aas/pdf/1996/08/ds1060.pdf>
- Source-extraction comparison and filtering practice, A&A,
  <https://www.aanda.org/articles/aa/full_html/2021/01/aa36561-19/aa36561-19.html>
- Astrometry.net: Lang et al. 2010,
  <https://iopscience.iop.org/article/10.1088/0004-6256/139/5/1782>
- Astrometry.net code README (source extraction, `image2xy`),
  <https://astrometry.net/doc/readme.html>
- ASTAP source, MPL 2.0, <https://github.com/han-k59/astap> --
  `unit_star_align.pas`. **Read for approach only; no code copied, and none
  may be: MPL 2.0 is file-level copyleft and incompatible with MIT.**
