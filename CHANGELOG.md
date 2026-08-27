# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2026-08-27

First public release. Public history begins here; the development history that
produced it is retained privately, which is why short SHAs cited in `docs/` do
not resolve in this repository.

**Provenance:** effectively none of this code was written by a human. It was
built over 14 days with Claude Code, directed and reviewed by one person.
[`docs/how-this-was-built.md`](docs/how-this-was-built.md) is the full account,
including what the approach cost.

### Added

- **A plate solver.** FITS bytes in, a verified TAN WCS out. One static binary,
  no runtime, built for headless automation rather than a GUI with a CLI
  attached.
- **`astap_cli` compatibility.** The single-dash argument grammar (`-f`, `-r`,
  `-fov`, `-ra`, `-spd`, `-d`, `-update`), both `.ini` sidecar formats, both
  `.wcs` formats, and the exit-code scheme -- so anything already shelling out
  to `astap_cli` can point at `psolve` with no change on its side. `-ra` is
  hours and `-spd` is south polar distance, confirmed against real recorded
  invocations rather than inferred. See [`docs/astap-compat.md`](docs/astap-compat.md).
- **Blind solving** against a `.psqidx` quad index, with a multiplicity-corrected
  confidence gate. The gate is the point: applying the single-hypothesis
  threshold to a blind search once produced a confident solve 87.77 degrees
  from the truth.
- **Reason codes** for every refusal, so a frame that does not solve says why
  rather than just failing.
- **A four-rung retry ladder** -- header scale/binning with catalogue refetch,
  matched-filter re-extraction, pair matching, tight search radius -- ordered so
  that a frame which solves today does not change its answer or even its route.
- **`psolve index build`** from a Gaia DR3 mirror
  ([`docs/index-building.md`](docs/index-building.md)), and a **prebuilt
  all-sky G≤14 index attached to this release** -- `.psidx` (257 MB) plus the
  paired `.psqidx` (428 MB) for blind solving, verified against the published
  `SHA256SUMS`. **The index files are not MIT**: derived from Gaia DR3, they
  are CC BY-NC 3.0 IGO -- non-commercial, attribution required
  ([`docs/data-licence.md`](docs/data-licence.md), and `INDEX-LICENCE.txt`
  ships alongside them). Narrow fields need a deeper index you build yourself;
  [which depth do you need](docs/index-building.md#which-depth-do-you-need-start-here)
  answers that from your field of view.
- **`scripts/demo.sh`** -- a complete solve on synthetic data with no index, no
  download and no network.
- **An `-update` safety model**: default off, two independent read-only
  switches, a full temp copy verified byte-identical before the rename. It
  exists because a header rewrite that shifted the data unit once corrupted
  four archive frames.

### Measured

The numbers behind the claims above, with the runs that produced them, are in
[`README.md`](README.md) and `docs/`. Two worth stating here:

- On 184 frames the deployment's ASTAP had **parked in production**, psolve
  recovers 72 (39.1%) in 369 s total.
- Over the full 10,376-frame corpus ASTAP had solved, psolve solves 99.93% with
  a 0.54" median centre separation from ASTAP's own recorded answer.

Both are against one observatory's data on one machine. They are what was
measured, not a claim about every rig.

### Known limits

- Fits a TAN WCS with no distortion terms, and `-sip` is accepted but
  discarded. Measured 2026-08-27 across 74 frames from three cameras: every one
  shows a positive residual-vs-radius correlation, and one camera shows it on
  27 of 27 frames -- so this is a real gap rather than a theoretical one. It
  affects the *accuracy* of solves that already succeed, not whether they
  succeed. See
  [`docs/superpowers/2026-08-27-distortion-signal.md`](docs/superpowers/2026-08-27-distortion-signal.md).
- **Windows is tested by machine, not by a person.** CI builds it natively and
  runs 620 of the 633 tests run, plus the end-to-end demo, and the released
  `.exe` is executed before upload. On 2026-08-27 it was also benchmarked on a
  real observatory capture machine against `astap_cli`: **93% vs 76% on science
  frames, and 0 of 30 on 15-second pointing probes**. A control re-ran those 30
  frames on macOS and got 0 of 30 again with the same reason code, against
  ASTAP's 26 on both platforms -- so that gap is psolve and those frames, not
  Windows
  ([benchmark](docs/superpowers/2026-08-27-windows-rig-benchmark.md)). psolve
  solved 5.5x faster there and failed 5x slower. **No human has used psolve on
  Windows interactively.** Note that Windows is not a peripheral target: the capture
  machine on the telescope runs it, and ASTAP-compatible mode exists so that
  N.I.N.A. can call psolve there mid-sequence. **On Windows the `-update`
  safety model is also weaker** -- `fits_update::same_directory` returns `None`
  unconditionally, so one of the three `.psolve-readonly` ancestor chains is
  unavailable. The canonical chain and `PSOLVE_READONLY` both still work.
- Four test files skip themselves without real telescope data, so a green CI run
  proves less than it looks. See [`CONTRIBUTING.md`](CONTRIBUTING.md).
- No MSRV is declared; it has not been measured.
- **A hardware-binned colour frame is binned again in software.** `decode()`
  superpixel-bins any frame carrying `BAYERPAT` 2x2, regardless of `XBINNING`.
  A camera that already binned 2x2 has summed one whole Bayer unit into each
  output pixel, so binning again halves the resolution a second time for
  nothing. Such frames still solve -- the retry ladder recovers the correct
  scale -- but at a coarser plate scale than the sensor delivered.

  The obvious fix (require `XBINNING <= 1` before superpixel-binning) is
  written and **deliberately not merged**: measured 2026-08-15 over 791 real
  2x2-binned colour frames it regressed 184 and newly solved 54, because
  `extract.rs`'s fixed `min_pix = 4` threshold had implicitly tuned itself
  around the too-coarse scale this defect produces. It is a two-part fix, not
  a one-liner, and the measurement predates the binning retry that now solves
  790 of those 791 -- so it needs re-measuring, not just merging.

[0.1.0]: https://github.com/astroops-cloud/psolve/releases/tag/v0.1.0
