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

## Follow-up: is the residual field the same shape frame to frame?

That question decides the architecture, so it was measured rather than assumed.
If the field repeats it is a property of the optics and can be calibrated once
and reused; if it varies it can only be fitted per frame, as ASTAP,
astrometry.net and Siril all do.

**Instrument, deliberately independent of `fit.rs`.** Take only psolve's WCS,
project Gaia through it to predicted pixel positions, and centroid the real
flux near each. The residual vector is measured minus predicted. This never
asks psolve what its own residuals were -- `radial_trend` is what raised the
question, so verifying it with itself would prove nothing. Validated first on
one frame: 1,619 matched stars, predictions landing 0.46 px from real flux,
which a wrong projection could not produce.

Residuals binned 4x4 across the frame, then every pair of frames correlated.

| sample | n | mean \|resid\| | repeatable | scatter | S/N | pattern correlation |
|---|---:|---:|---:|---:|---:|---:|
| ATR585M, mixed sessions | 9 | 0.659 px | 0.165 | 0.350 | 0.47 | **+0.398** |
| ATR585M, one session | 8 | **0.245 px** | 0.075 | 0.033 | **2.27** | **+0.871** |
| DWARFIII, mixed sessions | 8 | 1.094 px | 0.301 | 0.300 | 1.00 | **+0.760** |

**The field is real and repeatable, and the variation is dominated by what
happens BETWEEN sessions rather than between frames.** Within one night the
pattern correlates at +0.871 with signal 2.3x the scatter. Mixing sessions
drops that to +0.398 and nearly triples the apparent residual. On the first
9-frame sample every one of the 36 frame pairs correlated positively -- not one
disagreed about the shape -- which is what distinguishes a weak signal from no
signal.

### What that means for the design

A per-frame polynomial fitted from one frame's stars would be fitting 0.35 px
of scatter to find 0.165 px of optics. Averaging frames is what separates them,
and a single night is already enough (S/N 2.27).

So: **calibrate per optical configuration, not per frame and not once
forever.** The refresh trigger is a session or any change to the optical train.
`radial_trend` is already emitted per solve and is the natural staleness
detector -- if it climbs above its calibrated baseline, the calibration no
longer describes the optics.

This also sidesteps the retry ladder's rule. A stored correction adds no
per-frame free parameters, so it cannot quietly improve residuals by
overfitting; it is either right or wrong and that is testable against held-out
frames.

### How much is actually available, in arcseconds

At this rig's 2.454"/px:

| | px | arcsec |
|---|---:|---:|
| ATR585M within-session residual | 0.245 | 0.601" |
| its repeatable, correctable part | 0.075 | **0.184"** |
| DWARFIII repeatable part | 0.301 | **0.739"** |
| psolve/ASTAP agreement bar | | 30" |
| current corpus median separation | | 0.54" |

**This is an accuracy improvement, not a solve-rate one, and the honest
comparison is against 0.54" rather than 30".** The correctable systematic is
roughly a third to a half of psolve's remaining median disagreement with ASTAP
-- worth having for anyone doing photometry or astrometry on the output, worth
nothing for deciding whether a frame solves. Every frame in these samples
already solved.

### What the literature says, checked 2026-08-27

- ASTAP uses SIP to 3rd order and states it needs distortion to be "reasonably
  symmetric" (hnsky.org/sip.htm). Note that this deployment's own ASTAP
  sidecars carry no SIP keywords at all -- AstroOps does not pass `-sip`.
- astrometry.net defaults to SIP **order 2** and warns explicitly about
  overfitting with limited matches.
- Siril fits SIP per image **and** can save a distortion file for reuse across
  a sequence -- the closest existing tool to the design above.
- SIP models distortion in **detector** coordinates (static with respect to its
  physical cause); TPV models it in intermediate world coordinates and is
  "typically better suited to ground-based observations". The SIP-to-PV paper
  states the principle this measurement confirms: *characterise detector
  distortion separately from WCS fitting, since not all exposures contain
  sufficient reference stars.*
- MNRAS 502, 6216 compared a Gaia-referenced distortion solution against a
  self-referenced one: comparable precision, but the catalogue-referenced
  solution was **more stable across epochs** and more practical for
  fixed-orientation ground-based telescopes.
- LSST's DMTN-010 notes the whole FITS-WCS polynomial family "severely limits
  the ability to describe complex distortions" and recommends emitting SIP/TPV
  only for legacy compatibility. psolve must emit SIP regardless, because
  ASTAP compatibility is a core property.
