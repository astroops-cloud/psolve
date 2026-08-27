# psolve is more sensitive to an oversized search radius than ASTAP is

**Date:** 2026-08-26. Measured on real cloud-degraded frames staged by the
AstroOps session during a live centring failure. This is a finding **against**
psolve, and the scope matters more than the headline.

## The measurement

One frame, ATP585M on an SV555 through arriving cloud: 91 stars by N.I.N.A.'s
own count, 2.453"/px, field 2.616 x 1.472 deg, **half-diagonal 1.501 deg**.
Only the search radius varies.

| radius | psolve | ASTAP, same frame |
|---|---|---|
| **10.0 deg** (N.I.N.A.'s `SearchRadius`) | **NO_QUAD_MATCH** | solved, 156 ms |
| 5.0 deg | **NO_QUAD_MATCH** | -- |
| 2.5 deg | solved, log-odds 126.7, 53 matched | -- |
| 1.65 deg (psolve's default, 1.1x half-diag) | solved, log-odds 290.3, 108 matched | -- |
| 0.75 deg (0.5x half-diag) | solved, **log-odds 356.9, 140 matched** | solved, 127 ms |
| 0.40 deg | solved, log-odds 131.3, 51 matched | -- |
| 2.0 deg | -- | solved, 133 ms |

A clear optimum near 0.5x the half-diagonal, falling away in both directions,
and **failure outright above about 2.5 deg**. ASTAP solves the same frame at
every radius tried and its timings barely move.

The control frame (1306 stars, clear night) solves for both tools at every
radius, so this is not the harness.

## Why

The same mechanism as the tight-radius retry
(`2026-08-25-cross-frame-priors.md`), read backwards. A disc sized for
pointing error is larger than the frame; at 10 deg it is **44x the frame's
area**, and every catalogue star out there has no possible image counterpart.
Each one lowers completeness, and a quad matches only when all four of its
stars survive on both sides -- so the matchable fraction falls as
completeness to the fourth power. On a frame already starved of stars by
cloud, that is the difference between solving and not.

ASTAP's spiral search evidently does not degrade the same way. **This
document does not explain why**, because that would be a claim about
ASTAP's internals inferred from psolve's -- which is exactly the error the
next section records.

## Scope: the drop-in path is safe, the native flag is not

This does **not** affect ASTAP-compatibility mode, and the reason is a design
decision made earlier for a different purpose. `search_radius_deg` treats
`-r` as a **ceiling over the header-derived radius**, not as the radius:

```rust
base.min(a.radius_deg)      // base = header geometry
```

Verified end to end on the same frame: `psolve -f frame.fits -r 10 -fov 1.477
-ra ... -spd ...` exits **0** with `PLTSOLVD=T`. `min(1.651, 10)` uses the
header's own geometry and ignores the 10. Native mode with an explicit
`--radius 10` fails, and that is a caller assertion the CLI documents as
overriding its own judgement.

So a stock N.I.N.A. invocation -- `SearchRadius 10`, which it sends every
night -- is safe against this. That ceiling is now pinned by
`a_wide_r_never_widens_the_disc_beyond_the_header_geometry`, which tests the
opposite direction to the existing narrow-`-r` test and is mutation-checked:
treating `-r` as the radius fails it.

## The recommendation this nearly produced

On the psolve numbers alone the conclusion looked obvious and urgent --
`SearchRadius 10` is six times psolve's default and thirteen times its
optimum, so drop it in the N.I.N.A. profile tonight and recover the frames
the rig is failing to centre on.

**Then ASTAP was measured at the same radii and does not care.** Changing
that setting would have bought nothing, and it would have been a
mid-session change to a working rig on a clear night.

The inference was drawn from psolve's architecture and applied to a tool that
does not share it. Both are plate solvers taking a radius argument; that is
not enough to make one's sensitivity curve evidence about the other's. The
guard is cheap and was skipped: **measure the other tool before recommending
a change to it.**

## What this does not say

The frames actually failing in that live incident are **5 s L** centring
exposures, which were unavailable -- N.I.N.A. deletes them after each attempt.
These are 120 s narrowband from the same sky and hours. ASTAP's reported error
on the real frames is *"Not enough stars"*, the same failure as the 3-star
frame in this set, which **neither** solver solves at any radius or extraction
setting. Nothing here suggests a solver change would have helped that
incident; the lever is exposure time.
