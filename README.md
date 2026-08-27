# psolve

[![ci](https://github.com/astroops-cloud/psolve/actions/workflows/ci.yml/badge.svg)](https://github.com/astroops-cloud/psolve/actions/workflows/ci.yml)
[![licence: MIT](https://img.shields.io/badge/licence-MIT-blue.svg)](LICENSE)

A plate solver: FITS in, WCS out. Written in Rust, shipped as a single
static binary, built for headless automation rather than for a GUI that
happens to have a CLI.

It exists to compete with [ASTAP](https://www.hnsky.org/astap.htm) for this
workload — leaner, and able to say *why* a frame did not solve.

Two measurements against ASTAP, on frames ASTAP does not solve. **Every solve
in both was checked** — against the commanded pointing, and on a sample by
reprojecting the catalogue through the fitted WCS and measuring flux at the
predicted star positions — rather than counted on the tool's own say-so.

**Frames ASTAP actually failed in production** ([the stronger
provenance](docs/superpowers/2026-08-26-production-failure-benchmark.md)): the
AstroOps deployment ran its 12,620-frame archive through ASTAP on the live
ingest path and parked 1,088. On a stratified 184 of those, **psolve recovers
72 (39.1%)** — 54% of the SVBONY SV405CC frames — in **369 s total**, where
ASTAP parked all 184 and at its 180 s cap would spend up to **9.2 hours**.
Four of the 184 turned out to be **truncated files**, not hard frames, which
psolve separates as `CANNOT_READ` rather than reporting as a failed solve.

**Frames ASTAP has no recorded answer for**
([head to head](docs/superpowers/2026-08-25-astap-head-to-head.md), both tools
re-run): psolve solved **113 of 200, none of them wrong**; ASTAP reported 21,
of which **two were more than 10° out while reporting `PLTSOLVD=T`**, leaving
19 correct. Median wall 64 ms against 1,836 ms.

Those two populations are not the same claim and this file does not merge
them: a missing solve row can mean "never attempted", while a parked frame is
a measured failure. Full method and the corrections it forced:
[`docs/superpowers/2026-08-25-astap-head-to-head.md`](docs/superpowers/2026-08-25-astap-head-to-head.md).

That is not dominance in every respect and this file does not claim it.
ASTAP still wins one frame of the 200; its `d50` database is all-sky where
these indexes carry a declination cut; psolve fits a TAN WCS with no
distortion terms; and ASTAP has years of field use across thousands of setups
against one machine and one observatory here. ASTAP stays installed and keeps
running the AstroOps pipeline; `psolve` competes with it on measured merit,
and the switch-over (if it ever happens) is a symlink flip in both
directions — contingent on that merit, not assumed ahead of it.

```
psolve index build --input <DIR> --out <FILE> [OPTIONS]
psolve index info <FILE>
psolve solve <FILE> --index <FILE> [OPTIONS]
```

Full CLI reference: `psolve --help`.

## Try it in one command

No star index, no Gaia download, no telescope. This generates a synthetic star
field, builds an index from the catalogue that field was generated from, and
solves it:

```sh
./scripts/demo.sh
```

It prints the solve JSON and exits non-zero if the frame did not solve. Solving
your own frames needs a real index built from a Gaia DR3 mirror --
[`docs/index-building.md`](docs/index-building.md) -- which is a much longer
first step, and is why the demo does not use one. A built index is also
CC BY-NC 3.0 IGO rather than MIT ([`docs/data-licence.md`](docs/data-licence.md)),
which is why none ships here.

## Drop-in replacement for `astap_cli`

`psolve` also accepts `astap_cli`'s own single-dash argument grammar (`-f`,
`-r`, `-fov`, `-ra`, `-spd`, `-d`, `-update`, …) and writes `astap_cli`'s
own `.ini`/`.wcs` sidecar files and exit-code scheme, so anything that
already shells out to `astap_cli` — Siril, N.I.N.A., or AstroOps itself —
can point at `psolve` with no change on its side. ASTAP mode is entered
whenever `argv` contains `-f`.

The two invocations AstroOps issues in production, with **one change beyond the
binary name** — `-d` must point at a directory holding a psolve `.psidx` index,
not at ASTAP's own star-database directory. ASTAP's `-d /home/user/astap` holds
`d50_*.1476` files, which psolve cannot read:

```sh
# AstroOps' first attempt. When the frame's header carries OBJCTRA/OBJCTDEC
# (or decimal RA/DEC), psolve auto-detects it and this is an ordinary hinted
# solve. When it does not -- sentinel/absent pointing -- psolve now solves
# BLIND instead of refusing, auto-discovering the `.psqidx` blind-solve quad
# index `~/astroops/data` also holds alongside the `.psidx`; see below.
psolve -f <path>.fits -r 180 -fov 1.4770 -d ~/astroops/data -update

# The hinted narrow-radius retry, issued when the first attempt fails.
# -ra is HOURS (RA/15), -spd is SOUTH POLAR DISTANCE (dec + 90) -- both are
# properties of THIS frame's commanded pointing, not constants. Substituting
# another frame's path while leaving these as-is points the search at the
# wrong sky and the solve correctly refuses.
psolve -f <path>.fits -ra <ra_hours> -spd <dec_plus_90> -r 15 -fov 1.4770 -d ~/astroops/data -update
```

Verified end to end, fresh scratch copies, `-update` included, re-run
2026-08-23 on `ic2872/lights/O/2026-07-29_19-23-20_O_120.00s_100g_1x1_0001`:
both exit `0`, both write a `PLTSOLVD=T` `.ini`, and both rewrite the input
header in place with the pixel data verified byte-identical before the rename
is committed. The retry was run at that frame's own `-ra 11.468889
-spd 27.011111`; run verbatim with a different frame's coordinates it exits
`1` with `NO_QUAD_MATCH`, which is the correct refusal and the reason the
placeholders above are placeholders.

On this rig the first invocation also prints a radius warning -- `-fov 1.4770`
implies 0.812°, the header implies 1.657° -- and proceeds on the
header-derived value. That disagreement is a real property of AstroOps'
standard invocation on this optics set, not a psolve defect; see
[`docs/astap-compat.md`](docs/astap-compat.md).

**psolve solves blind.** A frame carrying no usable pointing — this archive
uses `DEC = -90.` as an "unset" sentinel — used to be solvable by ASTAP and
not by psolve, which returned `NO_HINT` unconditionally. Given a `.psqidx`
blind-solve quad index (native mode: `--quad-index <FILE>`; ASTAP-compatible
mode: auto-discovered from `-d`/`-D` alongside the `.psidx`, no flag needed —
the invocation above already gets it, since `~/astroops/data` holds both),
psolve now searches the index's precomputed quad codes instead of refusing,
and verifies any candidate against a multiplicity-corrected confidence gate
before accepting it — a plain `NO_HINT` is safer than a confident wrong
answer, and blind solving without that correction produced exactly that
(the motivating incident for this milestone, recorded in
[`docs/superpowers/specs/2026-08-15-blind-solve-design.md`](docs/superpowers/specs/2026-08-15-blind-solve-design.md)).
Without a `.psqidx` anywhere psolve can find one, the behaviour is unchanged:
`NO_HINT`, not a crash.

Note the two unit conventions ASTAP uses and psolve reproduces exactly: **`-ra`
is in hours**, and **`-spd` is south polar distance (`dec + 90`)**, not
declination. Full flag-by-flag reference, exit codes, sidecar byte formats, and
the `-update` safety model (temp-copy + verified rename, `PSOLVE_READONLY`,
`.psolve-readonly` markers, default off) are in
[`docs/astap-compat.md`](docs/astap-compat.md).

### Measured, not projected

> **On commit references.** Public history begins at `v0.1.0`. Short SHAs cited
> in this document (`297961b`, `3ba1c32`, and others) refer to the pre-release
> development history, which is retained privately and is not part of this
> repository -- they will not resolve here. The measurements themselves are
> reproducible from the flags, corpus and data each one names; the SHA records
> which build produced a figure, not where to find it.

Agreement, over the **full live corpus** -- all 10,376 frames ASTAP had
solved in this deployment: psolve solves **99.93%** of them (10,369/10,376)
as of 2026-08-25, with a **0.54″** median centre separation from ASTAP's own
recorded solution (p99 3.33″). Seven frames remain: four `NO_QUAD_MATCH`, two
`TOO_FEW_STARS`, one `LOW_CONFIDENCE`.

That figure moved 98.82% -> 99.63% -> 99.93% on 2026-08-25 through three
additions, each gated on the same rule -- a frame that solves today must not
change its answer or even its route, so every one is a **retry** reached only
after the existing ladder has failed:

| addition | corpus | measured |
|---|---|---|
| [pair matching](docs/superpowers/2026-08-25-pair-match-retry-results.md) when no quad budget finds a transform | 98.82% -> 99.63% | 84 frames, 0 regressions |
| [tight search radius](docs/superpowers/2026-08-25-cross-frame-priors.md) (0.5× the frame half-diagonal) | 99.63% -> **99.93%** | 31 frames, 0 regressions |
| [grid neighbour search](docs/superpowers/2026-08-25-data-structure-survey.md) in quad building | — | 2.9× on the stage, output-identical |

The **97.74%** (10141/10376, median 0.539″) this file carried until
2026-08-25 is superseded, not retracted: it was correct for the binary that
produced it. The agreement gate failed on two
of those frames (30.69″ and 38.22″ against a 30″ bar) -- both investigated
and found to be **ASTAP's** error on those specific frames, not psolve's;
both the failures and the findings are reported in full, not smoothed over,
in [`docs/astap-compat.md`](docs/astap-compat.md#measured-results-m3) and
[`docs/superpowers/2026-08-22-binning-retry-refetch-results.md`](docs/superpowers/2026-08-22-binning-retry-refetch-results.md).

**The 9,495-frame figure this file carried until now is retained, not
retracted, because it is still correct on its own corpus** -- 97.61%
(9268/9495), median 0.530″, p90 0.945″ -- and today's binary reproduces it
exactly (this run's p99 reads 3.105″ against the published 3.098″; that is
the percentile-estimator convention difference `docs/astap-compat.md`
already documents, not a behaviour change). What changed is that **that corpus contains no
2×2-binned frames at all**, and the 791 that exist solved at 0% until
2026-08-23, when the binning retry began refetching the catalogue at the
corrected search radius (790/791, median 0.707″). A restricted corpus and a
sampler that both happened to exclude the only failing population is why the
77.2%-of-all-failures hole was easy to leave alone; the full-corpus number
above is quoted first from now on for that reason. **It was not hidden**:
`docs/superpowers/2026-08-15-stratified-selection-results.md` recorded it in
full eight days earlier -- "none of them solve, 0/791, a pre-existing gap" --
and deferred it as out of that milestone's scope. The failure was a measured,
published finding left unscoped, not an instrument that could not see.

The 9,495-frame column itself replaced the M3 one this file carried before
it -- 97.09% (9219/9495), median 0.531″, p99 3.128″ -- measured before
conditional stratified star selection landed on `main` (`297961b`). Same
9,495 frames, same G≤14 index, newer binary: +49 frames net. The M3 numbers
were not wrong when taken; they are simply no longer what this binary does.
Over the full corpus the same before/after is 90.12% -> **97.74%**, and every
one of the 790 newly solving frames is 2×2-binned; not one previously
solving frame changed, byte for byte.

Timing across the **whole corpus**, not one frame: solver total **50.4 ms
median** as of 2026-08-25, down from 64.4 ms before that day's grid
neighbour search. Against ASTAP over the 200-frame head-to-head above,
process wall: **62 ms median against 1,826 ms**, and 149 s against 975 s in
total.

One caveat this file states rather than buries: the pair-matching retry made
*failure* more expensive. A `NO_QUAD_MATCH` cost ~70 ms before it existed and
~500 ms after, even with the early abort that measurement forced. A frame
that solves never reaches the retry, so the median is unaffected — but
"psolve fails fast" is weaker than it was.

The earlier single-frame table below is retained for the record. It read
**77.34 ms vs 100.86 ms** header-hinted and 75.16 vs 101.05 flag-hinted
(23–26% faster).

Both of those are **pointing-hinted** solves; until 2026-08-23 this file and
`docs/astap-compat.md` labelled the header-hinted form "blind", which was
harmless before psolve had a blind solver and misleading now that it does.
Real pointing-blind solving costs **1.243 s median, 2.668 s max** — roughly
20× the hinted path, and well inside the 5 s design bar. See
[`docs/superpowers/2026-08-15-blind-solve-results.md`](docs/superpowers/2026-08-15-blind-solve-results.md).

That reverses what this file said a commit ago, so here is what changed.
Most of it is a real fix: the auto `--cat-limit` used to decode and extract
the whole frame a second time just to count stars, ~70 ms of duplicated
work, now done once. The same pre-fix binary measured **147.68 ms**
header-hinted inside this very table. The rest is machine state — ASTAP, unchanged,
measured 131.71 ms in the earlier session and 100.86 ms here, so the two
sessions are not comparable and the earlier table's *ratio* was measured
under conditions that no longer exist. Both figures are reported rather than
the flattering one. psolve is still far off this project's original 12–14 ms
design projection. Full table, the pre-fix control row, and the per-stage
breakdown: [`docs/astap-compat.md`](docs/astap-compat.md#timing-task-12-re-measured-after-the-m3-final-review).
