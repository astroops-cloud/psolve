# Cross-frame priors: measured, and the answer is the search radius

**Date:** 2026-08-25. Ran against the 38 corpus frames still failing after the
pair-matching retry. 28 of them have a solved sibling in the same session
directory (often hundreds).

## The question, and the surprise

The idea was that a telescope shooting a sequence could hint frame N+1 from
frame N's solved answer -- a position good to arcseconds instead of the
commanded pointing's arcminutes. Four variants, to isolate which lever does
the work:

| variant | rescued |
|---|---:|
| A. commanded hint, default radius (baseline) | **0 / 28** |
| B. **prior position**, default radius | **0 / 28** |
| C. prior position + tight radius | 13 / 28 |
| D. prior position + tight radius + prior plate scale | 13 / 28 |

**The prior position buys nothing.** B is the cross-frame prior on its own and
it rescues zero frames. The reason is visible in the data: the commanded
pointing and the prior differ by a median of **0.0161 deg**, and the search
disc is degrees across. The centre was never the problem.

Everything C gains comes from the **radius**. And a prior is not needed to
shrink it -- the frame's own header gives its field size:

| variant | rescued |
|---|---:|
| E. **commanded** hint + half-diagonal radius | **14 / 28** |
| F. commanded hint + 0.75x half-diagonal | **23 / 28** |
| G. commanded hint + 1.25x half-diagonal | 10 / 28 |

E beats C. The sibling contributes nothing at all.

## The radius curve

psolve's default is **1.1x** the frame half-diagonal (half the diagonal plus a
10% pointing-error margin).

```
  0.35 x half-diag : 20 / 28  ####################
  0.50 x half-diag : 24 / 28  ########################   <- peak
  0.60 x half-diag : 22 / 28  ######################
  0.70 x half-diag : 22 / 28  ######################
  0.80 x half-diag : 23 / 28  #######################
  0.90 x half-diag : 16 / 28  ################
  1.00 x half-diag : 14 / 28  ##############
```

Broad and flat from 0.35 to 0.8, falling off above it. A disc at 0.5x does not
even reach the frame's corners -- and that is the point. Catalogue stars
outside the frame have no possible image counterpart, so every one of them
lowers completeness, and a quad needs all four of its stars present on both
sides.

## The rescued solves are correct

Checked against ASTAP's own recorded answer for the same frames:

- **24 of 28 rescued** at 0.5x half-diagonal
- agreement median **0.61"**, p90 1.81", **max 2.60"**
- **24 of 24 inside 5 arcsec; none beyond 60**
- log-odds min 16.0, median 41.1, against a gate of 12.0

No wrong answers.

## What this means

**Cross-frame priors are not worth building.** They rescue nothing the frame's
own header cannot, they require sequence state the solver does not have, and
they would tie a solve's result to the order frames arrive in.

**A radius retry is worth building**, and it needs nothing external: no
service, no sequence state, no orchestration. Same shape as every other rung
-- run it only after the existing ladder has failed, so a frame that solves
today never reaches it and cannot change its answer. On the corpus that is
~24 of the 38 remaining failures, taking the solve rate from 99.63% to about
**99.86%**.

Not yet implemented; this document is the measurement that justifies it.

## The same lever, pulled the wrong way

Measured 2026-08-26 and recorded in `2026-08-26-radius-sensitivity.md`: the
curve keeps falling above the default too. On a cloud-degraded frame psolve
**fails outright** at a 10 deg disc where it solves at 1.65, while ASTAP
solves the same frame at every radius tried. The tight-radius retry is one end
of a sensitivity psolve has and ASTAP does not.

## Footnote: the telescope-farm case

A separate question, and the earlier "a service buys 11%" measurement answered
only the local-batch case. For a farm the numbers that matter are different:

| | |
|---|---|
| median frame size | **16.8 MB** (p90 16.8, max 50.4) |
| ship one frame over 1 GbE | 135 ms |
| ship one frame over 2.5 GbE | 54 ms |
| ship one frame over WiFi 6 (~400 Mbps real) | 336 ms |
| **median solve** | **~48 ms** |

**Moving the frame to a central solver costs more than solving it.** On
anything short of 10 GbE the transfer dominates, and on wireless it dominates
by 7x. psolve is a single static binary with no runtime, so the cheap
architecture for a farm is to run it **at each node** and ship back a WCS --
a few hundred bytes instead of 16.8 MB.

What a central service would still buy a farm is operational, not
performance: one index copy to update, one place for metrics and queue
backpressure, and admission control. Those are real, but they are reasons to
centralise *coordination*, not *solving*.
