# Where the time actually goes, and which data-structure tricks pay

**Date:** 2026-08-25. Every number here is measured on this machine; the two
benchmarks are throwaway crates, not committed.

Prompted by a fair question: are there structural tricks -- of the
Merkle-tree variety -- that would make psolve faster or let it solve more?
Short answer: **one clear speed win, one genuinely applicable Merkle
application, one idea that measurement killed, and nothing that solves more.**

## 0. First, a reporting defect found on the way in

`timings_ms` looked alarming: on one frame the stages summed to 37.5 ms
against a reported total of 171.5 ms -- **78% unaccounted**. The obvious
hypothesis was a hidden bottleneck in the catalogue fetch.

Wrong. Split by `scale_source` over 25 frames:

| solve path | total | stages | gap |
|---|---|---|---|
| `header` (first attempt) | 52.9 ms | 51.5 ms | **1.5 ms** |
| `header/binning-retry` | 160.5 ms | 36.0 ms | **124.8 ms** |

`t_start` is set in `prepare()` and never reset, so `total` spans **every**
retry attempt while the per-stage numbers describe only the last one. There
is no hidden cost -- the gap is the earlier attempt, correctly spent.

It is still worth fixing, because a reader profiling a retried frame sees
stages that do not sum to the total with nothing saying why, and goes looking
for a bottleneck that is not there. This one cost an hour.

**Fixed** -- `Timings::caller` now reports that interval, measured rather
than derived as a residual.

## 1. The win: `build_quads` does an O(n^2) neighbour search

For each of n points it allocates a Vec of all n-1 squared distances and
partially selects the k nearest. Quad building is the largest reported stage
(18 ms of a 52 ms median solve), and it runs **twice** per attempt -- image
side and catalogue side -- with the catalogue side reaching `--cat-limit`
5000 points.

A uniform grid sized to hold ~k points per cell, searched by expanding rings,
returns **identical** neighbours:

| n | full scan | uniform grid | speedup | same answer | `build_quads` total |
|---|---|---|---|---|---|
| 200 | 0.39 ms | 0.10 ms | 3.8x | yes | 1.69 ms |
| 500 | 1.41 ms | 0.24 ms | 5.8x | yes | 5.67 ms |
| 1000 | 4.86 ms | 0.48 ms | 10.1x | yes | 14.99 ms |
| 2000 | 11.81 ms | 0.70 ms | 16.9x | yes | 36.86 ms |
| 4000 | 35.91 ms | 0.99 ms | 36.4x | yes | 113.23 ms |

The neighbour search is 25-32% of `build_quads` across that range, and the
saving grows with density -- exactly where this solver is slowest. The same
grid already exists in `pairmatch::Grid`; this is not new machinery.

**Done** (`perf(core): grid the neighbour search in build_quads`). Measured
end to end on 40 corpus frames: quads stage **19.5 ms -> 6.8 ms (2.9x)**,
whole solve **53.2 ms -> 39.3 ms (26% faster)**. Verified output-identical on
3,458 frames -- same 3,457 solved, zero outcome or matcher changes, and not
one solved position moved by as much as 0.001 arcsec.

One correction to the plan above: the benchmark that produced this table used
a stopping rule ("stop once k candidates are collected") that is **not
exact** -- a point two rings out can be nearer than one in this ring's
corner. It agreed with the scan on uniform random points, which is how it
would have shipped. The committed version stops only when the k-th candidate
is provably closer than anything unsearched.

## 2. Merkle tree: genuinely applicable, and to the right problem

Index integrity today is all-or-nothing. `verify_digest` rehashes the whole
record region: **1.09 s for the 448 MB `.psqidx`**, and the star indexes are
0.22 GB (g14), 1.03 GB (g16), 4.08 GB (g18). Because that is far too
expensive for the solve path, it runs only under `index info` /
`quad-index info --verify`.

Consequence: **nothing verifies the bytes a solve actually reads.** A
corrupted region returns plausible garbage -- a catalogue star at a wrong
position -- which is precisely this project's named failure mode, and the one
thing `records_sha256` exists to prevent.

A Merkle tree over fixed record blocks changes the economics. A solve touches
about **47 of 49,152** HEALPix cells (measured, 2.5 deg disc at nside=64) --
roughly 220 KB of a 0.22 GB index. Verifying those blocks plus ~16 sibling
hashes is on the order of a **millisecond**, against 1.09 s for the whole
file. That is cheap enough to run on every solve, and it localises damage to
a block instead of condemning the file.

The pairing check is already the right shape and should not change:
`QuadIndex::open` compares a **stored** `star_index_fingerprint` against the
star index's `records_sha256` in 0.01 s, no rehash.

**Worth doing, but it is a format change** -- a new optional section and a
version bump, against a `.psidx` the architecture notes describe as
load-bearing and not to be churned. Right idea, needs its own design.

## 3. Killed by measurement: the k-way merge

`brightest_in_disc` picks the next-brightest star by rescanning every cell
cursor, which is O(limit x cells) with a trig `angsep` per cursor per step.
That looks like a textbook binary-heap candidate, and a heap does win:

    2.5 deg disc, limit 5000:  merge 3.52 ms -> heap 1.56 ms  (2.3x)
    2.5 deg disc, limit 1500:  merge 2.08 ms -> heap 1.44 ms  (1.4x)
    0.5 deg disc, limit 300:   merge 2.03 ms -> heap 2.00 ms  (1.0x)

Identical output in all 27 configurations tested. But the absolute cost is
**1.5-3.5 ms** on a 52 ms solve. A 47-cell disc is simply too few cursors for
the asymptotics to bite.

**Not worth it.** Recorded because it was my first hypothesis for the 78% gap
in section 0, and it was wrong twice over: wrong about the location, and
wrong about the size of the prize.

## 4. Unmeasured candidate: 4-D code-space search

`match_quads` is brute force over `image_quads x catalogue_quads x 2
parities`. The module doc defends this, correctly, at the scale it was
written for (~377 x 200). The quad-budget retry now runs it at **1500 x
1500 x 2 = 4.5 million** 4-vector distance computations.

astrometry.net puts a k-d tree over code space for exactly this. Whether it
pays here is **not measured** -- the match stage is 10 ms median, so the
ceiling is modest, but it will be the dominant stage on any frame that
reaches the retry budget. Measure before building.

## 5. Nothing here solves more frames

The remaining 38 failures are 22 `NO_QUAD_MATCH`, 14 `LOW_CONFIDENCE`, 2
`TOO_FEW_STARS`, and the blind path solves 1 of them
(`2026-08-25-pair-match-retry-results.md`). They fail on **detection** -- the
best hypothesis reaches 4-7 inliers with no margin over the runner-up. No
index or search structure creates a star that was never detected.

## Order of work

1. ~~Grid neighbour search in `build_quads`~~ -- **done**, 2.9x on the stage,
   26% off the solve, output-identical.
2. ~~Make `timings_ms` add up~~ -- **done**, via a measured `caller` field
   reporting the interval between preparing the frame and starting this
   attempt. Residual is now 0.014-0.041 ms across 14 frames.
3. Measure the 4-D code-space search before deciding.
4. Merkle index verification -- real, but a format change deserving a design.
