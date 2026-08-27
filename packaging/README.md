# Packaging psolve

What ships, on which platform, and what is verified. Written 2026-08-23.

The short version: **Linux is built and tested, Windows is built and untested,
macOS is neither yet.** That asymmetry is not laziness -- it falls directly out
of the CI available when it was written: one runner, Linux containers only.

| platform | artifact | built by | executed by CI? |
|---|---|---|---|
| Linux amd64 | `.deb` | `deb` job, natively | **yes** -- installs its own artifact and runs it |
| Windows x86_64 | `.zip` of `psolve.exe` | `windows` job, cross-compiled with mingw-w64 | **no** -- see below |
| macOS arm64 | Homebrew formula | nothing yet | no |

## Linux

`cargo deb` from `[package.metadata.deb]` in `crates/psolve-cli/Cargo.toml`.
The job `dpkg -i`s the package it just built and runs the installed binary, so
it cannot ship a `.deb` that only `dpkg-deb` has ever opened.

## Windows -- built, NOT executed

Cross-compiled `x86_64-pc-windows-gnu`. `-gnu` rather than `-msvc` because
`-msvc` needs the Microsoft linker and Windows SDK, which a Linux container
cannot have. No practical difference for psolve: no C dependencies, and both
`memmap2` and `rayon` support the target.

**Nothing runs this binary.** There is no Wine on the runner and no Windows
runner on this GitLab. The job asserts what it can -- that the file exists and
`file(1)` reports `PE32+` -- which catches a mis-targeted build but proves
nothing about behaviour. Treat every `.exe` as unverified until someone runs it
on Windows.

### The Windows behaviour difference, stated rather than buried

`fits_update::same_directory` compares two paths by device+inode. `std` has no
portable device+inode API off Unix, so on Windows that function returns `None`
unconditionally and **one of the three `.psolve-readonly` ancestor chains is
permanently unavailable**.

This matters because `-update` is the only path that touches pixel data, and a
header rewrite that shifted the data unit silently corrupted four archive
frames once. What still holds on Windows:

- The **canonical** ancestor chain -- the module's one unconditional guarantee
  -- is unaffected. A `.psolve-readonly` marker on the real path's ancestors
  refuses the write, on every platform.
- `PSOLVE_READONLY` (any non-empty value) refuses, on every platform.
- Temp-copy, `fsync`, reparse-and-compare-pixels, then rename: unaffected.

What is weaker: the two best-effort *lexical* chains, which exist to catch a
marker placed on a symlinked tree rather than the file's physical location, are
reduced. Anyone running `-update` on Windows against a tree reached through
junctions should rely on `PSOLVE_READONLY`, not on marker placement.

## macOS -- needs a runner on a Mac

Apple's SDK licensing makes cross-compiling to Apple targets from Linux a
non-starter, so a Linux-only CI cannot produce a macOS build at all. Two
paths, and they are complementary rather than alternatives:

### 1. Homebrew tap (no runner needed)

`packaging/homebrew/psolve.rb` is written and syntax-checked. It builds from
source with the `rust` formula, so it needs no signing and no notarisation --
which is why it is preferred over a `.pkg`.

**It is not published, and two things block that:**

- **A tap is a repo.** The formula does nothing sitting in this directory; it
  has to live in one named `homebrew-<tap>`, installed with
  `brew tap astroops-cloud/<tap> <url>`. Creating that repo is an outward
  action and is deliberately not automated here.
- **The digest is not real yet.** The `url` now points at this repo's own
  `v0.1.0` source tarball, which anyone can reach, but the formula's `sha256`
  is still a deliberate placeholder that `brew` will refuse. It stays that way
  until someone computes the digest of that exact tarball
  (`curl -sL <url> | shasum -a 256`) -- a formula that installs whatever it
  downloaded is worse than one that refuses to install at all.

### 2. A self-hosted runner beside the rig data

A hosted macOS runner gives native macOS builds. What no hosted runner can give
is the second benefit: **the workstation holds the rig data**, so a runner
there could execute the four tests that skip everywhere else for want of
`~/astroops/data` and `~/astroops/library` -- the coverage gap
`crates/psolve-cli/tests/rig_data_dependence.rs` exists to pin. It is the only
way to close that gap in CI rather than merely record it.

**Read this before installing one.** The workstation is the machine that holds the frame
archive. A CI runner there executes arbitrary code from any branch, next to
data that is irreplaceable and that this repo treats as strictly read-only. The
protections that already exist are real but were designed against *accidents*,
not against a runner:

- `PSOLVE_READONLY` refuses every write when set to any non-empty value.
- A `.psolve-readonly` marker on the canonical ancestor chain refuses
  unconditionally.

Minimum sensible precautions, none of which are a substitute for deciding this
is worth it:

1. Run the runner as a **dedicated unprivileged user** with no read access to
   `~/astroops/library`, and grant read-only access to only the specific
   indexes the tests need.
2. Set `PSOLVE_READONLY=1` in the runner's environment, so any `-update` in any
   job refuses regardless of what the job asks for.
3. Place a `.psolve-readonly` marker at the root of `~/astroops`.
4. Label its jobs distinctly and do NOT make that label the workflow default,
   so a job cannot silently land on the host holding the archive.
5. Restrict it to protected branches, so an untrusted branch cannot run code
   beside the archive.

The honest summary: the macOS build is worth having, and the closed coverage
gap is worth more, but the second is bought with a runner on the machine that
must not lose data. That is a trade to make deliberately.

## Signing

Deliberately none. Distribution is through package managers -- Homebrew for
macOS, and winget is the equivalent path for Windows -- because neither
triggers Gatekeeper or SmartScreen, so no Apple Developer account (~99 USD/yr)
and no Windows EV certificate (~300+ USD/yr) is needed.

The cost of that choice: a **direct download** of the `.zip` or a `.pkg` will
warn on both platforms. A winget manifest also needs a publicly reachable
download URL, so it is blocked by the same LAN-only hosting that blocks a
public Homebrew tap.
