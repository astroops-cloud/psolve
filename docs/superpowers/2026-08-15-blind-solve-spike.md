# Blind-solve spike: quad formability at catalogue depth

Date: 2026-08-15. Read-only against the three existing `.psidx` star indexes
under `~/astroops/data/`. Throwaway tool lives at
`/private/tmp/claude-501/.../scratchpad/blindspike/` (path dependency on
`psolve-core`/`psolve-index`), never touched the repo working tree.

Non-modification verified: the three `.psidx` files have byte-identical size
and mtime before and after every run (`stat` captured pre-run and re-checked
post-run); `find ~/astroops/data -newer <touch -t reference>` and
`find ~/gaia-dr3 -newer <reference>` both return empty. (A blanket sweep of
all `~/astroops` shows unrelated files changing — that's the live production
pipeline's own logs/DBs running concurrently, not this spike; scoping the
check to `~/astroops/data`, the only subtree this spike ever opened, is
clean.)

## Method

- Bands: 0.25, 0.5, 1, 2, 4, 8 deg (doubling, per spec).
- Sky area covered by the dec<=45 catalogues: 35,211.6 deg^2 (derivation:
  2*pi*(sin45+1) sr). Tiles per band = area/band^2, matching the design
  doc's own table almost exactly (563,386 / 140,846 / 35,212 / 8,803 /
  2,201 / 550).
- Tile centres: 167 points stratified across 8 galactic-latitude bins
  (|b| edges 0/5/10/20/30/40/60/80/90, both hemispheres, 12 longitudes each,
  clipped to dec<=45), weighted by each bin's true full-sky solid-angle
  fraction for area-weighted statistics. Galactic <-> equatorial via the
  standard Hipparcos J2000 rotation matrix.
- Per tile: `Index::brightest_in_disc` (read-only, mmap) for the brightest N
  stars within radius = band/2, projected to the tangent plane at the tile
  centre (`psolve_core::project::radec_to_tangent`), then
  `psolve_core::quad::build_quads` reused verbatim.
- Two regimes measured:
  1. **Exploratory/uncapped** (star cap 300, neighbours 8, max_quads 20000):
     answers "how much quad-forming capacity exists" — reveals genuine star
     scarcity vs. combinatorial capacity.
  2. **Production-shaped** (star cap 12, neighbours 6, max_quads 25): a
     builder that stops once it has enough quads per tile, matching
     astrometry.net's 10-30/tile target and used for the storage/build-time
     extrapolations.

## 1-2. Quads/tile distribution and shallowest usable band

**Usable, defined**: area-weighted fraction of sky with <10 quads/tile
(under the production-shaped 25/tile-cap regime) is <=~5%, i.e. blind
solving isn't expected to fail across a material fraction of the sky at
that band.

G<=14, band=0.25 deg, mean quads/tile by galactic-latitude bin (production
cap = 25/tile):

| |b| bin | 0-10 | 10-20 | 20-30 | 30-40 | 40-60 | 60-80 | 80-90 |
|---|---|---|---|---|---|---|---|
| mean quads | 25.0 | 25.0 | ~24 | 22.4 | 26.1 | 10.6 | 12.0 |
| mean stars in tile | 19.6 | 19.1 | 15.7 | 9.8 | 6.6 | 4.1 | 4.4 |
| frac of tiles <10 quads | 0.00 | 0.00 | 0.00 | 0.19 | 0.33 | 0.74 | 0.75 |

(bins 4-5 land near the cap on average but are already degrading; bins 6-7
— |galactic latitude| beyond ~60 deg — fail outright on 3/4 of sampled
tiles.) G<=16 and G<=18 hold >=24.8 mean quads/tile in *every* bin,
including the worst (poles), at this same 0.25 deg band.

At G<=14, 0.25 deg: bins 0-3 (|b|<30, ~74% of sky by area) hit the 25-quad
cap cleanly. Bins 4-7 (|b|>30, ~26% of sky, worst at the galactic poles)
degrade sharply — mean stars in a 0.25 deg tile falls to 4-10, and 33-75%
of individual tiles fall below the 10-quad floor (many have 0-3 stars and
literally 0 quads). Area-weighted: 20% of the sky fails the usability bar
at G<=14/0.25 deg.

At G<=14, 0.5 deg: weighted fraction <10 quads drops to 1%, worst bin mean
stars = 16.4, mean quads = 480+. **G<=14's shallowest usable band is 0.5
deg, not 0.25 deg** — 0.25 deg has a real minimum-field-size problem at
this depth, concentrated at |galactic latitude| > ~40-60 deg.

At G<=16 and G<=18, 0.25 deg already clears the bar everywhere (weighted
frac<10 = 0.01, worst bin mean stars 13-14 and 36 respectively, mean quads
357+ and 1200+). **Shallowest usable band for both = 0.25 deg**, the
shallowest band tested.

Representative distribution at band=1.0 deg, exploratory/uncapped regime
(shows true combinatorial headroom, not the production cap), G<=16, n=167
tiles: mean 10,388, p10 7,841, p25 9,402, median 11,183, p75 11,402, p90
11,484, min 0 (one degenerate high-latitude tile), max 11,766. Every depth
and every band >=0.5 deg has 2-3 orders of magnitude more raw quad-forming
capacity than the 10-30/tile target — the real constraint is a storage
budget cap, not availability.

## 3. Storage

Production-shaped regime (12-star budget, 25-quad cap/tile), summed across
all 6 bands, at 24 bytes/quad (spec's assumed record size: a quantized
4x i16 code = 8 bytes + 4x u32 star references = 16 bytes):

| depth | quads | storage |
|---|---|---|
| G<=14 | 15,637,613 | 375.3 MB |
| G<=16 | 18,674,481 | 448.2 MB |
| G<=18 | 18,678,048 | 448.3 MB |

This closely **validates** the spec's ~15M-quad/~360MB estimate for
G<=14 (15.6M/375MB measured) — provided the 0.25 deg band's known gap at
high galactic latitude is accepted or the index is built one depth deeper.
G<=16/G<=18 total slightly higher (they don't have that gap, so 0.25 deg
hits its cap everywhere too).

Contrast: the exploratory/uncapped regime (star cap 300) produces 36-45M
quads / 860MB-1076MB — 2.3-2.9x bigger. **A per-tile quad-count cap in the
builder is not optional**; without one, storage blows the spec's budget by
roughly 3x.

## 4. Build cost

Single-threaded wall-clock, summing per-band `avg_ms/tile * tiles`, across
all 750,998 tiles (6 bands), production-shaped regime:

| depth | time |
|---|---|
| G<=14 | 692 s (~11.5 min) |
| G<=16 | 732 s (~12.2 min) |
| G<=18 | 884 s (~14.7 min) |

Dominated by the 563,386 tiles of the 0.25 deg band (~520-660s of the
total). Tile queries are embarrassingly parallel (independent disc queries
+ quad builds); with 8-16 cores this comes down to well under 2 minutes.
The exploratory/uncapped regime (star cap 300) costs 2528-2863s (42-48
min single-threaded) for the same sweep — confirming the per-tile emission
cap is also a ~3.5-4x build-time win, not just a storage win. Both regimes
are tractable; this is not a blocker.

## 5. Code-space distribution

1,734,729 quad codes sampled (G<=16, band=1.0 deg, exploratory regime,
pooled across 167 tiles). The canonical 4-vector is `[x_C, y_C, x_D, y_D]`.
20x20 histograms over three coordinate pairs:

| pair | bin mean | bin std | CV (std/mean) | max bin / mean | empty bins |
|---|---|---|---|---|---|
| (x_C, y_C) | 4,337 | 3,203 | 0.74 | 3.0x | 67/400 (17%) |
| (x_D, y_D) | 4,337 | 6,541 | 1.51 | 9.3x | 68/400 (17%) |
| (x_C, x_D) | 4,337 | 5,804 | 1.34 | 5.4x | 157/400 (39%) |

**The code space is clustered, not uniform.** Coefficients of variation of
0.74-1.51, hotspot bins holding 3-9.3x the mean occupancy, and up to 39% of
a naive equal-width grid's cells sitting empty. This is the same effect
astrometry.net's own literature notes (part of why their reference index
uses a kd-tree rather than a flat grid). **The spec's "a grid hash is
probably enough" assumption does not survive measurement.** A uniform
equal-width grid hash would produce badly unbalanced buckets — some cells
absorbing an order of magnitude more entries than others, which degrades
exactly the O(1)-lookup property a grid hash exists to provide. An
equal-population/quantile-based grid (adaptive bin edges) or a kd-tree is
needed; this should be settled with a small prototype before the `.psqidx`
format is locked, not deferred to implementation.

## Recommendation

**Proceed, with two reshapes before the format is fixed (spec phase 2):**

1. **Build the blind index from G<=16, not G<=14.** G<=14's 0.25 deg band
   has a genuine minimum-field-size gap across ~20% of the sky by area
   (galactic latitude beyond ~40-60 deg) — 33-75% of individual tiles
   there fall below the usability floor, some with literally 0-3 stars.
   G<=14's *usable* floor is 0.5 deg, not 0.25 deg. G<=16 and G<=18 both
   clear 0.25 deg cleanly everywhere; G<=16 is the shallower/cheaper of
   the two to build and index and is recommended as the source depth.
2. **Do not default to a uniform grid hash.** Measured code-space
   clustering (CV up to 1.5x, up to 9.3x hotspots, up to 39% empty cells)
   means a flat equal-width grid would have badly unbalanced buckets. Use
   quantile/equal-population bucket edges (simple, stays a grid) or a
   kd-tree; prototype the choice before locking `.psqidx`.

Storage (~375-450 MB) and build time (~12-15 min single-threaded, minutes
if parallelized) are both fine and do not gate the milestone — the spec's
sizing was directionally right. Nothing here says stop; two design
parameters (source depth, search-structure shape) need correcting before
committing to the format.
