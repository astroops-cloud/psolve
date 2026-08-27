# The fifth rung: falling back to blind when the hint is unsurvivable

**Date:** 2026-08-27. **Result: 24 of 26 previously-failing frames now solve.**

## The failure, and how it was misdiagnosed twice before being found

The Windows rig benchmark showed psolve failing **0 of 30** pointing-probe
frames where ASTAP solved 26. That was first attributed to the ATR585M
completeness problem, and separately suspected to be a Windows defect. **Both
were wrong**, and the sequence is worth recording because each hypothesis was
plausible and each was killed by measurement rather than argument:

| hypothesis | test | verdict |
|---|---|---|
| Windows defect | same frames on macOS | 0/30 there too, identical reason codes -- **not Windows** |
| trailed stars (374 of 914 rejected as elongated) | measured ellipticity vs science frames | probes **0.49**, science **0.57** -- probes are *less* elongated. **Refuted** |
| completeness (image sees deeper than catalogue) | g16 and g18 indexes, `--cat-limit` to 5000 | still fails at 19x the catalogue depth. **Refuted** |
| too few usable stars | relaxed every extraction filter, 320 -> 737 used | still fails. **Refuted** |
| **the hint is wrong** | compare header pointing to ASTAP's answer | **18.77-19.45 deg off, all 26 frames** |

**The mount was pointing 19 degrees from where it believed it was**, which is
exactly what a pointing-model build exists to discover and correct. psolve
searched 1.66 degrees around the mount's claim and never had a chance.

Confirmation from both directions: given the correct position psolve solved the
frames at **0.36" rms**; given no position at all it solved them **blind**, to
the same answer. The capability was already built and simply unreachable,
because a hint that exists is trusted even when it is wrong.

## Why not just widen the search radius

Checked, not assumed: **5, 15 and 25 degrees all still failed.** The catalogue
budget is fixed, so a wider disc spreads the same number of stars over more sky
and completeness in the actual field collapses. That is the radius sensitivity
`2026-08-26-radius-sensitivity.md` already measured, and it is why "just search
wider" is not the fix. A blind search never fetches a disc at all -- it looks
up each image quad's own scale-invariant code.

## The rung

Last on the ladder, guarded on `Outcome::Failed`, quad index opened lazily so a
frame that solves never pays for it. **Safe by construction under the ladder's
non-negotiable rule** -- a frame that solves today cannot change its answer or
its route, because it never reaches this rung.

Wired through **both** entry points. On the ASTAP surface it auto-discovers the
`.psqidx` from `-d`/`-D`, exactly as the hintless blind path already does, so
no flag appears in a grammar that must stay indistinguishable from
`astap_cli`'s. A fix reaching only `cmd_solve.rs` is the mistake that call site
has made before.

Blind failures are adopted through `keep_most_informative` rather than
discarded -- a rung whose failure leaves no trace it ran cost this project a
diagnosis once already.

## Verification, and the hole in it that had to be closed first

**Corpus, 1,102 ATR585M frames, fallback present:** 0 lost, 0 newly solving,
**0 answers moved by even 0.001"**.

**That run proved less than it looked.** `scripts/agreement.sh` never passed
`--quad-index`, so the fallback could not fire on a single frame. A regression
check that never reaches the code it is checking establishes only that the
change is inert -- worth knowing, since it is the configuration most callers are
in, but it is not evidence about the new path.

`agreement.sh` gained a `QUAD_INDEX` axis for that reason, recorded in each
row's `cmd` so a later reader can see whether a run exercised the fallback. The
armed re-run holds:

| | baseline | fallback armed |
|---|---|---|
| solved | 1,099 / 1,102 (99.73%) | **1,099 (99.73%)** |
| median separation | 0.132" | **0.132"** |
| gross errors >30" | 1 | **1** -- the same `_probe` frame arbitrated as ASTAP's error |

**The safety property that matters most here:** the three frames that
legitimately fail still refuse, with unchanged reason codes, when a quad index
IS available. This project's defining incident was a blind search returning a
confident answer **87.77 degrees** from the truth. A fallback that converted
honest refusals into confident guesses would be worse than the bug it fixes.

## Three-system measurement, 2026-08-27

The same 26 mis-pointed probe frames, the same all-sky G<=14 index pair, the
same source tree (`git archive main`), run on three machines. Specs are given
by role rather than by name.

| role | CPU | RAM | OS | binary | solved | wall | ms/frame |
|---|---|---|---|---|---|---|---|
| workstation | Apple M5 Max, 18C | 128 GB | macOS 15 (arm64) | native `cargo build --release` | **24/26** | 164 s | **6,326** |
| compute host | AMD Ryzen 9 9950X3D, 16C/32T | 91 GB | Arch (x86-64) | native, built in `rust:latest` (rustc 1.98.0) | **24/26** | 260 s | **10,010** |
| capture host | Intel i3-8109U, 2C/4T | 15.9 GB | Win 11 Pro 26200 | **cross-compiled `x86_64-pc-windows-gnu`** | **24/26** | 914 s | **35,143** |

`ms/frame` is total wall divided by 26 and therefore includes the two frames
that fail, each of which burns a full blind search before giving up.

**The correctness result is the one that matters: 24 of 26 on every platform,
the same 24.** Three architectures, three toolchains, two endianness-identical
but otherwise unrelated libm implementations, one answer. The blind path is
not carrying a platform-specific defect.

**Two caveats that belong on the timing numbers, not buried under them:**

1. The Windows binary is **not** the artefact CI builds and ships. The capture
   host has no Rust toolchain, so this binary was cross-compiled with the gnu
   toolchain from the Linux container; releases are built natively with msvc on
   `windows-latest`. The measurement is real, but it is a different binary.
2. **The Linux/macOS ordering is the opposite of what core count predicts and
   the cause is not established.** A 16C/32T Ryzen 9 9950X3D taking 58% longer
   than an 18-core M5 Max suggests the blind search is dominated by
   single-threaded work rather than parallel throughput -- but that is a
   hypothesis, not a measurement, and it is recorded here as one. Nothing was
   changed on the basis of it.

### What 35 s/frame means operationally

The capture host is the machine that matters for this rung, because it is the
one running the capture software that calls psolve mid-sequence. There, the
fallback costs **35 seconds per frame** -- roughly 700x a normal hinted solve
(measured median 50.4 ms) and about 17x a typical `astap_cli` probe solve.

That cost is only ever paid on a frame that has **already failed every cheaper
rung**, so the alternative is not a fast solve, it is no solve at all. But it
is long enough that it is a real scheduling consideration rather than a
rounding error: a sequence that trips the fallback on many frames in a row will
notice. This is the number to quote when deciding whether to ship the rung
enabled by default on a slow capture machine -- not the workstation's 6.3 s.

## What it does not fix

**2 of the 26 are not rescued**, and they are not mysterious: 7,221 and 8,361
detections against ~900 on a typical probe, of which only 261 and 339 survive
filtering. Noise-flooded frames, failing honestly.

**Cost.** A blind search on this hardware runs about 6.6 s, paid only by frames
that have already failed every cheaper rung. On the capture host -- a 2018
dual-core i3 -- expect several times that. The failure path was already the
expensive one (`2026-08-27-windows-rig-benchmark.md` measured 10 s there before
this rung existed); this makes it longer still, and the early abort on pair
matching is the fix for that, not this.

**It needs a `.psqidx`.** Without one the honest answer is still a refusal, and
the rung does not fire -- a slower refusal helps nobody.

## What the test asserts, and what it does not

The test asserts the **decision**, not the search. Whether a blind search then
succeeds is `solve_blind`'s own contract, covered against a real index by
`blind_solve.rs`. What is new -- and what could regress silently -- is that a
failed hinted solve consults it at all. So it checks the stderr announcement and
that the reported failure now speaks the blind search's vocabulary ("image
quads", "hypotheses") rather than the hinted path's dead end ("no catalogue
stars supplied").

It is synthetic and needs no rig data, because the failure is about the hint
rather than about the frames.
