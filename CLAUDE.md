# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

`psolve` is a plate solver: FITS bytes in, a verified TAN WCS out. Rust, one
static binary, no runtime, built for headless automation. It is a drop-in
replacement for `astap_cli`'s CLI grammar and sidecar formats.

**Framing, and it matters in anything you write here.** ASTAP is the industry
standard for this job and deserves that standing -- a full astronomy suite,
prebuilt all-sky databases, SIP distortion, years of field use across thousands
of setups. psolve is a narrower tool with different trade-offs (headless, JSON
output, reason codes, one static binary, an index tuned to one sky), not a
better ASTAP. Both benchmark populations in the README are frames ASTAP failed
on or has no record for, which is a **biased sample by construction** and is
labelled as such; the fair number is the 99.93% agreement on frames ASTAP did
solve. Do not write copy that reads as a scoreboard.

This repo was written almost entirely by Claude Code over 14 days
(`docs/how-this-was-built.md`), which the README states at the top.

## Commands

```sh
cargo test --workspace                 # 633 run + 2 ignored, ~3 min cold; all pass, clippy clean
cargo test -p psolve-cli --test blind_solve            # one test file
cargo test -p psolve-core no_filesystem                # one test target by name filter
cargo test -- --ignored --nocapture                    # the slow real-index/measurement tests
cargo clippy --workspace --all-targets                 # must stay clean
cargo build --release                  # target/release/psolve — required by scripts/agreement.sh
```

**Do not run `cargo fmt` over the repo.** It has never been rustfmt-clean and
there is deliberately no `cargo fmt --check` gate in CI (`CONTRIBUTING.md` says
why): a bare `cargo fmt` rewrites ~60 files — 819 differing hunks measured
2026-08-26, up from the 759 recorded when CI landed — burying a real change in
a whole-repo reformat. Reformatting is a decision to take on its own merits, in
its own commit. Format only what you touch, by hand, matching the surrounding
code.

Two tests are `#[ignore]`d because they are minutes-long measurements against
the real multi-GB indexes, not correctness checks
(`psolve-index/tests/blind_candidates_real_index.rs`,
`psolve-cli/tests/blind_measure_tolerances.rs`).

Tests that need real rig data (`~/astroops/data/*.psidx`, `*.psqidx`, frames
under `~/astroops/library/`) **skip with an `eprintln!` rather than fail** when
it is absent — keep that convention for any new one, so the suite still runs on
a machine without the artefacts. `psolve-cli` has no `[lib]` target, so
integration tests shell out to the compiled binary (`bin()` walks up from
`current_exe()`); unit tests for CLI internals live in co-located `mod tests`.

## Rig data (this machine)

- `~/astroops/` is **strictly read-only** for everything in this repo. Never
  pass `-update` at a path inside it; never write sidecars beside its frames.
  Copy to scratch first — every test that mutates a frame does.
- `~/astroops/data/` holds the built indexes: `gaia-dr3-g{14,16,18}-dec45-nside64.psidx`
  and the paired `gaia-dr3-g16-dec45-nside64.psqidx` (blind-solve quads).
- Ad-hoc measurement output goes in `.scratch/` (gitignored, regenerable from
  `scripts/`). Indexes and sidecars (`*.psidx`, `*.ini`, `*.wcs`) are gitignored
  except the reference sidecars under `crates/psolve-cli/tests/fixtures/`.

## Architecture

Three crates; the split is what makes "the solver cannot modify your data" a
property of the dependency graph rather than a promise.

| crate | role | dependency budget |
|---|---|---|
| `psolve-core` | FITS read → extract → quads → match → fit → verify. Bytes in, values out. | **none, not even dev-deps** |
| `psolve-index` | `.psidx` / `.psqidx` formats, mmap readers, Gaia builder | `memmap2` only |
| `psolve-cli` | the `psolve` binary: flags, JSON, sidecars, all output policy | the above + `rayon` |

Two structural guards enforce this and will fail loudly if broken
(`crates/psolve-core/tests/no_filesystem.rs`): a token scan rejecting
`fs`/`net`/`process`/`env`/`File`/`OpenOptions`/`PathBuf` anywhere in
`psolve-core/src` (**including in comments** — it fails closed, so reword the
prose), and an assertion that `psolve-core`'s `[dependencies]` is empty.

Two more guards of the same shape — a convention that fails rather than one
people are trusted to remember — live in `psolve-cli/tests/`:

- `rig_data_dependence.rs` pins **which** test files may skip themselves when
  the rig data is absent (currently four). Add a rig-dependent test and the
  suite fails until you list it there, which forces the choice: commit a
  fixture, or widen the coverage gap on purpose. Widening it is allowed;
  widening it silently is not.
- `fixtures_are_tracked.rs` shells out to `git check-ignore` to prove the
  committed reference sidecars are not caught by `.gitignore`'s `*.ini`/`*.wcs`
  rules. It **panics rather than skips** when `git` is missing, which is why CI
  installs git into every job.

Consequences worth internalising before designing anything:

- The catalogue is **passed in** to `psolve_core::solve`, never looked up. The
  CLI opens the index, fetches the disc, hands over `Vec<CatalogStar>`.
- `blind.rs` likewise receives already-resolved candidate quad positions; the
  `.psqidx` code-space lookup happens in `psolve-index::quad_reader` and is
  driven from `psolve-cli`.
- `psolve-cli::cmd_quadindex` is the one place allowed to depend on both
  `psolve-index` and `psolve-core`, so all quad-index geometry (tiling, band
  assignment, per-tile selection) lives there while `psolve-index::quad_builder`
  only knows the on-disk shape.

### Solve pipeline (`crates/psolve-core/src/`)

`fits.rs` (header cards + decode, untrusted input, never panics) → `background.rs`
(tiled surface — a global threshold makes a nebula one giant "star") →
`extract.rs` (threshold, connected components, rejections **counted by reason** —
those counts are the diagnostic value of a failed solve) → `quad.rs` (Lang-style
4-vector geometric hashes; parity deliberately not handled here) → `match_.rs`
(brute force both parities, vote in (scale, rotation) bins) → `pairmatch.rs`
(a RETRY only: match on star PAIRS when no quad budget finds a transform --
two correspondences plus the known scale fix a transform outright, so each
separation agreement is a hypothesis tested against every other star. Its doc
records the two voting designs that failed first, which is why it is shaped
as it is) → `fit.rs` (two
independent 3-parameter least squares in the tangent plane — no linear-algebra
crate needed) → `verify.rs` (Poisson log-odds against chance) → `solve.rs`
(orchestration, `Outcome`, `Timings`). `blind.rs` turns one quad-code match into
a candidate WCS and reuses `fit.rs` wholesale.

`project.rs` sits underneath all of it: great-circle separation, the gnomonic
(TAN) projection, and proper motion. Its tangent-plane coordinates are in
**degrees**, matching CDELT, so a fit yields CD-matrix coefficients with no unit
conversion in between; separations are haversine rather than `acos` of a dot
product, which loses precision exactly at the small angles this pipeline cares
about.

`verify.rs` carries the milestone's central lesson: the hinted gate
(`min_log_odds: 12.0`) is calibrated for **one** hypothesis against a known disc.
A blind solve tests thousands, so the gate is multiplicity-corrected. Applying
the hinted number to a blind search once produced a confident solve 87.77° from
the truth. Read that module's doc before touching any acceptance threshold.

### Index formats (`crates/psolve-index/src/`)

- `.psidx` (`format.rs`, `record.rs`, `reader.rs`) — HEALPix nested `nside=64`,
  **brightest-first within each cell**, 16-byte fixed records so the mmap casts
  straight to a slice. That sort is why one index serves every field size: a
  narrow field just reads less of the same run. Proper motion is applied at
  solve time from `DATE-OBS`.
- `.psqidx` (`quad_format.rs`, `quad_builder.rs`, `quad_reader.rs`,
  `blind_grid.rs`) — precomputed quad codes for blind solving, banded by
  angular scale, searched by an equal-population (quantile) grid over code
  space. A separate format on purpose: `.psidx` is load-bearing and must not
  churn, and each format's reader must **reject** the other's file, not
  misparse it.
- A `.psqidx` cannot be opened without its paired `.psidx`: `QuadIndex::open`
  takes the `Index` and checks `star_index_fingerprint`, because `star_idx`
  references resolved against the wrong star index yield confident garbage.

A build is byte-reproducible **on one host and not across hosts**, measured
2026-08-26 (`docs/superpowers/2026-08-25-index-depth.md`, which retracts an
earlier cross-host claim in place). The star data is confirmed identical --
`psolve index query` on a dense and a sparse disc returns byte-identical
output on macOS/arm64 and Linux/x86-64 -- so the divergence is downstream, in
the float `conditioning_key` that orders quads: a one-ULP difference is not a
tie the integer tie-break catches, it is a different ordering. Platform `libm`
trig is the supported suspect, not a measured cause. Correctness is untouched
and both artefacts solve. What this forbids is **identifying an index by its
bytes** -- verifying a distributed pair by hash, or answering "was this built
from that catalogue at that magnitude limit" from a digest. Nothing depends on
it today because indexes are built by whoever runs the tool; do not build a
verification story on the emitted order.

### The retry ladder (`psolve-cli::solve_with_binning_retry`)

Four rungs, run in order, each reached only when everything above it failed.
**The ordering is load-bearing and the rule behind it is not negotiable: a
frame that solves today must not change its answer or even its route.** That
makes each addition regression-free by construction rather than by
measurement.

1. the header scale/binning retry, refetching the catalogue at the corrected
   radius
2. matched-filter re-extraction (`MATCHED_FILTER_SIGMA`)
3. **pair matching** (`pairmatch`) -- the expensive one; measured p90 4.82 s
   against the quad path's 0.16 s, which is precisely why it is not the
   default
4. **tight search radius** -- refetch at `RADIUS_RETRY_HALF_DIAG_FRAC` (0.5)
   of the frame half-diagonal

Rungs 3 and 4 are handed **the best inputs any earlier rung produced** (the
refetched disc, the matched-filter star list, the corrected scale), not the
originals. Getting that wrong cost 14 frames when rung 3 first landed.

Two defects this ladder has already produced, both worth knowing before
adding a fifth rung:

- **A rung defaulted ON inside `solve_prepared` pre-empted the rungs above
  it.** 41 already-solving frames silently moved onto a different route.
  Nothing failed -- they still solved and still agreed to under 2″ -- which
  is exactly why it nearly shipped. A rung belongs in the ladder, not in the
  core's default path.
- **A rung's Outcome was adopted only when it SOLVED**, so every failure
  reported the previous rung's message with no trace the later one had run.
  Adopt the failure too, guarded by `keep_most_informative`.

### The two CLI surfaces

`main.rs` dispatches to **ASTAP mode whenever argv contains `-f`**, before the
native `--long`-flag parser ever runs. The two surfaces must not blend:

- Native exit codes: `0` solved · `1` not solved (a normal outcome — clouds are
  not a bug) · `2` usage/config · `3` index problem.
- ASTAP mode collapses everything non-success to `1`, deliberately, because its
  whole purpose is to be indistinguishable from `astap_cli` at the `$?` AstroOps
  branches on. Do not "fix" this by leaking the native scheme into it.
- `-ra` is **hours**; `-spd` is **south polar distance** (`dec + 90`). Confirmed
  against real recorded invocations, not inferred.
- Sidecar bytes (`sidecar.rs`) reproduce real `astap_cli` output exactly, in two
  structurally different `.ini` formats and two `.wcs` formats; ground truth is
  `docs/superpowers/2026-08-14-astap-format-facts.md`.

**Any behaviour change must be wired through BOTH entry points.** A binning-retry
fix once reached `cmd_solve.rs` only and left ASTAP dispatch stale; the blind
solve tests now run the same frame through both for exactly this reason.

### `-update` and other writes

`fits_update.rs` is the only path that touches pixel data, and a header rewrite
that shifted the data unit silently corrupted four archive frames once. The
rules (full statement in `docs/astap-compat.md#the--update-safety-model`, every
one exercised in `tests/fits_update.rs`): default off; `PSOLVE_READONLY`
(any non-empty value) refuses; a `.psolve-readonly` marker on the target's
canonical ancestor chain refuses unconditionally, with two best-effort lexical
chains as extra coverage; write a full temp copy, `fsync`, reparse it from a
fresh read and require byte-identical pixels, then `rename`; refuse rather than
shift when the header would need another 2880-byte block; refuse a target with
no write bits. Sidecar writes share the two safety switches via
`refuse_if_readonly_output`.

## CI and packaging

`.github/workflows/ci.yml` runs on GitHub Actions: `lint` (clippy
`-D warnings`) · `test` on `ubuntu-latest` **and** `macos-latest` · `package`
on `v*` tags only. It replaced a self-hosted GitLab pipeline on 2026-08-27;
that file is gone from the tree and lives in the pre-release history.

Two things about that pipeline that change how you work locally:

- **The suite must not run as root**, which is why the `test` job uses VM
  runners rather than a `container:` job. `fits_update.rs`'s post-rename
  fsync-failure test stages its failure with `chmod 0o311`, which denies root
  nothing — so under root it cannot arrange the condition it exists to
  exercise, and it **panics naming root as the cause** rather than passing
  vacuously. The workflow asserts `id -u` is non-zero so this fails loudly
  rather than silently. Do not "fix" it by making the test tolerate root.
- **A green pipeline proves less than it looks.** No hosted runner has the
  `~/astroops` data, so the four rig-dependent test files skip — the move from
  a self-hosted runner to GitHub changed nothing about this. Green means
  "compiles, clippy-clean, data-independent tests pass" — not that the
  agreement run holds, that blind solving still works against a real index, or
  that sidecar bytes still match `astap_cli`. Those are measured **locally
  against the rig** and cannot be automated here; `rig_data_dependence.rs` is
  what keeps the gap from growing silently.

Platform coverage is asymmetric and `packaging/README.md` states it plainly.
Measured from the `release` run on `v0.1.0`, 2026-08-27: the Linux amd64 `.deb`
is built **and executed** (the job `dpkg -i`s its own artifact and runs the
installed binary), the Linux and macOS bare binaries are built **and run**, and
and the Windows `.exe` is built **natively on `windows-latest` and executed**
since 2026-08-27 (it was previously cross-compiled with mingw-w64 and never
run). `ci.yml` runs the whole suite and the demo on all three platforms every
push: 620 of the 633 run pass on Windows, the 13 gaps being `#[cfg(unix)]`
symlink and permission tests. **No human has run psolve on Windows** — CI is
the only thing that has, and that distinction belongs in any copy you write. `packaging/homebrew/psolve.rb`
still carries a deliberately invalid `sha256` so it refuses to install rather
than installing whatever it downloaded. One shipped behaviour difference follows
from all this: on Windows `fits_update::same_directory` returns `None`
unconditionally, so one of the three `.psolve-readonly` ancestor chains is
permanently unavailable there. The canonical chain is unaffected.

## Conventions

- **The failure mode to design against is a plausible return value, not a
  panic.** Nearly every expensive defect here returned something acceptable-
  looking: a confident wrong solve, a cached result keyed on a version that
  never moved (hence the `git describe`-derived `build` field from `build.rs`),
  a discarded flag with no signal. Prefer a loud refusal, and when a flag is
  accepted but not applied, say so on stderr.
- **Measured, not projected.** Claims in docs carry the number, the flags, and
  the machine state. When a re-measurement contradicts an earlier claim, both
  figures are reported and the earlier one retracted in place — see the README's
  timing section and `docs/astap-compat.md`. Don't smooth over a failed criterion
  or a disagreement; investigate and record it.
- Reason codes (`psolve_core::ReasonCode`) are the machine contract for *why* a
  frame did not solve; keep them distinguishable (`NO_HINT` vs `FOV_MISMATCH` is
  a bug fixed once already) and add a new one rather than overloading.
- Columns in external catalogues are located **by name, never by position**.
- **Search `crates/`, not `.`.** Two gitignored git worktrees live inside the
  repo (`.claude/worktrees/`, `.worktrees/`), each a full copy of the crate
  tree. Anything that does not honour `.gitignore` counts them: `find . -name
  '*.rs'` returns 317 against 64 under `crates/`. `git worktree list` says what
  is there.
- `--cat-limit` is a COUNT and therefore means a different magnitude on every
  index; `--max-mag` is the ceiling that does not depend on which index the
  flag is pointed at.
- Commit subjects: `type(scope): summary`, e.g. `feat(index,cli): ...`,
  `fix(core): ...`, `docs: ...`. ASCII `--` rather than em dashes throughout the
  codebase's prose.

## Where the reasoning lives

Module docs are unusually load-bearing here — most "why" is in the source, not
in a wiki. Beyond those:

- `docs/superpowers/specs/` — design specs (`2026-08-13-psolve-design.md` is the
  whole product: architecture §4, index §5, pipeline §6, JSON contract §7.2,
  errors §9, testing §10, milestones §11).
- `docs/superpowers/plans/` — per-milestone implementation plans.
- `docs/superpowers/<date>-<topic>.md` — the flat measurement records, and the
  provenance behind every number in the README: the production-failure
  benchmark, the ASTAP head-to-heads, the pair-matching and matched-filter
  spikes, index depth, radius sensitivity. When a README claim needs checking,
  the run that produced it is one of these.
- `docs/how-this-was-built.md` — the provenance the README leads with: what the
  AI was good at, the five defects it produced that tests could not catch, and
  the three habits that made them survivable. Read before writing any copy
  comparing psolve to ASTAP.
- `docs/astap-compat.md` — flag-by-flag compatibility, sidecar bytes, exit codes,
  `-update` model, and the M3 agreement/timing measurements.
- `docs/index-building.md` + `scripts/fetch-gaia.sh` — Gaia DR3 mirror and
  `psolve index build`. `docs/data-licence.md`: a built index is CC BY-NC 3.0 IGO
  and the MIT code licence does not cover it.
- `.superpowers/sdd/<date>-<milestone>/` — the execution ledger for each
  milestone: `progress.md`, per-task `task-N-brief.md` / `task-N-report.md`, and
  `review-<a>..<b>.diff` snapshots. Continuing a milestone means reading its
  `progress.md` first.
- `scripts/agreement.sh` + `scripts/agreement-report.py` — the agreement run
  against the ASTAP solves in `~/astroops/state/catalogue.db` (opened
  `-readonly`); `sample [N]` for a stratified subset, `full` for every solved
  frame — 10,376 as of 2026-08-25. Quote the full corpus first: the older
  9,495-frame restricted corpus contains **no 2×2-binned frames at all**, which
  is how a 791-frame population that solved at 0% stayed invisible. These are
  ASTAP's answers, not ground truth: a disagreement gets investigated, not
  assumed to be psolve's fault.
