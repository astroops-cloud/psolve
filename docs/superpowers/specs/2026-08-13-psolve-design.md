# psolve — design

**Date:** 2026-08-13
**Status:** design approved, not yet implemented
**Repo:** `github.com/astroops-cloud/psolve`

---

## 1. What this is

A plate solver: FITS in, WCS out. Written in Rust, shipped as a single static
binary, and built for headless automation rather than for a GUI that happens to
have a CLI.

It exists to be a better ASTAP for this workload — leaner, faster, and able to
say *why* a frame did not solve. **ASTAP stays installed and keeps running the
AstroOps pipeline.** `psolve` competes with it on measured merit, and the
switch-over (if it ever happens) is a symlink flip in both directions.

A secondary goal shapes the CLI: Siril already integrates ASTAP by shelling out
to `astap_cli` and reading its `.ini`/`.wcs` sidecars. By being CLI-compatible,
`psolve` is usable from Siril and N.I.N.A. with no patch to either.

### Non-goals

- Replacing ASTAP for anyone else, or in the GUI.
- A general FITS or astronomy library. We implement what we need.
- MQTT inside the binary. The server speaks NDJSON; the AstroOps agent bridges.
- Beating astrometry.net at blind solving. Blind is a fallback path here.

---

## 2. Evidence: the spike (2026-08-13, the workstation)

Every number below was measured on the workstation against real frames from
`~/astroops/library`, single-threaded, before any design was committed to.
Inputs were verified byte-unchanged afterwards.

**Test frame:** `eagle/lights/H/2026-07-29_22-47-02_H_120.00s_100g_1x1_0001_-10.00.fits`
— 3840×2160, `BITPIX=16`, `BZERO=32768`, 16.6 MB. `FOCALLEN=243` mm,
`XPIXSZ=2.9` µm → **2.461 ″/px, 2.626° × 1.477°** (matches the documented 1.48°).

### 2.1 Rust extraction prototype

| stage | warm | cold |
|---|---:|---:|
| file read (16.6 MB) | 0.8 ms | 2.8 ms |
| header parse | 0.01 ms | 0.04 ms |
| decode BE-i16 → u16 | 0.9 ms | 2.5 ms |
| background (64k histogram, median + σ) | 2.3 ms | 4.4 ms |
| threshold + 8-connected flood fill + centroid | 3.8 ms | 7.0 ms |
| **total** | **~8 ms** | ~17 ms |

1,384 detections at 5σ / ≥4 px; 624 at ≥9 px. ASTAP selects 502 from the same
frame, so the extractor is in the right regime rather than finding nothing.

### 2.2 ASTAP control (CLI-2026.06.29, MPL 2.0, `-d ~/astap`, D50)

| frame | image stars / quads | time |
|---|---|---:|
| eagle H (rich) | 502 / 377 | **0.18 s** |
| eagle O | 322 / 246 | **0.10 s** |
| probe S (no solve) | — | 0.09 s |
| process startup floor | — | **0.00 s** |

ASTAP's own verbose output reports **282 database stars and 212 database quads
required for the 1.48° search window**.

### 2.3 What the measurements decided

1. **GPU: no.** Extraction is ~8 ms. A perfect GPU offload saves a few ms and
   costs the static-binary property plus driver surface. Dropped from the
   design; revisit only if sensor size grows by an order of magnitude.
2. **Star extraction is not the bottleneck**, contrary to the initial estimate
   of "tens of ms". The design's attention belongs on matching and on fixed
   per-invocation costs.
3. **ASTAP's cost is not process startup** (0.00 s), and it scales with star/quad
   count (0.10 s at 246 quads, 0.18 s at 377). There is a floor near 0.09 s that
   is neither startup nor image size; star-database loading is the likely
   candidate. An mmap'd index makes that floor ~0 for us.
4. **A ~10× per-frame win is realistic**: ~12–14 ms projected against ASTAP's
   measured 100–190 ms, single-threaded, before any parallelism.
5. **Approach B (quads built at solve time) is validated by ASTAP's own
   telemetry.** 282 catalogue stars for a 1.48° window is a few KB. A
   precomputed quad-hash index would be optimising a cost that measurably does
   not exist in the hinted case.
6. **New requirement:** the brightest object in the Eagle frame was a
   **63,104-pixel blob** — the nebula, not a star. Extended-source rejection is
   mandatory.

### 2.4 Measurements NOT made, and not to be assumed

- **True blind solve cost.** ASTAP solved a WCS- and `OBJCTRA`-stripped frame in
  0.18 s at `-r 180`. That is not credible as an all-sky search and is not
  reported as one. Blind performance is **unknown**, not fast.
- **A half-size-frame experiment was discarded**: the synthetic binned FITS
  failed to solve (undersampled PSF), so its timing measured a failure path.
- **`CLAUDE.md`'s "8,075 frames in 182 s" (22.5 ms/frame) is a throughput
  figure, not a per-frame one.** A single solve measures 100–190 ms, so that
  backfill must have run roughly 8-way parallel. Do not benchmark against 22.5 ms
  as if it were serial.

---

## 3. Decisions

| decision | choice | why |
|---|---|---|
| language / artifact | Rust, single static binary | speed ceiling; no runtime; Siril/N.I.N.A. adopt by path change |
| catalogue | Gaia DR3, own index format | control, size, and proper motions |
| matching | stars-only index, quads at solve time (B) | validated by §2.2; scale-free index; small enough to distribute |
| blind index (A) | designed-for, not built | additive sidecar if measurement ever demands it |
| GPU | no | §2.3 |
| distortion | TAN only in v1 | residual-vs-radius reported; data decides if SIP is needed |
| MQTT | out of the binary | agent bridges; solver stays broker-free and testable |

---

## 4. Architecture

A Cargo workspace. The split makes the read-only guarantee structural rather
than a matter of discipline.

| crate | contains | can it write? |
|---|---|---|
| `psolve-core` | FITS reading, extraction, quads, matching, WCS fit | **no filesystem write path exists in the crate** |
| `psolve-index` | index format reader (mmap) + Gaia builder | only when building an index |
| `psolve-render` | stretch, draw, PNG, card layout (feature-gated, default off) | only to explicit output paths |
| `psolve-cli` | the `psolve` binary — flags, JSON, batch, server | all output policy lives here |

`core` takes bytes and returns a solution. `-update`-rewrites-an-archive-frame
is therefore not a bug reintroducible by forgetting a default argument — it
would require adding a dependency. This is "guard the immutable tree
structurally, not by intention" expressed as a dependency graph.

**Dependencies:** `memmap2`, `rayon`, and a hand-rolled JSON writer. Rendering
adds a PNG encoder, a glyph rasteriser and an embedded SIL OFL font — all behind
a default-off feature, so `--no-default-features` yields the lean rig binary.
No FITS crate and no astronomy crate: the FITS we care about is ~60 lines.

---

## 5. The index

**Source:** Gaia DR3 — RA, Dec, G magnitude, `pmra`, `pmdec`.

**Structure:** HEALPix nested at `nside=64` (0.84 deg² cells, ~0.92° across; a
1.48° field touches 9–16 cells), configurable at build time. **Within each cell,
records are sorted brightest-first.**

That sort is the central design choice. Fetching a field's catalogue stars
becomes "seek to cell, read the first N records, stop" — a few KB, one seek. It
also means **one index serves every field size**: a narrower field reads further
down the same sorted run. There are no FOV bands to build, ship, or choose
between.

```
header    magic, format version, nside, epoch (2016.0), record count, mag limit
celltab   u64 offset per cell   (nside=64 → 49,152 cells → 384 KB)
records   fixed 16 bytes, brightest-first within each cell
```

**Record (16 B):** RA `u32` (~0.3 mas), Dec `i32`, G mag `u16` (millimag),
pmRA `i16`, pmDec `i16` (mas/yr), 2 spare. Fixed-width and aligned so the mmap
casts directly to a slice — the catalogue fetch does no parsing at all.

**Proper motion is applied at solve time** from `DATE-OBS`. Gaia's epoch is
2016.0 and frames are being taken in 2026: ten years of drift, corrected per
star, for 4 bytes per record. ASTAP's fixed-epoch databases do not do this, so
it is a plausible accuracy win as well as a correctness one.

**Depth is a measurement, not a decision.** The builder takes `--max-mag`; the
cut is chosen by building two or three and testing against real frames,
including the 61-star probe.

**Distribution** is the known weak point. The builder lives in the repo; users
need the built artifact. Plan: publish as a release asset, and for this rig
place it on the NAS `astro` share with a `SHA256SUMS`, matching the existing
rebuild-kit pattern.

---

## 6. Solve pipeline

### 6.1 Read

`BITPIX` 16 (unsigned via `BZERO`, what N.I.N.A. writes), 8, 32, and **−32
float** (what Siril writes). Both matter; the pipeline produces both.

**CFA frames must be handled.** The archive holds DWARF3 one-shot-colour data;
solving a raw Bayer mosaic is garbage. If `BAYERPAT` is present, 2×2 superpixel
bin to luminance before extraction. The resolution loss is irrelevant at these
scales and it is the difference between the archive backfill working and not.

Header parsing must survive blank cards, `CONTINUE`, `HIERARCH` and junk bytes
without panicking. Keys consumed: geometry; `DATE-OBS` (PM epoch);
`OBJCTRA`/`OBJCTDEC` (hint); `FOCALLEN`/`XPIXSZ`/`XBINNING` (FOV, same
derivation as `ingest.identify.header_fov`); and `FILTER`/`EXPOSURE`/`GAIN`/
`CCD-TEMP`/`OBJECT` passed through for the display card.

### 6.2 Extract

The spike used a **global** median+σ. That works on a flat field and fails on
gradients — moon, light pollution, amp glow — and is why its brightest "star"
was 63,104 px of Eagle Nebula.

Replace with a **background mesh**: tile at 64–128 px, median per tile,
bilinear-interpolate a background surface, subtract. Tiles much larger than
stars, much smaller than gradients. Nebulosity becomes background, which is
correct: nebulosity is not a star.

Then threshold at *k*σ (k = 5), 8-connected flood fill, and **every rejection is
counted by reason** — those counts are the report's diagnostic table:

| filter | default | catches |
|---|---|---|
| `npix` < min | 4 | hot pixels, cosmic rays |
| `npix` > max | ~25× median star area | nebulae, galaxy cores |
| peak ≥ saturation | 98% of range | biased centroids |
| ellipticity > | 0.6 | trailed stars |
| within edge margin | 8 px | truncated PSFs |

Centroids: flux-weighted first moment, then **windowed refinement**
(Gaussian-weighted, iterated) — meaningfully less biased, and cheap for ~500
stars. Second moments fall out of the same pass and yield **FWHM, ellipticity
and its position angle for free**. Keep the brightest 500 — the number ASTAP was
observed choosing.

### 6.3 Quads

astrometry.net's geometric hash. For each star take its nearest neighbours and
form 4-star sets; the two most widely separated (A, B) define a frame with
A = (0,0), B = (1,1); the other two give a 4-vector **(x_C, y_C, x_D, y_D)**
invariant to translation, rotation and scale. Canonical ordering
(`x_C ≤ x_D`, `x_C + x_D ≤ 1`) removes permutation ambiguity.

**Parity is tracked explicitly, never assumed.** Mirrored frames are real — an
odd number of reflections in the optical train produces them — and a solver that
assumes one handedness fails half the equipment it meets.

Scale reference: ASTAP made 377 quads from 502 stars (≈0.75/star).

### 6.4 Catalogue side

Search window from hint + FOV → HEALPix disc query → read brightest-first from
each overlapping cell.

**Depth is matched to the image, not maximised.** Pulling 50,000 catalogue stars
for a 500-star frame makes matching *harder*: true quads drown in plausible
false ones. Read until catalogue density ≈ image density. ASTAP's 282 stars for
a 1.48° window is that principle in action, and it is why brightest-first
ordering is the right layout — "read until dense enough, stop" is a sequential
scan.

Then apply proper motion to the `DATE-OBS` epoch, project gnomonically onto a
tangent plane at the hint centre, and build catalogue quads identically.

### 6.5 Match and fit

**Match:** KD-tree over catalogue quad codes; for each image quad, neighbours
within tolerance. Each candidate proposes a similarity transform (scale,
rotation, translation, parity). **Vote** into a coarse hash of those parameters:
true matches pile into one bin, false matches scatter. Robust, and faster than
RANSAC at this scale.

**Fit:** project all catalogue stars through the winning transform, pair to
detections by nearest neighbour within a few px, then linear least squares for
the TAN WCS (CD matrix + CRVAL/CRPIX). Sigma-clip and refit, twice.

### 6.6 Verify

The horizon probe treats "solved" as proof the telescope saw sky, *with no
threshold to calibrate*. That guarantee is worth exactly the false-positive
rate, so confidence is computed, not assumed: expected chance coincidences are
`n_image × n_cat × πr²/A`, and the observed match count is compared against that
as log-odds.

**Below threshold, `psolve` returns `LOW_CONFIDENCE` with the numbers attached
— never a solution it does not believe.** A solver that guesses would quietly
corrupt the horizon measurement with no visible symptom.

**Distortion:** TAN only in v1, but the report includes residual-vs-radius
correlation, so the data says whether SIP is needed rather than us guessing.

### 6.7 Blind

Same pipeline; the catalogue side iterates cells rather than taking one window.
Ordering is the advantage over ASTAP: given `--site lat,lon` and `DATE-OBS`,
**rank candidate cells by altitude and skip everything below the horizon** —
roughly half the sky, free, from data AstroOps already holds in
`core/planner/ephemeris` and `horizon.json`. Early-exit on first confident
solve, bounded by `--time-budget`.

### 6.8 Budget

Measured stages marked ✓; the rest projected.

| stage | ms |
|---|---:|
| read + decode | 1.8 ✓ |
| background mesh | ~3 |
| extract + centroid + moments | ~4 ✓ |
| image quads | <1 |
| catalogue fetch (mmap, ~300 stars) | <0.5 |
| catalogue quads | ~1 |
| match + vote | 1–2 |
| fit + verify | <1 |
| **total** | **~12–14** |

Against ASTAP's measured 100–190 ms on the same frames and machine.

---

## 7. Outputs

### 7.1 One run, three measurements

Centroiding every star to solve also yields, at no extra cost:

| measurement | from | what it tells you |
|---|---|---|
| pointing | the WCS | where the scope actually looked |
| focus / seeing | median FWHM of matched stars | is the night sharp; has focus drifted |
| tracking / guiding | median ellipticity **and its position angle** | stars are trailing, and in which direction |

`core/quality/gate.py` wants these. The elongation *angle* is the novel one: a
systematic direction indicates guiding drift or polar misalignment, and nothing
currently measures it.

### 7.2 JSON (the machine contract)

Abridged, and the values below are illustrative rather than a real solve:

```json
{ "psolve":"0.1.0", "build":"8365e60", "solved":true, "reason":null,
  "confidence":{"log_odds":412.3,"chance_matches":0.004},
  "wcs":{"crval":[274.689087,-13.810971],"crpix":[1919.5,1079.5],
         "cd":[[3.6727e-4,5.7397e-4],[-5.7400e-4,3.6725e-4]],
         "cdelt":[-6.8141e-4,6.8152e-4],
         "pc":[[-0.5390,-0.8423],[-0.8422,0.5389]],
         "parity":"mirrored"},
  "field":{"center":{"ra":274.689087,"dec":-13.810971,
                     "ra_hms":"18h18m45.4s","dec_dms":"-13°48'39.5\""},
           "fov_deg":[2.6262,1.4772],"scale_arcsec":2.4614,
           "orientation_pa":122.6,"scale_source":"header",
           "corners":[[273.35,-14.55],[275.72,-14.72],
                      [276.03,-13.07],[273.66,-12.90]]},
  "stars":{"detected":1384,"used":502,"matched":118,
           "rejected":{"too_small":760,"extended":3,"saturated":12,"elongated":9}},
  "fit":{"rms_arcsec":0.42,"rms_px":0.17,"max_residual_arcsec":1.31,"radial_trend":0.03},
  "quality":{"fwhm_arcsec":3.8,"ellipticity":0.11,"ellipticity_pa":78.0},
  "epoch":{"date_obs":"2026-07-29T10:47:02Z","pm_applied_years":10.58},
  "index":{"name":"gaia-dr3-g16-nside64","sha256":"3f2a…","cells_read":4},
  "timings_ms":{"read":0.9,"extract":4.1,"match":1.6,"total":11.4} }
```

A failed solve carries the same `psolve`/`build` pair and, whenever a
`--index` had already been opened before the failure occurred, the same
`index` shape as success — abridged:

```json
{ "psolve":"0.1.0", "build":"8365e60-dirty", "solved":false,
  "reason":"NO_QUAD_MATCH", "detail":"…",
  "stars":{"detected":214,"used":180,
           "rejected":{"too_small":30,"extended":1,"saturated":0,"elongated":3,"edge":0}},
  "index":{"name":"gaia-dr3-g16-nside64"},
  "timings_ms":{"total":9.7} }
```

**`psolve` is the crate version; `build` is what moves when behaviour
does.** `psolve` (`Cargo.toml`'s `version`) is bumped by hand and, in
practice, far less often than the analyser's actual output changes — a real
incident: a downstream consumer cached 2,000 solve results keyed in part on
`"psolve":"0.1.0"`, psolve was rebuilt from an edited working tree several
times in one day, and 202 of those 2,000 cached outcomes had silently
changed while the declared version stayed identical. `build` exists to be
the identifier that *does* move: it is derived from `git` at compile time
(`crates/psolve-cli/build.rs`, `git describe --tags --always --dirty`,
falling back to a bare short SHA, falling back to the literal `"unknown"` if
`git` is unavailable or the source is not a repository — e.g. a source
tarball — never fabricated, never a stale value from an earlier build) and
is present on every result, success or failure. **A consumer that wants to
detect "this is a different program than the one I last saw" — a changed
build, a different branch, a dirty rebuild — must key that check on `build`,
not on `psolve`.** The `-dirty` suffix is deliberate: a rebuild from
uncommitted local edits must be distinguishable from a clean build of the
same commit, which is exactly the case that produced the incident above.
`psolve` still answers a different question — "which release of the crate is
this" — and both fields are worth keeping for that reason; they are not
redundant, and a consumer should not conflate them.

**`index` is emitted on every failure that has one resolved, not only on
success.** Previously `index` appeared only inside `Outcome::Solved`'s JSON;
a second half of the same incident above was a consumer that keyed a
provenance record on `index` and consequently misclassified 656 of 2,000
samples that had landed on a failure branch and therefore never carried it.
`index` is `{"name":…}` — and only that — on both paths today; `cmd_solve.rs`
emits the identical shape on success and on failure. The richer
`sha256`/`cells_read`/etc. fields belong to a different command,
`psolve index info`/`index build` (`cmd_index.rs`), not to `solve`'s
`index` object on either path. `index` appears on any failure path
reached after `--index` was successfully opened — which in practice is every
JSON-emitting failure `psolve solve` has today, including `NO_HINT` and
every `Outcome::Failed` reason. A failure that occurs *before* an index is
resolved (a missing `--index` argument, a `--index` file that fails to open)
is a usage/config error reported on stderr with a non-JSON exit code (`2` or
`3`), never JSON with a fabricated or placeholder `index` value.

**`field.scale_source`** (added post-M3) says which plate scale actually
produced the solve: `"explicit"` when the caller passed a scale directly
(native mode's `--scale`; never second-guessed); `"header"` when the
header-derived scale (`FOCALLEN`/`XPIXSZ`/`XBINNING`) solved on the first
attempt, or no retry applied; `"header/binning-retry"` when the first,
header-derived attempt failed and a caller-side retry at `scale / XBINNING`
is what solved it instead — the fix for a driver that writes `XPIXSZ`
already multiplied by binning. Both production entry points (native `psolve
solve` and the ASTAP-compatible `-f` dispatch) apply the same retry; ASTAP
mode's own `.ini`/`.wcs` sidecar format has no field for it, so only this
JSON surfaces it.

**`cd` *and* `cdelt`+`pc` are always emitted.** `core/astrometry._with_pc`
exists only because ASTAP writes CD, Siril writes PC/CDELT, and every AstroOps
consumer assumed PC — so a valid ingest-solved frame raised `KeyError('PC1_1')`.
Emitting both eliminates that bug class at the source.

**`wcs.crpix` is 0-based, not FITS's 1-based `CRPIX1`/`CRPIX2`.** This
matches psolve-core's own internal pixel convention throughout (blob
centroids are array indices `0..nx`/`0..ny`; pixel index 0 is the first
column/row, not pixel index 1), and is emitted here exactly as psolve-core
computes it — no conversion happens between the solver and this JSON. A
consumer that reads `wcs.crpix` out of this JSON and writes it into a real
FITS header's `CRPIX1`/`CRPIX2` must add `1.0` to each axis first, or every
downstream WCS built that way is off by exactly one pixel. This is not
hypothetical: an M3 fix round found precisely that bug in this project's own
ASTAP-compatible `.ini`/`.wcs`/`-update` sidecar writers, which take this
same `Wcs` value — see `crates/psolve-cli/src/sidecar.rs`'s "CRPIX
convention" module doc for the fix and the reasoning, and
`docs/superpowers/2026-08-14-m3-first-real-frame.md`'s Task 11 section for
how it was found.

### 7.3 Markdown report (`--report`)

A human-readable fact-sheet: field centre (decimal and sexagesimal), FOV, scale,
orientation, parity, epoch and PM applied; fit statistics; frame quality;
the extraction rejection table; objects in field (optional, `--objects NGC.csv`);
and a provenance footer (version, index name + SHA, cells read).

**The failure report is the one that earns its keep.** ASTAP prints
`No solution found! :(`. `psolve` prints the reason code, the star and quad
counts, the catalogue density actually fetched, the best match score against the
threshold it needed to beat, and a suggested next action.

### 7.4 Images (feature-gated, off by default)

- **`--png`** — stretched, downsampled preview. `--stretch auto|asinh|linear|none`,
  `--size`, `--invert`.
- **`--annotate`** — layers over that preview, `--layers stars,grid,dso,compass,scale`:
  detected stars circled (radius ∝ FWHM, coloured by used / rejected / matched);
  matched catalogue stars as crosses **at their predicted positions**, so
  residuals are visible as the gap; RA/Dec graticule; OpenNGC objects labelled;
  a **parity-aware** compass rose; an arcmin scale bar.
  **This works on failures too** — an annotated image of an unsolved frame shows
  clouds, trailing, or nebula swamping at a glance.
- **`--card`** — the display image: the stretched frame plus a panel carrying
  target, coordinates, FOV, scale, orientation, capture metadata (filter,
  exposure, gain, temperature, date), FWHM and ellipticity, objects in field,
  and a provenance footer. `--card-theme dark|light`, `--card-size`,
  optional `--tint filter` using the palette `targets.yaml` already defines.

**Rendering is slower than solving** (tens of ms to encode a PNG against a
~12 ms solve), so images are strictly opt-in per invocation. Batch defaults to
none. Turning them on for 8,000 frames would make `psolve` slower than ASTAP,
which would be an absurd way to lose.

---

## 8. Interfaces

**8.1 ASTAP-compatible.** Accepts `-f -r -ra -spd -fov -d -o -update -z -s -t`;
writes the `.ini` sidecar (`PLTSOLVD=T`, `CRVAL1/2`, `CRPIX1/2`, `CDELT1/2`,
`CROTA1/2`, `CD1_1..CD2_2`) and `.wcs`. The unit conventions are preserved
deliberately: **`-ra` in hours, `-spd` = dec + 90**, exactly as
`ingest.identify.astap_solve` constructs them.

Triggered by argv0 (symlink `astap_cli → psolve` and the whole machine switches
at once) or explicit `--astap-compat`. Siril, N.I.N.A. and `identify.py` then
work with **no code changes anywhere** — so an A/B is a symlink flip, and so is
reverting.

**8.2 Native.** `psolve solve <file> [--hint ra,dec] [--fov deg] [--json]
[--report -] [--png|--annotate|--card <path>]`. JSON to stdout; writes nothing
unless handed an explicit path.

**8.3 Batch.** `psolve batch <dir> | --stdin` — walks a tree or reads paths from
stdin so it composes with `find`. Worker pool (`-j`, default = cores), one
shared index mmap, **NDJSON streamed as each frame completes**.
`--prior previous` seeds each solve from the last success — free speed and
accuracy on a sequential night, and impossible for a one-shot CLI.
Progress to stderr.

**8.4 Server.** `psolve serve --listen unix:/path | tcp:addr`. Line-delimited
JSON; accepts a path (same host) or length-prefixed FITS bytes (remote). Index
stays mmap'd, so per-request load is zero — this is where livestack's in-night
cadence comes from. `{"op":"health"}` mirrors the gateway convention.

**Subcommands:** `stars` (extraction only — the free QC sensor),
`verify` (check an existing header WCS against the catalogue without solving —
lets the 9,495 recorded solves be audited cheaply), `doctor` (index present,
valid, version, coverage; reports and never acts), `index build`, `index info`.

**stdout is results, stderr is logs.** Always. Batch NDJSON must never be
polluted.

---

## 9. Errors

**Reason codes:** `CANNOT_READ`, `UNSUPPORTED_FORMAT`, `NO_STARS`,
`TOO_FEW_STARS`, `EXTENDED_ONLY`, `NO_QUAD_MATCH`, `LOW_CONFIDENCE`,
`FOV_MISMATCH`, `NO_HINT`, `INDEX_MISSING`, `INDEX_TOO_SHALLOW`,
`TIME_BUDGET_EXCEEDED`.

`NO_HINT` (added post-M3) is distinct from `FOV_MISMATCH`: the latter means
a pointing hint *was* available but the field it implies did not match what
was found, a data problem; `NO_HINT` means no hint was available anywhere
(no `--hint`, no `OBJCTRA`/`OBJCTDEC`, no `RA`/`DEC`) -- a broken invocation
or an unsupported frame, not a data problem, and reporting it as
`FOV_MISMATCH` told a caller branching on `reason` the field of view
disagreed when in fact no hint was ever supplied.

**Exit codes distinguish "did not solve" from "you called it wrong":**

| code | meaning |
|---|---|
| 0 | solved |
| 1 | not solved — a *normal* outcome; clouds are not a bug |
| 2 | usage / configuration error |
| 3 | index problem |

A script must be able to tell a cloudy frame from a broken invocation. ASTAP
returning non-zero for everything is precisely what made a missing `-d` look
like bad weather for an entire session.

**No panics on input.** The parser eats untrusted files, returns typed errors,
and is fuzzed. Every solve is time-bounded; nothing hangs a batch.

**The read-only guarantee, in layers:** `psolve-core` has no write path at all;
native mode has no in-place write; `-update` exists only in the ASTAP-compat
shim. Above that, **`PSOLVE_READONLY=1` hard-disables every write**, and a
`.psolve-readonly` sentinel anywhere up the tree from the target does the same.
Set the env var once on the workstation and no caller — not a throwaway timing test, not
a future maintainer — can rewrite an archive frame.

---

## 10. Testing

**Synthetic closed loop — the most important test.** Generate a frame *from the
index* at a known WCS, solve it, demand the WCS back. Property-tested over
random pointings including mirrored parity, high declination and near-pole
fields. This validates the whole chain with no real data and catches the
sign/parity/RA-direction errors that are the classic way a solver is subtly
wrong — the same family as the 12-hour `DATE-OBS` bug, whose tell was an
*impossible* number rather than a surprising one.

**Agreement corpus — the 9,495 recorded ASTAP solves in `catalogue.db`.** Pull
`(path, ra_deg, dec_deg)`, solve each, compare. Metrics: solve rate, agreement
distribution, wall clock. **These are ASTAP's answers, not ground truth** — it
is an agreement test, and where the two disagree we investigate rather than
assume we are wrong. Accuracy comes from the synthetic loop and the internal
residual RMS.

**Control frames**, referenced by path + SHA-256 in `tests/corpus.toml` rather
than committed: rich (eagle H), medium (eagle O), sparse (the 61-star probe),
CFA (DWARF3), 32-bit float (Siril output), mirrored. "Run a known-good case
first, not third", encoded as a fixture list.

**CI split:** the CI host has neither frames nor index, so CI runs unit + synthetic +
fuzz only. Corpus benchmarks run on the workstation via a make target and commit their
report. **No test touches the network, ever.**

---

## 11. Implementation milestones

**This spec describes the whole product, which is more than one implementation
plan's worth of work.** It is deliberately written as one document because the
index format, the solve pipeline and the output contract constrain each other
and are not separable design problems. They *are* separable build problems:

| milestone | delivers | independently useful? |
|---|---|---|
| **M1** | index format, Gaia builder, mmap reader, `index build` / `index info` | yes — inspectable, benchmarkable alone |
| **M2** | FITS read, extract, quads, match, fit, verify; `solve --json`; synthetic closed-loop tests | yes — this is the solver |
| **M3** | ASTAP-compat CLI + agreement run against the 9,495-solve corpus | yes — this is the go/no-go on the whole premise |
| **M4** | batch and server modes | yes |
| **M5** | markdown reports, `--png`, `--annotate`, `--card` | yes |

**M3 is the gate.** If agreement and speed against the recorded corpus do not
hold up there, M4 and M5 are wasted work and the honest outcome is to keep using
ASTAP. Each milestone gets its own implementation plan; none should start before
the previous one's tests pass.

## 12. Open questions

These are known unknowns, recorded so they are not mistaken for settled:

1. **Index depth and resulting file size.** To be chosen by building two or
   three at different `--max-mag` and testing, including against the 61-star
   probe frame.
2. **True blind-solve performance** — of ASTAP and of `psolve`. Unmeasured
   (§2.4). Needs a frame with no pointing information and a real all-sky search.
3. **Whether TAN suffices** at 243 mm over a 2.6° field, or SIP is required.
   The residual-vs-radius statistic answers this once frames are solving.
4. **Index distribution** beyond "release asset + NAS". Fine for this rig;
   unsolved for anyone else.
5. **Font licensing** for the display card — an SIL OFL face is the intent, but
   the specific choice is unmade.

## 13. Deferred by intention

Precomputed quad-hash sidecar for blind solving (§3); SIP distortion terms;
annotated-image output beyond PNG; emitting matched star lists for stack
registration; GPU extraction. Each is additive to the formats defined here, and
none requires a rewrite to adopt later.
