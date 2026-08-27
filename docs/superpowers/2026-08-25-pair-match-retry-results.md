# Pair matching as the final retry: corpus results, and why there is no Pyramid spike

**Date:** 2026-08-25. Branch `pair-match-retry`. Spike that motivated it:
`2026-08-24-pair-matching-spike.md`.

## Result

| | before | after |
|---|---|---|
| solved (10,376 frames) | 10,254 (**98.82%**) | 10,338 (**99.63%**) |
| regressions | -- | **0** |
| route changed on already-solving frames | -- | **0** |
| newly solved | -- | **84**, all via pair matching |
| agreement with ASTAP, newly solved | -- | median **0.8"**, p90 2.4", max 52.6", none over 60" |
| agreement, all solved | -- | median 0.5", p99 3.3" |
| solve wall | -- | median **64 ms**, p99 482 ms |
| null test (hint a verified 40 deg away) | -- | **0 of 146** |

Exactly 10,254 frames still answer through quads -- the same set as before,
not merely the same count.

## The ordering defect the corpus run caught

The first implementation defaulted `pair_retry` **on** inside
`solve_prepared`, so the first attempt took the pair path whenever quads
failed. That pre-empted the two retries above it, and **41 frames that had
solved through the binning retry began solving through pair matching
instead**.

Nothing failed. They still solved, still agreed with ASTAP to under two
arcseconds. That is exactly why it would have shipped: the only symptom was a
field in the JSON, and the JSON field only existed because `matcher` was
added in the same commit. Confirmed by building the pre-change binary and
running those frames through both.

It mattered because "a frame that solves today cannot change its route" was
the entire argument for adding this as a retry rather than a replacement. A
claim that is true by construction stops being worth anything the moment the
construction changes.

**The fix:** `pair_retry` defaults off; `solve_with_binning_retry` enables it
for one final attempt after the scale/binning retry and the matched-filter
re-extraction. Both entry points call that function, so both inherit it.

Ordering alone cost 14 frames (87 -> 73), because the spike had let pair
matching see every rung's improved inputs. So the last rung is now handed the
best any rung produced -- the refetched catalogue disc, the matched-filter
star list when it found more stars, and the corrected plate scale. Pair
matching converts pixel separations to angles, so it is the rung most
sensitive to a wrong scale. That recovered 73 -> 83 -> 84. The remaining 3
against the spike's 87 are the price of not pre-empting, and are not worth
taking back.

## No Pyramid spike: the prize was measured and it is one frame

The star-tracker research recommended Pyramid -- unique-triangle
identification with a fourth-star confirmation, over a k-vector index. The
hinted path does not want it: this implementation already tests each
hypothesis against every star in the frame, which is strictly more evidence
than three further triangles, and Pyramid's cheaper confirmation exists only
because a star tracker cannot afford verification on rad-hard hardware.

That left blind solving as the place Pyramid's machinery might earn a
whole-sky pair-separation index. **Measured first, before building
anything:**

- **The 40 hardest hinted failures, through the blind path: 40 of 40 solve.**
  Blind is not the weak link. Its `.psqidx` code-space search picks catalogue
  quads differently from the hinted path's `build_quads`, and that selection
  succeeds where the hinted one starves.
- **The 38 frames that still fail after this change, through the blind path:
  1 of 38 solve.** There is no gap for Pyramid to close.

**Verdict: do not build it.** The whole-sky k-vector index is a large piece
of work whose measured prize on this corpus is a single frame.

### The confound that nearly made this wrong

The blind probe uses the **g16** index; the hinted corpus run uses **g14**.
Read carelessly, "blind solves 40/40" says a deeper catalogue fixes these
frames. Running the **pre-change** binary (no pair retry) on the same 40:

    g14: 0 of 40 solved     g16: 0 of 40 solved

Depth changes nothing. The 122 failures were a matching problem throughout,
and blind's success is its matching path, not its catalogue. Reporting the
blind number without separating the index would have been a measurement
without its invocation.

## What is left

38 frames: 22 `NO_QUAD_MATCH`, 14 `LOW_CONFIDENCE`, 2 `TOO_FEW_STARS`. Blind
solves one of them. They are a detection problem, not a matching one -- the
dense fields where the best hypothesis reaches 4-7 inliers with no margin
over the runner-up.

Also still open, unrelated and small: the `LOW_CONFIDENCE` detail string
prints the criteria that passed and omits `min_matched`, the one that
failed.
