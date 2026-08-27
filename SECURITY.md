# Security

## Why this file exists

`psolve` reads **untrusted input by design**. A FITS file is arbitrary bytes
from a camera, a mount, an archive, or a stranger, and `crates/psolve-core/src/fits.rs`
is written so that no malformed header, truncated data unit or hostile
dimension field can panic the process. That is a security property, so it can
have security bugs.

It also *writes*, under `-update`, into files that are often irreplaceable
observations.

## In scope

- **The FITS reader** (`psolve-core`): panics, out-of-bounds reads, unbounded
  allocation, or non-termination on any input file. A crash on a malformed
  frame is a bug here even without a path to code execution, because the
  documented contract is that it never panics.
- **The index readers** (`psolve-index`): `.psidx` and `.psqidx` are mmap'd and
  cast to fixed-size records. A malformed or truncated index that produces
  out-of-bounds access is in scope.
- **`-update` and sidecar writing** (`psolve-cli`): any path by which psolve
  writes outside the file it was pointed at, writes when `PSOLVE_READONLY` is
  set, writes past a `.psolve-readonly` marker, or leaves a partially written
  frame behind. The safety model is documented in
  `docs/astap-compat.md#the--update-safety-model`.

## Not in scope

- **An index you built yourself is data you trust.** psolve validates enough to
  refuse an index that is not one (each format's reader rejects the other's
  file), but it does not defend against an index crafted by an attacker who
  already has write access to your data directory.
- **A wrong solve is not a vulnerability.** A confident wrong answer is a
  serious defect and is treated as one -- please file it as a normal issue with
  the frame -- but it is a correctness bug, not a security boundary being
  crossed.
- Anything requiring an attacker who can already write to your filesystem or
  run code as your user.

## Reporting

Use GitHub's **private vulnerability reporting** on this repository
(Security -> Report a vulnerability). That keeps the report private until
there is something to release.

Please include the input file if you can share it, the exact command line, and
what you expected instead. A crashing FITS file is the single most useful
attachment; if it contains observations you would rather not publish, say so
and send only the header.

## What to expect

This is a one-maintainer project, so the honest answer is: no guaranteed
response time, and no bug bounty. What you will get is an acknowledgement that
the report was received and read, and a public fix with the finding recorded
rather than quietly patched.
