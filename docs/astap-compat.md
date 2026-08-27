# ASTAP compatibility

> **On commit references.** Public history begins at `v0.1.0`. Short SHAs cited
> in this document (`297961b`, `3ba1c32`, and others) refer to the pre-release
> development history, which is retained privately and is not part of this
> repository -- they will not resolve here. The measurements themselves are
> reproducible from the flags, corpus and data each one names; the SHA records
> which build produced a figure, not where to find it.

`psolve` can be invoked with `astap_cli`'s own argument grammar and produces
`astap_cli`'s own output files (`.ini`, `.wcs`) and exit-code scheme, so that
AstroOps, Siril, or N.I.N.A. can point at `psolve` instead of `astap_cli`
with no change on their side. This document is the compatibility contract:
exactly what is supported, what is silently accepted but not wired to the
solve, and what is deliberately different.

**Mode detection.** ASTAP mode is entered whenever `argv` contains `-f`
(ASTAP's own filename flag). Native mode (`psolve solve <FILE> --index
<FILE> [OPTIONS]`) is everything else. The two surfaces never blend: a
native `--long` flag inside an ASTAP-mode invocation (e.g. `-f x.fits
--index i.psidx`) is a hard parse error, not a silently-ignored extra.

> **Recorded deviation from the spec.** Spec §8.1 specifies ASTAP mode is
> triggered by **argv0** (a symlink `astap_cli → psolve`) or by an explicit
> `--astap-compat` flag. This branch triggers on `-f` instead, and
> implements neither of the two specified triggers. In practice the
> symlink-flip deployment still works, because every real ASTAP invocation
> carries `-f` (it is ASTAP's required input-file flag), so an `astap_cli`
> symlink pointed at `psolve` lands in ASTAP mode on every invocation
> AstroOps, Siril or N.I.N.A. actually issues. What the deviation does cost:
> an argv with no `-f` at all goes to **native** mode even when the binary
> was invoked through the `astap_cli` name — see the `--help` row under
> "Exit codes" for the one place that is user-visible — and there is no way
> to *force* ASTAP mode for an argv that happens to carry no `-f`. Recorded
> here rather than left silent; not fixed in M3.

## Supported flags

| Flag | Meaning | Wired to the solve? |
|---|---|---|
| `-f <path>` | input FITS file (required) | yes |
| `-r <deg>` | search radius, degrees | yes — see "effective search radius" below |
| `-fov <deg>` | field **diameter**, degrees (`0` = auto in real ASTAP; psolve treats an absent `-fov` the same way) | yes — narrows `-r` |
| `-ra <hours>` | right ascension of the pointing hint, **in hours** | yes |
| `-spd <deg>` | **south polar distance** of the pointing hint (`dec + 90`), degrees | yes |
| `-d <path>` / `-D <name>` | directory holding the star database | yes — resolved to a `.psidx` AND (if present) a `.psqidx` blind-solve quad index (see below) |
| `-o <base>` | output base path for the `.ini`/`.wcs` sidecars | yes |
| `-update` | write the solution into the input FITS header in place | yes — see "the `-update` safety model" |
| `-wcs` | write the `.wcs` sidecar as a true FITS block instead of the default text style | yes |

### `-ra` is hours, `-spd` is `dec + 90` — exact, not approximate

Both conversions were confirmed against real recorded AstroOps invocations
on this machine, not inferred from the flag names:

- **`-ra` is in hours.** A real invocation carried `-ra 16.950000` against
  that frame's own `OBJCTRA='16 57 00'` header card — `16h57m = 16.950000
  h`, an exact match. Converting to degrees is `ra_deg = ra_hours * 15`.
- **`-spd` is south polar distance**, i.e. `dec_deg + 90`, not declination
  itself. A real invocation carried `-spd 49.666667` against
  `OBJCTDEC='-40 20 00'` (`-40.333333°`), and `-40.333333 + 90 =
  49.666667` exactly. Converting back is `dec_deg = spd_deg - 90`.

Getting either wrong produces a hint that is wrong by a factor of 15 or by
90 degrees, and the solver then fails with `NO_QUAD_MATCH` — which reads as
"unsolvable frame," not "the caller mistranslated the units." A hint
requires **both** `-ra` and `-spd`; half a hint (only one of the two) is
treated as no hint at all, and psolve then falls back to the frame's own
`OBJCTRA`/`OBJCTDEC` header cards if present.

### Effective search radius

`-r` is the raw radius ASTAP was invoked with; `-fov` is the field
**diameter**. When `-fov` is present, psolve's actual catalogue-search
radius is `min(r, (fov / 2) * 1.10)` — half the diameter plus a 10%
pointing-error margin — never wider than the caller's own `-r`, only
narrower. This is deliberate (a search disc much wider than the frame's own
footprint dilutes the catalogue with stars no quad can match and costs
matches monotonically), but it means the effective radius can be
**narrower than the frame's true angular footprint** when `-fov` under-states
it — and this is not a hypothetical edge case. AstroOps' real deployment
passes the same fixed `-fov 1.4770` for every invocation regardless of
rig; on the `eagle`/`ic4604` rig (true field 2.626°×1.477°, where `1.4770`
matches only the short axis, not the diameter), a random sample of **25
real ASTAP-solved frames from that rig found 21 of 25 (84%) failing to
solve at all** under the literal hinted-retry flags, including the
project's own `eagle` reference frame — while every one of those same
frames solves cleanly at psolve's native-mode default (header-optics-
derived) radius. See Task 12's timing report for the full measurement.

### `-d` / `-D`: resolving ASTAP's star database directory to psolve's index

ASTAP's `-d`/`-D` names a directory holding its own `d50_*`-style database
files. psolve's equivalent is a single `.psidx` file built by `psolve index
build`. Resolution: scan the given directory for `.psidx` files; if more
than one is present, the alphabetically-first path wins (deterministic,
independent of filesystem iteration order). A missing `-d`, a directory
that does not exist, and a directory with no `.psidx` file are all reported
identically, matching what ASTAP itself would say for "no usable star
database": `ERROR=No star database found.`, exit code 1.

The same directory is ALSO scanned for a `.psqidx` -- a blind-solve quad
index built by `psolve quad-index build`, sorted-first-match exactly like
the `.psidx` scan above, independently of it (a directory can hold a
`.psidx` with no `.psqidx`, and vice versa). No flag selects this; it is
purely presence-based. **A missing or unopenable `.psqidx` is silently
treated as "blind solving unavailable" for this invocation, never an
error** -- an ordinary hinted invocation has no need of one at all, unlike
a missing `.psidx`, which is fatal to every invocation.

### Blind solving: no `-ra`/`-spd`, no header hint, and a `.psqidx` present

When neither `-ra`/`-spd` nor the frame's own `OBJCTRA`/`OBJCTDEC`/`RA`/`DEC`
resolve a pointing hint, psolve no longer reports "Not enough stars."
unconditionally. It looks for a `.psqidx` in the `-d`/`-D` directory
(above); if one opens successfully (paired correctly against the `.psidx`
also in that directory), psolve searches it -- the frame's own image quads
against the index's precomputed code space -- clusters the candidates that
survive, and refines the best-agreed cluster through the exact same
match/fit pipeline a hinted solve always runs. The result is judged against
a MULTIPLICITY-CORRECTED confidence gate before being accepted (see
`docs/superpowers/specs/2026-08-15-blind-solve-design.md` for the full
derivation and the motivating incident it exists to prevent: an
uncorrected blind gate accepting a confidently wrong position). Without a
usable `.psqidx`, behaviour is exactly as before: `ERROR=Not enough
stars.`, `psolve: reason=NO_HINT` on stderr, exit code 1.

There is no dedicated ASTAP-style flag for this -- AstroOps' real blind
invocation (`-r 180`, no `-ra`/`-spd`) already reaches it automatically the
moment a `.psqidx` sits beside the `.psidx` it was invoking anyway. Native
mode's equivalent is the explicit `--quad-index <FILE>` flag (`psolve
--help`).

## Flags accepted and ignored

Two different reasons a flag is accepted without erroring:

**Parsed, validated, and echoed into `CMDLINE=`, but not applied to the
solve — and psolve warns about it on stderr, every time any of them is
present:**

- `-z` (downsample factor) — psolve-core has no downsampling stage at all.
- `-s` (max stars) — plausibly maps to psolve-core's own `keep` extraction
  parameter, but left unwired: unverified against real behaviour, and a
  wrong guess risks silently degrading the solve rate.
- `-t` (quad tolerance) — ASTAP's own quad-match tolerance; no verified
  equivalent among psolve-core's own tunables (which measure different,
  non-interchangeable things).
- `-m` (minimum star size, arcsec) — same reasoning as `-t`.

The warning looks like:

```
psolve: warning: -z, -s accepted but not applied to the solve (no verified psolve equivalent for them yet)
```

**Accepted silently, no warning** (ASTAP's own analysis-only options, which
have no bearing on whether or how a frame solves):

`-log`, `-sip`, `-check`, `-progress` (no value), and `-speed`, `-analyse`,
`-extract`, `-extract2` (each takes a value, which is consumed and
discarded so it is never misread as the next flag).

Any flag not in either list — including psolve's own native `--long`
flags used inside an ASTAP-mode invocation — is a hard parse error.

## The `.ini` sidecar

Byte-exact ASTAP compatibility, because AstroOps parses this file directly.

**Success** — 14 keys, fixed order, LF-only line endings, trailing newline:

```
PLTSOLVD=T
CRPIX1= 1.9205000000000000E+003
CRPIX2= 1.0805000000000000E+003
CRVAL1= 2.5423046742390622E+002
CRVAL2=-4.0311880588850023E+001
CDELT1= 6.8154932258843713E-004
CDELT2= 6.8151366119530501E-004
CROTA1=-5.8859778367665449E+001
CROTA2=-5.8866887820396883E+001
CD1_1= 3.5245253250848707E-004
CD1_2= 5.8334097357301367E-004
CD2_1=-5.8335417754934037E-004
CD2_2= 3.5236170894630648E-004
CMDLINE=<the command line verbatim>
```

Number format: one mantissa digit, a decimal point, **16 more mantissa
digits**, `E`, sign, a **3-digit zero-padded exponent** (`E+003`, `E-004` —
never `E-4` or `E-04`). Non-negative values carry a single leading space
where a negative would put its `-`.

**Failure** — structurally different, with a quirk reproduced exactly
because a consumer that skips byte 0 would break on a file that lacked it:

```
<a literal blank line -- byte 0 of the file is \n>
PLTSOLVD=F
CMDLINE=<the command line verbatim>
ERROR=<message>
```

`CMDLINE` comes **before** `ERROR`, and none of the `CRPIX`/`CRVAL`/`CD`
keys appear. Every solve failure in ASTAP mode — whatever psolve-core's own
internal reason code says (`NO_QUAD_MATCH`, `LOW_CONFIDENCE`,
`TOO_FEW_STARS`, or no pointing hint being resolvable at all) — collapses
to ASTAP's own `Not enough stars.` wording; a missing or unusable star
database instead reports `No star database found.` — the two failure
strings real ASTAP is observed to use.

## The `.wcs` sidecar

Two formats, and the default is the one that matters — it is what 100% of
real production `.wcs` files on this machine actually are:

- **Default (no `-wcs`)** — FITS-card-styled **text**: an LF after each
  ~80-character card, **not** padded to a 2880-byte block, containing the
  original capture header (byte-exact pass-through, `BITPIX` forced to `8`
  and `NAXIS` forced to `0` since a `.wcs` describes no pixel data of its
  own, `NAXIS1`/`NAXIS2` dropped) followed by the solved WCS keywords.
- **With `-wcs`** — a true FITS block: exactly 8640 bytes (3×2880), **zero
  newlines**, 108 cards of exactly 80 bytes each, an `END` card, and blank
  padding to the end of the block.

`.wcs` values use **12** mantissa digits (not the `.ini`'s 16).

## Exit codes

Two independent, deliberately decoupled schemes:

**Native mode** (`psolve solve` / `psolve index`) — unchanged, the richer
scheme:

| Code | Meaning |
|---|---|
| 0 | success |
| 1 | normal negative outcome (not solved) |
| 2 | usage/config error |
| 3 | index problem |

**ASTAP mode** — collapsed to ASTAP's own observed two-code scheme, so a
drop-in replacement is indistinguishable at the `$?` AstroOps actually
branches on:

| Code | Meaning |
|---|---|
| 0 | successful solve |
| 1 | everything else — a malformed invocation, a missing input file, an unresolvable star database, an unsolved frame, a refused sidecar write, or a refused `-update` write |

`--help` is deliberately **not** in that table. Real `astap_cli` exits `0`
for it, and so does psolve — but psolve reaches that `0` through **native**
mode, because `--help` carries no `-f` and `-f` is what selects ASTAP mode
(see the recorded deviation under "Mode detection"). So `psolve --help`
prints *psolve's* usage text, not ASTAP's, even when psolve is invoked
through an `astap_cli` symlink. The exit code a script branches on is
unaffected; the text on stdout is not ASTAP's.

That collapse is deliberate: a third distinct code in ASTAP mode would be a
native convention leaking into a mode whose entire purpose is to be
indistinguishable from the real `astap_cli`. In particular, a read-only
refusal — of the `.ini`/`.wcs` sidecar writes or of an `-update` write — is
exit code `1` here, not native mode's `3`.

## The `-update` safety model

`-update` writes the solved WCS directly into the input FITS file's header.
This is the one path in the whole crate that mutates a file that may be the
user's only copy of a frame — a header rewrite that shifts the pixel data
silently corrupted four archive frames once before this compatibility
layer existed, which is why the rules below are non-negotiable.

**Default off.** Only the literal `-update` flag enables this at all; it is
never implied by anything else.

**`PSOLVE_READONLY` refuses.** Any non-empty value (checked with
`var_os`, so even a non-UTF-8 value still refuses rather than being
silently treated as "unset") makes every `-update` write on the process
fail closed, before a single byte is read or written.

**A `.psolve-readonly` marker anywhere on the target's canonical (physical)
ancestor chain always refuses the write — unconditionally.** The canonical
chain is `std::fs::canonicalize`'s output: the real directories the
target's bytes actually live in, walked to the filesystem root. This does
not depend on how the path was spelled, on the process's working
directory, on any environment variable, or on any symlink anywhere;
canonicalization failing is itself treated as a refusal (fail closed), not
a reason to fall back to an unresolved path. **This is the guarantee to
rely on: to protect a tree, put the marker in the tree the frames
physically live in.**

Two further ancestor chains are walked as **additional, best-effort**
coverage, for a marker placed on a tree that is not the file's physical
location (e.g. a locally-named directory whose contents are a symlink into
a mounted share):

- *physical-lexical* — the path as given, made absolute against the
  process's real (`getcwd(3)`) current directory if it was relative, with
  no symlink resolved in the joined path itself. Available whenever the
  cwd can be read, which is essentially always.
- *logical-lexical* — the path as given, made absolute against `$PWD`
  instead. Available **only** when `$PWD` is set, absolute, free of `.`/`..`
  components, and verifies (by device+inode, never by string) as actually
  naming the current directory. Otherwise this chain is simply omitted.

The reason a third chain exists at all: no operating system records a
*logical* working directory — `getcwd(3)`, which both `canonicalize` and
the physical-lexical chain are built on, returns the kernel's physical,
symlink-resolved cwd. Only a shell's `$PWD` convention preserves the
as-typed (possibly symlinked) directory. So for a **relative path invoked
from inside a symlinked working directory**, the canonical and
physical-lexical chains both already resolve past the symlink and cannot
see a marker placed on the symlinked tree's own name — only `$PWD` can
reconstruct that, and only when it can be trusted. Concretely: `$PWD`
unset (any non-shell launcher — cron, systemd, `subprocess`), stale (e.g.
Python's `os.chdir()`), or naming the same directory through a different,
unmarked twin symlink, are all out of scope for the logical-lexical chain
and fall back to lexical-only coverage above the file — in every one of
those cases, a marker on the **canonical** chain still refuses, per the
unconditional guarantee above. When a relative path is given and the
logical-lexical chain could not be built, psolve prints a warning to
stderr naming the file and the reason, at the moment it decides to let the
write proceed — not a refusal, since refusing every relative invocation
under a non-shell launcher would break real, harmless usage for a hazard
that only exists when a marker sits on a symlinked tree.

**Never write in place.** A complete new copy is written to a temp file
beside the target and `fsync`ed there, then `rename`d over the target —
atomic on the same filesystem, so an interrupted run (including `kill -9`)
cannot leave a truncated or half-written frame; the old directory entry or
the new one exists afterwards, never a mix.

**Verified before the rename, not assumed.** The temp file is reparsed
from a fresh `std::fs::read` (never the in-memory buffer that built it):
its data unit must begin at the same byte offset as the original's, and
its pixel bytes must be **byte-identical** to the original's. Any mismatch
deletes the temp file and returns an error — the rename never happens.

**A header that would grow the block count is refused, not shifted.** If
the new header needs more 2880-byte blocks than the original had, nothing
is written and no rename occurs; the pixel data is never moved to make
room, and the header is never truncated to force a fit.

**A target with no write bits set is refused.** `chmod a-w`/`chmod 444` on
the target file itself refuses the write (`rename` would otherwise
silently discard that protection and replace the inode outright); a
perfectly writable file with unusual ownership is not covered by this
narrower check.

Every one of these refusals is exercised in `crates/psolve-cli/tests/fits_update.rs`.

## Measured results (M3)

**No projected or aspirational numbers below — every figure here was
measured, not designed to.** The heading says M3 because that is the
milestone these measurements were taken for, and the anchor is linked from
the README; the agreement figures below are kept current as `main` moves,
with the M3 column retained beside them rather than overwritten.

### Agreement (Task 11, full corpus)

**Three columns, not one, and no figure here has ever been overwritten.**
The first two are over the **9495**-frame corpus this section has always
used; the third is over the **full live corpus of 10,376 frames**, which is
the number to quote from now on. The corpus, the G≤14 index and the report
script are identical between columns 1 and 2; what changed there is the
binary -- conditional stratified star selection landed on `main` (`297961b`)
after M3 and moved the solve rate by +49 frames net (55 previously-failing
frames now solve, 6 previously-solving ones now fail). The M3 column is kept
rather than overwritten because the investigation below was conducted
against it.

Column 3 is a different corpus, not a different binary policy: today's
binary reproduces column 2 **to the digit** on column 2's own 9,495 frames.
The reason a third column was needed is that **the 9,495-frame corpus
contains no 2×2-binned frames at all**, and 791 such frames exist in the
live database. They solved at 0% until 2026-08-23, when the binning retry
began refetching the catalogue at the corrected search radius
(`3ba1c32`) -- 790 of 791, median 0.707″. Full measurement, criterion by
criterion, in
[`docs/superpowers/2026-08-22-binning-retry-refetch-results.md`](superpowers/2026-08-22-binning-retry-refetch-results.md).

| | Task 11 (M3), 9495 frames | current (`297961b`), 9495 frames | **full live corpus** (`3ba1c32`), 10,376 frames |
|---|---|---|---|
| Solve rate | 9219 / 9495 = 97.09% | 9268 / 9495 = 97.61% | **10141 / 10376 = 97.74%** |
| Separation median | 0.531″ | 0.530″ | **0.539″** |
| Separation p90 | 0.947″ | 0.945″ | **1.011″** |
| Separation p99 | 3.128″ | 3.098″ | **3.132″** |
| Separation max | 30.338″ | 30.688″ (same frame) | **38.217″** (a different frame -- arbitrated, ASTAP's error) |
| Gross errors (>30″) | 1 | 1 (same frame) | **2** |

The corpus-wide median and p90 rise slightly in column 3 **because the
population changed, not because any frame's answer did**: 790 bin-2 frames
at a 0.707″ median joined a bin-1 population at 0.531″. Restricted to
binning=1, the distribution is identical before and after the refetch change
-- median 0.531″, p90 0.967″, p99 3.202″, max 30.688″ -- and all 9,585
binning=1 frames produce a **byte-identical** solve record across the
change.

Immediately before that change (`ca1dc73`) the full-corpus rate was
**9351 / 10376 = 90.12%**; the 791 bin-2 frames were 77.2% of every solve
failure in the corpus.

Separations are over the frames both tools solved. An independent re-run on
the `blind-solve` branch (`5c2e73b`, Task 8 step 4, no `--quad-index`
anywhere in it) reproduces the current column: 9268/9495, median 0.530″, p90
0.946″, p99 3.107″. The p99 spread across the two runs (3.098″ vs 3.107″) is
a percentile-estimator convention difference plus slight corpus drift, not a
behaviour change — see
`docs/superpowers/2026-08-15-blind-solve-results.md` section 4.

Two M3 figures were **not** re-measured after that change and are therefore
reported as of Task 11 only, not claimed as current:

- Scale-ratio outliers (psolve fitted / header-optics expected, >5% off):
  **0** (M3).
- Parity mismatches (against the 301 frames that carry a full header WCS):
  **0** (M3).

**The agreement gate, as specified (`MAX_GROSS_ERRORS = 0`, 30″ bar),
FAILED -- on exactly one frame in the first two columns above, and on two in
the third.** The second one is `SVBONY_SV405CC/NGC_3372/.../0444.fits` at
38.217″, a 2×2-binned frame that did not solve at all until `3ba1c32`. It
was arbitrated by the same reprojection method used below and reached the
same verdict -- **ASTAP's error, not psolve's** -- on four independent
signals: ASTAP's recorded centre leaves that session's smooth 2-3″-per-frame
tracking track by 39.6″ for one 15 s frame and returns 15 s later, psolve's
WCS lands 100% of reprojected Gaia stars on real light where ASTAP's centre
puts 61% of them at background, re-hinting psolve at ASTAP's own centre
converges back to psolve's answer 38.30″ away from the hint, and psolve's
internal fit statistics on that frame are unremarkable against its
neighbours. Full write-up in
[`docs/superpowers/2026-08-22-binning-retry-refetch-results.md`](superpowers/2026-08-22-binning-retry-refetch-results.md)
section 4, including the method's own caveat (`astap_cli` is not installed
on this machine, so ASTAP's candidate WCS is psolve's CD with `CRVAL` moved
to ASTAP's recorded centre -- it isolates the centre and cannot see a shear).

The original single-frame failure:
`SVBONY_SV405CC/NGC_3372/.../0050.fits` separated from ASTAP's recorded
centre by 30.338″ at M3 (0.338″ over the bar) and by 30.688″ now (0.688″
over). The investigation below was run against the M3 value; the 0.35″ shift
between the two is far below the margin of anything it concluded, and every
finding in it stands unchanged.

Investigation — reprojecting Gaia catalogue stars through each candidate
WCS and measuring the pixel flux at the predicted positions, checked
against two clean session-neighbour control frames so the metric's own
reliability is visible rather than assumed — found that **on this frame,
ASTAP is wrong and psolve is right**: psolve's WCS puts catalogue stars at
a 4616 ADU median peak (99.8% landing on real starlight), while ASTAP's WCS
for the same frame puts them at 124 ADU against a ~60 ADU background (30.5%
on light) — essentially background. Both control frames scored
statistically identically under both tools' WCS (>99.5% on light either
way), so the metric does not favour psolve by construction. Four
independent corroborations (re-hinting psolve at ASTAP's own centre still
reconverges to psolve's answer; a cubic fit through the imaging session's
147 other solved frames predicts the true pointing to within 8.0″ of
psolve's answer versus 32.6″ of ASTAP's; ASTAP's own `.ini` for this frame
is internally sheared relative to its neighbours; a fresh, live
`astap_cli` run against this exact frame today reproduces the same wrong
answer) are recorded in full in `docs/superpowers/2026-08-14-m3-first-real-frame.md`.

### Re-run 2026-08-27: four gross errors, and a third arbitrated to ASTAP

The full corpus was re-run on 2026-08-27 (10,376 frames, same gate constants).
The headline figures reproduce exactly -- **10,369 solved (99.93%), median
0.541", p99 3.316"**, the same seven failures (4 `NO_QUAD_MATCH`, 2
`TOO_FEW_STARS`, 1 `LOW_CONFIDENCE`). **The gate still FAILS, now on four
frames rather than two:**

| separation | frame | status |
|---:|---|---|
| **52.57"** | `library/_probe/.../2026-07-30_19-38-19_O_60.00s_..._0001` | **ASTAP's error** -- arbitrated below |
| 40.90" | `DWARFIII/C_92/.../failed_C 92_15s60_..._20250523-183554357` | **undetermined** |
| 38.22" | `SV405CC/NGC_3372/.../0444.fits` | ASTAP's error (arbitrated 2026-08-22) |
| 30.69" | `SV405CC/NGC_3372/.../0050.fits` | ASTAP's error (arbitrated at M3) |

The two new entries are not a regression: the 52.57" frame is an ATR585M
`_probe` exposure and the 40.90" one is a DWARFIII frame the capture pipeline
itself named `failed_`. Neither was in the earlier corpus slices that produced
the two-error count.

**The 52.57" frame: ASTAP's error, on four independent signals.**

`PROBE_az195_alt60`, a 60 s pointing-check exposure. ASTAP's recorded centre is
219.62999, -66.12977; psolve returns 219.59469, -66.13282.

1. **Reprojection, with controls.** Gaia stars at G<=9 projected through each
   candidate WCS and centroided against real flux: **psolve 19/38 (50%), ASTAP
   1/40 (2.5%)**, a 20:1 ratio that holds at every magnitude cut. Two control
   frames from the same probe session, where the two tools agree to under 0.1",
   score **96% and 100%** under the same metric -- so a correct WCS scores near
   perfect here and the arbiter discriminates rather than favouring psolve by
   construction. psolve's own 50% reflects a genuinely hard frame, not a
   marginal solve.
2. **Live `astap_cli`, run against this exact frame on 2026-08-27**, reproduces
   its recorded answer to 0.9" (`CRVAL1 = 219.63025`). The database row is not
   stale; ASTAP consistently produces this result. (Note this is a capability
   the earlier two arbitrations lacked and explicitly caveated -- `astap_cli`
   is installed on this machine now.)
3. **ASTAP's own sidecar contradicts itself.** The `.ini` it wrote carries
   `PLTSOLVD=T` **and** `ERROR=Not enough stars.` **and** a scale warning, in
   the same file. It flagged the frame as problematic while reporting success
   -- which is the precise failure mode this project's reason codes exist to
   avoid.
4. **Re-hinting psolve at ASTAP's own centre does not move it.** Given
   219.63025, -66.12978 as the hint with a 1 degree radius, psolve walks
   **52.80" away from the hint it was given** and lands **0.17"** from its
   original unhinted answer.

**The 40.90" frame is left undetermined**, deliberately. The same arbiter gives
psolve 48% against ASTAP's 21% -- 2.3:1, favouring psolve but far below the
20:1 of the frame above, and both scores sit well under the 96-100% a clean
frame produces. That is not enough signal to call, and the frame is one the
capture pipeline had already marked `failed_`. It is recorded as an open
disagreement, not as a third ASTAP error.

Both facts are reported, deliberately, side by side: **the gate fails on
its own terms** (psolve disagreed with ASTAP by more than the specified
bar, on one frame out of 9495), **and the disagreement on that one frame is
ASTAP's error, not psolve's.** The four next-largest separations at M3 (27.25″,
25.59″, 22.28″, 15.39″; not re-derived since) are recorded as **undetermined** for lack of
signal-to-noise for the same reprojection arbiter to discriminate — they
are not implied to be additional ASTAP errors.

### Timing (Task 12, re-measured after the M3 final review)

Measured serially (never in parallel — a previous, parallel-contention
measurement of 114.7 ms/solve was single-solve latency only under 12-way
load, not what a lone invocation costs) on a single real frame, release
builds, one discarded warm-up round, then **9 interleaved rounds** — every
row runs once per round, in the same order, so all rows see the same machine
state — median reported, exact flags shown for every row:

> **Terminology.** These two modes were labelled "Blind" and "Hinted" until
> 2026-08-23. Neither is a blind solve: **every row below is a
> pointing-hinted solve**, and the rows differ only in where the pointing
> comes from -- the frame's own header cards, or an explicit flag. The
> mislabelling predates psolve having a blind solver at all; now that it has
> one, calling a 77 ms header-hinted solve "blind" invites reading it as a
> blind-solve timing, which it is not by a factor of ~20. Real
> pointing-blind timings are in
> `docs/superpowers/2026-08-15-blind-solve-results.md` section 3 (median
> 1.243 s, max 2.668 s). ASTAP's `-r 180` here widens its *search radius*
> around the header pointing; it does not remove the hint.

| Hint source | Tool | Flags | Median (9 runs) |
|---|---|---|---|
| Header | ASTAP | `astap_cli -f <F> -r 180 -fov 1.4770 -d /home/user/astap` | **100.86 ms** |
| Header | psolve | `psolve solve <F> --index <psidx>` (no `--hint`; falls back to header `OBJCTRA`/`OBJCTDEC`; radius auto-derived from optics, 1.6569°) | **77.34 ms** |
| Header | psolve, ASTAP mode | `psolve -f <F> -r 180 -fov 1.4770 -d <dir with the .psidx>` | **75.63 ms** |
| Flag | ASTAP | `astap_cli -f <F> -ra 16.425176 -spd 66.561173 -r 15 -fov 1.4770 -d /home/user/astap` | **101.05 ms** |
| Flag | psolve | `psolve solve <F> --index <psidx> --hint 246.377634,-23.438827 --radius 0.81235` (radius = `(fov/2)*1.10`, matching what ASTAP mode's own `-r 15 -fov 1.4770` resolves to internally) | **75.16 ms** |
| Flag | psolve, ASTAP mode | `psolve -f <F> -ra 16.425176 -spd 66.561173 -r 15 -fov 1.4770 -d <dir with the .psidx>` | **75.45 ms** |

All six solved (ASTAP `PLTSOLVD=T`; psolve `"solved":true`). A second,
independent 9-round measurement a minute later reproduced every median to
within 0.8 ms.

**psolve is now faster than ASTAP in both modes on this measurement**: 23.5
ms (23.3%) faster on the header-hinted form, 25.9 ms (25.6%) faster on the
flag-hinted form, and the drop-in
ASTAP-mode invocation — the one AstroOps actually issues — is faster still.

**This reverses the earlier reported result, so read how it was obtained.**
The previous table reported psolve 18–21% *slower* (159.90/157.81 ms against
ASTAP's 131.71/132.68 ms). Two separate things changed, and only one of them
is a psolve improvement:

1. **A real fix, worth ~70 ms.** The auto `--cat-limit` used to run a whole
   second decode + background + extract pass over the frame purely to count
   stars, before `solve()` did the identical work again. `psolve-core` now
   splits into `prepare()` (decode/background/extract) and
   `solve_prepared()`, so it happens once. Measured here in the same
   interleaved run, the pre-fix binary on the same frame: **147.68 ms**
   header-hinted, **145.59 ms** flag-hinted -- against the post-fix
   **77.34**/**75.16**.
2. **The machine is not in the state the earlier run measured.** ASTAP,
   unchanged, measures 100.86 ms here against 131.71 ms then, and the
   pre-fix psolve binary measures 147.68 ms here against 159.90 ms then.
   Both tools are faster now; ASTAP disproportionately so. That earlier
   session's background load is not recoverable, so the two tables are not
   directly comparable — which is exactly why the pre-fix binary was
   re-measured *inside this table*, under the same conditions as everything
   else. The ~70 ms attributed to the fix is a within-table difference; the
   ~30 ms shift in ASTAP's own figure is not.

See `docs/superpowers/2026-08-14-m3-first-real-frame.md`'s Task 12 section
for the full per-stage breakdown.

### The `timings_ms` JSON fields

Native mode's `psolve solve` prints a `timings_ms` object in its JSON
output (ASTAP mode does not — it writes sidecar files, not JSON, so these
fields are only visible through native mode). On a successful solve it
carries all ten keys below; on a failed solve it carries only `total`.

| Key | What it measures |
|---|---|
| `decode` | Decoding the FITS pixel data into the in-memory image buffer. |
| `background` | Background/noise estimation across the frame. |
| `extract` | Star detection and centroiding. |
| `caller` | Time spent in the CLI between preparing the frame and starting this solve attempt: the catalogue **index disc query**, and — when the frame is retried — every earlier attempt that failed. |
| `quads` | Building geometric quads from the extracted (and catalogue) stars for matching. |
| `catalogue` | **In-solver catalogue preparation only** — converting the already-fetched catalogue stars into the solver's internal representation. **This is not the cost of fetching those stars.** |
| `match` | Quad matching against the catalogue. |
| `fit` | Fitting the WCS transform from matched stars. |
| `verify` | Verifying the fit (residuals, confidence). |
| `total` | **The CLI's own wall clock**, not the sum of the eight stages above. It starts after the index is opened and spans everything from resolving the search radius through the solve returning — which includes the catalogue **index disc query** (`Index::brightest_in_disc`, reading the on-disk `.psidx`), which runs between `prepare()` and `solve_prepared()` and so is outside every instrumented stage. |

**The stages now account for the whole solve: `total` equals their sum.**
Measured over 14 corpus frames, the residual is 0.014-0.041 ms -- the
measurement overhead at stage boundaries.

**Correction — this section previously said the gap between `total` and the
sum of the other fields "is the index disc query, and it is small", citing
1.0 ms and 0.9 ms.** That was true only of a frame solving on its FIRST
attempt. `PreparedFrame::t_start` is set once in `prepare()` and never reset,
so on a **retried** frame `total` spans every attempt while the per-stage
numbers describe only the last one. Measured over 25 corpus frames split by
`scale_source`:

| solve path | total | stages | shortfall |
|---|---|---|---|
| `header` (first attempt) | 52.9 ms | 51.5 ms | 1.5 ms |
| `header/binning-retry` | 160.5 ms | 36.0 ms | **124.8 ms** |

On one frame that was 78% of the solve unattributed, and it was read as a
hidden bottleneck in the catalogue fetch and chased for an hour. It was the
earlier attempt, correctly spent and simply unreported. The `caller` field
now reports that interval directly, and it is measured rather than derived as
a residual -- a residual would silently absorb any future unaccounted time
while still claiming to be the disc query, which is how this section came to
be wrong in the first place.

**Correction — this document previously reported that gap as ~74 ms and
attributed it to the disc query. Both halves of that were wrong.** The gap
was real (65.5 ms header-hinted / 65.2 ms flag-hinted on the pre-fix binary, re-measured
under the same conditions), but it was not the lookup: it was the auto
`--cat-limit` running a second, complete decode + background + extract over
the frame just to count stars, before `solve()` did the same work again. The
decisive measurement isolates the probe from the query on the **pre-fix**
binary, by passing `--cat-limit 537` explicitly — 537 is exactly what the
auto path computes for this frame, so the catalogue, the solve and the disc
query are all identical and only the star-count probe is skipped:

| pre-fix binary, same frame | wall clock | stage sum | gap |
|---|---:|---:|---:|
| auto `--cat-limit` | 147.7 ms | 76.6 ms | **66.1 ms** |
| explicit `--cat-limit 537` | 83.8 ms | 77.8 ms | **0.96 ms** |

The gap was the probe. A consumer that read the old text and set out to
optimise the disc query would have been chasing ~1 ms while ~64 ms of
duplicated pixel work sat next to it.

## Swapping psolve in under N.I.N.A. does NOT cost the ERROR-PLATESOLVE event

**Resolved 2026-08-15. An earlier revision of this section warned that it did; that
warning was wrong and is retracted here rather than deleted, because the reasoning is
worth keeping and a stale warning reads exactly like a live one.**

ninaAPI does not observe the solver — it synthesises its events by regex-matching
N.I.N.A.'s log, and the plate-solve one matches `^ASTAP - Plate solve failed.`, an
ASTAP-specific literal. psolve never prints that string; a compat failure prints
`psolve: Not enough stars.` plus its own reason code. So the concern was that a
drop-in would silently lose the event.

**It does not, and the reason is the shape of the mimicry.** That string is a literal
inside `NINA.Core.dll` — one of N.I.N.A.'s own localisation resources, in two variants:

```
ASTAP - Plate solve failed. No output file found.
ASTAP - Plate solve failed.
```

**N.I.N.A.'s ASTAP adapter writes it; nothing reads the binary's output for this.** And
when psolve is symlinked as `astap_cli`, N.I.N.A. is still configured with the ASTAP
solver *type* pointing at that path, so the ASTAP adapter is what runs. The event fires
regardless of which binary sits behind the path — the swap is at the file-path level,
never at the log level.

Why it mattered enough to chase: the astroops gateway's stall detector reads event
*silence*, and during a plate-solve retry storm `ERROR-PLATESOLVE` is often the only
thing N.I.N.A. emits — 121 measured in one night. Had the event vanished, a rig that was
furiously retrying would have read as a rig that had gone quiet.

### The residual, checked: sidecar naming

The `No output file found.` variant fires when the solver does not write the sidecar
N.I.N.A. expects. **A solve that worked but wrote the wrong file looks — to N.I.N.A. and
to the gateway — exactly like a solve that failed.** Verified on the same frame, both
invoked without `-o`:

| | writes |
|---|---|
| `astap_cli` | `astapframe.ini`, `astapframe.wcs` beside the frame |
| `psolve` | `frame.ini`, `frame.wcs` beside the frame |

Same `<basename>.ini` / `<basename>.wcs` convention, same directory. This path is not
taken. Worth keeping in mind when testing sidecar fidelity, though: it is a route by
which a *content* bug surfaces as a spurious solve failure rather than as bad output.

### Still unrun

One deliberately-unsolvable frame through psolve under N.I.N.A., then read
`event-history`. Compiled evidence establishes who writes the line, not that the whole
chain fires end to end. Needs the capture host and an operator.

Credit: found, and then corrected, by the `astroops-nina` session — which read the
literal out of `NINA.Core.dll` from the local NuGet cache at the rig's exact version
(3.2.0.9001), no rig and no Windows required.
