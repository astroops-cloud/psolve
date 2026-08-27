# psolve

[![ci](https://github.com/astroops-cloud/psolve/actions/workflows/ci.yml/badge.svg)](https://github.com/astroops-cloud/psolve/actions/workflows/ci.yml)
[![licence: MIT](https://img.shields.io/badge/licence-MIT-blue.svg)](LICENSE)

> ## ⚠️ This was vibecoded
>
> **Effectively none of this code was written by a human.** It was built over
> 14 days with [Claude Code](https://claude.com/claude-code) — the Rust, the
> tests, the documentation, all of it — directed, reviewed and measured by one
> person who hand-wrote essentially nothing. I started it to find out what
> vibecoding could actually do. I published it because the results surprised
> me, not because the world needs another plate solver.
>
> **The operative risk is not who wrote it.** It is that psolve has *fourteen
> days, one observatory and one operator* of real use behind it. Every frame it
> has ever solved in anger came from one rig under one sky. That would be the
> honest warning even if a human had typed every line.
>
> The provenance cuts both ways too. Everything here is measured rather than
> asserted — 633 tests run plus 2 ignored, and every number below carries the run that produced
> it, including the ones that were wrong and got retracted.
> [How this was built](docs/how-this-was-built.md) is the full story, including
> what the approach cost.

A plate solver: FITS in, WCS out. Written in Rust, shipped as a single
static binary, built for headless automation rather than for a GUI that
happens to have a CLI.

## About ASTAP

[ASTAP](https://www.hnsky.org/astap.htm) is the industry standard for this job
and it deserves that standing. It is a complete astronomy suite — solving,
stacking, photometry, annotation, analysis — with a GUI and a CLI, prebuilt
all-sky star databases you can just download, SIP distortion support, and years
of field use across thousands of setups on Windows, Linux and macOS. It solves
the overwhelming majority of frames put in front of it, quickly and correctly.
It has run the pipeline this project came out of since that pipeline existed,
and it stays installed.

As of 2026-08-27 psolve is going into that same pipeline **alongside** it --
the author eating his own dogfood. Not as a replacement: the last per-rig
measurement (2026-08-23) had psolve winning by a wide margin on one camera,
level on another, and **losing on the primary instrument**, for a diagnosed
reason ([a completeness⁴
problem](docs/superpowers/2026-08-24-atr585m-diagnostic.md)). **Re-measured 2026-08-27: the regression
stands.** On frames ASTAP has no answer for, psolve solves 34.1% of the primary
rig's against ~67% on the other two, and 64% of those failures are
`TOO_FEW_STARS` -- a detection shortfall, which the 2026-08-25 improvements
could not touch because they are all matching rungs
([full measurement](docs/superpowers/2026-08-27-per-rig-remeasure.md)).

On frames ASTAP *did* solve, psolve reproduces 1,099 of 1,102 on that same rig
at 0.132" median, so it is not broken there -- it is weaker specifically on
marginal frames. Both tools file their answers independently and ASTAP remains
the one the pipeline trusts.

psolve is not a better ASTAP. It is a **narrower** tool with a different set of
trade-offs, and for some jobs those trade-offs are the ones you want.

### What each one is good at

ASTAP's own site is the authority on ASTAP; if anything in this column is
wrong, please file an issue and I will correct it.

| | ASTAP | psolve |
|---|---|---|
| **Scope** | Complete suite: solve, stack, photometry, annotate, analyse | Plate solving only |
| **Interface** | GUI and CLI | CLI only, no GUI, no runtime |
| **Star database** | Prebuilt all-sky databases, downloadable | You [build it yourself](docs/index-building.md#which-depth-do-you-need-start-here) from a Gaia DR3 mirror — you pick the depth and the declination cut |
| **Distortion** | SIP | **TAN only, no distortion terms** ([measured: this data wants one](docs/superpowers/2026-08-27-distortion-signal.md)) |
| **Platforms** | Windows, Linux, macOS — all field-proven | Linux, macOS **and Windows** all built and tested by CI; but **no human has run the Windows or Linux build** |
| **Field use** | Years, thousands of setups | One observatory, one machine, 14 days |
| **Sidecars** | `.ini` / `.wcs`, header `-update` | Byte-identical `.ini` / `.wcs`, same `-update` |
| **Machine output** | Sidecars and exit codes | Sidecars *plus* structured JSON: WCS, star counts, per-stage timings, fit residuals |
| **Why a frame failed** | Exit code | **11 distinct reason codes** plus per-reason star-rejection counts |
| **Deployment** | Install the application | One static binary, no runtime, no config |
| **Licence** | **MPL 2.0** | MIT (a built index is CC BY-NC 3.0 IGO, and is not MIT) |

### Nobody has actually used psolve on Windows

As of 2026-08-27 CI builds psolve natively on `windows-latest` and runs **620
of the 633 tests run** there (the 13 not run are Unix-only symlink and
permission cases), plus the end-to-end synthetic demo, which solves. The released `.exe`
is built on that same runner and executed before upload -- it is no longer a
cross-compiled binary nobody ever ran.

That is the entire extent of it. **No human has installed psolve on a Windows
machine, pointed it at a real frame, or run it during an imaging session** --
though that is about to change, and Windows matters here more than the ordering
of this section suggests.

**Windows is where the drop-in replacement claim actually gets tested.** This
project's ASTAP-compatible mode exists so that capture software can call psolve
instead of `astap_cli` mid-sequence, and that software -- N.I.N.A. in this
deployment -- runs on a Windows mini-PC bolted to the telescope. The Linux and
macOS builds are where psolve is *developed*; the Windows build is where it
would actually do the job it was written for.

The same caveat applies more weakly to Linux: CI tests it, but every real frame
this project has ever solved was solved on macOS. Machine-verified and
field-tested are different claims, and this project has a lot of the first and
one observatory's worth of the second, on one platform.

If you run it on Windows, the maintainer would genuinely like to hear how it
went -- that is a gap only a user can close.

**Reasons you might actually want psolve:** you are automating a pipeline and
want a machine-readable answer rather than a parsed log; you want to know *why*
a frame did not solve (`NO_QUAD_MATCH` vs `TOO_FEW_STARS` vs `INDEX_TOO_SHALLOW`
vs `CANNOT_READ` are different problems with different fixes); you want one
static binary in a container with no runtime; you want an index tuned to your
own sky rather than an all-sky one; or you want the `-update` write path to
refuse rather than risk your frames.

**Reasons you should stay on ASTAP:** you want a GUI; you need distortion
correction; you want a star database you can download instead of build; you are
on Windows; you want a tool with a decade of other people having hit the bugs
first. Any of those, and ASTAP is the right answer.

psolve speaks ASTAP's own CLI grammar and writes its sidecar bytes exactly, so
trying it costs a symlink — and going back costs the same symlink.

## Where psolve does well

Two measurements, and the caveat comes first because it is load-bearing:
**both populations are frames ASTAP failed on or has no record for.** That is a
biased sample by construction — measuring a challenger only on the incumbent's
hardest cases flatters the challenger, and says nothing about the 10,376 frames
ASTAP solved perfectly well, which is the actual job and which it did. These
numbers show psolve covers some cases that fell through; they are not a
scoreboard.

**Every solve in both was checked** — against the commanded pointing, and on a
sample by reprojecting the catalogue through the fitted WCS and measuring flux
at the predicted star positions — rather than counted on the tool's own say-so.

**Frames ASTAP failed in production** ([method](docs/superpowers/2026-08-26-production-failure-benchmark.md)):
the AstroOps deployment ran its 12,620-frame archive through ASTAP on the live
ingest path and parked 1,088. On a stratified 184 of those, **psolve recovers
72 (39.1%)** — 54% of the SVBONY SV405CC frames — in **369 s total**. Four of
the 184 turned out to be **truncated files** rather than hard frames, which
psolve separates as `CANNOT_READ` instead of reporting a failed solve.

**Frames ASTAP has no recorded answer for**
([head to head](docs/superpowers/2026-08-25-astap-head-to-head.md), both tools
re-run): psolve solved **113 of 200, none of them wrong**; ASTAP reported 21,
of which **two** were more than 10° out while reporting `PLTSOLVD=T`, leaving
19 correct.

Two of twenty-one is a small number and is quoted as an observed instance, not
as a characterisation of ASTAP's reliability -- on the 10,376 frames it solved
in production it is the thing psolve is checked *against*. It is noted at all
because a confidently wrong answer is worse than a refusal, which is this
project's own thesis and applies to it equally.

**Deliberately no speed ratio here.** See below for why the obvious one is
misleading.

Those two populations are not the same claim and this file does not merge them:
a missing solve row can mean "never attempted", while a parked frame is a
measured failure.

And on the full corpus — the fair comparison, on frames ASTAP *did* solve —
psolve agrees with ASTAP on **99.93%** of 10,376 frames to a 0.54″ median
centre separation. Agreement, not victory: those are ASTAP's answers being
reproduced.

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

It prints the solve JSON and exits non-zero if the frame did not solve.

**Solving your own frames needs a real star index.** There are two ways to get
one.

**Download a prebuilt one.** The [v0.1.0 release](https://github.com/astroops-cloud/psolve/releases/tag/v0.1.0)
carries an all-sky G≤14 pair -- `.psidx` (257 MB) for hinted solving and
`.psqidx` (428 MB) if you also want blind solving. Verify with the published
`SHA256SUMS`. **These files are NOT MIT**: they are derived from Gaia DR3 and
are CC BY-NC 3.0 IGO -- non-commercial, attribution required. `INDEX-LICENCE.txt`
ships beside them and [`docs/data-licence.md`](docs/data-licence.md) explains
what that constrains.

**Or build your own**, which you will want if your fields are narrow -- G≤14
returns as few as 3 stars on a 0.25° field in sparse sky, which is a refusal
rather than a solve. Start with
**[which depth do you need](docs/index-building.md#which-depth-do-you-need-start-here)**,
which answers that from your field of view;
[`docs/index-building.md`](docs/index-building.md) has the whole procedure.
Building is seconds once the Gaia mirror is on disk -- fetching that mirror is
the slow part, and is why the demo uses synthetic data instead.

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
median** as of 2026-08-25, down from 64.4 ms before that day's grid neighbour
search. **That is the honest speed number for this project** -- it is measured
over 10,376 frames that solve.

**The head-to-head medians are not a like-for-like speed comparison, and this
file will not present them as one.** Over that 200-frame set the raw figures
are psolve 62 ms against ASTAP 1,826 ms, 149 s against 975 s in total. But
those two medians are taken over different populations:

- psolve's 62 ms is the median of the **96 frames it solved**. Its failures are
  broken out separately in the source document -- `TOO_FEW_STARS` at 51 ms,
  `NO_QUAD_MATCH` at 3,286 ms.
- ASTAP's 1,826 ms is the median over **all 200**, of which 181 were not correct
  solves. On a set chosen because ASTAP has no answer for it, that median is
  mostly **the cost of a search giving up** -- its max on the run is 160.6
  seconds, which is an exhausted search, not a solve.

Comparing a solved-only median against an everything-included median and
calling the ratio "speed" is the same right-number-wrong-population error this
project has already had to retract once. The frames both tools solved are the
only fair basis, and n=18 is too small to claim much from.

Two different re-runs are quoted across this file, and they are not the same
measurement: **62 ms / 1,826 ms / 975.3 s at `c91cd0a`**, and **64 ms /
1,836 ms / 985.4 s at `0579c33`**. Both are in
[the head-to-head document](docs/superpowers/2026-08-25-astap-head-to-head.md);
neither supersedes the other, they are separate runs of the same 200 frames.

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
