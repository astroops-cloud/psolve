# How ASTAP does it, and where psolve differs

**Date:** 2026-08-24. **Source:** ASTAP at `github.com/han-k59/astap`,
`unit_find_quads.pas` and `unit_astrometric_solving.pas`. **Licence: MPL 2.0.**

> **No ASTAP code is or may be copied into psolve.** MPL 2.0 is file-level
> copyleft: any file containing its code must remain MPL, which is
> incompatible with psolve's MIT licensing. What follows describes the
> **approach** -- facts and ideas, which carry no copyright -- so that psolve's
> own implementation can be designed with knowledge of a working reference.
> Anything written as a result must be written from scratch.

Read because psolve loses 19 frames to ASTAP on the primary rig and four
successive hypotheses for why had been refuted by measurement.

## Four design differences, each matching a measured deficit

| | ASTAP | psolve | measured consequence |
|---|---|---|---|
| **quads per star** | exactly **one**, from the star's **3 nearest** neighbours; total quads = star count, **no fixed cap** | combinations from the **6 nearest**, interleaved across seeds, **capped at 600 total** | at 500 stars psolve's cap binds; raising it to 1500 recovers **4 of 7** star-rich frames |
| **minimum star size** | **`hfd_min = max(0.8, min_star_size_arcsec / (binning * arcsec_per_px))`** -- specified in **arcsec**, converted to pixels by the plate scale, and measured as **HFD** | **`min_pix = 4`** -- a fixed **connected-pixel count** | on the ATR585M (2.46"/px, the finest-sampled rig) the fixed floor rejects **76-96%** of detections |
| **image star count** | `max_stars` (default 500) **reduced to the database's expected star count for this FOV** -- "Database limit for this FOV is N stars" | fixed **`keep = 500`**, independent of what the catalogue can supply | at `keep=500` completeness is **32.5%**; without the cap it is **71.7%** |
| **downsampling** | auto-bins so the scale stays coarser than ~1.5"/px, capped at 16 | no astrometric downsampling (only CFA superpixel binning) | not a factor for this fleet -- ATR585M at 2.46"/px would not be binned by that rule either |

## The two that matter most

### 1. Star size in arcsec, not pixels -- and HFD, not pixel count

Two separate ideas, both absent from psolve.

**Physical units.** ASTAP's floor is an angular size divided by the plate
scale. psolve's is a pixel count. A fixed pixel count means a different
physical threshold on every rig, which is exactly why it bites hardest on the
finest-sampled one. `ExtractParams` already contains the counter-example --
`max_pix_factor` is documented as relative *because* "a fixed pixel count is
wrong at a different focal length" -- so psolve applies the principle at the
top of the size range and not at the bottom.

**A threshold-independent measure.** Connected-pixel count depends on the
detection threshold: lower `--sigma` and every blob grows. Measured on frame
15181, dropping sigma 5.0 -> 2.5 took detections from 266 to 51,796 while
usable stars rose only 8 -> 28, because the additions were single-pixel noise
that the floor then rejected. HFD does not move that way. **psolve's
`too_small` count is partly an artefact of its own threshold.**

### 2. Match the image star count to what the catalogue can supply

ASTAP computes the database's expected star count for the field
(`density * fov^2 * aspect`) and, if that is smaller than `max_stars`, uses
the smaller number.

This is the completeness problem solved from the other side. psolve keeps 500
image stars regardless; when the catalogue can only supply a few hundred in
that field, the extra image stars are ones with no possible counterpart, and
every quad built from them is wasted.

Worth noting how this differs from a test already run and recorded as refuted:
lowering `--cat-limit` reduces the **catalogue** to match the image. ASTAP
reduces the **image** to match the catalogue. Those are different levers and
only the second was ASTAP's. The refutation of the first stands; it says
nothing about the second.

## What this does NOT change

The quad-budget retry spec (`specs/2026-08-24-quad-budget-retry-design.md`)
stands as written. Its measurement -- 5 of 19 recovered at `max_quads` 1500,
41/41 controls unaffected -- is unchanged by anything here, and ASTAP's
one-quad-per-star design is an argument for the direction rather than against
the specific fix.

Two psolve hypotheses were **refuted** by reading the source, and are recorded
so they are not revived:

- *"psolve's 600 quads come from only ~30 stars because each star yields
  C(6,3)=20 combinations."* **False.** `build_quads` already collects per-seed
  and interleaves, with a comment saying it does so precisely to avoid
  spatially-clustered truncation.
- *"ASTAP must use a wider code tolerance."* Not supported -- its quad
  tolerance is a user setting the source comments describe as needing to stay
  small, and psolve's own tolerance was measured to 2.5x with no recoveries.

## Suggested reading order for anyone following up

`unit_find_quads.pas` for quad formation (218 lines, one procedure does the
work). `unit_astrometric_solving.pas` around the solve entry point for the
star-count limit, the scale-aware `hfd_min`, and the binning rule.

---

## CORRECTION, same day: the scale-aware size filter was tested and does NOT help

The table above lists ASTAP's `hfd_min` against psolve's `min_pix = 4` and
attributes the 76-96% `too_small` rejection rate to it. **That attribution is
wrong, and testing it is what showed so.**

psolve computes `fwhm_px` from second moments already -- just *after* the
`min_pix` filter rather than before it -- so the swap looked like a
reordering. Instrumented to compute FWHM for every rejected blob and
cross-matched against catalogue stars reprojected through ASTAP's WCS:

| frame | in-frame catalogue stars | kept today | rejected as `too_small` **and real** | recovered by an FWHM >= 0.8px floor |
|---|---:|---:|---:|---:|
| 15181 (star-rich) | 1,599 | 519 | 40 (2.5%) | **4** |
| 15186 (star-poor) | 138 | 5 | 6 (4.3%) | **1** |

**The blobs `min_pix` rejects are overwhelmingly not stars.** Their median
`npix` is 1 and their median FWHM is 0.00 px -- single-pixel noise. An
FWHM-based, scale-aware floor would recover **one to four stars per frame**,
which is nothing against a completeness deficit of hundreds.

So the `too_small` count, which looked damning at 76-96%, is a red herring:
it is dominated by noise the filter is correctly discarding.

### What that leaves for the star-poor frames

Frame 15186 has **138 catalogue stars in the frame and psolve detects about
11 of them** (5 kept, 6 rejected). That is not a filtering problem at any
threshold -- **it is a detection problem.** ASTAP solved the same frame
reporting "Used stars down to magnitude: 9.3", i.e. on a handful of bright
stars.

The remaining lead is therefore `find_stars` in `unit_star_align.pas` --
ASTAP's detector -- and not its size filter. That is a larger investigation
than a threshold change and it has no design yet.

### Also refuted while checking

*"ASTAP falls back to three-star triangles when the star count is low."*
**No.** The source contains a comment saying triples "can be beneficial" for
low star counts and that stricter tolerances would be required. It is a note
about what might help, not an implementation. There is no triangle path.

### Standing status of the four differences

| difference | status |
|---|---|
| quad count scales with star count | **confirmed useful** -- 4 of 7 star-rich frames recovered, spec written |
| scale-aware HFD size floor | **tested, does not help** -- recovers 1-4 stars per frame |
| image star count capped by catalogue availability | **tested -- no-op on this fleet** (see below) |
| adaptive downsampling | not a factor for this fleet |

---

## The star-count cap: tested, and it is a no-op here

ASTAP caps `max_stars` at the database's expected star count for the field.
The psolve equivalent needs no code change -- `--keep` is the same knob -- so
the rule was applied directly to all 19 lost frames: count the catalogue stars
actually available in the frame footprint, then solve with
`--keep min(500, available)`.

**The cap never binds.** Catalogue stars available per ATR585M field, at
G<=14, across the 19: **648 to 18,189, median 2,093.** Every one exceeds 500,
so `min(500, available)` is 500 -- identical to the default. All 19 frames
returned exactly the same result capped as uncapped.

The rule is sound and psolve would benefit from it on a field where the
catalogue is genuinely sparse. **That field does not occur on this fleet**,
because the G<=14 index is deep enough that an ATR585M frame always has
thousands of catalogue stars available. ASTAP needs the rule because its `d50`
database is shallower; psolve does not, because its index is not.

Worth keeping as a note rather than a task: if psolve is ever pointed at a
much shallower catalogue, or at a much narrower field, this becomes live.

## Final status of the four differences

| difference | status |
|---|---|
| **quad count scales with star count** | **confirmed useful** -- 4 of 7 star-rich frames recovered; spec written |
| scale-aware HFD size floor | tested, recovers 1-4 stars per frame -- not the deficit |
| image star count capped by catalogue availability | tested, **no-op** -- the cap never binds on this fleet |
| adaptive downsampling | not a factor -- ATR585M at 2.46"/px would not be binned by ASTAP's rule either |

**One of four is actionable.** Reading a working reference implementation
produced one confirmed fix and three refutations, which is a better return
than it sounds: each refutation closed a line of investigation that looked
plausible from the outside, and two of them (the size floor, the star cap) had
already been written into this project's own docs as probable causes.

The star-poor group remains unexplained. Frame 15186 has 138 catalogue stars
in the frame and psolve detects about 11 of them, against ASTAP solving it on
stars down to magnitude 9.3. That is a detector difference, and nothing in
ASTAP's solving unit accounts for it -- the next place to look is
`unit_star_align.pas`'s `find_stars`, which this comparison did not open.

