# Does index size matter, and should we load all of Gaia?

**Date:** 2026-08-25. Measured on this machine against the three indexes that
already exist here. All timings are **warm cache** -- see the caveat at the
foot, it is the one that would change the answer.

| index | records | size | mean/cell | max/cell |
|---|---|---|---|---|
| `g14-dec45` | 14,631,960 | 0.22 GB | 349 | 5,751 |
| `g16-dec45` | 68,832,757 | 1.03 GB | 1,642 | 57,250 |
| `g18` | 273,800,820 | 4.08 GB | 6,528 | 330,236 |

All three have the same 49,152 cells and ~41,940 occupied. **19x the records
from g14 to g18, identical cell geometry.**

## 1. Size barely affects query speed, by design

A 2.5 deg disc touches **47 of 49,152 cells** at every depth. Records are
brightest-first within a cell, so a query reads the head of each cell's run
and stops -- it never touches the faint tail that makes the deep index big.

`brightest_in_disc`, same disc, same limit:

| depth | limit 300 | limit 1500 | limit 5000 |
|---|---|---|---|
| g14 | 1.25 ms | 1.65 ms | 2.91 ms |
| g16 | 1.75 ms | 1.52 ms | 2.68 ms |
| g18 | 3.31 ms | 1.73 ms | 2.92 ms |

End-to-end, 20 frames that solve, all three indexes:

| index | solved | wall median | wall p90 |
|---|---|---|---|
| g14 (0.22 GB) | 20/20 | 57 ms | 165 ms |
| g16 (1.03 GB) | 20/20 | 58 ms | 166 ms |
| g18 (4.08 GB) | 20/20 | 61 ms | 180 ms |

**19x the data costs 7% of wall time and changes no answer.** This is the
brightest-first layout earning its keep; it is the single best thing about the
`.psidx` format.

## 2. A deeper index returns the *same stars* -- until the shallow one runs dry

Compared as **sets** (magnitude ties order differently between builds, which
is not a difference in content):

| field | limit | shallow has | deep has | in common | deep-only | faintest G |
|---|---|---|---|---|---|---|
| Orion r=2.5 | 1500 | 1500 | 1500 | **1500** | **0** | 12.21 |
| Carina r=2.5 | 5000 | 5000 | 5000 | **5000** | **0** | 11.43 |
| Carina r=1 | 300 | 300 | 300 | **300** | **0** | 10.12 |
| Orion r=0.25 | 300 | **165** | 300 | 165 | 135 | 14.99 |
| sparse north r=0.25 | 300 | **30** | 263 | 30 | 233 | 17.99 |
| Orion r=1 | 1500 | **787** | 1500 | 787 | 713 | 15.13 |

Every row where the shallow index had enough stars: **zero difference**.
Depth is a no-op for the fields this rig shoots.

It only bites when the disc runs dry -- a **small radius** or a **sparse
patch of sky**. Orion at r=0.25 deg holds 165 stars at G<=14 and 300+ at
G<=18.

This is consistent with the two measurements taken earlier: the 40 hardest
corpus failures solve 0/40 on g14 **and** 0/40 on g16 with the pre-change
binary. Those frames were never short of catalogue stars.

### A false alarm worth recording

The first comparison was position-by-position and reported the sets diverging
at index 129 of 300 in Carina -- alarming, since two indexes disagreeing
about the brightest stars in a field is how confident garbage is produced.
They were two stars **both at G = 9.273**, a magnitude tie ordered
differently by the two builds. Same set, same range (3.838..10.115). The
comparison was wrong, not the data.

## 3. So: load the whole of Gaia?

**Pro -- one real case.** Narrow fields and sparse sky. A long-focal-length
rig with a 0.25 deg field in a thin patch gets 30 usable catalogue stars from
g14 and 263 from g18. If that rig is ever pointed at this solver, depth is
the difference between solving and `INDEX_TOO_SHALLOW`.

**Con -- size, which is not a speed problem but is a storage one.** Gaia DR3
is ~1.81 billion sources. At this format's 16 bytes per record that is
**~29 GB** for the full catalogue, before dropping the declination cut the
existing indexes use. g18 alone is 4.08 GB.

**Con -- warm-cache measurements flatter it.** Everything above was measured
with the file already in the page cache. 4 GB fits comfortably; 29 GB will
not stay resident on a machine also running the imaging stack, so cold
queries would fault to disk. **This is the number I have not measured, and it
is the one that decides the question at full depth.**

**Con -- depth makes over-fetching easy, and over-fetching measurably hurts.**
`--cat-limit` defaults to 3x the frame's usable star count, clamped to
300..5000. On a deep index that same limit reaches much fainter: Orion at
r=0.25 deg, limit 1500, pulls 1,172 stars down to **G = 18.00**. Faint
catalogue stars with no detected counterpart lower completeness, and lower
completeness is what starves quad matching -- measured earlier in this
milestone, where raising `--cat-limit` made matching worse rather than
better.

**Con -- licence.** A built index is CC BY-NC 3.0 IGO
(`docs/data-licence.md`); the MIT code licence does not cover it. Bigger
index, same restriction, more of it.

## Recommendation

**Do not build a full-Gaia index. Keep depth matched to field size.**

- g14 for the current rigs. It is never short on these fields, and it is the
  cheapest thing to keep resident.
- g16/g18 only for a rig whose field genuinely runs the disc dry -- the test
  is "does `brightest_in_disc` return fewer stars than the limit asked for",
  which is cheap to check and is the honest trigger for going deeper.
- If depth is ever raised, **cap `--cat-limit` by magnitude, not just count**,
  so a deep index cannot quietly hand the solver mag-18 stars the frame could
  never have detected.

The last point is the actionable one and it is independent of ever building a
deeper index: the current limit is a count with no magnitude ceiling, so its
behaviour depends on which index it is pointed at.

---

## Addendum, 2026-08-26: which depth for a BLIND index

The above covers hinted solving. Blind additionally needs a `.psqidx`, and the
question of which star index to pair it with was raised by the AstroOps
deployment, where the operator's ruling is that users build their own indices
from a mounted catalogue rather than receiving one baked into an image.

**Building a quad index needs only the `.psidx`, never the source catalogue.**
`quad-index build --star-index <FILE>` is the sole interface; `process_tile`
reads star records from the opened index and `star_index_fingerprint` is taken
from it at build time -- which is exactly the value `QuadIndex::open` checks
later. There is no build-from-source variant for a locally-built pair to be
inferior to. That also means blind capability costs **zero catalogue bytes
moved** on any host that already has a star index.

### Cost

| | g14 pair | g16 pair |
|---|---|---|
| star index | 223.6 MB | 1,050.7 MB |
| quad index | **363.0 MB** | 427.9 MB |
| total | **~587 MB** | ~1,479 MB |
| build time (quad) | **63.2 s** | -- |
| quads | 15,859,821 | 18,692,947 |

The quad index barely shrinks with catalogue depth because `TILE_QUAD_CAP = 25`
caps it per sky tile rather than per star. **The saving is in the star index.**

### The build is deterministic ON a host, and NOT reproducible across hosts

Built independently on two machines from the same g14 `.psidx`:

```
host A  macOS 25.5.0, arm64    63.2 s   380,639,800 bytes   15,859,821 quads
host B  Linux, x86-64          66.8 s   380,639,800 bytes   15,859,821 quads
                        star_index_fingerprint 4a604280caff1d85 on both
                        per-band counts identical on both:
                        14502076 / 1022961 / 254321 / 63793 / 15553 / 1117
```

**The payload digests differ:**

```
host B  (two independent builds)  25c26ce5c495be5fd33c3589fb4ee925c71ff21906c4048475c9b9ab94ff0bca
host A                            05c5e25e342c4d81814a21b089367af1bfc1d26b063098c33e8431d185fa8127
```

So the property splits, and only one half holds:

- **Deterministic on a host: yes.** Two independent host B builds, 14 minutes
  apart to different paths, were byte-identical *including* the header.
- **Reproducible across hosts: no.**

Identical size, identical quad count and identical per-band counts with a
different digest means the quads differ in **value or order**, not in which
tiles were swept or how many survived each. The counts are stable because
`TILE_QUAD_CAP` is a count; what fills the slots is not.

> **Correction, 2026-08-26.** This section first claimed the build was
> deterministic across architectures, on the strength of matching size, quad
> count and fingerprint. **That was the weaker evidence read as the stronger
> claim.** Same size and same counts do not imply same bytes, and when the
> digests were compared they differed. Caught by the AstroOps session
> computing the digest I had asked for -- the negative result arrived because
> the check was run, not because either of us reasoned better.
>
> The mechanism is **not diagnosed**, but two candidates are now ruled out
> rather than assumed:
>
> - **Not thread scheduling.** `--jobs 4`, `17` and `18` on the same host all
>   produce the same digest as the default. The CLI documents "Deterministic
>   regardless of `--jobs`" and that claim holds under test.
> - **Not an unstable tie-break.** The per-seed sort breaks exact ties on
>   `a.idx.cmp(&b.idx)` -- integer star indices, deterministic by
>   construction.
>
> **The star data is confirmed identical across the two hosts**, so the
> divergence is created downstream of the catalogue. `psolve index query` on
> two discs -- dense Carina (8,787 rows) and sparse Orion (320) -- returned
> byte-identical output on macOS/arm64 and Linux/x86-64:
>
> ```
> disc A  rows 8787  sha256 84319a4a64a68757993c97cf0bf927fe...
> disc B  rows  320  sha256 2a5477724e7e99999a4bfaf2d3adf294...
> ```
>
> So the reader and the scaled-integer to float conversion are not involved,
> and **a future reader need not re-open them.**
>
> What remains is that the ordering key is a float. `conditioning_key` is
> computed from `quad_code`, derived from tangent-plane geometry, and a
> one-ULP difference is **not a tie** the integer tie-break can catch -- it is
> a definite but different ordering. Platform `libm` trig is the obvious
> suspect. **That is now well-supported inference rather than a guess, and it
> is still not a measurement**; instrumenting further was judged not worth it,
> because the operational answer does not depend on which call diverges.

**What this does and does not affect.** It does not touch correctness: both
artefacts solve, and the 33-frame comparison below stands untouched. A
per-host-deterministic index that solves correctly is a good index.

It does affect anything that **identifies an index by its bytes** --
distributing a prebuilt pair and verifying the download, or answering "was
this built from that catalogue at that magnitude limit" by hash. On this
evidence such a check must be per-host, or the digest must be taken over
something canonical rather than over the emitted order. Nothing is blocked
today, because indices are built by whoever runs the tool rather than
distributed; it is worth knowing before anyone builds a verification story on
the bytes.

### Does the shallower pair degrade quietly?

The concern worth testing was not the solve rate but **wrong answers**: blind
acceptance is multiplicity-corrected, so a shallower catalogue might buy a
confident coordinate rather than a refusal. Measured on frames with known
truth, all pointing and WCS cards blanked in place so blind is genuinely
exercised:

| | g14 pair | g16 pair |
|---|---|---|
| 18 corpus frames (solvable) | **18/18, 0 wrong** | 18/18, 0 wrong |
| 15 marginal frames (pair-rung only) | **0/15, 0 wrong** | 0/15, 0 wrong |

On the solvable set the two pairs returned the **same answer to a tenth of an
arcsecond on every frame**, and g14 was marginally faster. On the marginal set
neither depth solves -- blind is a differently-constrained tier, not a stronger
one -- and both **refuse rather than answer wrongly**.

**33 frames, zero wrong answers.** Not proof: 33 is a modest sample for an
absence claim about a rare failure. But there is no sign of the degradation,
and the gate held on exactly the frames where a weaker one would not have.

**Conclusion: g14 is a sound default for a blind index.** The coverage limit
from the body of this document still applies -- a narrow-field rig runs its
disc dry sooner at g14 -- but that announces itself as `INDEX_TOO_SHALLOW`,
not as a wrong answer.
