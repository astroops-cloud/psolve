# psolve on Windows, on the capture machine, against ASTAP

**Date:** 2026-08-27. **The first time psolve has run on Windows at all**, and
the first head-to-head where both tools ran on the *same machine, same frames,
same disk, minutes apart*. Every previous comparison in this project ran the
two tools on different hardware or compared against stored database rows.

Run by Claude Code over SSH. No human has still used psolve on Windows
interactively.

## The machines, by role

Timings are only interpretable with these, and the third one reframes
everything below.

| role | CPU | cores | RAM | OS |
|---|---|---|---|---|
| workstation (all prior measurements) | Apple M5 Max | 18 | 128 GB | macOS 26.5.2, arm64 |
| linux host | AMD Ryzen 9 9950X3D | 16C / 32T | 91 GB | Arch Linux, x86_64 |
| **capture host** | **Intel i3-8109U @ 3.0 GHz** | **2C / 4T** | 15.9 GB | Windows 11 Pro 26200, AMD64 |

**The capture machine is a 2018 dual-core i3.** psolve's numbers here are not
comparable to the 50.4 ms corpus median taken on an 18-core M5 Max -- that gap
is silicon, not Windows. The head-to-head *is* fair, because both binaries ran
on that same i3.

## Method

75 LIGHT frames, stratified 4 per session across 22 sessions so one large
target could not dominate. Frames copied into a scratch directory first:
`astap_cli` writes its `.ini` beside the frame it solves, and the capture tree
is not ours to write into. Neither tool given `-update`.

- psolve: the released `v0.1.0` Windows binary, verified against the published
  `SHA256SUMS`, with the all-sky G<=14 index -- whose SHA256 computed **on
  Windows** also matched the published checksum, incidentally validating the
  release digests from a third machine and a different OS.
- `astap_cli`: the rig's own installation, its own local `d50` database.

Separations are computed from the exported coordinate pairs by a second
implementation, **not** by the benchmark script -- see "what went wrong" below.

## Result

| | psolve | ASTAP |
|---|---:|---:|
| **all 75 frames** | 42 (56%) | **60 (80%)** |
| **science frames** (n=45) | **42 (93%)** | 34 (76%) |
| **probe frames** (n=30) | **0 (0%)** | 26 (87%) |

**The flat number is a trap and this document will not lead with it.** Read
across all 75 frames psolve loses badly. Split by what the frames actually are,
it wins on real imaging and loses *every single* pointing probe. Same data,
opposite conclusions, and the flat number is the one that would have been
quoted.

Probe frames are 15-second pointing checks at arbitrary alt/az. They are a real
part of the workload -- the mount uses them -- but they are not imaging, and a
sample that is 40% probes describes a night that never happened.

Where both solved (n=34): **median separation 0.698"**, p90 1.584", max 7.781".

### Timing

| | psolve | ASTAP |
|---|---:|---:|
| median, solved | **330 ms** | 1,814 ms |
| max, solved | 18,742 ms | 13,049 ms |
| median, failed | **10,001 ms** | ~2,000 ms |

**psolve solves 5.5x faster and fails 5x slower.** The failure cost scales
with the CPU and has now been measured three times on the same rung: **3.3 s**
on the workstation at the time of the corpus run, **2.2 s** on the workstation
today against these frames, and **10.0 s** on the capture host. It is the last
figure that matters operationally, because that is the machine with capture
software waiting on it. Four frames
*solved* but took 17-19 s, so this is not only a failure-path problem: the
expensive rungs also fire on frames that eventually succeed.

That moves the pair-matching early abort
(`2026-08-25-astap-head-to-head.md`, "the fix is an early abort") from a
performance nicety to something with operational consequence.

## The operational consequence: it is the failure cost, not the solve rate

The solve rate is the interesting number. **The failure cost is the one an
operator would feel**, and it was sitting in the data above unmultiplied until
a review pointed at it.

Measured, from this run's own rows:

| | psolve | ASTAP |
|---|---:|---:|
| the 30 probe frames, total wall | **299.7 s (5.0 min)** | 117.3 s |
| ...outcome | **0 solved** | 26 solved |
| the 45 science frames, psolve total | 71.0 s for 42 solves | — |

**psolve spends five minutes failing to solve the probe frames that ASTAP
handles in under two, and solves 42 science frames in seventy seconds.**

Scale matters here and the earlier framing of this was wrong in a way worth
correcting: a **single pointing run fires 3 probes**, so putting psolve first
costs about **30 seconds** of dead time per run. The 30 probes measured here
are not a sample -- they are the *complete* 2026-08-24 pointing-model build, 10
runs of 3. So a full pointing-model build costs **5 minutes** of blocked
sequence before ASTAP is reached at all.

### Which implies invocation order is a decision, not a preference

- **psolve first:** ~30 s wasted per pointing run, ~5 min per model build, and
  ASTAP solves them afterwards regardless.
- **ASTAP first:** costs nothing extra on probes, and psolve still catches the
  science frames ASTAP parks -- which is the 93%-vs-76% result, and the actual
  case for running psolve at all.

That decision is the operator's, not this document's. What the document can say
is that **0 of 30 is a capability gap, not a tuning problem**: a solver that
handles 93% of science frames and 0% of probes is failing on a *property of
those frames* -- too few detected stars in a 15-second exposure -- not on
difficulty in general.

Both readings point at the same fix, and it is not the matcher. The early abort
would cut the wasted time roughly tenfold; the completeness work in
`extract.rs` would remove the failures themselves.

## Control: are the probe failures a Windows problem?

Asked immediately, because "psolve fails every probe frame on Windows" invites
exactly one wrong reading, and the benchmark as first written did not exclude
it. The 30 probe frames were copied back to the workstation and both tools
re-run on them there, same frames, same index.

| | Windows (i3, 2C) | macOS (M5 Max, 18C) |
|---|---|---|
| **psolve** | 0/30, `NO_QUAD_MATCH` ×30 | **0/30, `NO_QUAD_MATCH` ×30** |
| **ASTAP** | 26/30 | **26/30** |
| psolve wall | 300 s | 65 s |
| ASTAP wall | 117 s | 71 s |

**Identical outcomes on both platforms** -- not similar, identical, down to the
reason code and the count, for both tools. Only wall time differs, by roughly
the ratio of the two CPUs.

**So there is no Windows defect.** psolve solves these frames exactly as well
on Windows as on an 18-core workstation, which is to say not at all. The
failure is a property of psolve and of the frames, and Windows is exonerated as
a variable.

That makes this the strongest evidence yet for the completeness diagnosis: the
same 30 frames, two operating systems, two CPU architectures, two builds from
different toolchains (msvc and Apple clang), and the same 0/30 with the same
reason code -- against a reference tool that gets 26/30 on both. A
platform-specific bug could not produce that symmetry.

It is also a caution about the benchmark's own framing. Measured on Windows
alone, "0 of 30" reads as a Windows result. It is not one, and only running the
control could establish that.

## What this confirms, independently

psolve failing **0 of 30** probe frames is the ATR585M completeness problem
reproducing on a different OS, a different CPU and a different index build from
the macOS measurements. `2026-08-24-atr585m-diagnostic.md` diagnosed it as
stars not detected; `2026-08-27-per-rig-remeasure.md` measured 34% on that
rig's hard frames against ~67% elsewhere. This is the same defect seen a third
way.

It also confirms the inverse: on frames with enough stars, psolve is both more
successful (93% vs 76%) and much faster than ASTAP on the same hardware.

## A Windows-only integration hazard, in the caller not in psolve

PowerShell's `$ErrorActionPreference = "Stop"` treats **any** native-command
write to stderr as a *terminating* error. psolve writes its progress line
(`solving <path>: N catalogue stars within R deg of RA,DEC`) to stderr, keeping
stdout pure JSON so it can be piped -- correct behaviour that makes a strict
PowerShell script abort on the first frame.

This cannot happen on Linux or macOS, CI cannot see it because CI does not
drive psolve from PowerShell, and it will meet the first Windows user who
scripts around psolve. The fix is one line in the *caller*:

```powershell
$ErrorActionPreference = "Continue"   # psolve's progress goes to stderr
```

## What went wrong in the instrument, recorded because it nearly published

**1. The separation column read `0"` for every frame.** That is "two
independent solvers agree exactly", an extraordinary claim, and it was a bug in
the benchmark's own PowerShell arithmetic. Real median: 0.698". The column was
removed rather than repaired -- the CSV carries both coordinate pairs and the
caller computes it where it can be checked.

**2. The first sample was 45% probe frames**, which is what surfaced the
probe/science split in the first place.

**3. The completion check could not fail.** The wait loop polled for
`bench.csv` *existing* -- and a stale copy from the dry run already did, so it
reported "complete" at frame 37. The dry run is the one with the broken
separation column and the skewed sample; it came close to being documented as
the real result. Poll something that can change.

## Reproducing

`scripts/bench-windows.ps1 <frames-per-session>` on a Windows host with
`astap_cli` installed, psolve and an index staged under
`C:\Users\<user>\psolve-test`. It writes `out\bench.csv`; compute separations
from the coordinate columns yourself.
