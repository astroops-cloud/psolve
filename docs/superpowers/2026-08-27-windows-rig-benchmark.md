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

**psolve solves 5.5x faster and fails 5x slower.** On the workstation a
`NO_QUAD_MATCH` costs 3.3 s; on the hardware that actually runs the observatory
it costs **ten seconds**, with the capture software waiting. Four frames
*solved* but took 17-19 s, so this is not only a failure-path problem: the
expensive rungs also fire on frames that eventually succeed.

That moves the pair-matching early abort
(`2026-08-25-astap-head-to-head.md`, "the fix is an early abort") from a
performance nicety to something with operational consequence.

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
