# Looking outside plate solvers: star trackers, and what AI would and would not buy

**Date:** 2026-08-24. Written after four detector/matcher changes took the
corpus from 90.12% to 98.82% and left **122 failures**, of which the hard core
is `NO_QUAD_MATCH` on frames where psolve detects too few of the catalogue
stars present.

The measured cause is now precise: **a quad matches only if all four of its
stars survive on both sides, so the matchable fraction goes as
completeness^4** -- 42.4% completeness gives 3.2%, 63.1% gives 15.9%
(`2026-08-24-atr585m-diagnostic.md`). Everything tried so far has attacked
*completeness*. This looks at attacking the **exponent** instead.

## 1. Star trackers have been solving this problem harder than plate solvers

Spacecraft attitude determination is the same problem under worse constraints:
identify an unknown star field, in real time, on radiation-hardened hardware,
with no pointing prior. The literature calls it **lost-in-space**, and its
state of the art is the **Pyramid** algorithm (Mortari et al.).

**It is architecturally different from quad hashing, in exactly the way that
matters here.**

| | psolve / the quad-hash family | Pyramid |
|---|---|---|
| primitive | a 4-star **hash code** | a **pairwise interstar angle** |
| candidate pool | quads from each star's ~6 nearest neighbours, capped at 600 | in principle every star **pair**, `n(n-1)/2` |
| locality | quads are **local** by construction | pairs are **global** -- any two stars |
| identification | one quad code matches one catalogue quad | find a **unique triangle**, then confirm with a **fourth** star `r` such that `{i,j,r}`, `{i,k,r}`, `{j,k,r}` are all unique |
| designed for | rich, complete star lists | **spikes** -- false stars -- explicitly |

The consequence for psolve's failure mode: a quad dies if **any one** of four
specific, mutually-neighbouring stars is missing. Pyramid never depends on a
specific configuration surviving -- it asks whether *any* triangle among all
pairs resolves uniquely, and the pool of candidate triangles is
combinatorially larger and not restricted to local neighbourhoods.

That is the difference between "the four stars I chose must all be there" and
"three of the however-many stars I have must agree". On a frame where
completeness is 42%, the first is a losing proposition and the second is not
obviously so.

**The k-vector** is the enabling data structure: a searchless range lookup
that returns every catalogue pair whose interstar angle matches a measured one
within tolerance, in constant time, without a search. psolve's `.psqidx`
quantile grid is a cousin of this idea applied to quad codes; the k-vector
applies it to a **1-dimensional** quantity (an angle), which is much cheaper
to index and much more robust to one star being absent.

### Honest assessment

This is **not an incremental fix**. It is a different matching stage --
different index format, different search, different verification. Against
that: it is a mature, published, heavily-benchmarked approach designed for
precisely the regime psolve is failing in, and psolve already has the pieces
that make it feasible (a HEALPix-indexed catalogue, a tangent-plane
projection, a verification gate).

It is the only candidate found that attacks the **exponent** rather than the
base.

## 2. What AI would buy, and what it would cost

Deep-learning source detection is real and measurably better than classical
thresholding at low SNR. DEEPSOURCE reports essentially perfect purity and
completeness down to **SNR 4** and beats classical extractors on all metrics;
ConvoSource outperforms Gaussian-fitting at low SNR. U-Net and Mask R-CNN
segmentation are the common architectures.

**Relevant, because detection is exactly where psolve loses.** Completeness on
the failing frames is ~28% after the matched filter, against 62-69% where it
solves. A detector that works at SNR 4 would attack that directly.

**But it would cost psolve the property that makes it what it is.**
`psolve-core` has **zero dependencies, not even dev-dependencies**, enforced by
a token scan that fails the build. A learned detector means a runtime (ONNX,
tract, candle, or hand-rolled inference), a weights file of tens to hundreds of
MB shipped beside a 0.23 GB index, and a model whose behaviour on this rig's
frames is only as good as its training set. The "one static binary, no runtime"
claim in the README would stop being true.

There is also a subtler cost this project has already paid once. A learned
detector is not inspectable in the way `rejected.too_small` is: psolve's
failure diagnosis today comes from **counts by reason**, and "the network
scored it 0.3" replaces that with something nobody can act on. The whole value
proposition in the README is *able to say why a frame did not solve*.

**Verdict: not now, and probably not here.** The gain is real but it is
available more cheaply -- the matched filter landed today captured part of it
for zero dependencies -- and the cost lands on the properties this project is
built around. If a learned detector were ever wanted, the honest shape is a
separate optional binary that emits a star list psolve consumes, not a model
inside `psolve-core`.

Worth noting one narrow AI use that costs nothing: **using a learned detector
offline to generate ground-truth star lists** for tuning the classical one.
That is a measurement tool, not a runtime dependency.

## 3. What is not worth chasing

- **A better PSF model / fitting photometry.** The failures are not
  mis-measured stars; they are stars never detected. Refining measurement of
  what is already found does not move completeness.
- **A deeper catalogue.** Tested and refuted: G<=16 with 14,719 available
  stars still fails, and raising `--cat-limit` makes matching worse by
  lowering completeness.
- **GPU acceleration.** Solve time is 70 ms median and not a bottleneck for
  any consumer here. Speed is not the problem; the 122 failures are.

## Recommendation, in order

1. **Prototype pair-angle matching with a k-vector index**, on the 122
   corpus failures, out-of-tree. It is the only approach found that changes
   the completeness exponent, and the question it answers is cheap to ask:
   does a triangle-from-pairs search resolve frames a quad search cannot?
2. **The cross-shaped shape test** from the aperture-SNR experiment -- a dozen
   lines, worth about a dozen frames, and independent of 1.
3. **Leave AI alone** unless the dependency budget changes, and if it ever
   does, as a separate binary rather than inside the core.

## Sources

- Mortari et al., *The Pyramid Star Identification Technique*, NAVIGATION,
  <https://onlinelibrary.wiley.com/doi/pdf/10.1002/j.2161-4296.2004.tb00349.x>
- *Lost-in-Space Pyramid Algorithm for Robust Star Pattern Recognition*,
  <https://www.researchgate.net/publication/254199748_Lost-in-Space_Pyramid_Algorithm_for_Robust_Star_Pattern_Recognition>
- *A Survey of Lost-in-Space Star Identification Algorithms Since 2009*,
  <https://www.semanticscholar.org/paper/A-Survey-of-Lost-in-Space-Star-Identification-Since-Rijlaarsdam-Yous/a9bd71ff181887fcc938095d956c0b923cd72a84>
- *Geometric voting algorithm for star trackers*,
  <https://www.researchgate.net/publication/3007679_Geometric_voting_algorithm_for_star_trackers>
- *Detection and Classification of Astronomical Targets with Deep Neural
  Networks in Wide-field Small Aperture Telescopes*,
  <https://iopscience.iop.org/article/10.3847/1538-3881/ab800a>
- *Semi-supervised Source Detection in Astronomical Images*,
  <https://arxiv.org/pdf/2606.09219>
- *A review of source detection approaches in astronomical images*, MNRAS,
  <https://academic.oup.com/mnras/article/422/2/1674/1040345>
