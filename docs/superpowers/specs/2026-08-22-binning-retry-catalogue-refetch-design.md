# Binning-Retry Catalogue Refetch — Design

**Date:** 2026-08-22
**Status:** proposed
**Size:** one focused fix plus its measurement. Not a milestone.
**Depends on:** nothing. Applies to `main` as it stands.

## 1. The problem

Every 2×2-binned CFA frame in this deployment's corpus fails to solve. All 791
of them, no exceptions, all `NO_QUAD_MATCH`.

Measured 2026-08-22, release build of `aa73521`, hints taken from
`catalogue.db`'s commanded pointing, `gaia-dr3-g14-dec45-nside64.psidx`, the
same invocation shape `scripts/agreement.sh` uses (`psolve solve <path>
--index <idx> --hint <ra>,<dec>`, defaults elsewhere):

| population | n | solved |
|---|---:|---:|
| SVBONY SV405CC, `XBINNING=2`, CFA | 791 | **0 (0.0%)** |
| everything else in the corpus | 9,582 | 9,342 (97.5%) |

That single cell is **76.7% of all solve failures** in the corpus. Closing it
moves the live-corpus solve rate from ~90.1% to a projected ~97.7%.

The frames are not marginal. Given a correct search radius, the *unmodified*
binary solves 790 of the 791 at a 0.707″ median separation from ASTAP's own
recorded centre — better than the corpus-wide median. They are ordinary frames
that psolve refuses for a reason that has nothing to do with them.

## 2. Root cause

`XPIXSZ` is ambiguous when `XBINNING > 1`: some drivers write the physical
pixel size, some write the already-binned one. `pixel_scale_arcsec` assumes the
former and multiplies by the binning factor. This rig's driver writes the
latter (`XPIXSZ = 9.26` where the sensor pixel is 4.63 µm), so the
header-derived scale comes out at exactly twice the truth: 15.72″/px where the
frame is 7.86″/px.

`solve_with_binning_retry` (`crates/psolve-cli/src/cmd_solve.rs:396`) exists
precisely for this. It detects the case, and on failure re-solves once at
`scale / XBINNING`. That half of it works.

**The other half is missing.** Line 438 passes the *same* `catalog` slice to
the retry:

```rust
let retry_result = psolve_core::solve::solve_prepared(prepared, catalog, &retry_opts);
```

The catalogue was fetched by the caller, before the retry, at a radius derived
from the uncorrected scale — `header_radius_deg` is built on the same inflated
scale, so the disc comes out at 6.02° where the frame's true half-diagonal plus
margin is 3.01°. Twice the radius is four times the sky area, and the
catalogue budget (`--cat-limit`, auto-sized to ~1,500 here) is spent spreading
the brightest stars across 113 deg² of sky when the frame covers 14. Almost
none of the fetched stars fall inside the frame, so no consistent transform
exists to be found.

The retry then reports `NO_QUAD_MATCH` — "no consistent transform", which reads
as *this frame is unsolvable*. The truth is *we searched a disc twice too
wide*. This is the project's signature failure shape: not a crash, a plausible
negative.

Confirmed on frame
`archive/fits/SVBONY_SV405CC/NGC_3372/2025/05/20/0g/15.0s/Light/2025-05-20_19-44-18__-9.90_15.00s_0000.fits`,
whose stderr states it outright:

```
1500 catalogue stars within 6.019281491299753 deg of 161.0792,-59.8892
header scale 15.7203"/px did not solve; retrying once at 7.8601"/px
```

At `--radius 3.0` the same binary solves the same frame.

**The retry has never once succeeded on this corpus.** All 9,342 currently
solving frames report `scale_source: "header"`; none reports
`"header/binning-retry"`. It was added for these 810-odd frames and has a 0%
success rate against them.

## 3. What is NOT the fix

The obvious candidate was the CFA decode fix on the unmerged
`fix-cfa-double-binning` branch (`7ebda12`), which stops software
superpixel-binning a frame the camera already hardware-binned. It cherry-picks
onto today's `HEAD` cleanly. Measured over the same 791 frames:

| configuration | solved | rate | sep median | sep p90 |
|---|---:|---:|---:|---:|
| `HEAD`, auto radius | 0/791 | 0.0% | — | — |
| `HEAD` + decode fix, auto radius | 0/791 | 0.0% | — | — |
| **`HEAD`, corrected radius** | **790/791** | **99.9%** | **0.707″** | **1.378″** |
| `HEAD` + decode fix, corrected radius | 776/791 | 98.1% | 0.919″ | 1.759″ |

The decode fix neither closes the hole nor helps once the hole is closed: on
top of the corrected radius it **costs 14 net frames** (15 regress, 1 newly
solves) and widens median separation by 0.2″. Its own commit message predicted
why — `extract.rs`'s fixed `min_pix = 4` was implicitly tuned around the
coarser double-binned plate scale, and a correct decode halves the scale so
that stars span fewer pixels than the floor admits.

This also resolves the contradiction between that branch's report ("184
regress, 54 newly solve" over 791 bin-2 frames) and the 0/791 measured here.
Both are correct about different harnesses: the branch was measured where the
radius was already adequate, so the decode fix's cost was visible; today's
`agreement.sh` uses the auto radius, where the population is 0% either way and
regression is unmeasurable.

The decode fix stays unmerged, deliberately (§9). It is correct and currently
harmful, and that pairing must be written down or it will be re-litigated.

## 4. Approach

**When the binning retry fires, refetch the catalogue at the corrected radius
before re-solving.**

Radius scales linearly with assumed plate scale, so the corrected radius is the
header-derived radius divided by `XBINNING` — the same divisor already applied
to the scale, for the same reason.

Rules:

1. The retry recomputes `radius_corrected = radius_header / xbinning`, re-applies
   whatever caps the calling mode applied to the original radius, refetches the
   catalogue through `select_catalog`, and re-solves with both the corrected
   scale and the corrected catalogue.
2. **An explicit caller-supplied radius is never overridden.** Native mode's
   `--radius` is a caller assertion, exactly as `--scale` is; when it is
   present the retry keeps today's scale-only behaviour. ASTAP mode's `-r` cap
   is re-applied to the corrected value rather than discarded, preserving
   `search_radius_deg`'s existing "never wider than the caller's own `-r`"
   guarantee.
3. When the header lacks the optics keywords, `header_radius_deg` is `None` and
   the retry's `header_scale` is also `None` — the retry does not fire at all,
   and this change adds no new behaviour to that path.
4. The fix lives inside `solve_with_binning_retry`, which both entry points
   already call (`cmd_solve.rs:795`, `cmd_solve.rs:1126`, `main.rs:320`), so
   ASTAP-compatible dispatch gets it structurally rather than by discipline.
   This is the rule the 2026-08-14 scale retry broke once already by landing in
   `cmd_solve.rs` alone.

## 5. Components

`solve_with_binning_retry` cannot refetch today: it receives `catalog: &[CatalogStar]`
and has no access to the index. It gains one parameter — everything needed to
redo the fetch, and nothing else:

```rust
pub(crate) struct CatalogRefetch<'a> {
    index: &'a Index,
    hint_ra: f64,
    hint_dec: f64,
    radius_header_deg: f64,   // header-derived, BEFORE any cap was applied
    radius_cap: Option<f64>,  // ASTAP mode's -r; None in native mode
    limit: usize,
    explicit_radius: bool,    // native --radius; suppresses the refetch
}
```

`radius_header_deg` is deliberately the **uncapped** header-derived value, not
the radius the first fetch actually used. Dividing an already-capped radius by
`XBINNING` and dividing the header value then re-capping are different numbers
whenever the cap bound the first fetch; only the latter is correct. The
distinction is invisible in native mode (no cap) and load-bearing in ASTAP
mode.

If the corrected radius comes out equal to the one already fetched — a cap
binding both times — the refetch is skipped and the retry behaves exactly as it
does today. No query is issued to arrive at the same disc.

`psolve-core` is untouched. The refetch is a `select_catalog` call, which is
CLI-side because the index is CLI-side — the same boundary every other
catalogue fetch already respects, and the reason `psolve-core`'s
no-filesystem guarantee survives this change unmodified.

The `scale_source` reported on a retry success stays `"header/binning-retry"`.
It will start appearing in the corpus for the first time, which is itself a
useful signal that the path is live.

## 6. Cost

One extra disc query and catalogue conversion, on frames that have already
failed once. The disc query is measured at ~1.0 ms against the 234 MB `.psidx`
(`docs/astap-compat.md`, `timings_ms` section). A failed bin-2 solve already
costs roughly 2× a normal solve because the retry re-runs the match; this adds
about 1 ms to that, and only for `XBINNING > 1` frames.

Against it: 790 frames that currently cost a full failed solve and produce
nothing will instead solve.

## 7. Why this cannot regress

Regression-freedom here is **structural, then confirmed by measurement** — the
strongest form of the bar this work is held to.

The refetch is reachable only by a frame that (a) failed its first solve
attempt, and (b) has `XBINNING > 1`, and (c) was not given an explicit radius.
Condition (a) alone excludes every frame that solves today.

That is not an argument, it is a corpus fact: all 9,342 currently-solving
frames report `scale_source: "header"`, meaning none of them reaches the retry
at all. There is no frame in the corpus whose current success this change can
touch.

The measurement in §8 confirms it rather than establishing it.

## 8. Acceptance criteria

1. **The hole closes.** ≥ 785 of the 791 bin-2 CFA frames solve, with median
   separation from ASTAP's recorded centre ≤ 1.0″. (Measured ceiling for this
   approach: 790/791 at 0.707″.)
2. **No regressions.** Every frame solving before this change still solves
   after it, over the full live corpus, compared per frame and not in
   aggregate. Expected count: zero, per §7.
3. **Live corpus solve rate ≥ 97.5%**, up from ~90.1%.
4. **Both entry points.** The same frame solves through native `psolve solve`
   and through ASTAP-compatible `psolve -f ... -d <dir>`, asserted in a test
   that runs both, per the standing rule.
5. **The 38.2″ disagreement is arbitrated.** The corrected-radius run produces
   one solve that disagrees with ASTAP's recorded centre by 38.2″, over the 30″
   bar. It is not a regression — the frame does not solve at all today — but it
   gets the same reprojection arbitration the NGC 3372 case got
   (`docs/superpowers/2026-08-14-m3-first-real-frame.md`), and the finding is
   reported whichever way it lands.
6. **One frame still fails** with the corrected radius. Its reason code is
   recorded, not hand-waved.

## 9. The measurement instrument must be fixed first

`scripts/agreement.sh`'s stratified sampler carries this comment:

> Binning is NOT stratified: every ASTAP-solved frame in this database is
> binning=1 (verified separately; there are no 2x2 rows to draw from), so a
> binning axis in the sampler would silently produce an empty stratum rather
> than a real cross-section.

There are 791. The assertion was true when written and has silently expired,
and the consequence is that `agreement.sh sample` — the cheap run, the one
used for iteration — structurally cannot see the entire failing population.
That is why a 0%-solving population of 791 frames sat undetected behind a
reported 97.6% headline.

Two changes, both prerequisites for trusting the acceptance measurement:

- Stratify on binning, or fail loudly if a stratum the sampler assumes is
  empty turns out not to be. A sampler that silently omits an axis is the same
  defect class as the solver returning a plausible negative.
- Correct the comment to state what is true, dated, rather than deleting it.

## 10. Risks

**The corrected radius is right for this rig by construction, not by
detection.** The fix does not identify which `XPIXSZ` convention a header uses
— it cannot, from the header alone. It retries the alternative. A rig whose
driver writes the physical pixel size is unaffected: its first attempt
succeeds and the retry never fires. A rig that fails its first attempt *for an
unrelated reason* and happens to be binned will now also pay a refetch before
failing, which costs ~1 ms and changes no outcome.

**A second retry is not proposed.** Only one alternative convention exists, so
one retry covers it. Generalising to a radius ladder would be a search, and a
search needs a confidence gate — out of scope here, and the blind-solve
milestone already documents why that is not a small addition.

## 11. Deferred by intention

- **The CFA decode fix (`7ebda12`).** Correct, and measurably harmful until
  extraction thresholds stop being fixed pixel counts. Left unmerged with this
  document as the reason.
- **Scale-aware extraction.** `min_pix = 4` and `edge_margin = 8` are
  plate-scale-dependent quantities hardcoded as independent ones; `k_sigma`
  and `max_pix_factor` already adapt. Fixing this is the prerequisite for the
  decode fix and the plausible route into the remaining ~240-frame failure
  tail (ATR585M at 87.4%, library frames at 82.2%, globulars). Its own design
  cycle, informed by this one's evidence.
- **The failure tail itself**, including the globular population whose
  detections die below the `min_pix` floor before selection runs.
