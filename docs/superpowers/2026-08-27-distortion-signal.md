# Does this data want a distortion model?

**Date:** 2026-08-27. Measured before designing anything, because "should we
build SIP support" is answerable from frames already on disk and was about to
be answered from intuition instead.

## The instrument

`fit.rs` already emits the diagnostic. `FitResult::radial_trend` is the
correlation between a star's fit residual and its radius from the field centre,
and the field's own doc comment states the reading:

> Near zero means TAN is sufficient; a systematic positive value is the data
> asking for a distortion model.

So the question needs no new code, only a sample.

## Method

74 frames, sampled at a fixed stride across each camera's holdings so the
selection is not clustered in one target or one night. Release build,
`gaia-dr3-g14-dec45-nside64.psidx`, default flags, header-hinted. Only solved
frames counted. `signif` is the median divided by the standard error of the
mean -- a crude check that the median is displaced from zero by more than the
scatter, not a formal test.

## Result

| camera | n | rms″ median | `radial_trend` median | p90 | positive | signif |
|---|---:|---:|---:|---:|---:|---:|
| ATR585M (primary rig) | 22 | 0.556 | **+0.139** | 0.257 | 15/22 | 2.9 |
| SVBONY SV405CC | 25 | 2.973 | **+0.095** | 0.179 | 19/25 | 4.6 |
| DWARFIII | 27 | 1.919 | **+0.338** | 0.434 | **27/27** | 15.6 |

**Every camera shows a positive radial trend, and DWARFIII shows it on every
single frame.** By the field's own criterion this data is asking for a
distortion model. The DWARFIII result is the strongest and is physically
plausible: it is the smallest, widest-field optic of the three.

## What this does NOT establish

Three things, and they are why this document does not conclude "build it now".

**It does not size the win.** `radial_trend` is a *correlation*, not an
amplitude. A correlation of +0.34 says residuals grow with radius; it does not
say by how much, nor how much of the 1.9″ rms a radial term would remove. The
honest next measurement is a held-out one: fit with and without the extra
terms and compare residuals **on stars excluded from the fit**, because adding
free parameters always lowers residuals on the stars they were fitted to. That
is overfitting, and it looks exactly like success.

**It does not touch the solve rate.** Every frame in this table already
solved. Residuals of 0.56″ sit far inside the 30″ bar the agreement run uses.
Distortion would make good solves more accurate; there is no evidence here
that it recovers a frame that currently fails.

**It does not bear on the ATR585M regression.** That was diagnosed
(`2026-08-24-atr585m-diagnostic.md`) as a completeness⁴ problem -- stars not
detected. Distortion is about where detected stars land. Different failure,
and the primary rig has the *weakest* radial signal of the three.

## Recommendation

**Distortion is worth building, and is not urgent.** It is an accuracy
refinement on frames that already solve, with a real and repeatable signal
behind it. It belongs as its own milestone, whose acceptance criterion is the
held-out comparison above -- not a lower rms on the fitted stars.

Scope, when it happens: an N-parameter normal-equation solve (`fit.rs` already
has `solve3` by Gaussian elimination and needs the general form, staying inside
`psolve-core`'s zero-dependency budget), `A_p_q`/`B_p_q` output in the WCS and
both sidecar formats, and honouring `-sip` instead of accepting-then-discarding
it. **Opt-in**, because turning it on unconditionally would change the answer
for frames that solve today, which the retry ladder's rule forbids.
