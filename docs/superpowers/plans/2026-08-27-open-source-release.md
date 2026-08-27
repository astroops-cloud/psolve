# Open-Source Release Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Publish `psolve` as `github.com/astroops-cloud/psolve` under MIT, with the
tree scrubbed of personal identity and homelab topology, a first step a stranger can
run, and CI a fork can execute.

**Architecture:** Scrub the HEAD tree in the GitLab working copy behind a structural
guard test, then export only tracked files with `git archive` into a fresh repo whose
history is one commit. Verify against a clone pulled back from GitHub while the repo
is still private; flip public last.

**Tech Stack:** Rust (workspace, 3 crates), GitHub Actions, `gh` CLI, POSIX shell.

**Spec:** `docs/superpowers/specs/2026-08-27-open-source-release-design.md`

## Global Constraints

- **Do not run `cargo fmt`.** 819 differing hunks; format only what you touch, by hand.
- Substitutions inside `.wcs` fixtures must be **character-for-character the same
  width**. Every card is exactly 80 bytes; `reference-block.wcs` must stay a whole
  multiple of 2880 (currently 8640).
- The canonical substitution is `<home>` → `/home/user` — exactly 10 characters
  either way, so no FITS card needs re-wrapping.
- ASCII `--` rather than em dashes in prose.
- Commit subjects: `type(scope): summary`.
- The suite must not run as root.
- Baseline before any change: `cargo test --workspace` green at **631** tests,
  `cargo clippy --workspace --all-targets` clean.
- Nothing is pushed to GitHub before Task 10; nothing is public before Task 11.

---

### Task 1: The scrub guard (fails first, by design)

Makes the scrub permanent rather than a one-time cleanup -- the same shape as
`no_filesystem.rs` and `fixtures_are_tracked.rs`: a convention that fails rather than
one people are trusted to remember.

**This file ships publicly, so it must not name the values it protects.** A guard
that spells out the coordinate it forbids publishes the coordinate. So the site cards
are pinned **positively** -- they must equal their redacted form -- and everything
else is a generic class: a macOS home path, an RFC1918 address, a LAN-only domain.
One-time scrubs that cannot recur (former machine nicknames) are verified once at
release in Task 11, not pinned here.

**Files:**
- Create: `crates/psolve-cli/tests/tree_is_scrubbed.rs`

**Interfaces:**
- Consumes: nothing.
- Produces: `FORBIDDEN_PATTERNS` and `REDACTED_SITE_CARDS` -- what Tasks 2-5 drive to green.

- [ ] **Step 1: Write the guard test**

```rust
//! Pins the published tree free of the classes of leak that recur.
//!
//! This repo is public and this file is in it, so the guard must not name the
//! values it protects: a test that spells out the coordinate it forbids
//! publishes the coordinate. The observation-site cards are therefore pinned
//! POSITIVELY -- they must equal their redacted form -- and everything else is
//! a generic class rather than a private literal.
//!
//! It scans `git ls-files`, so it sees what a stranger clones, not the working
//! directory, which also holds gitignored in-repo worktrees. Like
//! `fixtures_are_tracked.rs` it PANICS rather than skips when `git` is missing:
//! a guard that quietly does nothing on the machine that matters is worse than
//! no guard.

use std::process::Command;

/// (pattern, what to use instead) -- the message is the fix, so a failure does
/// not send anyone hunting for the convention.
const FORBIDDEN_PATTERNS: &[(&str, &str)] = &[
    // Three (pattern, fix) pairs: a macOS home directory, an RFC1918 address,
    // a LAN-only domain. They are deliberately NOT reproduced in this
    // document -- quoting them here would make the plan itself fail the guard,
    // which is the same trap the guard was rewritten to avoid. Read them in
    // crates/psolve-cli/tests/tree_is_scrubbed.rs.
];

/// The three observation-site cards in their redacted form, byte for byte.
/// Pinned positively so this file never has to name the real values.
const REDACTED_SITE_CARDS: &[&str] = &[
    "SITEELEV=     000.000000000000 / [m] Observation site elevation",
    "SITELAT =           -00.000000 / [deg] Observation site latitude",
    "SITELONG=           000.000000 / [deg] Observation site longitude",
];

/// This file necessarily contains the patterns it forbids.
const SELF: &str = "crates/psolve-cli/tests/tree_is_scrubbed.rs";

fn repo_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("crates/<pkg> is two levels below the repo root")
        .to_path_buf()
}

fn tracked_files() -> Vec<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo_root())
        .args(["ls-files", "-z"])
        .output()
        .expect("git ls-files -- git must be installed to run this suite");
    assert!(out.status.success(), "git ls-files failed");
    String::from_utf8_lossy(&out.stdout)
        .split('\0')
        .filter(|s| !s.is_empty() && *s != SELF)
        .map(str::to_string)
        .collect()
}

#[test]
fn no_tracked_file_carries_a_personal_path_or_private_address() {
    let root = repo_root();
    let mut hits: Vec<String> = Vec::new();
    for rel in tracked_files() {
        let Ok(bytes) = std::fs::read(root.join(&rel)) else { continue };
        let body = String::from_utf8_lossy(&bytes);
        for (pattern, fix) in FORBIDDEN_PATTERNS {
            if body.contains(pattern) {
                hits.push(format!("{rel}: {pattern:?} -- {fix}"));
            }
        }
    }
    assert!(
        hits.is_empty(),
        "{} tracked file(s) carry a scrubbed pattern:\n{}",
        hits.len(),
        hits.join("\n")
    );
}

#[test]
fn the_observation_site_cards_are_redacted() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    for name in ["reference.wcs", "reference-block.wcs"] {
        let body = std::fs::read_to_string(dir.join(name)).expect(name);
        for card in REDACTED_SITE_CARDS {
            assert!(
                body.contains(card),
                "{name} does not carry the redacted card {card:?} -- either the scrub \
                 was not applied or a substitution changed the card's width"
            );
        }
    }
}

/// The byte-exact sidecar tests only hold if the cards stay 80 bytes and the
/// block stays padded. Scrubbing is where that gets broken, so it is asserted
/// next to the scrub rather than left to a distant test.
#[test]
fn the_fits_block_fixture_is_still_whole() {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/reference-block.wcs");
    let n = std::fs::metadata(&p).expect("reference-block.wcs").len();
    assert_eq!(n % 2880, 0, "reference-block.wcs is {n} bytes, not a whole multiple of 2880");
    assert_eq!(n, 8640, "reference-block.wcs changed size -- a substitution was not same-width");
}
```

- [ ] **Step 2: Run it and confirm it FAILS on the real surface**

Run: `cargo test -p psolve-cli --test tree_is_scrubbed`
Expected: `no_tracked_file_...` FAILS listing files; `the_observation_site_cards_are_redacted`
FAILS because the cards still hold real values. Record the file count -- Tasks 2-5 drive it to zero.

- [ ] **Step 3: Commit the guard**

```bash
git add crates/psolve-cli/tests/tree_is_scrubbed.rs
git commit -m "test(cli): pin the published tree free of personal paths and site coordinates

Pins the site cards positively, by their redacted form, rather than forbidding
the real values -- this file is published, so a guard naming what it protects
would publish it."
```

---

### Task 2: Scrub the sidecar fixtures, same-width

**Files:**
- Modify: `crates/psolve-cli/tests/fixtures/reference.wcs` (lines 35-37, 78-86)
- Modify: `crates/psolve-cli/tests/fixtures/reference-block.wcs` (same cards, unwrapped)
- Modify: `crates/psolve-cli/tests/fixtures/reference.ini:14`
- Modify: `crates/psolve-cli/tests/fixtures/reference-failure.ini:3`
- Modify: `docs/superpowers/2026-08-14-astap-format-facts.md` (the quoted header block)

**Interfaces:**
- Consumes: `FORBIDDEN` from Task 1.
- Produces: fixtures whose cards are unchanged in width, so `sidecar_ini.rs` and
  `sidecar_wcs.rs` keep passing untouched.

The exact cards, measured (`[len=80]` each):

```
SITEELEV=     <site-elev> / [m] Observation site elevation
SITELAT =           <site-lat> / [deg] Observation site latitude
SITELONG=           <site-long> / [deg] Observation site longitude
```

- [ ] **Step 1: Record the byte sizes before touching anything**

```bash
wc -c crates/psolve-cli/tests/fixtures/reference.wcs crates/psolve-cli/tests/fixtures/reference-block.wcs
# expect 7028 and 8640
```

- [ ] **Step 2: Apply the same-width substitutions**

```bash
python3 - <<'PY'
import io
subs = [
    ("<site-elev>", "000.000000000000"),   # 16 -> 16
    ("<site-lat>",       "-00.000000"),          # 10 -> 10
    ("<site-long>",       "000.000000"),          # 10 -> 10
    ("<home>",       "/home/user"),          # 10 -> 10
    ("inbox/the capture host/",   "inbox/capture/"),      # 7 -> 7 inside the path
]
files = [
    "crates/psolve-cli/tests/fixtures/reference.wcs",
    "crates/psolve-cli/tests/fixtures/reference-block.wcs",
    "crates/psolve-cli/tests/fixtures/reference.ini",
    "crates/psolve-cli/tests/fixtures/reference-failure.ini",
    "docs/superpowers/2026-08-14-astap-format-facts.md",
]
for p in files:
    b = io.open(p, "rb").read()
    n0 = len(b)
    for a, c in subs:
        assert len(a) == len(c), (a, c)
        b = b.replace(a.encode(), c.encode())
    assert len(b) == n0, f"{p}: size changed {n0} -> {len(b)}"
    io.open(p, "wb").write(b)
    print(f"{p}: {n0} bytes, unchanged")
PY
```

- [ ] **Step 3: Prove the cards did not move**

```bash
wc -c crates/psolve-cli/tests/fixtures/reference.wcs crates/psolve-cli/tests/fixtures/reference-block.wcs
awk 'length($0)!=80 && length($0)!=0 {print FILENAME": line "NR" is "length($0)" bytes"}' \
  crates/psolve-cli/tests/fixtures/reference.wcs
```
Expected: 7028 and 8640 unchanged; the `awk` prints only the final short line (57 bytes).

- [ ] **Step 4: Run the byte-exact sidecar tests**

Run: `cargo test -p psolve-cli --test sidecar_wcs --test sidecar_ini --test tree_is_scrubbed`
Expected: `sidecar_*` PASS unchanged; `tree_is_scrubbed` still fails, but on fewer files.

- [ ] **Step 5: Commit**

```bash
git add crates/psolve-cli/tests/fixtures docs/superpowers/2026-08-14-astap-format-facts.md
git commit -m "chore(fixtures): redact site coordinates and home paths, same-width

The .wcs fixtures are FITS cards: every substitution is character-for-character
the same width, so reference.wcs stays 7028 bytes and reference-block.wcs stays
8640 (3 x 2880). The byte-exact sidecar tests are unchanged and still pass."
```

---

### Task 3: Scrub source and test literals

**Files:**
- Modify: `crates/psolve-cli/src/astap_args.rs:67,68,577,586,598,893,894,898,902,910,911`
- Modify: `crates/psolve-cli/tests/astap_cli.rs:156,186`
- Modify: `crates/psolve-cli/tests/fits_update.rs:1291`

`astap_cli.rs` is **not** a rename. Both tests pass `-d <astap-dir>` with the
comment *"real ASTAP's own directory: real, but holds no .psidx"*. That directory
does not exist on CI, so the test currently passes for a reason its own comment
denies. Give it a created temp directory: real everywhere, empty everywhere, and the
comment becomes true.

- [ ] **Step 1: Replace the doc-comment and unit-test literals**

```bash
python3 - <<'PY'
import io
for p in ["crates/psolve-cli/src/astap_args.rs", "crates/psolve-cli/tests/fits_update.rs"]:
    s = io.open(p, encoding="utf-8").read()
    s = s.replace("<home>", "/home/user")
    io.open(p, "w", encoding="utf-8").write(s)
    print("scrubbed", p)
PY
```

- [ ] **Step 2: Give `astap_cli.rs` a real empty directory**

Add this helper near `bin()` in `crates/psolve-cli/tests/astap_cli.rs`:

```rust
/// A directory that really exists and really holds no `.psidx`.
///
/// This used to be a hardcoded path to real ASTAP's own install directory,
/// which meant the test asserted "a real directory with no index" on exactly
/// one machine and asserted nothing about realness anywhere else -- the
/// database error it checks for is also what a NONEXISTENT directory
/// produces, so the assertion could not tell the two apart. A created temp
/// dir makes the comment true on every machine.
fn empty_db_dir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("psolve-empty-db-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}
```

Then in `the_real_blind_invocation_is_accepted_by_dispatch_and_reaches_the_solve_path`,
replace the `"-d", "<astap-dir>",` pair with:

```rust
            "-d",
            empty_db_dir("blind").to_str().unwrap(),
```

and in `the_real_hinted_retry_is_accepted_by_dispatch_and_reaches_the_solve_path`:

```rust
            "-d",
            empty_db_dir("hinted").to_str().unwrap(),
```

- [ ] **Step 3: Run the affected tests**

Run: `cargo test -p psolve-cli --test astap_cli --test fits_update`
Expected: PASS. Both invocations must still fail with `"No star database found."`

- [ ] **Step 4: Prove the new test can still fail**

Temporarily point `empty_db_dir` at a directory holding a `.psidx` and confirm the
assertion breaks; then revert. A guard that cannot bite proves nothing.

- [ ] **Step 5: Commit**

```bash
git add crates/psolve-cli/src/astap_args.rs crates/psolve-cli/tests/astap_cli.rs crates/psolve-cli/tests/fits_update.rs
git commit -m "test(cli): use a real empty temp dir instead of a machine-specific path

The -d literal pointed at real ASTAP's install directory, so the 'real but
holds no index' the comment claims held on one machine only -- and the error
it asserts is also what a nonexistent directory yields, so the test could not
tell the difference. A created temp dir is real and empty everywhere."
```

---

### Task 4: Rewrite the measurement data interiors

**Files:**
- Modify: `docs/superpowers/data/task-11-agreement-full-9495.ndjson.gz`
- Modify: `docs/superpowers/data/task-11-agreement-sample-300.ndjson.gz`

Delete nothing: ten documents cite these files. Rewrite `path` and `cmd[0]` only;
every measured field stays byte-identical.

- [ ] **Step 1: Record what must not change**

```bash
for f in docs/superpowers/data/*.ndjson.gz; do
  echo "$f rows=$(gzip -dc "$f" | wc -l) frame_ids_sha=$(gzip -dc "$f" | python3 -c "
import sys, json, hashlib
h = hashlib.sha256()
for line in sys.stdin:
    r = json.loads(line)
    h.update(repr([r.get(k) for k in ('frame_id','db_ra','db_dec','naxis1','naxis2','binning')]).encode())
print(h.hexdigest()[:16])")"
done
```

- [ ] **Step 2: Rewrite the two path fields**

```bash
python3 - <<'PY'
import gzip, json, glob
for p in sorted(glob.glob("docs/superpowers/data/*.ndjson.gz")):
    rows = [json.loads(l) for l in gzip.open(p, "rt")]
    for r in rows:
        if isinstance(r.get("path"), str):
            r["path"] = r["path"].replace("<astroops>/archive/fits", "<archive>")
        cmd = r.get("cmd")
        if isinstance(cmd, list):
            r["cmd"] = [c.replace("<dev-repo>", "<repo>")
                         .replace("<astroops>/archive/fits", "<archive>")
                         .replace("<home>", "/home/user") if isinstance(c, str) else c
                        for c in cmd]
    with gzip.open(p, "wt", compresslevel=9) as out:
        for r in rows:
            out.write(json.dumps(r) + "\n")
    print("rewrote", p, len(rows), "rows")
PY
```

- [ ] **Step 3: Prove the measurements survived**

Re-run Step 1's command. `rows` and `frame_ids_sha` must be identical to what it
printed before. If either moved, the rewrite touched a measured field — revert and fix.

- [ ] **Step 4: Commit**

```bash
git add docs/superpowers/data
git commit -m "chore(data): replace archive paths with <archive> in the agreement data

Rewrites path and cmd only. Row count and a digest over frame_id/db_ra/db_dec/
naxis/binning are unchanged, so the ten documents citing these files still cite
the same measurements."
```

---

### Task 5: Scrub prose, packaging and metadata; drop GitLab CI

**Files:**
- Modify: `README.md:63`, `docs/astap-compat.md`, `docs/superpowers/2026-08-14-m3-first-real-frame.md`,
  `docs/superpowers/2026-08-15-stratified-selection-results.md`,
  `docs/superpowers/2026-08-25-index-depth.md`,
  `docs/superpowers/plans/2026-08-13-m1-star-index.md`,
  `docs/superpowers/plans/2026-08-14-m3-astap-compat.md`,
  `docs/superpowers/specs/2026-08-13-psolve-design.md`
- Modify: `packaging/README.md`, `packaging/homebrew/psolve.rb`
- Modify: `crates/psolve-cli/Cargo.toml:26`
- Delete: `.gitlab-ci.yml`

Host names become what the tables are actually contrasting. `the workstation` → `macos-arm64`,
`host B` → `linux-x86-64`: in `docs/superpowers/2026-08-25-index-depth.md` those two
rows exist precisely to contrast architectures, so the replacement says more than the
nickname did.

`.gitlab-ci.yml` goes now rather than at export: its runner tag is functional, not a
comment, so it cannot be neutralised without breaking the pipeline it names. GitHub
Actions replaces it in Task 6 and the GitLab repo is frozen after Task 10 — its
history keeps the file.

- [ ] **Step 1: Substitute in prose**

```bash
python3 - <<'PY'
import io
subs = [
    ("<home>", "/home/user"),
    ("the workstation", "macos-arm64"),
    ("host B", "linux-x86-64"),
    ("<LAN>", "<LAN>"),
    ("neil@the former maintainer address", "neil@asdf.systems"),
]
files = """README.md docs/astap-compat.md
docs/superpowers/2026-08-14-m3-first-real-frame.md
docs/superpowers/2026-08-15-stratified-selection-results.md
docs/superpowers/2026-08-25-index-depth.md
docs/superpowers/plans/2026-08-13-m1-star-index.md
docs/superpowers/plans/2026-08-14-m3-astap-compat.md
docs/superpowers/specs/2026-08-13-psolve-design.md
packaging/README.md packaging/homebrew/psolve.rb
crates/psolve-cli/Cargo.toml""".split()
for p in files:
    s = io.open(p, encoding="utf-8").read()
    for a, b in subs:
        s = s.replace(a, b)
    io.open(p, "w", encoding="utf-8").write(s)
    print("scrubbed", p)
PY
```

- [ ] **Step 2: Fix the sentences the substitution left ungrammatical**

Read each hit by hand. `the capture host` in prose is not a token swap:
- `docs/astap-compat.md:646` — "Needs the capture host and an operator" → "Needs the capture
  host and an operator".
- `docs/superpowers/specs/2026-08-13-psolve-design.md:5` — the `**Repo:**` line becomes
  `github.com/astroops-cloud/psolve`.
- `packaging/homebrew/psolve.rb` — `homepage` and `url` point at
  `https://github.com/astroops-cloud/psolve`. **Keep the deliberately invalid
  `sha256` and its comment**: the formula must keep refusing to install until a real
  `v0.1.0` tarball exists (Task 11 computes it).
- `packaging/README.md` — the "LAN-only" section is now wrong rather than private;
  rewrite it to say the formula is unpublished pending a real release digest.

- [ ] **Step 3: Scrub the release documents themselves**

`docs/superpowers/specs/2026-08-27-open-source-release-design.md` and
`docs/superpowers/plans/2026-08-27-open-source-release.md` quote the real site
coordinates and home paths in order to describe scrubbing them. Published as-is they
leak exactly what the job removes. Rewrite both to name the **cards and the classes**
rather than the values: `SITELAT`/`SITELONG`/`SITEELEV` "redacted to zeroes,
same width", `<home>` rather than the real home, `<host-a>`/`<host-b>` for the
nicknames. Both documents stay fully readable -- the substitution table's point is
the widths, not the digits.

- [ ] **Step 4: Delete the GitLab pipeline**

```bash
git rm .gitlab-ci.yml
```

- [ ] **Step 5: The guard must now be green**

Run: `cargo test -p psolve-cli --test tree_is_scrubbed`
Expected: PASS. If any file remains, it is listed with the token and its replacement.

- [ ] **Step 6: Full suite**

Run: `cargo test --workspace && cargo clippy --workspace --all-targets`
Expected: 631 green, clippy clean. **State the count in the commit if it moved.**

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "chore: scrub homelab topology from prose and packaging; drop GitLab CI

Host nicknames become the property the tables contrast (macos-arm64,
linux-x86-64), which reads better than the nicknames did. the runner tag is
functional rather than cosmetic, so .gitlab-ci.yml goes rather than being
half-scrubbed; GitHub Actions replaces it and this repo's history keeps it."
```

---

### Task 6: GitHub Actions

**Files:**
- Create: `.github/workflows/ci.yml`
- Modify: `CLAUDE.md` (the `## CI and packaging` section)
- Modify: `packaging/README.md` (the platform-coverage statement)

Use **VM runners, not container jobs**: `fits_update.rs` stages a failure with
`chmod 0o311`, which denies root nothing, and it panics naming root rather than
passing vacuously.

- [ ] **Step 1: Write the workflow**

```yaml
name: ci

on:
  push:
    branches: [main]
    tags: ['v*']
  pull_request:

jobs:
  lint:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - run: rustup component add clippy
      - run: cargo clippy --workspace --all-targets -- -D warnings

  test:
    strategy:
      fail-fast: false
      matrix:
        os: [ubuntu-latest, macos-latest]
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      # Not a container job on purpose: fits_update.rs stages a permission
      # failure with chmod 0o311, which denies root nothing, so under root it
      # panics naming root rather than passing vacuously.
      - run: id -u    # must not be 0
      - run: cargo test --workspace
      - name: demo runs with no index and no network
        run: ./scripts/demo.sh

  package:
    if: startsWith(github.ref, 'refs/tags/v')
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - run: cargo build --release
      - uses: actions/upload-artifact@v4
        with:
          name: psolve-linux-amd64
          path: target/release/psolve
```

- [ ] **Step 2: Rewrite CLAUDE.md's CI section**

Replace the GitLab paragraph. Keep both lessons, restated for the new home: the
suite must not run as root (hence VM runners), and **a green pipeline proves less
than it looks** — GitHub's runners have no `~/astroops` either, so the same four
rig-dependent files skip and `rig_data_dependence.rs` is what stops that gap widening.

- [ ] **Step 3: Re-measure the platform-coverage claim -- DEFERRED to after Task 11 Step 5**

`packaging/README.md` and `CLAUDE.md` both state macOS is neither built nor
executed, because Apple SDK licensing rules out cross-compiling from Linux. The
`macos-latest` job changes that -- but **it cannot run until the repo exists on
GitHub**, and the rule is "run the job, then write what it did", not "edit the
sentence to match the new setup".

So both claims are left standing, correct as of today, and this step moves to
immediately after Task 11 Step 5 (the first green CI run), where the result is a
measurement rather than an expectation. If the macOS job fails there, the
existing sentences remain true and only the reason changes.

- [ ] **Step 4: Commit**

```bash
git add .github/workflows/ci.yml CLAUDE.md packaging/README.md
git commit -m "ci: run on GitHub Actions, ubuntu and macos VM runners"
```

---

### Task 7: A first step that needs no Gaia

**Files:**
- Create: `crates/psolve-cli/examples/synth_field.rs`
- Create: `scripts/demo.sh`
- Modify: `README.md` (quickstart block near the top)

The generator is lifted from `crates/psolve-cli/tests/cli_solve_success.rs`, which
already builds a synthetic field plus a matching catalogue and asserts a real solve.
The example only *writes* the two files; the script drives the CLI.

**Interfaces:**
- Produces: `synth_field <dir>` writes `<dir>/field.fits` and `<dir>/cat/a.csv`.

- [ ] **Step 1: Write the example**

```rust
//! Writes a synthetic star field and the catalogue it was generated from, so
//! `scripts/demo.sh` can build an index and solve it with no Gaia mirror and
//! no network. A built index is CC BY-NC 3.0 IGO; this repo is MIT, so the
//! demo data is synthetic on purpose rather than for convenience.
//!
//! The generator is the one `tests/cli_solve_success.rs` uses.

use psolve_core::fit::Wcs;

const NX: usize = 640;
const NY: usize = 480;
const SCALE_ARCSEC: f64 = 2.4614;

fn scatter(i: usize) -> (f64, f64) {
    let mut z = (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    let mut next = || {
        z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut x = z;
        x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        x ^ (x >> 31)
    };
    let a = next();
    let b = next();
    ((a >> 11) as f64 / (1u64 << 53) as f64, (b >> 11) as f64 / (1u64 << 53) as f64)
}

fn truth_wcs(ra0: f64, dec0: f64) -> Wcs {
    let s = SCALE_ARCSEC / 3600.0;
    Wcs { crval: [ra0, dec0], crpix: [NX as f64 / 2.0, NY as f64 / 2.0], cd: [[-s, 0.0], [0.0, s]] }
}

fn build_fixture(ra0: f64, dec0: f64, n: usize) -> (Vec<u8>, String) {
    let w = truth_wcs(ra0, dec0);
    let margin = 40.0;
    let mut pix = Vec::new();
    for i in 0..n {
        let (u, v) = scatter(i);
        pix.push((margin + u * (NX as f64 - 2.0 * margin), margin + v * (NY as f64 - 2.0 * margin)));
    }
    let mut img = vec![1000f64; NX * NY];
    for (i, v) in img.iter_mut().enumerate() {
        *v += ((i * 2654435761usize) % 97) as f64 * 0.4;
    }
    let sigma = 1.8f64;
    let mut csv = String::from("ra,dec,pmra,pmdec,phot_g_mean_mag\n");
    for (k, &(cx, cy)) in pix.iter().enumerate() {
        let peak = 8000.0 - (k % 20) as f64 * 150.0;
        let r = 5i64;
        for dy in -r..=r {
            for dx in -r..=r {
                let x = cx.round() as i64 + dx;
                let y = cy.round() as i64 + dy;
                if x < 0 || y < 0 || x >= NX as i64 || y >= NY as i64 {
                    continue;
                }
                let ex = x as f64 - cx;
                let ey = y as f64 - cy;
                img[y as usize * NX + x as usize] +=
                    peak * (-(ex * ex + ey * ey) / (2.0 * sigma * sigma)).exp();
            }
        }
        let (ra, dec) = w.pix_to_radec(cx, cy);
        csv.push_str(&format!("{ra:.8},{dec:.8},0,0,{:.2}\n", 12.0 + (k % 10) as f64 * 0.1));
    }
    let cards = [
        "SIMPLE  =                    T".to_string(),
        "BITPIX  =                   16".to_string(),
        "NAXIS   =                    2".to_string(),
        format!("NAXIS1  = {NX:>20}"),
        format!("NAXIS2  = {NY:>20}"),
        "BZERO   =                32768".to_string(),
    ];
    let mut s = String::new();
    for c in &cards {
        s.push_str(&format!("{c:<80}"));
    }
    s.push_str(&format!("{:<80}", "END"));
    while !s.len().is_multiple_of(2880) {
        s.push(' ');
    }
    let mut out = s.into_bytes();
    for v in &img {
        let clamped = v.clamp(0.0, 65535.0) as u16;
        out.extend_from_slice(&((clamped as i32 - 32768) as i16).to_be_bytes());
    }
    while !out.len().is_multiple_of(2880) {
        out.push(0);
    }
    (out, csv)
}

fn main() {
    let dir = std::env::args().nth(1).expect("usage: synth_field <dir>");
    let dir = std::path::PathBuf::from(dir);
    let cat = dir.join("cat");
    std::fs::create_dir_all(&cat).expect("create output dir");
    let (fits, csv) = build_fixture(83.822, -5.391, 60);
    std::fs::write(dir.join("field.fits"), &fits).expect("write field.fits");
    std::fs::write(cat.join("a.csv"), csv).expect("write catalogue");
    println!("{}", dir.display());
}
```

- [ ] **Step 2: Write the driver**

```sh
#!/bin/sh
# A complete psolve run with no Gaia mirror, no network and no rig data:
# generate a synthetic field, build an index from the catalogue it came from,
# solve it. Everything lands in a temp directory.
set -eu

d=$(mktemp -d)
trap 'rm -rf "$d"' EXIT

echo "==> generating a synthetic field in $d"
cargo run --release -q -p psolve-cli --example synth_field -- "$d"

echo "==> building an index"
cargo run --release -q -p psolve-cli -- index build \
    --input "$d/cat" --out "$d/demo.psidx" --max-mag 20 --nside 64

echo "==> solving"
cargo run --release -q -p psolve-cli -- solve "$d/field.fits" \
    --index "$d/demo.psidx" --hint 83.822,-5.391

echo "==> done"
```

- [ ] **Step 3: Make it executable and run it**

```bash
chmod +x scripts/demo.sh && ./scripts/demo.sh
```
Expected: JSON with `"solved":true` and a `crval` within a few arcseconds of
`83.822,-5.391`.

- [ ] **Step 4: Prove it needs nothing from this machine**

```bash
env -i PATH=/usr/bin:/bin:/usr/local/bin HOME=/nonexistent sh ./scripts/demo.sh
```
Expected: still solves. If it fails, the demo depends on rig state and is not a demo.

- [ ] **Step 5: Put it in the README**

Add above the ASTAP section:

````markdown
## Try it in one command

No star index and no Gaia download required -- this generates a synthetic field,
builds an index from the catalogue it came from, and solves it:

```sh
./scripts/demo.sh
```

Solving your own frames needs a real index; see
[`docs/index-building.md`](../../index-building.md).
````

- [ ] **Step 6: Commit**

```bash
git add crates/psolve-cli/examples/synth_field.rs scripts/demo.sh README.md
git commit -m "feat(cli): a synthetic end-to-end demo needing no Gaia index"
```

---

### Task 8: Metadata, contributor rules, provenance note

**Files:**
- Modify: `Cargo.toml`, `crates/psolve-core/Cargo.toml`, `crates/psolve-index/Cargo.toml`, `crates/psolve-cli/Cargo.toml`
- Create: `CONTRIBUTING.md`
- Modify: `docs/astap-compat.md` (header note), `README.md` (header note)

- [ ] **Step 1: Measure the MSRV, do not guess it**

```bash
for v in 1.79 1.81 1.83 1.85; do
  rustup toolchain install "$v" --profile minimal >/dev/null 2>&1
  printf "%s: " "$v"
  cargo +$v check --workspace >/dev/null 2>&1 && echo OK || echo FAIL
done
```
Take the lowest OK. `is_multiple_of` on integers is used in the fixture builders and
is recent, so expect a floor above 1.79.

- [ ] **Step 2: Add workspace metadata**

In root `Cargo.toml` under `[workspace.package]`:

```toml
repository = "https://github.com/astroops-cloud/psolve"
homepage = "https://github.com/astroops-cloud/psolve"
readme = "README.md"
rust-version = "<measured in Step 1>"
```

Add to `psolve-core/Cargo.toml` and `psolve-index/Cargo.toml`:

```toml
description = "..."         # core: "Plate-solving pipeline: FITS bytes in, a verified TAN WCS out. No dependencies."
                            # index: "psolve star and quad index formats (.psidx / .psqidx)."
repository.workspace = true
readme.workspace = true
rust-version.workspace = true
keywords = ["astronomy", "astrometry", "plate-solving", "fits", "wcs"]
categories = ["science", "command-line-utilities"]
```

- [ ] **Step 3: Write CONTRIBUTING.md**

Four rules, each with its reason — an unexplained rule gets "fixed":

```markdown
# Contributing

## Do not run `cargo fmt`

This repo has never been rustfmt-clean and there is deliberately no
`cargo fmt --check` gate. A bare `cargo fmt` rewrites ~60 files (819 differing
hunks, measured 2026-08-26), burying a real change in a whole-repo reformat.
Format only what you touch, by hand, matching the surrounding code.

## Some tests skip themselves, and one file polices that

Tests needing multi-GB indexes or real frames print `skipping` and pass when that
data is absent, so the suite runs anywhere. `rig_data_dependence.rs` pins exactly
which files may do this. Add a fifth and the suite fails until you list it --
which forces the choice: commit a fixture, or widen the coverage gap on purpose.
Widening it is allowed; widening it silently is not.

## The guards fail closed

`psolve-core` has no dependencies, not even dev-dependencies, and may not name
`fs`/`net`/`process`/`env`/`File`/`PathBuf` **even in a comment** --
`no_filesystem.rs` scans tokens and cannot tell prose from code, so reword the
prose. `tree_is_scrubbed.rs` keeps personal paths and machine names out of a
published tree. Neither is negotiable in a PR; both tell you the fix in the
failure message.

## Measured, not projected

Any number in a doc carries the invocation, the flags and the machine state that
produced it. When a re-measurement contradicts an earlier claim, both figures are
reported and the earlier one retracted in place. Don't smooth over a failed
criterion -- investigate it and record what you found.
```

- [ ] **Step 4: Add the provenance note**

At the top of `docs/astap-compat.md` and in the README's measurement section:

```markdown
> **On commit references.** Public history begins at `v0.1.0`. Short SHAs cited in
> these documents (`297961b`, `3ba1c32`, ...) refer to the pre-release development
> history, which is retained privately and is not part of this repository. The
> measurements themselves are reproducible from the flags and data each document
> names.
```

- [ ] **Step 5: Verify and commit**

```bash
cargo test --workspace && cargo clippy --workspace --all-targets
git add -A && git commit -m "docs: contributor rules, crate metadata, and the v0.1.0 provenance note"
```

---

### Task 8b: GitHub conventions -- the earned set

Added 2026-08-27 at the user's request, after the question "does it follow
GitHub conventions?". The repo had no `.github/` at all. This is the subset
that earns its place; `CODE_OF_CONDUCT.md`, `.editorconfig` and `cargo-deny`
were considered and deferred as ceremony for contributors who do not exist yet.

**A `rustfmt.toml` and a `cargo fmt --check` gate are NOT part of this** and
must not be added silently. It is the single most expected Rust convention and
it directly contradicts a documented decision of this repo (819 differing
hunks, no gate, deliberately). Adding it because "that is what Rust repos do"
is precisely the failure the Global Constraints exist to prevent.

**Files:**
- Create: `SECURITY.md`
- Create: `.github/ISSUE_TEMPLATE/bug_report.yml`, `.github/ISSUE_TEMPLATE/config.yml`
- Create: `.github/PULL_REQUEST_TEMPLATE.md`
- Create: `CHANGELOG.md`
- Create: `.github/dependabot.yml`
- Create: `.github/workflows/release.yml`
- Modify: `README.md` (badges)

- [ ] **Step 1: `SECURITY.md` -- the one piece this project actually earns**

psolve parses untrusted FITS and `fits.rs` is built to never panic on malformed
input, so a disclosure path is load-bearing rather than decorative. State: what
is in scope (the FITS parser, the index readers, `-update`), what is not (an
index you built yourself is data you trust), how to report privately (GitHub
private vulnerability reporting), and the honest response expectation for a
one-maintainer project.

- [ ] **Step 2: Issue and PR templates shaped around the diagnostics**

A psolve bug report is unactionable without the reason code, the invocation and
the index. `bug_report.yml` asks for: exact command line, `psolve --help`
version banner, the reason code from the JSON, index file and how it was built,
frame dimensions/binning/`BAYERPAT`, and whether `scripts/demo.sh` passes.
`config.yml` points questions at Discussions rather than issues.
`PULL_REQUEST_TEMPLATE.md` carries three checkboxes: suite green with the count
stated, clippy clean, and `cargo fmt` NOT run.

- [ ] **Step 3: `CHANGELOG.md`, keep-a-changelog, seeded at v0.1.0**

One `## [0.1.0]` section. It is the first public release, so it describes what
psolve is rather than what changed, and it links the measurement documents
behind the headline numbers rather than restating them.

- [ ] **Step 4: `dependabot.yml`**

`cargo` and `github-actions` ecosystems, monthly. Three dependencies total, so
the noise floor is near zero.

- [ ] **Step 5: `release.yml` -- a tag builds attachable binaries**

Task 6's `package` job uploads a CI artifact, which expires and is not a
release. On `v*` tags: build on ubuntu and macos, attach both binaries plus the
`.deb` to the GitHub Release. Deliberately no Windows binary attached: it is
cross-compiled and never executed, and attaching an unverified `.exe` to a
release states more confidence than exists.

- [ ] **Step 6: README badges**

CI status, licence, MSRV. Three lines, and the MSRV one must match the measured
`rust-version` from Task 8 rather than being chosen to look modern.

- [ ] **Step 7: Verify and commit**

```bash
cargo test --workspace && cargo clippy --workspace --all-targets
git add -A && git commit -m "chore: SECURITY.md, issue/PR templates, changelog, dependabot, release workflow"
```

---

### Task 9: Branch decision (GATE — needs the user)

Four branches are superseded; **`fix-cfa-double-binning` is not** and must survive:
`main:crates/psolve-core/src/fits.rs:160,223` still bins any `BAYERPAT` frame 2×2
regardless of `XBINNING`.

- [ ] **Step 1: Confirm nothing unique is lost**

```bash
for b in fix-ci-root pair-match-retry spike/pair-voting binning-retry-refetch; do
  echo "== $b: $(git rev-list --count main..$b) ahead"
done
```

- [ ] **Step 2: STOP. Get explicit confirmation before deleting any branch.**

- [ ] **Step 3: Delete only the four, local and remote**

```bash
git branch -D fix-ci-root pair-match-retry spike/pair-voting binning-retry-refetch
git push origin --delete fix-ci-root pair-match-retry blind-solve binning-retry-refetch
git worktree prune
```

- [ ] **Step 4: Prove the survivor survived, locally AND on the remote**

```bash
git rev-parse --verify fix-cfa-double-binning
git ls-remote --heads origin fix-cfa-double-binning     # must print a ref
```
A local branch is one copy. *Commit is not push* -- if the remote has no
`fix-cfa-double-binning`, push it before deleting anything else.

- [ ] **Step 5: Prove GitLab still resolves every cited SHA**

The squash makes this repo the only place the docs' provenance survives, so it is
checked rather than assumed:

```bash
git grep -hoE '`[0-9a-f]{7,40}`' -- '*.md' | tr -d '`' | sort -u | while read -r sha; do
  git cat-file -e "${sha}^{commit}" 2>/dev/null || echo "DANGLING: $sha"
done
```
Expected: no `DANGLING` lines. (Some matches are content digests rather than commits;
investigate any hit rather than assuming which kind it is.)

---

### Task 10: Build the public repo (private on GitHub)

**Files:**
- Create: `~/Projects/github.com/AstroOps-Cloud/psolve/` (a new git repo)

- [ ] **Step 1: Confirm the account before creating anything**

```bash
gh auth status
```
Expected: **Active account: `astroops-cloud`**. `gh`'s active account is global, not
per-directory — creating from `the personal account` produces a private repo in the wrong
place that looks like success.

- [ ] **Step 2: Export only tracked files**

```bash
dest=~/Projects/github.com/AstroOps-Cloud/psolve
mkdir -p "$dest"
git archive main | tar -x -C "$dest"
```

`git archive` emits tracked content only, so the two in-repo worktrees, `target/`
and `.scratch/` cannot ride along through forgetfulness — they were never eligible.

- [ ] **Step 3: Prove the export is clean**

```bash
cd "$dest"
test ! -e .claude/worktrees && test ! -e .worktrees && test ! -e target && test ! -e .scratch && echo "clean"
diff <(cd <dev-repo> && git ls-files) \
     <(find . -type f | sed 's|^\./||' | sort) && echo "tree matches git ls-files"
```

- [ ] **Step 4: One commit, one tag**

```bash
cd "$dest"
git init -b main
git add -A
git -c user.name="NRF" commit -m "psolve v0.1.0: a plate solver in Rust

FITS bytes in, a verified TAN WCS out. One static binary, no runtime, built
for headless automation, and a drop-in replacement for astap_cli's argument
grammar and sidecar formats.

Public history begins here. The pre-release development history is retained
privately; short SHAs cited in docs/ refer to it."
git tag -a v0.1.0 -m "psolve v0.1.0"
git config user.email    # must be neil@asdf.systems, applied by includeIf
```

- [ ] **Step 5: Prove SSH authenticates as the right account**

```bash
ssh -T git@github.com 2>&1 | head -1
```
Expected: `Hi astroops-cloud!`. If it says `Hi the personal account!`, the connection is riding
the other account's control master -- `%C` hashes (host, port, user), which is
identical for both aliases, so without a distinct `ControlPath` this fails *toward*
the personal account while appearing to succeed. Fix `~/.ssh/config` before pushing.

- [ ] **Step 6: Create the PRIVATE repo and push**

```bash
gh repo create astroops-cloud/psolve --private --source=. --remote=origin --push
git push origin v0.1.0
gh repo view astroops-cloud/psolve --json owner,visibility
```
Expected: owner `astroops-cloud`, visibility `PRIVATE`.

- [ ] **Step 7: Commit the plan's completion state in the GitLab repo**

```bash
cd <dev-repo>
git commit -am "docs: record the public export at v0.1.0" || true
```

---

### Task 11: Verify from the clone, then flip (GATE — irreversible)

The working copy's reflogs and unreachable objects make a local clean result
meaningless. **The clone is what a stranger receives, so the clone is what gets
checked.**

- [ ] **Step 1: Clone from GitHub into scratch**

```bash
scratch=$(mktemp -d)
git clone git@github.com:astroops-cloud/psolve.git "$scratch/psolve"
cd "$scratch/psolve"
```

- [ ] **Step 2: Sweep the clone for every token**

```bash
for t in "<home>" "the workstation" "host B" "the capture host" "<LAN>" "the self-hosted GitLab" \
         "the former maintainer address" "<site-lat>" "<site-long>" "<site-elev>"; do
  n=$(git grep -I --fixed-strings -l "$t" -- . | grep -v tree_is_scrubbed | wc -l | tr -d ' ')
  printf "%-20s %s\n" "$t" "$n"
done
```
Expected: `0` on every line. Any non-zero is a stop.

- [ ] **Step 3: Confirm history is one commit and carries nothing extra**

```bash
git rev-list --all --count          # expect 1
git count-objects -vH | grep size-pack
git rev-list --objects --all | wc -l
```

- [ ] **Step 4: Run the suite and the demo from the clone**

```bash
cargo test --workspace
cargo clippy --workspace --all-targets
./scripts/demo.sh
```
Expected: green, clippy clean, demo solves. Report the test count.

- [ ] **Step 5: Confirm CI is green on GitHub before the flip**

```bash
gh run list --repo astroops-cloud/psolve --limit 5
```

- [ ] **Step 6: STOP. This is the irreversible step. Get explicit confirmation.**

- [ ] **Step 7: Flip public**

```bash
gh repo edit astroops-cloud/psolve --visibility public --accept-visibility-change-consequences
```

- [ ] **Step 8: Compute the real Homebrew digest, now that a tarball exists**

```bash
curl -sL https://github.com/astroops-cloud/psolve/archive/v0.1.0.tar.gz | shasum -a 256
```
Replace the deliberately invalid `sha256` in `packaging/homebrew/psolve.rb` with the
real one, commit, push. Until this step the formula refuses to install, which is the
intended behaviour, not a bug.
