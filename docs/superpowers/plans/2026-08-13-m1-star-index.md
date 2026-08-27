# M1: Star Index — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the Gaia DR3 star index that `psolve` solves against — an on-disk format, a builder, an mmap reader, and `psolve index build` / `psolve index info`.

**Architecture:** Stars are packed into 16-byte fixed records, bucketed by HEALPix nested cell (nside=64) and sorted brightest-first *within* each cell. A fetch is therefore "seek to cell, read the first N records, stop". The file is mmap'd and the record region is cast directly to a slice — no parsing at solve time. The build sorts entirely in RAM (a G<16 index is ~3.4 GB against 128 GB available), so no external merge sort is needed.

**Tech Stack:** Rust 2021 (stable), `memmap2`, `rayon`. No astronomy crates — HEALPix is implemented here and validated against Gaia's own encoding.

**Spec:** `docs/superpowers/specs/2026-08-13-psolve-design.md` (this plan implements §5 and the `index` parts of §8)

## Global Constraints

- Rust 2021 edition, stable toolchain. Workspace at repo root, crates under `crates/`.
- **M1 dependencies are exactly `memmap2` and `rayon`.** No FITS crate, no astronomy crate, no CSV crate, no serde. Adding a dependency is a design change, not an implementation detail.
- Record size is **exactly 16 bytes**. Default `nside = 64`. Catalogue epoch is **2016.0** (Gaia DR3 `ref_epoch`).
- **Records are sorted ascending by magnitude within each cell.** Every reader relies on this; it is the whole point of the format.
- **The index is not Gaia-only.** Column names, catalogue epoch, magnitude limit and declination range are all build-time options. Gaia DR3 is the default, not an assumption baked into the code.
- **stdout is results, stderr is logs and progress.** Always.
- Exit codes: `0` success · `1` normal negative outcome · `2` usage/config error · `3` index problem.
- **No test may touch the network.** Fixtures are committed.
- No `unwrap()`/`expect()` on any path that consumes external data (files, CSV, user args). Return typed errors.

---

## Verified facts this plan depends on

Measured on 2026-08-13, not assumed. Do not "correct" these from memory.

**Gaia DR3 bulk source:** `https://cdn.gea.esac.esa.int/Gaia/gdr3/gaia_source/`
- **3,386 files**, `GaiaSource_<hp8start>-<hp8end>.csv.gz`, **701.3 GB gzipped total**.
- Filename indices are HEALPix **level 8** (nside=256, npix=786432; max index observed 786431).
- Directory listing is JS-driven; the machine-readable listing is
  `https://gaia.eu-1.cdn77-storage.com/?prefix=Gaia/gdr3/gaia_source/&delimiter=/`
  (S3-style XML, 1000 keys per page, paginate with `&marker=`).

**File format is ECSV, not plain CSV:**
- **1,000 leading comment lines** starting with `#` (a YAML header).
- The CSV header row is the **first non-`#` line**; data follows.
- **152 columns.** Needed columns, **1-based**: `source_id`=3, `ra`=6, `dec`=8, `pmra`=14, `pmdec`=16, `phot_g_mean_mag`=70.
- Column *positions* are recorded here for orientation only — **the parser must look columns up by name** from the header row, never by hardcoded index.
- `pmra`/`pmdec` can be **empty** (Gaia DR3 has ~340M two-parameter sources with no proper motion). Empty must become 0, not an error. (The sampled file happened to contain none; global data does.)

**Gaia encodes HEALPix in `source_id`** — verified against real rows:
- `hp12 = source_id >> 35` (level 12, nside 4096)
- `hp6  = hp12 >> 12` (level 6, nside 64) — nested scheme, so dropping levels is a right shift of 2 bits per level.
- This gives free ground truth for our own HEALPix implementation. Task 2 exploits it.

**Indicative magnitude distribution** (one file, so varies with galactic latitude; scaled against Gaia DR3's ~1.81 billion sources):

| cut | fraction | ≈ sources | ≈ index size @16 B |
|---|---|---|---|
| G<12 | 1.2% | 22M | 0.35 GB |
| G<14 | 4.2% | 75M | 1.2 GB |
| G<16 | 11.7% | 212M | 3.4 GB |
| G<18 | 28.3% | 511M | 8.2 GB |

**Start with G<14** — smallest useful build, fast to iterate on. Deepening is a rebuild with one flag changed, and the spec says depth is chosen by measurement (§5, §12.1).

**Declination limits cut this further.** A site at latitude φ can never point above declination φ+90°, and less than that once an altitude floor and the frame's own size are accounted for. Both `fetch-gaia.sh` and `index build` take the range. See the rig profile in Task 11 for this rig's worked numbers.

Be honest about the size of this win: a `--max-dec 45` cut removes **14.6%** of the celestial sphere, not a third. It is worth taking — it is free, and those stars can never appear in a frame — but it does not transform the download.

---

## File Structure

| file | responsibility |
|---|---|
| `Cargo.toml` | workspace root, shared profile |
| `crates/psolve-index/src/lib.rs` | public API re-exports only |
| `crates/psolve-index/src/healpix.rs` | `ang2pix_nest`, `pix2ang_nest`, `cells_in_disc` |
| `crates/psolve-index/src/record.rs` | `StarRecord` — pack/unpack the 16 bytes |
| `crates/psolve-index/src/format.rs` | `Header` — encode/decode, magic, offsets |
| `crates/psolve-index/src/builder.rs` | in-memory sort → write index file |
| `crates/psolve-index/src/reader.rs` | mmap, validate, cell slices, brightest-N |
| `crates/psolve-index/src/gaia.rs` | ECSV parsing, column lookup, mag filter |
| `crates/psolve-index/src/error.rs` | `IndexError` |
| `crates/psolve-cli/src/main.rs` | arg dispatch, exit codes |
| `crates/psolve-cli/src/cmd_index.rs` | `index build`, `index info` |
| `crates/psolve-index/tests/fixtures/gaia_healpix.csv` | **already committed** — 144 real Gaia rows, 12 per base face |
| `scripts/fetch-gaia.sh` | stream-and-discard downloader |

---

## Task 1: Workspace scaffold

**Files:**
- Create: `Cargo.toml`, `crates/psolve-index/Cargo.toml`, `crates/psolve-index/src/lib.rs`, `crates/psolve-index/src/error.rs`
- Create: `rust-toolchain.toml`

**Interfaces:**
- Consumes: nothing
- Produces: crate `psolve_index`; `IndexError` enum used by every later task

- [ ] **Step 1: Write the workspace root `Cargo.toml`**

```toml
[workspace]
resolver = "2"
members = ["crates/psolve-index", "crates/psolve-cli"]

[workspace.package]
version = "0.1.0"
edition = "2021"
license = "MIT"

[profile.release]
opt-level = 3
lto = true
codegen-units = 1
```

- [ ] **Step 2: Write `rust-toolchain.toml`**

```toml
[toolchain]
channel = "stable"
components = ["rustfmt", "clippy"]
```

- [ ] **Step 3: Write `crates/psolve-index/Cargo.toml`**

```toml
[package]
name = "psolve-index"
version.workspace = true
edition.workspace = true
license.workspace = true

[dependencies]
memmap2 = "0.9"
rayon = "1"
```

- [ ] **Step 4: Write `crates/psolve-index/src/error.rs`**

```rust
use std::fmt;

#[derive(Debug)]
pub enum IndexError {
    Io(std::io::Error),
    BadMagic,
    UnsupportedVersion(u32),
    Truncated { expected: u64, actual: u64 },
    BadNside(u32),
    ChecksumMismatch,
    MissingColumn(String),
    BadColumnSpec(String),
    BadRange { what: &'static str, reason: String },
    MalformedRow { line: u64, reason: String },
}

impl fmt::Display for IndexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IndexError::Io(e) => write!(f, "io: {e}"),
            IndexError::BadMagic => write!(f, "not a psolve index (bad magic)"),
            IndexError::UnsupportedVersion(v) => write!(f, "unsupported index version {v}"),
            IndexError::Truncated { expected, actual } => {
                write!(f, "index truncated: expected {expected} bytes, found {actual}")
            }
            IndexError::BadNside(n) => write!(f, "nside {n} is not a power of two in 1..=4096"),
            IndexError::ChecksumMismatch => write!(f, "index record checksum mismatch"),
            IndexError::MissingColumn(c) => write!(f, "catalogue csv missing column '{c}'"),
            IndexError::BadColumnSpec(s) => write!(
                f,
                "bad --columns entry '{s}' (expected key=name, key one of \
                 ra,dec,mag,pmra,pmdec,source_id)"
            ),
            IndexError::BadRange { what, reason } => write!(f, "bad {what}: {reason}"),
            IndexError::MalformedRow { line, reason } => {
                write!(f, "catalogue csv line {line}: {reason}")
            }
        }
    }
}

impl std::error::Error for IndexError {}

impl From<std::io::Error> for IndexError {
    fn from(e: std::io::Error) -> Self {
        IndexError::Io(e)
    }
}
```

- [ ] **Step 5: Write `crates/psolve-index/src/lib.rs`**

```rust
//! The psolve star index: HEALPix-bucketed, magnitude-sorted, mmap-friendly.

pub mod error;
pub mod healpix;

pub use error::IndexError;
```

- [ ] **Step 6: Create a placeholder `healpix.rs` so the crate compiles**

```rust
//! HEALPix nested-scheme indexing. Filled in by Task 2.
```

- [ ] **Step 7: Verify it builds**

Run: `cargo build`
Expected: compiles clean (warnings about the empty module are acceptable)

- [ ] **Step 8: Commit**

```bash
git add Cargo.toml rust-toolchain.toml crates/psolve-index
git commit -m "feat(index): workspace scaffold and error type"
```

---

## Task 2: HEALPix ang2pix_nest, validated against Gaia

This is the highest-risk code in M1 — a subtly wrong HEALPix puts stars in the wrong cells and every solve fails in a way that looks like bad data. The fixture makes that impossible to miss: Gaia's own `source_id` is the ground truth.

**Files:**
- Modify: `crates/psolve-index/src/healpix.rs`
- Test: `crates/psolve-index/tests/healpix_gaia.rs`
- Uses: `crates/psolve-index/tests/fixtures/gaia_healpix.csv` (already committed)

**Interfaces:**
- Consumes: `IndexError` (Task 1)
- Produces:
  - `pub fn ang2pix_nest(nside: u32, ra_deg: f64, dec_deg: f64) -> u64`
  - `pub fn npix(nside: u32) -> u64`
  - `pub fn is_valid_nside(nside: u32) -> bool`

- [ ] **Step 1: Write the failing test**

Create `crates/psolve-index/tests/healpix_gaia.rs`:

```rust
use psolve_index::healpix::ang2pix_nest;

/// Gaia DR3 encodes the level-12 nested HEALPix pixel in source_id:
///   hp12 = source_id >> 35,  hp6 = hp12 >> 12
/// The fixture holds 144 real rows, 12 from each of the 12 base faces.
/// If our nested indexing disagrees with Gaia's, this catches it.
fn fixture() -> Vec<(u64, f64, f64, u64, u64)> {
    let raw = include_str!("fixtures/gaia_healpix.csv");
    raw.lines()
        .filter(|l| !l.starts_with('#'))
        .skip(1) // header row
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let f: Vec<&str> = l.split(',').collect();
            (
                f[0].parse().unwrap(),
                f[1].parse().unwrap(),
                f[2].parse().unwrap(),
                f[3].parse().unwrap(),
                f[4].parse().unwrap(),
            )
        })
        .collect()
}

#[test]
fn fixture_is_complete() {
    let rows = fixture();
    assert_eq!(rows.len(), 144, "fixture should hold 144 rows");
    let mut faces: Vec<u64> = rows.iter().map(|r| r.4 / 4096).collect();
    faces.sort_unstable();
    faces.dedup();
    assert_eq!(faces.len(), 12, "fixture must cover all 12 base faces");
}

/// nside=64 is the level we actually build at, and agreement there is exact.
#[test]
fn matches_gaia_level_6_exactly() {
    for (source_id, ra, dec, _, hp6) in fixture() {
        let got = ang2pix_nest(64, ra, dec);
        assert_eq!(
            got, hp6,
            "source_id {source_id} at ra={ra} dec={dec}: got hp6 {got}, Gaia says {hp6}"
        );
    }
}

/// At level 12 a pixel is only ~0.86 arcmin across, and Gaia assigns source_id
/// early in processing without recomputing it when the astrometry is later
/// refined. A source sitting a fraction of an arcsec from a pixel edge can
/// therefore legitimately fall on the other side of it from where its id says.
///
/// Exactly one row of the 144 does this (verified 2026-08-13): the two pixels
/// are adjacent and the star is near-equidistant from both centres. So the
/// assertion is "agrees, or disagrees only by sitting on the boundary" —
/// demanding exact equality here would encode a false expectation.
#[test]
fn matches_gaia_level_12_allowing_boundary_sources() {
    let rows = fixture();
    let exact = rows
        .iter()
        .filter(|(_, ra, dec, hp12, _)| ang2pix_nest(4096, *ra, *dec) == *hp12)
        .count();
    assert!(
        exact >= rows.len() - 1,
        "only {exact}/{} rows matched Gaia exactly at level 12; at most one \
         boundary source is expected. If this drops further, the bug is in \
         ang2pix_nest, not in the fixture.",
        rows.len()
    );
    // That the single mismatch really is a boundary case — adjacent pixel,
    // star equidistant — is proved in tests/healpix_disc.rs once pix2ang_nest
    // exists (Task 3).
}

#[test]
fn nested_levels_are_a_right_shift() {
    // The nested scheme's defining property: dropping one level drops 2 bits.
    // Checked against our OWN indices at both levels — this is a property of
    // the implementation, so involving Gaia here would only import its
    // boundary ambiguity into a test that is not about absolute correctness.
    for (_, ra, dec, _, _) in fixture() {
        let p12 = ang2pix_nest(4096, ra, dec);
        assert_eq!(ang2pix_nest(2048, ra, dec), p12 >> 2);
        assert_eq!(ang2pix_nest(1024, ra, dec), p12 >> 4);
        assert_eq!(ang2pix_nest(64, ra, dec), p12 >> 12);
    }
}

#[test]
fn poles_and_wraparound_do_not_panic() {
    for &nside in &[1u32, 2, 64, 4096] {
        for &(ra, dec) in &[
            (0.0, 90.0), (0.0, -90.0), (359.9999, 0.0), (0.0, 0.0),
            (180.0, 89.9999), (180.0, -89.9999), (360.0, 45.0),
        ] {
            let p = ang2pix_nest(nside, ra, dec);
            assert!(p < psolve_index::healpix::npix(nside), "pix {p} out of range");
        }
    }
}
```

- [ ] **Step 2: Run it to confirm it fails**

Run: `cargo test -p psolve-index --test healpix_gaia`
Expected: FAIL — `ang2pix_nest` does not exist

- [ ] **Step 3: Implement `healpix.rs`**

```rust
//! HEALPix nested-scheme indexing (Górski et al. 2005).
//!
//! Only what psolve needs: point -> pixel, pixel -> centre, and a disc query.
//! Validated against Gaia DR3's own source_id encoding in tests/healpix_gaia.rs,
//! which is real ground truth rather than a reimplementation of our own belief.

use std::f64::consts::PI;

const TWO_THIRDS: f64 = 2.0 / 3.0;

/// Total pixels at this nside. 12 base faces, each nside x nside.
pub fn npix(nside: u32) -> u64 {
    12 * (nside as u64) * (nside as u64)
}

/// nside must be a power of two so that nesting is a bit shift.
pub fn is_valid_nside(nside: u32) -> bool {
    nside >= 1 && nside <= 4096 && nside.is_power_of_two()
}

/// Interleave the low bits of x (even positions) and y (odd positions).
/// This is what makes a nested pixel index a quadtree path.
fn interleave(x: u32, y: u32) -> u64 {
    fn spread(v: u32) -> u64 {
        let mut n = v as u64 & 0x0000_ffff;
        n = (n | (n << 8)) & 0x00ff_00ff;
        n = (n | (n << 4)) & 0x0f0f_0f0f;
        n = (n | (n << 2)) & 0x3333_3333;
        n = (n | (n << 1)) & 0x5555_5555;
        n
    }
    spread(x) | (spread(y) << 1)
}

/// RA/Dec in degrees -> nested pixel index at `nside`.
pub fn ang2pix_nest(nside: u32, ra_deg: f64, dec_deg: f64) -> u64 {
    let order = nside.trailing_zeros();
    let ns = nside as i64;

    // colatitude/longitude, normalised
    let dec = dec_deg.clamp(-90.0, 90.0);
    let theta = (90.0 - dec).to_radians();
    let mut phi = ra_deg.to_radians() % (2.0 * PI);
    if phi < 0.0 {
        phi += 2.0 * PI;
    }

    let z = theta.cos();
    let za = z.abs();
    let tt = (phi / (PI / 2.0)) % 4.0; // in [0,4)

    let (face, ix, iy);
    if za <= TWO_THIRDS {
        // equatorial belt
        let temp1 = ns as f64 * (0.5 + tt);
        let temp2 = ns as f64 * z * 0.75;
        let jp = (temp1 - temp2) as i64; // ascending edge
        let jm = (temp1 + temp2) as i64; // descending edge
        let ifp = jp >> order; // 0..4
        let ifm = jm >> order;
        face = if ifp == ifm {
            (ifp & 3) + 4
        } else if ifp < ifm {
            ifp & 3
        } else {
            (ifm & 3) + 8
        };
        ix = jm & (ns - 1);
        iy = ns - (jp & (ns - 1)) - 1;
    } else {
        // polar caps
        let ntt = (tt as i64).min(3);
        let tp = tt - ntt as f64;
        let tmp = ns as f64 * (3.0 * (1.0 - za)).max(0.0).sqrt();
        let jp = ((tp * tmp) as i64).min(ns - 1);
        let jm = (((1.0 - tp) * tmp) as i64).min(ns - 1);
        if z >= 0.0 {
            face = ntt;
            ix = ns - jm - 1;
            iy = ns - jp - 1;
        } else {
            face = ntt + 8;
            ix = jp;
            iy = jm;
        }
    }

    (face as u64) * (ns as u64) * (ns as u64) + interleave(ix as u32, iy as u32)
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p psolve-index --test healpix_gaia`
Expected: all 5 tests PASS. **Do not adjust the fixture** — it is Gaia's own encoding, and the tests are already written to tolerate the one known boundary source. If `matches_gaia_level_6_exactly` fails, the bug is certainly in `ang2pix_nest`.

- [ ] **Step 5: Export it from lib.rs**

`lib.rs` already has `pub mod healpix;` from Task 1. Confirm `cargo build` is clean.

- [ ] **Step 6: Commit**

```bash
git add crates/psolve-index/src/healpix.rs crates/psolve-index/tests/healpix_gaia.rs
git commit -m "feat(index): healpix ang2pix_nest, validated against Gaia source_id"
```

---

## Task 3: pix2ang_nest and cells_in_disc

**Files:**
- Modify: `crates/psolve-index/src/healpix.rs`
- Test: `crates/psolve-index/tests/healpix_disc.rs`

**Interfaces:**
- Consumes: `ang2pix_nest`, `npix`, `is_valid_nside` (Task 2)
- Produces:
  - `pub fn pix2ang_nest(nside: u32, pix: u64) -> (f64, f64)` returning `(ra_deg, dec_deg)` of the pixel centre
  - `pub fn cells_in_disc(nside: u32, ra_deg: f64, dec_deg: f64, radius_deg: f64) -> Vec<u64>`
  - `pub fn max_pixrad_deg(nside: u32) -> f64`

**Note on `cells_in_disc`:** it is implemented as a brute-force scan over all pixels, comparing each centre against `radius + max_pixrad`. At nside=64 that is 49,152 distance evaluations, well under a millisecond, and it is *correct by construction* — no boundary reasoning to get wrong. The spec's budget (§6.8) allows this; optimise only if profiling says so. Being deliberately simple here is a decision, not an oversight.

- [ ] **Step 1: Write the failing test**

Create `crates/psolve-index/tests/healpix_disc.rs`:

```rust
use psolve_index::healpix::{ang2pix_nest, cells_in_disc, max_pixrad_deg, npix, pix2ang_nest};

fn angsep_deg(ra1: f64, dec1: f64, ra2: f64, dec2: f64) -> f64 {
    let (r1, d1, r2, d2) = (
        ra1.to_radians(), dec1.to_radians(), ra2.to_radians(), dec2.to_radians(),
    );
    let (dr, dd) = (r2 - r1, d2 - d1);
    let a = (dd / 2.0).sin().powi(2) + d1.cos() * d2.cos() * (dr / 2.0).sin().powi(2);
    2.0 * a.sqrt().min(1.0).asin().to_degrees()
}

#[test]
fn pix2ang_round_trips_through_ang2pix() {
    for &nside in &[1u32, 4, 64, 256] {
        for pix in 0..npix(nside) {
            let (ra, dec) = pix2ang_nest(nside, pix);
            assert_eq!(
                ang2pix_nest(nside, ra, dec), pix,
                "nside {nside} pix {pix} centre ({ra},{dec}) did not map back"
            );
        }
    }
}

#[test]
fn disc_contains_the_centre_cell() {
    for &(ra, dec) in &[(0.0, 0.0), (274.689, -13.811), (45.0, 89.0), (200.0, -89.5)] {
        let cells = cells_in_disc(64, ra, dec, 1.5);
        assert!(
            cells.contains(&ang2pix_nest(64, ra, dec)),
            "disc at ({ra},{dec}) omitted its own centre cell"
        );
    }
}

#[test]
fn disc_is_a_superset_of_a_brute_force_point_test() {
    // Every cell whose CENTRE lies within the radius must be returned.
    let (ra, dec, r) = (274.689, -13.811, 2.0);
    let got = cells_in_disc(64, ra, dec, r);
    for pix in 0..npix(64) {
        let (pra, pdec) = pix2ang_nest(64, pix);
        if angsep_deg(ra, dec, pra, pdec) <= r {
            assert!(got.contains(&pix), "cell {pix} inside radius but not returned");
        }
    }
}

#[test]
fn disc_returns_sorted_unique_cells() {
    let cells = cells_in_disc(64, 100.0, 20.0, 3.0);
    let mut sorted = cells.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(cells, sorted, "cells must be sorted and unique");
}

#[test]
fn full_sky_radius_returns_every_cell() {
    assert_eq!(cells_in_disc(4, 0.0, 0.0, 180.0).len() as u64, npix(4));
}

#[test]
fn max_pixrad_shrinks_with_nside() {
    assert!(max_pixrad_deg(64) < max_pixrad_deg(32));
    assert!(max_pixrad_deg(64) > 0.0);
}

#[test]
fn the_gaia_level_12_mismatch_is_a_genuine_boundary_case() {
    // Task 2 established that at most one fixture row disagrees with Gaia at
    // level 12. Now that pix2ang_nest exists, prove that disagreement is a
    // star sitting on a pixel edge rather than a wrong pixel choice.
    let raw = include_str!("fixtures/gaia_healpix.csv");
    let pixel_scale_deg =
        (4.0 * std::f64::consts::PI / npix(4096) as f64).sqrt().to_degrees();

    for line in raw.lines().filter(|l| !l.starts_with('#')).skip(1) {
        if line.trim().is_empty() {
            continue;
        }
        let f: Vec<&str> = line.split(',').collect();
        let (ra, dec): (f64, f64) = (f[1].parse().unwrap(), f[2].parse().unwrap());
        let hp12: u64 = f[3].parse().unwrap();
        let got = ang2pix_nest(4096, ra, dec);
        if got == hp12 {
            continue;
        }
        let (ora, odec) = pix2ang_nest(4096, got);
        let (gra, gdec) = pix2ang_nest(4096, hp12);
        let d_ours = angsep_deg(ra, dec, ora, odec);
        let d_gaia = angsep_deg(ra, dec, gra, gdec);
        let centres = angsep_deg(ora, odec, gra, gdec);
        assert!(
            centres <= 1.5 * pixel_scale_deg,
            "source {}: our pixel and Gaia's are {centres} deg apart — not \
             adjacent, so this is a bug rather than a boundary case",
            f[0]
        );
        assert!(
            (d_ours - d_gaia).abs() < 0.25 * pixel_scale_deg,
            "source {}: star is {d_ours} from our centre but {d_gaia} from \
             Gaia's — not equidistant, so our pixel choice is wrong",
            f[0]
        );
    }
}

#[test]
fn padding_bound_covers_every_point() {
    // The property cells_in_disc depends on: any point is within
    // max_pixrad_deg of the centre of the cell that contains it. If this is
    // ever false, disc queries silently drop stars near the edge.
    for &nside in &[4u32, 64, 256] {
        let bound = max_pixrad_deg(nside);
        let mut worst: f64 = 0.0;
        // deterministic pseudo-random sweep, no rand dependency
        for i in 0..20_000u64 {
            let ra = (i as f64 * 0.618_033_988_749_9 * 360.0) % 360.0;
            let z = ((i as f64 * 0.381_966_011_25) % 2.0) - 1.0;
            let dec = z.asin().to_degrees();
            let pix = ang2pix_nest(nside, ra, dec);
            let (cra, cdec) = pix2ang_nest(nside, pix);
            let sep = angsep_deg(ra, dec, cra, cdec);
            worst = worst.max(sep);
            assert!(
                sep <= bound,
                "nside {nside}: point ({ra},{dec}) is {sep} deg from its cell centre, \
                 bound is {bound}"
            );
        }
        assert!(worst > 0.0, "sweep produced no separations at nside {nside}");
    }
}
```

- [ ] **Step 2: Run it to confirm it fails**

Run: `cargo test -p psolve-index --test healpix_disc`
Expected: FAIL — `pix2ang_nest`, `cells_in_disc`, `max_pixrad_deg` do not exist

- [ ] **Step 3: Implement — append to `healpix.rs`**

```rust
/// Undo the bit interleave: recover (x, y) from a within-face nested index.
fn deinterleave(n: u64) -> (u32, u32) {
    fn compact(v: u64) -> u32 {
        let mut n = v & 0x5555_5555;
        n = (n | (n >> 1)) & 0x3333_3333;
        n = (n | (n >> 2)) & 0x0f0f_0f0f;
        n = (n | (n >> 4)) & 0x00ff_00ff;
        n = (n | (n >> 8)) & 0x0000_ffff;
        n as u32
    }
    (compact(n), compact(n >> 1))
}

/// Face -> ring offset, and face -> phi offset. The canonical HEALPix tables.
const JRLL: [i64; 12] = [2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4];
const JPLL: [i64; 12] = [1, 3, 5, 7, 0, 2, 4, 6, 1, 3, 5, 7];

/// Nested pixel index -> (ra_deg, dec_deg) of the pixel centre.
///
/// Standard ring-based inversion (Górski et al. 2005): recover the ring number
/// `jr` from the face and face-local (ix, iy), then invert the three regimes
/// (north cap / equatorial belt / south cap) in closed form.
pub fn pix2ang_nest(nside: u32, pix: u64) -> (f64, f64) {
    let ns = nside as i64;
    let npface = (ns as u64) * (ns as u64);
    let face = (pix / npface) as usize;
    let (ix, iy) = deinterleave(pix % npface);
    let (ix, iy) = (ix as i64, iy as i64);

    let np = npix(nside) as f64;
    let fact2 = 4.0 / np; // 1 / (3 nside^2)
    let fact1 = (2 * ns) as f64 * fact2; // 2 / (3 nside)

    // Ring number, counted from the north pole: 1 ..= 4*nside-1
    let jr = JRLL[face] * ns - ix - iy - 1;

    let (nr, z, kshift): (i64, f64, i64) = if jr < ns {
        // north polar cap
        let nr = jr;
        (nr, 1.0 - (nr * nr) as f64 * fact2, 0)
    } else if jr > 3 * ns {
        // south polar cap
        let nr = 4 * ns - jr;
        (nr, (nr * nr) as f64 * fact2 - 1.0, 0)
    } else {
        // equatorial belt
        (ns, (2 * ns - jr) as f64 * fact1, (jr - ns) & 1)
    };

    // Longitude index within the ring.
    let mut jp = (JPLL[face] * nr + ix - iy + 1 + kshift) / 2;
    if jp > 4 * ns {
        jp -= 4 * ns;
    }
    if jp < 1 {
        jp += 4 * ns;
    }

    let phi = (jp as f64 - (kshift as f64 + 1.0) * 0.5) * (PI / 2.0 / nr as f64);
    let dec = z.clamp(-1.0, 1.0).asin().to_degrees();
    let ra = phi.to_degrees().rem_euclid(360.0);
    (ra, dec)
}

/// An upper bound on the angular distance from a pixel centre to any point
/// inside that pixel, in degrees.
///
/// Used to pad disc queries so a cell that merely *overlaps* the disc is never
/// missed. It must be an OVER-estimate: an under-estimate silently drops
/// catalogue stars near the disc edge, which would look like a sparse field
/// rather than like a bug. The bound is the radius of a spherical cap of four
/// times the pixel area, which is comfortably larger than the true maximum at
/// every nside and is verified by `padding_bound_covers_every_point`.
pub fn max_pixrad_deg(nside: u32) -> f64 {
    let area = 4.0 * PI / npix(nside) as f64;
    let cap = 4.0 * area;
    let cos_r = 1.0 - cap / (2.0 * PI);
    cos_r.clamp(-1.0, 1.0).acos().to_degrees()
}

/// All nested cells at `nside` that could overlap a disc of `radius_deg`.
/// Brute-force scan: correct by construction, ~50k evaluations at nside=64.
pub fn cells_in_disc(nside: u32, ra_deg: f64, dec_deg: f64, radius_deg: f64) -> Vec<u64> {
    let limit = radius_deg + max_pixrad_deg(nside);
    let (r0, d0) = (ra_deg.to_radians(), dec_deg.to_radians());
    let (sin_d0, cos_d0) = (d0.sin(), d0.cos());
    let mut out = Vec::new();
    for pix in 0..npix(nside) {
        let (pra, pdec) = pix2ang_nest(nside, pix);
        let (pr, pd) = (pra.to_radians(), pdec.to_radians());
        let cos_sep = sin_d0 * pd.sin() + cos_d0 * pd.cos() * (pr - r0).cos();
        if cos_sep.clamp(-1.0, 1.0).acos().to_degrees() <= limit {
            out.push(pix);
        }
    }
    out
}
```

**Implementation note for the engineer:** the `pix2ang_nest` body above sketches two routes and is deliberately left for you to finish *properly*. The reliable approach is: implement the standard `nest2ring` conversion, then the well-documented `ring2ang` inversion (three regimes: north cap, equatorial belt, south cap). Do not ship the sketch. The `pix2ang_round_trips_through_ang2pix` test is the arbiter — it checks every pixel at four nsides against the already-validated forward transform, so a correct implementation is unambiguous and a wrong one cannot pass.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p psolve-index --test healpix_disc`
Expected: all 8 PASS. Two are the demanding ones: `pix2ang_round_trips_through_ang2pix` exercises every pixel at four nsides (12 + 192 + 49,152 + 786,432), and `padding_bound_covers_every_point` checks the disc-query padding property that `cells_in_disc` silently depends on. If the round-trip fails, the bug is in `pix2ang_nest` — `ang2pix_nest` was already validated against Gaia in Task 2, so it is the trustworthy half.

- [ ] **Step 5: Commit**

```bash
git add crates/psolve-index/src/healpix.rs crates/psolve-index/tests/healpix_disc.rs
git commit -m "feat(index): pix2ang_nest and cells_in_disc"
```

---

## Task 4: StarRecord — the 16-byte packing

**Files:**
- Create: `crates/psolve-index/src/record.rs`
- Modify: `crates/psolve-index/src/lib.rs` (add `pub mod record;`)
- Test: inline `#[cfg(test)]` in `record.rs`

**Interfaces:**
- Consumes: nothing
- Produces:
  - `pub const RECORD_BYTES: usize = 16;`
  - `pub struct StarRecord { pub ra_scaled: u32, pub dec_scaled: i32, pub mag_milli: i16, pub pmra_mas: i16, pub pmdec_mas: i16 }`
  - `pub fn pack(ra_deg: f64, dec_deg: f64, mag: f32, pmra: f32, pmdec: f32) -> (StarRecord, bool)` — bool is `true` if any field was clamped
  - `impl StarRecord { pub fn ra_deg(&self) -> f64; pub fn dec_deg(&self) -> f64; pub fn mag(&self) -> f32; pub fn pmra_mas_yr(&self) -> f32; pub fn pmdec_mas_yr(&self) -> f32; pub fn to_bytes(&self) -> [u8; 16]; pub fn from_bytes(b: &[u8; 16]) -> Self }`

- [ ] **Step 1: Write `record.rs` with its failing tests**

```rust
//! The 16-byte star record. Fixed width and little-endian so the mmap'd
//! record region can be cast to a slice with no parsing at solve time.
//!
//! Layout:
//!   0..4   ra_scaled   u32   ra_deg / 360 * 2^32      (~0.3 mas, wraps)
//!   4..8   dec_scaled  i32   dec_deg / 90 * i32::MAX  (~0.15 mas, saturates)
//!   8..10  mag_milli   i16   G magnitude * 1000
//!  10..12  pmra_mas    i16   pmRA*  mas/yr
//!  12..14  pmdec_mas   i16   pmDec  mas/yr
//!  14..16  reserved

pub const RECORD_BYTES: usize = 16;

const RA_SCALE: f64 = 4_294_967_296.0 / 360.0; // 2^32 / 360
// i32::MAX / 90, NOT 2^31/90: at dec = +90 the latter computes exactly 2^31,
// which does not fit i32, and the cast wraps to i32::MIN -- decoding back as
// -90. Any row with dec >= 90 clamps to 90.0 and lands in the wrong
// hemisphere. Precision is unchanged (~0.15 mas per LSB).
const DEC_SCALE: f64 = 2_147_483_647.0 / 90.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StarRecord {
    pub ra_scaled: u32,
    pub dec_scaled: i32,
    pub mag_milli: i16,
    pub pmra_mas: i16,
    pub pmdec_mas: i16,
}

/// Pack a Gaia row. Returns the record and whether any field was clamped.
/// Clamping is counted rather than rejected: a star with absurd proper motion
/// is still a usable position, and a silent drop would skew the catalogue.
pub fn pack(ra_deg: f64, dec_deg: f64, mag: f32, pmra: f32, pmdec: f32) -> (StarRecord, bool) {
    let mut clamped = false;

    let ra = ra_deg.rem_euclid(360.0);
    let ra_scaled = (ra * RA_SCALE).round() as u64 as u32;

    let dec = dec_deg.clamp(-90.0, 90.0);
    if dec != dec_deg {
        clamped = true;
    }
    // Saturating, unlike RA's wrap: declination is not cyclic, so an
    // out-of-range value must land at the near pole, never the far one.
    let dec_scaled = (dec * DEC_SCALE)
        .round()
        .clamp(i32::MIN as f64, i32::MAX as f64) as i32;

    let m = (mag * 1000.0).round();
    if m < i16::MIN as f32 || m > i16::MAX as f32 {
        clamped = true;
    }
    let mag_milli = m.clamp(i16::MIN as f32, i16::MAX as f32) as i16;

    let clamp_pm = |v: f32, clamped: &mut bool| -> i16 {
        // NaN means "no proper motion measured" -- Gaia DR3 has ~340M
        // two-parameter sources -- and is a silent zero. An infinity is
        // corrupt rather than missing, so it falls through to be clamped
        // and counted like any other absurd value.
        if v.is_nan() {
            return 0;
        }
        let r = v.round();
        if r < i16::MIN as f32 || r > i16::MAX as f32 {
            *clamped = true;
        }
        r.clamp(i16::MIN as f32, i16::MAX as f32) as i16
    };
    let pmra_mas = clamp_pm(pmra, &mut clamped);
    let pmdec_mas = clamp_pm(pmdec, &mut clamped);

    (StarRecord { ra_scaled, dec_scaled, mag_milli, pmra_mas, pmdec_mas }, clamped)
}

impl StarRecord {
    pub fn ra_deg(&self) -> f64 {
        self.ra_scaled as f64 / RA_SCALE
    }
    pub fn dec_deg(&self) -> f64 {
        self.dec_scaled as f64 / DEC_SCALE
    }
    pub fn mag(&self) -> f32 {
        self.mag_milli as f32 / 1000.0
    }
    pub fn pmra_mas_yr(&self) -> f32 {
        self.pmra_mas as f32
    }
    pub fn pmdec_mas_yr(&self) -> f32 {
        self.pmdec_mas as f32
    }

    pub fn to_bytes(&self) -> [u8; RECORD_BYTES] {
        let mut b = [0u8; RECORD_BYTES];
        b[0..4].copy_from_slice(&self.ra_scaled.to_le_bytes());
        b[4..8].copy_from_slice(&self.dec_scaled.to_le_bytes());
        b[8..10].copy_from_slice(&self.mag_milli.to_le_bytes());
        b[10..12].copy_from_slice(&self.pmra_mas.to_le_bytes());
        b[12..14].copy_from_slice(&self.pmdec_mas.to_le_bytes());
        b
    }

    pub fn from_bytes(b: &[u8; RECORD_BYTES]) -> Self {
        StarRecord {
            ra_scaled: u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            dec_scaled: i32::from_le_bytes([b[4], b[5], b[6], b[7]]),
            mag_milli: i16::from_le_bytes([b[8], b[9]]),
            pmra_mas: i16::from_le_bytes([b[10], b[11]]),
            pmdec_mas: i16::from_le_bytes([b[12], b[13]]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_is_exactly_sixteen_bytes() {
        assert_eq!(RECORD_BYTES, 16);
        let (r, _) = pack(0.0, 0.0, 0.0, 0.0, 0.0);
        assert_eq!(r.to_bytes().len(), 16);
    }

    #[test]
    fn position_round_trips_within_a_milliarcsecond() {
        for &(ra, dec) in &[
            (0.0, 0.0), (274.689087, -13.810971), (359.9999999, 89.9999),
            (180.0, -89.9999), (45.0, 45.0),
        ] {
            let (r, _) = pack(ra, dec, 12.0, 0.0, 0.0);
            assert!((r.ra_deg() - ra).abs() * 3_600_000.0 < 1.0, "ra {ra} -> {}", r.ra_deg());
            assert!((r.dec_deg() - dec).abs() * 3_600_000.0 < 1.0, "dec {dec} -> {}", r.dec_deg());
        }
    }

    #[test]
    fn magnitude_round_trips_to_a_millimag() {
        for &m in &[-1.44f32, 0.0, 6.0, 12.3456, 17.641426, 21.0] {
            let (r, clamped) = pack(0.0, 0.0, m, 0.0, 0.0);
            assert!(!clamped, "mag {m} should not clamp");
            assert!((r.mag() - m).abs() < 0.001, "mag {m} -> {}", r.mag());
        }
    }

    #[test]
    fn proper_motion_round_trips_and_covers_barnards_star() {
        // Barnard's Star has the largest known proper motion, ~10.4 arcsec/yr.
        let (r, clamped) = pack(0.0, 0.0, 9.5, -798.58, 10328.12);
        assert!(!clamped, "Barnard's Star must fit without clamping");
        assert_eq!(r.pmra_mas_yr(), -799.0);
        assert_eq!(r.pmdec_mas_yr(), 10328.0);
    }

    #[test]
    fn absurd_proper_motion_clamps_and_reports() {
        let (_, clamped) = pack(0.0, 0.0, 9.5, 99_000.0, 0.0);
        assert!(clamped, "out-of-range pm must set the clamped flag");
    }

    #[test]
    fn missing_proper_motion_becomes_zero_not_nan() {
        let (r, clamped) = pack(10.0, 20.0, 15.0, f32::NAN, f32::NAN);
        assert!(!clamped);
        assert_eq!(r.pmra_mas_yr(), 0.0);
        assert_eq!(r.pmdec_mas_yr(), 0.0);
    }

    #[test]
    fn bytes_round_trip_exactly() {
        let (r, _) = pack(274.689087, -13.810971, 14.25, -12.5, 33.75);
        assert_eq!(StarRecord::from_bytes(&r.to_bytes()), r);
    }

    #[test]
    fn byte_layout_is_little_endian() {
        // Round-tripping alone would still pass if both sides swapped to
        // big-endian together, and little-endian is an on-disk contract.
        let r = StarRecord { ra_scaled: 1, dec_scaled: 0, mag_milli: 0, pmra_mas: 0, pmdec_mas: 0 };
        assert_eq!(&r.to_bytes()[0..4], &[1, 0, 0, 0]);
    }

    #[test]
    fn the_poles_do_not_flip_hemisphere() {
        let (n, _) = pack(0.0, 90.0, 10.0, 0.0, 0.0);
        assert!((n.dec_deg() - 90.0).abs() < 1e-6, "north pole decoded as {}", n.dec_deg());
        let (s, _) = pack(0.0, -90.0, 10.0, 0.0, 0.0);
        assert!((s.dec_deg() + 90.0).abs() < 1e-6, "south pole decoded as {}", s.dec_deg());
    }

    #[test]
    fn out_of_range_declination_clamps_to_the_near_pole_not_the_far_one() {
        let (r, clamped) = pack(0.0, 999.0, 10.0, 0.0, 0.0);
        assert!(clamped);
        assert!(r.dec_deg() > 89.0, "dec 999 should clamp toward +90, got {}", r.dec_deg());
    }

    #[test]
    fn ra_wraps_rather_than_overflowing() {
        let (a, _) = pack(360.0, 0.0, 10.0, 0.0, 0.0);
        let (b, _) = pack(0.0, 0.0, 10.0, 0.0, 0.0);
        assert_eq!(a.ra_scaled, b.ra_scaled);
    }
}
```

- [ ] **Step 2: Add the module to `lib.rs`**

```rust
pub mod record;
```

- [ ] **Step 3: Run the tests**

Run: `cargo test -p psolve-index record`
Expected: all 11 PASS

- [ ] **Step 4: Commit**

```bash
git add crates/psolve-index/src/record.rs crates/psolve-index/src/lib.rs
git commit -m "feat(index): 16-byte StarRecord with clamping counters"
```

---

## Task 5: File format header

**Files:**
- Create: `crates/psolve-index/src/format.rs`
- Modify: `crates/psolve-index/src/lib.rs`
- Test: inline `#[cfg(test)]`

**Interfaces:**
- Consumes: `IndexError` (Task 1), `RECORD_BYTES` (Task 4)
- Produces:
  - `pub const MAGIC: [u8; 8] = *b"PSIDX\0\0\0";`
  - `pub const FORMAT_VERSION: u32 = 1;`
  - `pub const HEADER_BYTES: usize = 128;`
  - `pub const RECORD_ALIGN: u64 = 4096;`
  - `pub struct Header { pub version: u32, pub nside: u32, pub epoch: f64, pub n_records: u64, pub mag_limit: f32, pub records_offset: u64, pub records_sha256: [u8; 32], pub name: [u8; 32] }`
  - `impl Header { pub fn to_bytes(&self) -> [u8; 128]; pub fn from_bytes(b: &[u8]) -> Result<Header, IndexError>; pub fn cell_table_offset() -> u64; pub fn cell_table_bytes(nside: u32) -> u64; pub fn name_str(&self) -> &str }`

- [ ] **Step 1: Write `format.rs` with its failing tests**

```rust
//! On-disk layout:
//!   0..128                      Header
//!   128..128+8*(npix+1)         cell offset table (u64 LE, n+1 entries;
//!                               cell i occupies records [tab[i], tab[i+1]) )
//!   records_offset..            records, 16 bytes each, 4096-aligned so the
//!                               mmap'd region is page-aligned and castable

use crate::error::IndexError;

pub const MAGIC: [u8; 8] = *b"PSIDX\0\0\0";
pub const FORMAT_VERSION: u32 = 1;
pub const HEADER_BYTES: usize = 128;
pub const RECORD_ALIGN: u64 = 4096;

#[derive(Debug, Clone, PartialEq)]
pub struct Header {
    pub version: u32,
    pub nside: u32,
    pub epoch: f64,
    pub n_records: u64,
    pub mag_limit: f32,
    pub records_offset: u64,
    pub records_sha256: [u8; 32],
    pub name: [u8; 32],
}

impl Header {
    pub fn cell_table_offset() -> u64 {
        HEADER_BYTES as u64
    }

    /// npix + 1 entries, so every cell has an explicit end.
    pub fn cell_table_bytes(nside: u32) -> u64 {
        (crate::healpix::npix(nside) + 1) * 8
    }

    /// Where records begin: after the cell table, rounded up to RECORD_ALIGN.
    pub fn records_offset_for(nside: u32) -> u64 {
        let end = Self::cell_table_offset() + Self::cell_table_bytes(nside);
        end.div_ceil(RECORD_ALIGN) * RECORD_ALIGN
    }

    pub fn name_str(&self) -> &str {
        let end = self.name.iter().position(|&c| c == 0).unwrap_or(self.name.len());
        std::str::from_utf8(&self.name[..end]).unwrap_or("")
    }

    pub fn to_bytes(&self) -> [u8; HEADER_BYTES] {
        let mut b = [0u8; HEADER_BYTES];
        b[0..8].copy_from_slice(&MAGIC);
        b[8..12].copy_from_slice(&self.version.to_le_bytes());
        b[12..16].copy_from_slice(&self.nside.to_le_bytes());
        b[16..24].copy_from_slice(&self.epoch.to_le_bytes());
        b[24..32].copy_from_slice(&self.n_records.to_le_bytes());
        b[32..36].copy_from_slice(&self.mag_limit.to_le_bytes());
        b[36..44].copy_from_slice(&self.records_offset.to_le_bytes());
        b[44..76].copy_from_slice(&self.records_sha256);
        b[76..108].copy_from_slice(&self.name);
        b
    }

    pub fn from_bytes(b: &[u8]) -> Result<Header, IndexError> {
        if b.len() < HEADER_BYTES {
            return Err(IndexError::Truncated {
                expected: HEADER_BYTES as u64,
                actual: b.len() as u64,
            });
        }
        if b[0..8] != MAGIC {
            return Err(IndexError::BadMagic);
        }
        let version = u32::from_le_bytes([b[8], b[9], b[10], b[11]]);
        if version != FORMAT_VERSION {
            return Err(IndexError::UnsupportedVersion(version));
        }
        let nside = u32::from_le_bytes([b[12], b[13], b[14], b[15]]);
        if !crate::healpix::is_valid_nside(nside) {
            return Err(IndexError::BadNside(nside));
        }
        let mut sha = [0u8; 32];
        sha.copy_from_slice(&b[44..76]);
        let mut name = [0u8; 32];
        name.copy_from_slice(&b[76..108]);
        Ok(Header {
            version,
            nside,
            epoch: f64::from_le_bytes(b[16..24].try_into().unwrap_or([0; 8])),
            n_records: u64::from_le_bytes(b[24..32].try_into().unwrap_or([0; 8])),
            mag_limit: f32::from_le_bytes(b[32..36].try_into().unwrap_or([0; 4])),
            records_offset: u64::from_le_bytes(b[36..44].try_into().unwrap_or([0; 8])),
            records_sha256: sha,
            name,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Header {
        let mut name = [0u8; 32];
        name[..20].copy_from_slice(b"gaia-dr3-g14-nside64");
        Header {
            version: FORMAT_VERSION,
            nside: 64,
            epoch: 2016.0,
            n_records: 75_000_000,
            mag_limit: 14.0,
            records_offset: Header::records_offset_for(64),
            records_sha256: [7u8; 32],
            name,
        }
    }

    #[test]
    fn header_round_trips() {
        let h = sample();
        assert_eq!(Header::from_bytes(&h.to_bytes()).unwrap(), h);
    }

    #[test]
    fn header_is_exactly_128_bytes() {
        assert_eq!(sample().to_bytes().len(), 128);
    }

    #[test]
    fn rejects_bad_magic() {
        let mut b = sample().to_bytes();
        b[0] = b'X';
        assert!(matches!(Header::from_bytes(&b), Err(IndexError::BadMagic)));
    }

    #[test]
    fn rejects_future_version() {
        let mut h = sample();
        h.version = 99;
        let b = h.to_bytes();
        assert!(matches!(Header::from_bytes(&b), Err(IndexError::UnsupportedVersion(99))));
    }

    #[test]
    fn rejects_non_power_of_two_nside() {
        let mut h = sample();
        h.nside = 63;
        let b = h.to_bytes();
        assert!(matches!(Header::from_bytes(&b), Err(IndexError::BadNside(63))));
    }

    #[test]
    fn rejects_truncated_header() {
        let b = sample().to_bytes();
        assert!(matches!(
            Header::from_bytes(&b[..60]),
            Err(IndexError::Truncated { .. })
        ));
    }

    #[test]
    fn records_are_page_aligned() {
        for &nside in &[1u32, 64, 256, 4096] {
            assert_eq!(Header::records_offset_for(nside) % RECORD_ALIGN, 0);
            assert!(
                Header::records_offset_for(nside)
                    >= Header::cell_table_offset() + Header::cell_table_bytes(nside)
            );
        }
    }

    #[test]
    fn name_str_strips_nul_padding() {
        assert_eq!(sample().name_str(), "gaia-dr3-g14-nside64");
    }
}
```

- [ ] **Step 2: Add `pub mod format;` to `lib.rs`**

- [ ] **Step 3: Run the tests**

Run: `cargo test -p psolve-index format`
Expected: all 8 PASS

- [ ] **Step 4: Commit**

```bash
git add crates/psolve-index/src/format.rs crates/psolve-index/src/lib.rs
git commit -m "feat(index): on-disk header and layout constants"
```

---

## Task 6: Builder

**Files:**
- Create: `crates/psolve-index/src/builder.rs`, `crates/psolve-index/src/sha256.rs`
- Modify: `crates/psolve-index/src/lib.rs`
- Test: `crates/psolve-index/tests/builder.rs`

**Interfaces:**
- Consumes: `StarRecord`, `pack` (Task 4); `Header`, `records_offset_for` (Task 5); `ang2pix_nest` (Task 2)
- Produces:
  - `pub struct BuildStats { pub written: u64, pub clamped: u64, pub skipped: u64 }`
  - `pub struct Builder { .. }`
  - `impl Builder { pub fn new(nside: u32, mag_limit: f32, epoch: f64, name: &str) -> Result<Builder, IndexError>; pub fn push(&mut self, ra_deg: f64, dec_deg: f64, mag: f32, pmra: f32, pmdec: f32); pub fn finish<W: Write + Seek>(self, out: &mut W) -> Result<BuildStats, IndexError> }`
  - `pub fn sha256(data: &[u8]) -> [u8; 32]` (in `sha256.rs`)

**Note:** `sha256.rs` is a from-scratch FIPS-180-4 implementation (~80 lines) because the Global Constraints allow only `memmap2` and `rayon`. It is verified against the three standard NIST test vectors.

- [ ] **Step 1: Write `sha256.rs` with NIST vectors**

```rust
//! Minimal SHA-256 (FIPS 180-4). Present because the index header stores a
//! record-region digest and M1's dependency budget is memmap2 + rayon only.

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

#[derive(Clone)]
pub struct Sha256 {
    h: [u32; 8],
    buf: [u8; 64],
    buflen: usize,
    total: u64,
}

impl Default for Sha256 {
    fn default() -> Self {
        Self::new()
    }
}

impl Sha256 {
    pub fn new() -> Self {
        Sha256 {
            h: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
                0x5be0cd19,
            ],
            buf: [0u8; 64],
            buflen: 0,
            total: 0,
        }
    }

    fn compress(&mut self, block: &[u8; 64]) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                block[4 * i], block[4 * i + 1], block[4 * i + 2], block[4 * i + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let (mut a, mut b, mut c, mut d) = (self.h[0], self.h[1], self.h[2], self.h[3]);
        let (mut e, mut f, mut g, mut hh) = (self.h[4], self.h[5], self.h[6], self.h[7]);
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g; g = f; f = e;
            e = d.wrapping_add(t1);
            d = c; c = b; b = a;
            a = t1.wrapping_add(t2);
        }
        for (i, v) in [a, b, c, d, e, f, g, hh].iter().enumerate() {
            self.h[i] = self.h[i].wrapping_add(*v);
        }
    }

    pub fn update(&mut self, mut data: &[u8]) {
        self.total = self.total.wrapping_add(data.len() as u64);
        if self.buflen > 0 {
            let take = (64 - self.buflen).min(data.len());
            self.buf[self.buflen..self.buflen + take].copy_from_slice(&data[..take]);
            self.buflen += take;
            data = &data[take..];
            if self.buflen == 64 {
                let b = self.buf;
                self.compress(&b);
                self.buflen = 0;
            }
        }
        while data.len() >= 64 {
            let mut b = [0u8; 64];
            b.copy_from_slice(&data[..64]);
            self.compress(&b);
            data = &data[64..];
        }
        if !data.is_empty() {
            self.buf[..data.len()].copy_from_slice(data);
            self.buflen = data.len();
        }
    }

    pub fn finalize(mut self) -> [u8; 32] {
        let bits = self.total.wrapping_mul(8);
        self.update(&[0x80]);
        // update() bumped total; undo it for the length field already captured
        while self.buflen != 56 {
            self.update(&[0x00]);
        }
        let mut b = self.buf;
        b[56..64].copy_from_slice(&bits.to_be_bytes());
        self.compress(&b);
        let mut out = [0u8; 32];
        for (i, v) in self.h.iter().enumerate() {
            out[4 * i..4 * i + 4].copy_from_slice(&v.to_be_bytes());
        }
        out
    }
}

pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut s = Sha256::new();
    s.update(data);
    s.finalize()
}

pub fn hex(d: &[u8; 32]) -> String {
    d.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nist_empty_string() {
        assert_eq!(
            hex(&sha256(b"")),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn nist_abc() {
        assert_eq!(
            hex(&sha256(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn nist_two_block() {
        assert_eq!(
            hex(&sha256(
                b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
            )),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }

    #[test]
    fn streaming_matches_one_shot() {
        let data: Vec<u8> = (0..1000u32).map(|i| (i % 251) as u8).collect();
        let mut s = Sha256::new();
        for chunk in data.chunks(7) {
            s.update(chunk);
        }
        assert_eq!(s.finalize(), sha256(&data));
    }
}
```

- [ ] **Step 2: Run the SHA-256 tests**

Run: `cargo test -p psolve-index sha256`
Expected: 4 PASS. If `nist_empty_string` fails, the padding logic in `finalize` is wrong — fix it there, the vectors are correct.

- [ ] **Step 3: Write the failing builder test**

Create `crates/psolve-index/tests/builder.rs`:

```rust
use psolve_index::builder::Builder;
use psolve_index::format::{Header, RECORD_ALIGN};
use psolve_index::healpix::{ang2pix_nest, npix};
use psolve_index::record::{StarRecord, RECORD_BYTES};
use std::io::Cursor;

fn build(nside: u32, stars: &[(f64, f64, f32)]) -> Vec<u8> {
    let mut b = Builder::new(nside, 20.0, 2016.0, "test-index").unwrap();
    for &(ra, dec, mag) in stars {
        b.push(ra, dec, mag, 0.0, 0.0);
    }
    let mut buf = Cursor::new(Vec::new());
    b.finish(&mut buf).unwrap();
    buf.into_inner()
}

fn read_cell_table(bytes: &[u8], nside: u32) -> Vec<u64> {
    let off = Header::cell_table_offset() as usize;
    let n = (npix(nside) + 1) as usize;
    (0..n)
        .map(|i| {
            let s = off + i * 8;
            u64::from_le_bytes(bytes[s..s + 8].try_into().unwrap())
        })
        .collect()
}

fn cell_records(bytes: &[u8], nside: u32, cell: u64) -> Vec<StarRecord> {
    let h = Header::from_bytes(bytes).unwrap();
    let tab = read_cell_table(bytes, nside);
    let (a, b) = (tab[cell as usize], tab[cell as usize + 1]);
    (a..b)
        .map(|i| {
            let s = (h.records_offset + i * RECORD_BYTES as u64) as usize;
            StarRecord::from_bytes(bytes[s..s + RECORD_BYTES].try_into().unwrap())
        })
        .collect()
}

#[test]
fn header_reports_what_was_written() {
    let bytes = build(4, &[(10.0, 10.0, 12.0), (20.0, 20.0, 13.0)]);
    let h = Header::from_bytes(&bytes).unwrap();
    assert_eq!(h.n_records, 2);
    assert_eq!(h.nside, 4);
    assert_eq!(h.epoch, 2016.0);
    assert_eq!(h.name_str(), "test-index");
    assert_eq!(h.records_offset % RECORD_ALIGN, 0);
}

#[test]
fn cell_table_is_monotonic_and_spans_every_record() {
    let stars: Vec<(f64, f64, f32)> = (0..500)
        .map(|i| (i as f64 * 0.7 % 360.0, (i as f64 * 0.37 % 170.0) - 85.0, 10.0 + (i % 9) as f32))
        .collect();
    let bytes = build(8, &stars);
    let tab = read_cell_table(&bytes, 8);
    assert_eq!(tab[0], 0);
    assert_eq!(*tab.last().unwrap(), 500);
    for w in tab.windows(2) {
        assert!(w[1] >= w[0], "cell table must be non-decreasing");
    }
}

#[test]
fn every_star_lands_in_its_own_healpix_cell() {
    let stars: Vec<(f64, f64, f32)> = (0..300)
        .map(|i| (i as f64 * 1.13 % 360.0, (i as f64 * 0.61 % 176.0) - 88.0, 12.0))
        .collect();
    let bytes = build(8, &stars);
    for &(ra, dec, _) in &stars {
        let cell = ang2pix_nest(8, ra, dec);
        let recs = cell_records(&bytes, 8, cell);
        assert!(
            recs.iter().any(|r| (r.ra_deg() - ra).abs() < 1e-4 && (r.dec_deg() - dec).abs() < 1e-4),
            "star ({ra},{dec}) missing from cell {cell}"
        );
    }
}

#[test]
fn records_are_sorted_brightest_first_within_each_cell() {
    // All in one small patch so they share cells, with magnitudes out of order.
    let stars: Vec<(f64, f64, f32)> = (0..200)
        .map(|i| (100.0 + (i % 10) as f64 * 0.01, 20.0 + (i / 10) as f64 * 0.01,
                  20.0 - (i * 7 % 200) as f32 * 0.05))
        .collect();
    let bytes = build(64, &stars);
    let tab = read_cell_table(&bytes, 64);
    let mut checked = 0;
    for cell in 0..npix(64) {
        if tab[cell as usize + 1] - tab[cell as usize] < 2 {
            continue;
        }
        let recs = cell_records(&bytes, 64, cell);
        for w in recs.windows(2) {
            assert!(w[0].mag_milli <= w[1].mag_milli, "cell {cell} not brightest-first");
        }
        checked += 1;
    }
    assert!(checked > 0, "test data produced no multi-star cell");
}

#[test]
fn record_digest_matches_the_record_region() {
    let bytes = build(4, &[(1.0, 2.0, 11.0), (3.0, 4.0, 12.0)]);
    let h = Header::from_bytes(&bytes).unwrap();
    let start = h.records_offset as usize;
    let end = start + (h.n_records as usize) * RECORD_BYTES;
    assert_eq!(h.records_sha256, psolve_index::sha256::sha256(&bytes[start..end]));
}

#[test]
fn empty_build_is_valid() {
    let bytes = build(4, &[]);
    let h = Header::from_bytes(&bytes).unwrap();
    assert_eq!(h.n_records, 0);
    let tab = read_cell_table(&bytes, 4);
    assert!(tab.iter().all(|&v| v == 0));
}

#[test]
fn rejects_invalid_nside() {
    assert!(Builder::new(63, 20.0, 2016.0, "x").is_err());
}
```

- [ ] **Step 4: Run it to confirm it fails**

Run: `cargo test -p psolve-index --test builder`
Expected: FAIL — `Builder` does not exist

- [ ] **Step 5: Implement `builder.rs`**

```rust
//! Builds an index in memory, then writes it.
//!
//! The whole catalogue is sorted in RAM: a G<16 build is ~212M records
//! (~3.4 GB) against 128 GB available, so an external merge sort would be
//! complexity bought for no reason. If a future build does not fit, that is a
//! design change, not a tuning knob.

use crate::error::IndexError;
use crate::format::{Header, FORMAT_VERSION};
use crate::healpix::{ang2pix_nest, is_valid_nside, npix};
use crate::record::{pack, StarRecord, RECORD_BYTES};
use crate::sha256::Sha256;
use std::io::{Seek, SeekFrom, Write};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct BuildStats {
    pub written: u64,
    pub clamped: u64,
    pub skipped: u64,
}

pub struct Builder {
    nside: u32,
    mag_limit: f32,
    epoch: f64,
    name: [u8; 32],
    /// (cell, record) pairs, sorted at finish().
    rows: Vec<(u64, StarRecord)>,
    stats: BuildStats,
}

impl Builder {
    pub fn new(nside: u32, mag_limit: f32, epoch: f64, name: &str) -> Result<Builder, IndexError> {
        if !is_valid_nside(nside) {
            return Err(IndexError::BadNside(nside));
        }
        let mut n = [0u8; 32];
        let src = name.as_bytes();
        let take = src.len().min(32);
        n[..take].copy_from_slice(&src[..take]);
        Ok(Builder {
            nside,
            mag_limit,
            epoch,
            name: n,
            rows: Vec::new(),
            stats: BuildStats::default(),
        })
    }

    pub fn push(&mut self, ra_deg: f64, dec_deg: f64, mag: f32, pmra: f32, pmdec: f32) {
        if !ra_deg.is_finite() || !dec_deg.is_finite() || !mag.is_finite() {
            self.stats.skipped += 1;
            return;
        }
        let cell = ang2pix_nest(self.nside, ra_deg, dec_deg);
        let (rec, clamped) = pack(ra_deg, dec_deg, mag, pmra, pmdec);
        if clamped {
            self.stats.clamped += 1;
        }
        self.rows.push((cell, rec));
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn finish<W: Write + Seek>(mut self, out: &mut W) -> Result<BuildStats, IndexError> {
        // Sort by cell, then by magnitude ascending: brightest first within a cell.
        // This ordering IS the format's contract; every reader depends on it.
        self.rows.sort_unstable_by(|a, b| {
            a.0.cmp(&b.0).then(a.1.mag_milli.cmp(&b.1.mag_milli))
        });

        let n = self.rows.len() as u64;
        let npix = npix(self.nside);
        let records_offset = Header::records_offset_for(self.nside);

        // Cell table: npix+1 offsets, so cell i is [tab[i], tab[i+1]).
        let mut tab = vec![0u64; (npix + 1) as usize];
        for (cell, _) in &self.rows {
            tab[(*cell + 1) as usize] += 1;
        }
        for i in 1..tab.len() {
            tab[i] += tab[i - 1];
        }

        // Records, contiguous and already in final order.
        let mut records = Vec::with_capacity(self.rows.len() * RECORD_BYTES);
        let mut digest = Sha256::new();
        for (_, rec) in &self.rows {
            let b = rec.to_bytes();
            digest.update(&b);
            records.extend_from_slice(&b);
        }

        let header = Header {
            version: FORMAT_VERSION,
            nside: self.nside,
            epoch: self.epoch,
            n_records: n,
            mag_limit: self.mag_limit,
            records_offset,
            records_sha256: digest.finalize(),
            name: self.name,
        };

        out.write_all(&header.to_bytes())?;
        for v in &tab {
            out.write_all(&v.to_le_bytes())?;
        }
        let written_so_far = Header::cell_table_offset() + (tab.len() as u64) * 8;
        let pad = records_offset - written_so_far;
        out.write_all(&vec![0u8; pad as usize])?;
        out.write_all(&records)?;
        out.seek(SeekFrom::Start(0))?;

        self.stats.written = n;
        Ok(self.stats)
    }
}
```

- [ ] **Step 6: Add `pub mod builder; pub mod sha256;` to `lib.rs`, run the tests**

Run: `cargo test -p psolve-index`
Expected: all builder + sha256 + record + format + healpix tests PASS

- [ ] **Step 7: Commit**

```bash
git add crates/psolve-index/src/builder.rs crates/psolve-index/src/sha256.rs \
        crates/psolve-index/src/lib.rs crates/psolve-index/tests/builder.rs
git commit -m "feat(index): in-memory builder and sha256 digest"
```

---

## Task 7: mmap reader

**Files:**
- Create: `crates/psolve-index/src/reader.rs`
- Modify: `crates/psolve-index/src/lib.rs`
- Test: `crates/psolve-index/tests/reader.rs`

**Interfaces:**
- Consumes: `Header` (Task 5), `StarRecord` (Task 4), `cells_in_disc` (Task 3), `Builder` (Task 6, for tests)
- Produces:
  - `pub struct Index { .. }`
  - `impl Index { pub fn open(path: &Path) -> Result<Index, IndexError>; pub fn header(&self) -> &Header; pub fn cell(&self, cell: u64) -> &[u8]; pub fn cell_len(&self, cell: u64) -> usize; pub fn star(&self, cell: u64, i: usize) -> Option<StarRecord>; pub fn brightest_in_disc(&self, ra_deg: f64, dec_deg: f64, radius_deg: f64, limit: usize) -> Vec<StarRecord>; pub fn verify_digest(&self) -> Result<(), IndexError> }`

- [ ] **Step 1: Write the failing test**

Create `crates/psolve-index/tests/reader.rs`:

```rust
use psolve_index::builder::Builder;
use psolve_index::healpix::ang2pix_nest;
use psolve_index::reader::Index;
use std::io::Write;

fn write_index(dir: &std::path::Path, nside: u32, stars: &[(f64, f64, f32)]) -> std::path::PathBuf {
    let mut b = Builder::new(nside, 20.0, 2016.0, "test").unwrap();
    for &(ra, dec, mag) in stars {
        b.push(ra, dec, mag, 0.0, 0.0);
    }
    let mut buf = std::io::Cursor::new(Vec::new());
    b.finish(&mut buf).unwrap();
    let p = dir.join("test.psidx");
    let mut f = std::fs::File::create(&p).unwrap();
    f.write_all(&buf.into_inner()).unwrap();
    p
}

fn tmpdir() -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("psolve-test-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn opens_and_reports_the_header() {
    let d = tmpdir();
    let p = write_index(&d, 8, &[(10.0, 20.0, 11.0), (10.1, 20.1, 12.0)]);
    let idx = Index::open(&p).unwrap();
    assert_eq!(idx.header().n_records, 2);
    assert_eq!(idx.header().nside, 8);
    assert_eq!(idx.header().name_str(), "test");
}

#[test]
fn digest_verifies() {
    let d = tmpdir();
    let p = write_index(&d, 8, &[(1.0, 2.0, 11.0), (3.0, 4.0, 12.0)]);
    Index::open(&p).unwrap().verify_digest().unwrap();
}

#[test]
fn rejects_a_corrupted_record_region() {
    let d = tmpdir();
    let p = write_index(&d, 8, &[(1.0, 2.0, 11.0), (3.0, 4.0, 12.0)]);
    let mut bytes = std::fs::read(&p).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 0xff;
    std::fs::write(&p, &bytes).unwrap();
    let idx = Index::open(&p).unwrap();
    assert!(idx.verify_digest().is_err(), "corruption must be detected");
}

#[test]
fn cell_lookup_returns_the_stars_in_that_cell() {
    let d = tmpdir();
    let stars = [(10.0, 20.0, 11.0), (10.01, 20.01, 12.0), (200.0, -40.0, 13.0)];
    let p = write_index(&d, 64, &stars);
    let idx = Index::open(&p).unwrap();
    let c = ang2pix_nest(64, 10.0, 20.0);
    assert!(idx.cell_len(c) >= 1);
    let far = ang2pix_nest(64, 200.0, -40.0);
    assert_eq!(idx.cell_len(far), 1);
}

#[test]
fn brightest_in_disc_is_sorted_and_limited() {
    let d = tmpdir();
    // 60 stars in a tight patch, magnitudes deliberately shuffled.
    let stars: Vec<(f64, f64, f32)> = (0..60)
        .map(|i| (100.0 + (i % 8) as f64 * 0.02, 20.0 + (i / 8) as f64 * 0.02,
                  9.0 + ((i * 13) % 60) as f32 * 0.1))
        .collect();
    let p = write_index(&d, 64, &stars);
    let idx = Index::open(&p).unwrap();
    let got = idx.brightest_in_disc(100.08, 20.08, 1.0, 10);
    assert_eq!(got.len(), 10, "limit must be honoured");
    for w in got.windows(2) {
        assert!(w[0].mag() <= w[1].mag(), "result must be brightest-first");
    }
    let all = idx.brightest_in_disc(100.08, 20.08, 1.0, 10_000);
    assert!(all.len() >= 10);
    assert!(all[0].mag() <= all[all.len() - 1].mag());
}

#[test]
fn disc_far_from_any_star_is_empty() {
    let d = tmpdir();
    let p = write_index(&d, 64, &[(10.0, 20.0, 11.0)]);
    let idx = Index::open(&p).unwrap();
    assert!(idx.brightest_in_disc(200.0, -60.0, 0.5, 100).is_empty());
}

#[test]
fn rejects_a_non_index_file() {
    let d = tmpdir();
    let p = d.join("junk.psidx");
    std::fs::write(&p, b"this is not an index at all, not even close").unwrap();
    assert!(Index::open(&p).is_err());
}

#[test]
fn rejects_a_truncated_file() {
    let d = tmpdir();
    let p = write_index(&d, 8, &[(1.0, 2.0, 11.0)]);
    let bytes = std::fs::read(&p).unwrap();
    std::fs::write(&p, &bytes[..bytes.len() - 8]).unwrap();
    assert!(Index::open(&p).is_err(), "truncated index must not open");
}
```

- [ ] **Step 2: Run it to confirm it fails**

Run: `cargo test -p psolve-index --test reader`
Expected: FAIL — `Index` does not exist

- [ ] **Step 3: Implement `reader.rs`**

```rust
//! Read-only, mmap-backed index access.
//!
//! There is no write path in this module, by design: psolve-core must not be
//! able to modify anything on disk (spec section 4).

use crate::error::IndexError;
use crate::format::Header;
use crate::healpix::{cells_in_disc, npix};
use crate::record::{StarRecord, RECORD_BYTES};
use crate::sha256::sha256;
use memmap2::Mmap;
use std::fs::File;
use std::path::Path;

pub struct Index {
    map: Mmap,
    header: Header,
    /// npix+1 cumulative record offsets, decoded once at open.
    cell_table: Vec<u64>,
}

impl Index {
    pub fn open(path: &Path) -> Result<Index, IndexError> {
        let file = File::open(path)?;
        // SAFETY: the index is a read-only, immutable artifact. A concurrent
        // truncation would be UB; we accept that as we accept it for any mmap.
        let map = unsafe { Mmap::map(&file)? };
        let header = Header::from_bytes(&map)?;

        let npix = npix(header.nside);
        let tab_off = Header::cell_table_offset() as usize;
        let tab_bytes = Header::cell_table_bytes(header.nside) as usize;
        let need = header.records_offset + header.n_records * RECORD_BYTES as u64;
        if (map.len() as u64) < need {
            return Err(IndexError::Truncated { expected: need, actual: map.len() as u64 });
        }

        let mut cell_table = Vec::with_capacity((npix + 1) as usize);
        for i in 0..(npix + 1) as usize {
            let s = tab_off + i * 8;
            cell_table.push(u64::from_le_bytes(
                map[s..s + 8].try_into().map_err(|_| IndexError::Truncated {
                    expected: (tab_off + tab_bytes) as u64,
                    actual: map.len() as u64,
                })?,
            ));
        }

        Ok(Index { map, header, cell_table })
    }

    pub fn header(&self) -> &Header {
        &self.header
    }

    fn record_bytes(&self) -> &[u8] {
        let s = self.header.records_offset as usize;
        let e = s + self.header.n_records as usize * RECORD_BYTES;
        &self.map[s..e]
    }

    /// Recompute the record digest and compare. O(file size) — for `doctor`,
    /// not for the solve path.
    pub fn verify_digest(&self) -> Result<(), IndexError> {
        if sha256(self.record_bytes()) == self.header.records_sha256 {
            Ok(())
        } else {
            Err(IndexError::ChecksumMismatch)
        }
    }

    pub fn cell_len(&self, cell: u64) -> usize {
        if cell + 1 >= self.cell_table.len() as u64 {
            return 0;
        }
        (self.cell_table[cell as usize + 1] - self.cell_table[cell as usize]) as usize
    }

    /// Raw bytes for a cell's records, brightest-first.
    pub fn cell(&self, cell: u64) -> &[u8] {
        if cell + 1 >= self.cell_table.len() as u64 {
            return &[];
        }
        let (a, b) = (self.cell_table[cell as usize], self.cell_table[cell as usize + 1]);
        let base = self.header.records_offset as usize;
        &self.map[base + a as usize * RECORD_BYTES..base + b as usize * RECORD_BYTES]
    }

    pub fn star(&self, cell: u64, i: usize) -> Option<StarRecord> {
        let c = self.cell(cell);
        let s = i * RECORD_BYTES;
        c.get(s..s + RECORD_BYTES)
            .and_then(|b| b.try_into().ok())
            .map(|b: &[u8; RECORD_BYTES]| StarRecord::from_bytes(b))
    }

    /// The `limit` brightest stars within `radius_deg` of the given position.
    ///
    /// Each cell is already magnitude-sorted, so this is a k-way merge over a
    /// handful of sorted runs -- not a sort of the whole neighbourhood. It
    /// reads only as far into each run as it needs to.
    pub fn brightest_in_disc(
        &self,
        ra_deg: f64,
        dec_deg: f64,
        radius_deg: f64,
        limit: usize,
    ) -> Vec<StarRecord> {
        let cells = cells_in_disc(self.header.nside, ra_deg, dec_deg, radius_deg);
        // cursor per cell
        let mut cursors: Vec<(u64, usize)> = cells.iter().map(|&c| (c, 0usize)).collect();
        let mut out = Vec::with_capacity(limit.min(1024));

        while out.len() < limit {
            let mut best: Option<(usize, StarRecord)> = None;
            for (ci, (cell, pos)) in cursors.iter().enumerate() {
                if let Some(rec) = self.star(*cell, *pos) {
                    if best.as_ref().map_or(true, |(_, b)| rec.mag_milli < b.mag_milli) {
                        best = Some((ci, rec));
                    }
                }
            }
            match best {
                None => break,
                Some((ci, rec)) => {
                    cursors[ci].1 += 1;
                    out.push(rec);
                }
            }
        }
        out
    }
}
```

**AMENDMENT (found by the Task 7 review — the Step 3 code above is incomplete).**
`open()` as written above has two reachable panics on untrusted input. Both must be fixed:

1. **The length check ignores the cell table.** `need` is computed only from
   `records_offset + n_records * 16`, but `records_offset` and `nside` are independent
   header fields, so a file with a small `n_records` and a large `nside` passes the check
   while the cell-table decode still reads past EOF — and that decode uses a direct
   `map[s..s+8]` slice, so it panics instead of returning `Truncated`. Take the **max** of
   the record-region and cell-table requirements, guard the arithmetic with
   `checked_mul`/`checked_add`, and decode with `map.get(s..s + 8)`.

2. **Cell-table contents are never validated.** A corrupt-but-not-truncated file can carry a
   non-monotonic table, or a final entry that disagrees with `n_records`. `cell()` then
   evaluates `&self.map[base + a * 16 .. base + b * 16]` with `a > b`, which panics on
   `start > end` in **both debug and release**. Validate inside the existing decode loop —
   each entry non-decreasing, final entry equal to `n_records` — so `cell()`/`cell_len()`
   are provably panic-free for anything that passed `open()`. No extra pass, no hot-path cost.

The `Builder` already guarantees both invariants (prefix-sum table, final entry = record
count), so this validates the format's real contract rather than inventing a stricter one.

Also fix the test helper: `tmpdir()` keys only on `std::process::id()`, which is constant
across the test binary, so all tests in the file share one directory and race under
`cargo test`'s default parallelism. Use a process-wide atomic counter.

- [ ] **Step 4: Add `pub mod reader;` to `lib.rs` and run**

Run: `cargo test -p psolve-index --test reader`
Expected: all 8 PASS

- [ ] **Step 5: Commit**

```bash
git add crates/psolve-index/src/reader.rs crates/psolve-index/src/lib.rs \
        crates/psolve-index/tests/reader.rs
git commit -m "feat(index): mmap reader with k-way merge brightest_in_disc"
```

---

## Task 8: Gaia ECSV parser

**Files:**
- Create: `crates/psolve-index/src/gaia.rs`
- Modify: `crates/psolve-index/src/lib.rs`
- Test: `crates/psolve-index/tests/gaia.rs`, `crates/psolve-index/tests/fixtures/gaia_sample.csv`

**Interfaces:**
- Consumes: `IndexError` (Task 1)
- Produces:
  - `pub struct GaiaRow { pub source_id: u64, pub ra: f64, pub dec: f64, pub mag: f32, pub pmra: f32, pub pmdec: f32 }`
  - `pub struct GaiaColumns { .. }`
  - `pub struct ColumnNames { pub ra: String, pub dec: String, pub mag: String, pub pmra: String, pub pmdec: String, pub source_id: String }` with `Default` = Gaia DR3 names
  - `impl ColumnNames { pub fn with_overrides(spec: &str) -> Result<ColumnNames, IndexError> }` — parses `"ra=RAJ2000,mag=Vmag"`
  - `pub struct RowFilter { pub max_mag: f32, pub min_dec: f64, pub max_dec: f64 }` with `Default` = keep everything
  - `impl RowFilter { pub fn validate(&self) -> Result<(), IndexError> }`
  - `pub fn find_columns(header_line: &str, names: &ColumnNames) -> Result<GaiaColumns, IndexError>`
  - `pub fn parse_row(cols: &GaiaColumns, line: &str, line_no: u64) -> Result<Option<GaiaRow>, IndexError>` — `Ok(None)` means "skip this row" (no magnitude)
  - `pub fn read_ecsv<R: BufRead, F: FnMut(GaiaRow)>(r: R, names: &ColumnNames, filter: &RowFilter, f: F) -> Result<u64, IndexError>`

**Why the indirection:** the format is not Gaia-specific and should not pretend to be. A user with Tycho-2, a Vizier export, or their own reduced catalogue supplies `--columns ra=RAJ2000,dec=DEJ2000,mag=Vmag` and it works. `source_id` is **optional** — it exists only so the Task 2 fixture can cross-check HEALPix against Gaia's encoding, and reduced shards drop it.

- [ ] **Step 1: Create the fixture `crates/psolve-index/tests/fixtures/gaia_sample.csv`**

This mimics the real ECSV shape: `#` comment block, header row, data. Real column *names* in real order for the ones we need; the rest are elided since the parser must look up by name.

```
# %ECSV 1.0
# ---
# delimiter: ','
# datatype:
# - {name: solution_id, datatype: int64}
# - {name: source_id, datatype: int64}
solution_id,designation,source_id,random_index,ref_epoch,ra,ra_error,dec,dec_error,parallax,parallax_error,parallax_over_error,pm,pmra,pmra_error,pmdec,pmdec_error,phot_g_mean_mag
1636148068921376768,Gaia DR3 4295806720,4295806720,100,2016.0,44.99615537864534,0.3,0.005615226341865997,0.2,1.1,0.1,10.0,12.6,12.616485,0.2,0.13794228,0.2,17.641426
1636148068921376768,Gaia DR3 34361129088,34361129088,101,2016.0,45.00432028915398,0.3,0.021047763781174733,0.2,1.1,0.1,10.0,35.2,35.230515,0.2,0.13369285,0.2,14.128453
1636148068921376768,Gaia DR3 38655544960,38655544960,102,2016.0,45.004978371745516,0.3,0.019879675701858944,0.2,1.1,0.1,10.0,35.3,,0.2,,0.2,9.5
1636148068921376768,Gaia DR3 38655544961,38655544961,103,2016.0,45.1,0.3,0.02,0.2,1.1,0.1,10.0,1.0,1.0,0.2,1.0,0.2,
```

Note the third data row has **empty `pmra`/`pmdec`** (a two-parameter source) and the fourth has an **empty magnitude** (must be skipped).

- [ ] **Step 2: Write the failing test `crates/psolve-index/tests/gaia.rs`**

```rust
use psolve_index::gaia::{find_columns, parse_row, read_ecsv, ColumnNames, RowFilter};
use std::io::BufReader;

const SAMPLE: &str = include_str!("fixtures/gaia_sample.csv");

fn header_line() -> &'static str {
    SAMPLE.lines().find(|l| !l.starts_with('#')).unwrap()
}

fn gaia() -> ColumnNames {
    ColumnNames::default()
}

fn mag_only(max_mag: f32) -> RowFilter {
    RowFilter { max_mag, ..RowFilter::default() }
}

#[test]
fn finds_columns_by_name_not_position() {
    let c = find_columns(header_line(), &gaia()).unwrap();
    // Reordering the header must still work -- that is the point of by-name lookup.
    let reordered = "phot_g_mean_mag,dec,ra,pmdec,pmra,source_id";
    let c2 = find_columns(reordered, &gaia()).unwrap();
    assert_ne!(
        (c.ra, c.dec), (c2.ra, c2.dec),
        "indices should differ between the two layouts"
    );
}

#[test]
fn rejects_a_header_missing_a_required_column() {
    let bad = "source_id,ra,dec,pmra,pmdec"; // no phot_g_mean_mag
    assert!(find_columns(bad, &gaia()).is_err());
}

#[test]
fn header_without_source_id_is_accepted() {
    // Reduced shards drop source_id; nothing in the index needs it.
    let c = find_columns("ra,dec,pmra,pmdec,phot_g_mean_mag", &gaia()).unwrap();
    let row = parse_row(&c, "10.0,20.0,1.0,2.0,12.5", 1).unwrap().unwrap();
    assert_eq!(row.source_id, 0);
    assert!((row.ra - 10.0).abs() < 1e-9);
    assert!((row.mag - 12.5).abs() < 1e-6);
}

#[test]
fn a_non_gaia_catalogue_works_via_column_overrides() {
    // A Vizier-style export: different names, same meaning.
    let names = ColumnNames::with_overrides(
        "ra=RAJ2000,dec=DEJ2000,mag=Vmag,pmra=pmRA,pmdec=pmDE",
    )
    .unwrap();
    let csv = "RAJ2000,DEJ2000,Vmag,pmRA,pmDE\n120.5,-33.25,8.75,-4.5,6.25\n";
    let mut got = Vec::new();
    read_ecsv(BufReader::new(csv.as_bytes()), &names, &RowFilter::default(), |r| {
        got.push(r)
    })
    .unwrap();
    assert_eq!(got.len(), 1);
    assert!((got[0].ra - 120.5).abs() < 1e-9);
    assert!((got[0].dec + 33.25).abs() < 1e-9);
    assert!((got[0].mag - 8.75).abs() < 1e-6);
    assert!((got[0].pmdec - 6.25).abs() < 1e-6);
}

#[test]
fn unknown_or_malformed_column_overrides_are_rejected() {
    // A silently-ignored typo would build the index from the wrong column.
    assert!(ColumnNames::with_overrides("magnitude=Vmag").is_err());
    assert!(ColumnNames::with_overrides("ra").is_err());
    assert!(ColumnNames::with_overrides("").is_ok(), "empty spec = defaults");
}

#[test]
fn parses_a_normal_row() {
    let c = find_columns(header_line(), &gaia()).unwrap();
    let line = SAMPLE.lines().filter(|l| !l.starts_with('#')).nth(1).unwrap();
    let row = parse_row(&c, line, 1).unwrap().unwrap();
    assert_eq!(row.source_id, 4295806720);
    assert!((row.ra - 44.99615537864534).abs() < 1e-12);
    assert!((row.dec - 0.005615226341865997).abs() < 1e-12);
    assert!((row.mag - 17.641426).abs() < 1e-5);
    assert!((row.pmra - 12.616485).abs() < 1e-4);
}

#[test]
fn empty_proper_motion_becomes_zero() {
    let c = find_columns(header_line(), &gaia()).unwrap();
    let line = SAMPLE.lines().filter(|l| !l.starts_with('#')).nth(3).unwrap();
    let row = parse_row(&c, line, 3).unwrap().unwrap();
    assert_eq!(row.pmra, 0.0);
    assert_eq!(row.pmdec, 0.0);
    assert!((row.mag - 9.5).abs() < 1e-5);
}

#[test]
fn empty_magnitude_is_skipped_not_an_error() {
    let c = find_columns(header_line(), &gaia()).unwrap();
    let line = SAMPLE.lines().filter(|l| !l.starts_with('#')).nth(4).unwrap();
    assert!(parse_row(&c, line, 4).unwrap().is_none());
}

#[test]
fn read_ecsv_skips_comments_and_applies_the_mag_cut() {
    let mut got = Vec::new();
    let n = read_ecsv(BufReader::new(SAMPLE.as_bytes()), &gaia(), &mag_only(15.0), |r| {
        got.push(r)
    })
    .unwrap();
    // rows: 17.64 (cut), 14.13 (kept), 9.5 (kept), no-mag (skipped)
    assert_eq!(got.len(), 2, "mag cut should keep exactly two rows");
    assert_eq!(n, 2);
    assert!(got.iter().all(|r| r.mag <= 15.0));
}

#[test]
fn read_ecsv_with_a_generous_cut_keeps_all_magnitude_bearing_rows() {
    let mut got = Vec::new();
    read_ecsv(BufReader::new(SAMPLE.as_bytes()), &gaia(), &RowFilter::default(), |r| {
        got.push(r)
    })
    .unwrap();
    assert_eq!(got.len(), 3);
}

#[test]
fn read_ecsv_applies_the_declination_cut() {
    // Sample decs are 0.00562, 0.02105, 0.01988 (plus one row with no mag).
    let mut got = Vec::new();
    read_ecsv(
        BufReader::new(SAMPLE.as_bytes()),
        &gaia(),
        &RowFilter { min_dec: 0.01, ..RowFilter::default() },
        |r| got.push(r),
    )
    .unwrap();
    assert_eq!(got.len(), 2, "min_dec should exclude the 0.00562 row");
    assert!(got.iter().all(|r| r.dec >= 0.01));

    let mut south = Vec::new();
    read_ecsv(
        BufReader::new(SAMPLE.as_bytes()),
        &gaia(),
        &RowFilter { max_dec: 0.01, ..RowFilter::default() },
        |r| south.push(r),
    )
    .unwrap();
    assert_eq!(south.len(), 1, "max_dec should keep only the 0.00562 row");
}

#[test]
fn an_impossible_declination_range_is_rejected() {
    let f = RowFilter { min_dec: 40.0, max_dec: -40.0, ..RowFilter::default() };
    assert!(f.validate().is_err(), "min above max must be rejected");
    let g = RowFilter { min_dec: -200.0, ..RowFilter::default() };
    assert!(g.validate().is_err(), "out-of-range declination must be rejected");
    assert!(RowFilter::default().validate().is_ok());
}

#[test]
fn a_short_row_is_an_error_not_a_panic() {
    let c = find_columns(header_line(), &gaia()).unwrap();
    assert!(parse_row(&c, "1,2,3", 7).is_err());
}
```

- [ ] **Step 3: Run it to confirm it fails**

Run: `cargo test -p psolve-index --test gaia`
Expected: FAIL — `psolve_index::gaia` does not exist

- [ ] **Step 4: Implement `gaia.rs`**

```rust
//! Gaia DR3 ECSV parsing.
//!
//! The bulk files at cdn.gea.esac.esa.int are ECSV: 1,000 leading '#' comment
//! lines carrying a YAML header, then a CSV header row, then 152 columns of
//! data. Columns are located BY NAME -- never by hardcoded index -- because a
//! column order change would otherwise corrupt the whole catalogue silently.

use crate::error::IndexError;
use std::io::BufRead;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GaiaRow {
    pub source_id: u64,
    pub ra: f64,
    pub dec: f64,
    pub mag: f32,
    pub pmra: f32,
    pub pmdec: f32,
}

/// Column NAMES to look for. Defaults are Gaia DR3's; override for any other
/// catalogue. The index format is not Gaia-specific and should not pretend
/// to be -- a Tycho-2 or Vizier export is a legitimate source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnNames {
    pub ra: String,
    pub dec: String,
    pub mag: String,
    pub pmra: String,
    pub pmdec: String,
    pub source_id: String,
}

impl Default for ColumnNames {
    fn default() -> Self {
        ColumnNames {
            ra: "ra".into(),
            dec: "dec".into(),
            mag: "phot_g_mean_mag".into(),
            pmra: "pmra".into(),
            pmdec: "pmdec".into(),
            source_id: "source_id".into(),
        }
    }
}

impl ColumnNames {
    /// Parse `"ra=RAJ2000,dec=DEJ2000,mag=Vmag"` over the defaults.
    /// An unknown key is an error rather than a silent no-op: a typo that is
    /// ignored produces an index built from the wrong column.
    pub fn with_overrides(spec: &str) -> Result<ColumnNames, IndexError> {
        let mut c = ColumnNames::default();
        for part in spec.split(',').filter(|p| !p.trim().is_empty()) {
            let (k, v) = part
                .split_once('=')
                .ok_or_else(|| IndexError::BadColumnSpec(part.to_string()))?;
            let v = v.trim().to_string();
            match k.trim() {
                "ra" => c.ra = v,
                "dec" => c.dec = v,
                "mag" => c.mag = v,
                "pmra" => c.pmra = v,
                "pmdec" => c.pmdec = v,
                "source_id" => c.source_id = v,
                _ => return Err(IndexError::BadColumnSpec(part.to_string())),
            }
        }
        Ok(c)
    }
}

/// Which rows to keep. A fixed observatory never sees the whole sky, so a
/// declination cut removes stars that cannot appear in any frame it takes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RowFilter {
    pub max_mag: f32,
    pub min_dec: f64,
    pub max_dec: f64,
}

impl Default for RowFilter {
    fn default() -> Self {
        RowFilter { max_mag: f32::INFINITY, min_dec: -90.0, max_dec: 90.0 }
    }
}

impl RowFilter {
    pub fn validate(&self) -> Result<(), IndexError> {
        if !(-90.0..=90.0).contains(&self.min_dec) || !(-90.0..=90.0).contains(&self.max_dec) {
            return Err(IndexError::BadRange {
                what: "declination range",
                reason: format!("{}..{} is outside -90..90", self.min_dec, self.max_dec),
            });
        }
        if self.min_dec > self.max_dec {
            return Err(IndexError::BadRange {
                what: "declination range",
                reason: format!("min {} is above max {}", self.min_dec, self.max_dec),
            });
        }
        Ok(())
    }

    fn keeps(&self, row: &GaiaRow) -> bool {
        row.mag <= self.max_mag && row.dec >= self.min_dec && row.dec <= self.max_dec
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GaiaColumns {
    /// usize::MAX when the catalogue has no source_id column.
    pub source_id: usize,
    pub ra: usize,
    pub dec: usize,
    pub pmra: usize,
    pub pmdec: usize,
    pub mag: usize,
    pub width: usize,
}

fn column_of(header: &[&str], name: &str) -> Result<usize, IndexError> {
    header
        .iter()
        .position(|h| h.trim() == name)
        .ok_or_else(|| IndexError::MissingColumn(name.to_string()))
}

pub fn find_columns(header_line: &str, names: &ColumnNames) -> Result<GaiaColumns, IndexError> {
    let h: Vec<&str> = header_line.trim_end().split(',').collect();
    Ok(GaiaColumns {
        // Optional: reduced shards drop it, and nothing in the index needs it.
        // It is read when present only so tests can cross-check HEALPix
        // against Gaia's own source_id encoding.
        source_id: column_of(&h, &names.source_id).unwrap_or(usize::MAX),
        ra: column_of(&h, &names.ra)?,
        dec: column_of(&h, &names.dec)?,
        pmra: column_of(&h, &names.pmra)?,
        pmdec: column_of(&h, &names.pmdec)?,
        mag: column_of(&h, &names.mag)?,
        width: h.len(),
    })
}

/// Ok(None) means "no usable magnitude, skip this source".
pub fn parse_row(
    cols: &GaiaColumns,
    line: &str,
    line_no: u64,
) -> Result<Option<GaiaRow>, IndexError> {
    let f: Vec<&str> = line.trim_end().split(',').collect();
    let need = cols.width;
    if f.len() < need {
        return Err(IndexError::MalformedRow {
            line: line_no,
            reason: format!("expected {need} fields, found {}", f.len()),
        });
    }

    let mag_raw = f[cols.mag].trim();
    if mag_raw.is_empty() {
        return Ok(None);
    }

    let num = |i: usize, what: &str| -> Result<f64, IndexError> {
        f[i].trim().parse::<f64>().map_err(|e| IndexError::MalformedRow {
            line: line_no,
            reason: format!("{what}: {e}"),
        })
    };
    // Empty proper motion is normal: Gaia DR3 has ~340M two-parameter sources.
    let opt = |i: usize| -> f32 {
        let s = f[i].trim();
        if s.is_empty() { 0.0 } else { s.parse::<f32>().unwrap_or(0.0) }
    };

    Ok(Some(GaiaRow {
        source_id: if cols.source_id == usize::MAX {
            0
        } else {
            f[cols.source_id].trim().parse().unwrap_or(0)
        },
        ra: num(cols.ra, "ra")?,
        dec: num(cols.dec, "dec")?,
        mag: mag_raw.parse::<f32>().map_err(|e| IndexError::MalformedRow {
            line: line_no,
            reason: format!("phot_g_mean_mag: {e}"),
        })?,
        pmra: opt(cols.pmra),
        pmdec: opt(cols.pmdec),
    }))
}

/// Stream an ECSV/CSV file, invoking `f` for every row the filter keeps.
/// Returns the number of rows passed to `f`.
pub fn read_ecsv<R: BufRead, F: FnMut(GaiaRow)>(
    r: R,
    names: &ColumnNames,
    filter: &RowFilter,
    mut f: F,
) -> Result<u64, IndexError> {
    filter.validate()?;
    let mut cols: Option<GaiaColumns> = None;
    let mut kept = 0u64;
    for (i, line) in r.lines().enumerate() {
        let line = line?;
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        match &cols {
            None => cols = Some(find_columns(&line, names)?),
            Some(c) => {
                if let Some(row) = parse_row(c, &line, i as u64)? {
                    if filter.keeps(&row) {
                        f(row);
                        kept += 1;
                    }
                }
            }
        }
    }
    Ok(kept)
}
```

**AMENDMENT (found by the Task 8 review — two defects in the Step 4 code above).**

1. **`opt()` launders a malformed proper motion into `0.0`.** Empty must stay a legitimate
   zero (Gaia DR3 has ~340M two-parameter sources), but a *non-empty, unparseable* value
   like `N/A` must be a `MalformedRow` error naming the field — otherwise corrupt data is
   indistinguishable from a genuine PM-less star. Change `opt` to return
   `Result<f32, IndexError>`: early-return `Ok(0.0)` when empty, otherwise `parse().map_err(..)`.
   `source_id` stays lenient (advisory, nothing in the index needs it) but say so in a comment.
2. **`read_ecsv` returns `Ok(0)` for a file with no header row.** An empty or all-comments
   file — a truncated download that lost its header — is then indistinguishable from a real
   catalogue with zero rows. Return an error when `cols` is still `None` after the loop. A
   file WITH a header and zero data rows must still succeed.

- [ ] **Step 5: Add `pub mod gaia;` to `lib.rs` and run**

Run: `cargo test -p psolve-index --test gaia`
Expected: all 13 PASS

- [ ] **Step 6: Commit**

```bash
git add crates/psolve-index/src/gaia.rs crates/psolve-index/src/lib.rs \
        crates/psolve-index/tests/gaia.rs crates/psolve-index/tests/fixtures/gaia_sample.csv
git commit -m "feat(index): gaia ECSV parser with by-name column lookup"
```

---

## Task 9: CLI `index build`

**Files:**
- Create: `crates/psolve-cli/Cargo.toml`, `crates/psolve-cli/src/main.rs`, `crates/psolve-cli/src/cmd_index.rs`
- Test: `crates/psolve-cli/tests/cli_build.rs`

**Interfaces:**
- Consumes: `Builder` (Task 6), `read_ecsv` (Task 8), `Index` (Task 7)
- Produces: `psolve index build --input <dir> --out <file> [--max-mag F] [--nside N] [--name S] [--jobs N]`

- [ ] **Step 1: Write `crates/psolve-cli/Cargo.toml`**

```toml
[package]
name = "psolve-cli"
version.workspace = true
edition.workspace = true
license.workspace = true

[[bin]]
name = "psolve"
path = "src/main.rs"

[dependencies]
psolve-index = { path = "../psolve-index" }
rayon = "1"
```

- [ ] **Step 2: Write the failing test `crates/psolve-cli/tests/cli_build.rs`**

```rust
use std::process::Command;

fn bin() -> std::path::PathBuf {
    let mut p = std::env::current_exe().unwrap();
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("psolve")
}

fn tmpdir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("psolve-cli-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

const SAMPLE: &str = include_str!("../../psolve-index/tests/fixtures/gaia_sample.csv");

#[test]
fn builds_an_index_from_a_directory_of_csv() {
    let d = tmpdir("build");
    let input = d.join("in");
    std::fs::create_dir_all(&input).unwrap();
    std::fs::write(input.join("GaiaSource_000000-000001.csv"), SAMPLE).unwrap();
    let out = d.join("out.psidx");

    let o = Command::new(bin())
        .args(["index", "build", "--input"])
        .arg(&input)
        .arg("--out")
        .arg(&out)
        .args(["--max-mag", "20", "--nside", "64", "--name", "test-build"])
        .output()
        .unwrap();

    assert!(o.status.success(), "stderr: {}", String::from_utf8_lossy(&o.stderr));
    assert!(out.exists(), "index file was not created");

    let idx = psolve_index::reader::Index::open(&out).unwrap();
    assert_eq!(idx.header().n_records, 3);
    assert_eq!(idx.header().name_str(), "test-build");
    assert_eq!(idx.header().nside, 64);
    idx.verify_digest().unwrap();
}

#[test]
fn build_emits_json_on_stdout_and_progress_on_stderr() {
    let d = tmpdir("streams");
    let input = d.join("in");
    std::fs::create_dir_all(&input).unwrap();
    std::fs::write(input.join("a.csv"), SAMPLE).unwrap();
    let out = d.join("out.psidx");

    let o = Command::new(bin())
        .args(["index", "build", "--input"])
        .arg(&input)
        .arg("--out")
        .arg(&out)
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&o.stdout);
    assert!(stdout.trim_start().starts_with('{'), "stdout must be JSON, got: {stdout}");
    assert!(stdout.contains("\"n_records\""));
}

#[test]
fn missing_input_directory_exits_2() {
    let d = tmpdir("missing");
    let o = Command::new(bin())
        .args(["index", "build", "--input", "/nonexistent/nowhere", "--out"])
        .arg(d.join("x.psidx"))
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2), "usage errors exit 2");
}

#[test]
fn declination_limits_shrink_the_index() {
    let d = tmpdir("dec");
    let input = d.join("in");
    std::fs::create_dir_all(&input).unwrap();
    std::fs::write(input.join("a.csv"), SAMPLE).unwrap();

    let build = |tag: &str, extra: &[&str]| -> u64 {
        let out = d.join(format!("{tag}.psidx"));
        let o = Command::new(bin())
            .args(["index", "build", "--input"])
            .arg(&input)
            .arg("--out")
            .arg(&out)
            .args(["--max-mag", "20"])
            .args(extra)
            .output()
            .unwrap();
        assert!(o.status.success(), "stderr: {}", String::from_utf8_lossy(&o.stderr));
        psolve_index::reader::Index::open(&out).unwrap().header().n_records
    };

    // Sample decs are 0.00562, 0.02105, 0.01988.
    assert_eq!(build("all", &[]), 3);
    assert_eq!(build("north", &["--min-dec", "0.01"]), 2);
    assert_eq!(build("south", &["--max-dec", "0.01"]), 1);
}

#[test]
fn a_non_gaia_catalogue_builds_via_column_overrides() {
    let d = tmpdir("columns");
    let input = d.join("in");
    std::fs::create_dir_all(&input).unwrap();
    std::fs::write(
        input.join("vizier.csv"),
        "RAJ2000,DEJ2000,Vmag,pmRA,pmDE\n120.5,-33.25,8.75,-4.5,6.25\n10.0,5.0,9.5,0,0\n",
    )
    .unwrap();
    let out = d.join("v.psidx");
    let o = Command::new(bin())
        .args(["index", "build", "--input"])
        .arg(&input)
        .arg("--out")
        .arg(&out)
        .args([
            "--columns",
            "ra=RAJ2000,dec=DEJ2000,mag=Vmag,pmra=pmRA,pmdec=pmDE",
            "--epoch",
            "1991.25",
            "--max-mag",
            "20",
        ])
        .output()
        .unwrap();
    assert!(o.status.success(), "stderr: {}", String::from_utf8_lossy(&o.stderr));
    let idx = psolve_index::reader::Index::open(&out).unwrap();
    assert_eq!(idx.header().n_records, 2);
    assert_eq!(idx.header().epoch, 1991.25, "--epoch must reach the header");
}

#[test]
fn bad_column_override_exits_2() {
    let d = tmpdir("badcol");
    let input = d.join("in");
    std::fs::create_dir_all(&input).unwrap();
    std::fs::write(input.join("a.csv"), SAMPLE).unwrap();
    let o = Command::new(bin())
        .args(["index", "build", "--input"])
        .arg(&input)
        .arg("--out")
        .arg(d.join("x.psidx"))
        .args(["--columns", "magnitude=Vmag"])
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2));
}

#[test]
fn impossible_declination_range_exits_2() {
    let d = tmpdir("badrange");
    let input = d.join("in");
    std::fs::create_dir_all(&input).unwrap();
    std::fs::write(input.join("a.csv"), SAMPLE).unwrap();
    let o = Command::new(bin())
        .args(["index", "build", "--input"])
        .arg(&input)
        .arg("--out")
        .arg(d.join("x.psidx"))
        .args(["--min-dec", "40", "--max-dec", "-40"])
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2));
}

#[test]
fn building_deeper_than_the_mirror_exits_2() {
    let d = tmpdir("mirror");
    let input = d.join("in");
    std::fs::create_dir_all(&input).unwrap();
    std::fs::write(input.join("a.csv"), SAMPLE).unwrap();
    std::fs::write(
        input.join("mirror.json"),
        r#"{"max_mag":14,"min_dec":-90,"max_dec":45,"files":3386}"#,
    )
    .unwrap();

    // Deeper than the mirror holds -> refuse rather than build a short index.
    let deep = Command::new(bin())
        .args(["index", "build", "--input"])
        .arg(&input)
        .arg("--out")
        .arg(d.join("deep.psidx"))
        .args(["--max-mag", "16"])
        .output()
        .unwrap();
    assert_eq!(deep.status.code(), Some(2));

    // Wider declination than the mirror holds -> also refuse.
    let wide = Command::new(bin())
        .args(["index", "build", "--input"])
        .arg(&input)
        .arg("--out")
        .arg(d.join("wide.psidx"))
        .args(["--max-mag", "14", "--max-dec", "80"])
        .output()
        .unwrap();
    assert_eq!(wide.status.code(), Some(2));

    // Within the mirror -> builds.
    let ok = Command::new(bin())
        .args(["index", "build", "--input"])
        .arg(&input)
        .arg("--out")
        .arg(d.join("ok.psidx"))
        .args(["--max-mag", "14", "--max-dec", "45"])
        .output()
        .unwrap();
    assert!(ok.status.success(), "stderr: {}", String::from_utf8_lossy(&ok.stderr));
}

#[test]
fn compressed_shards_are_reported_not_silently_skipped() {
    let d = tmpdir("gz");
    let input = d.join("in");
    std::fs::create_dir_all(&input).unwrap();
    // A .gz we cannot decode, and no plain .csv at all.
    std::fs::write(input.join("GaiaSource_000000-003111.csv.gz"), b"\x1f\x8b\x08junk").unwrap();
    let o = Command::new(bin())
        .args(["index", "build", "--input"])
        .arg(&input)
        .arg("--out")
        .arg(d.join("x.psidx"))
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2), "must not report success with nothing readable");
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(
        err.contains("compressed") || err.contains("gzip"),
        "stderr should name the compressed files as the reason, got: {err}"
    );
}

#[test]
fn invalid_nside_exits_2() {
    let d = tmpdir("nside");
    let input = d.join("in");
    std::fs::create_dir_all(&input).unwrap();
    std::fs::write(input.join("a.csv"), SAMPLE).unwrap();
    let o = Command::new(bin())
        .args(["index", "build", "--input"])
        .arg(&input)
        .arg("--out")
        .arg(d.join("x.psidx"))
        .args(["--nside", "63"])
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2));
}
```

`psolve-index` is already a normal dependency of `psolve-cli`, so integration tests can use it directly — do **not** add a `[dev-dependencies]` duplicate.

- [ ] **Step 3: Run it to confirm it fails**

Run: `cargo test -p psolve-cli --test cli_build`
Expected: FAIL — binary does not exist / no `index` subcommand

- [ ] **Step 4: Write `crates/psolve-cli/src/main.rs`**

```rust
//! psolve — plate solver. M1 ships the `index` subcommand only.
//!
//! Exit codes (spec section 9):
//!   0 success · 1 normal negative outcome · 2 usage/config · 3 index problem

mod cmd_index;

use std::process::ExitCode;

const USAGE: &str = "\
psolve 0.1.0

USAGE:
    psolve index build --input <DIR> --out <FILE> [OPTIONS]
    psolve index info <FILE>

BUILD OPTIONS:
    --max-mag <F>   faintest magnitude to include     [default: 14]
    --min-dec <D>   southern declination limit, deg   [default: -90]
    --max-dec <D>   northern declination limit, deg   [default: 90]
    --nside <N>     HEALPix nside, power of two       [default: 64]
    --epoch <Y>     catalogue epoch, decimal year     [default: 2016.0 (Gaia DR3)]
    --columns <S>   column name overrides for a non-Gaia catalogue,
                    e.g. ra=RAJ2000,dec=DEJ2000,mag=Vmag
    --name <S>      index name stored in the header   [default: derived]
    --jobs <N>      parallel file readers             [default: cores]

A fixed observatory never sees the whole sky: --min-dec/--max-dec drop stars
that cannot appear in any frame it takes, shrinking the index accordingly.
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

    match refs.as_slice() {
        ["index", "build", rest @ ..] => cmd_index::build(rest),
        ["index", "info", rest @ ..] => cmd_index::info(rest),
        ["-h"] | ["--help"] | [] => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
        other => {
            eprintln!("psolve: unknown command {other:?}\n\n{USAGE}");
            ExitCode::from(2)
        }
    }
}

/// Minimal flag reader: `--name value`. Returns None if absent.
pub fn flag<'a>(args: &'a [&'a str], name: &str) -> Option<&'a str> {
    args.iter().position(|a| *a == name).and_then(|i| args.get(i + 1)).copied()
}
```

- [ ] **Step 5: Write `crates/psolve-cli/src/cmd_index.rs` (build only for now)**

```rust
use crate::flag;
use psolve_index::builder::Builder;
use psolve_index::gaia::{read_ecsv, ColumnNames, RowFilter};
use psolve_index::sha256::hex;
use rayon::prelude::*;
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

/// Plain `.csv` only, plus a count of compressed files we had to skip.
///
/// psolve has no gzip decoder -- the dependency budget is memmap2 + rayon --
/// so a `.csv.gz` here would be parsed as binary, fail the header lookup, and
/// be skipped with a warning. That silently yields a SHORT index, which is
/// the worst possible failure: it looks like a successful build. Reject
/// compressed input loudly instead.
fn csv_files(dir: &Path) -> std::io::Result<(Vec<PathBuf>, usize)> {
    let mut out = Vec::new();
    let mut compressed = 0usize;
    for e in std::fs::read_dir(dir)? {
        let p = e?.path();
        if !p.is_file() {
            continue;
        }
        let n = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if n.ends_with(".gz") || n.ends_with(".bz2") || n.ends_with(".zst") {
            compressed += 1;
        } else if n.ends_with(".csv") {
            out.push(p);
        }
    }
    out.sort();
    Ok((out, compressed))
}

/// What the local catalogue mirror actually contains, from `mirror.json`
/// written by fetch-gaia.sh. Absent for a bring-your-own directory, in which
/// case no validation happens. Hand-parsed: the dependency budget has no
/// JSON crate, and this reads three numbers from a file we wrote ourselves.
fn read_mirror(dir: &Path) -> Option<(f32, f64, f64)> {
    let txt = std::fs::read_to_string(dir.join("mirror.json")).ok()?;
    let num = |key: &str| -> Option<f64> {
        let at = txt.find(&format!("\"{key}\""))?;
        let after_colon = txt[at..].find(':')? + at + 1;
        let val: String = txt[after_colon..]
            .chars()
            .skip_while(|c| c.is_whitespace())
            .take_while(|c| c.is_ascii_digit() || matches!(c, '-' | '+' | '.' | 'e' | 'E'))
            .collect();
        val.parse().ok()
    };
    Some((num("max_mag")? as f32, num("min_dec")?, num("max_dec")?))
}

pub fn build(args: &[&str]) -> ExitCode {
    let Some(input) = flag(args, "--input") else {
        eprintln!("psolve index build: --input <DIR> is required");
        return ExitCode::from(2);
    };
    let Some(out) = flag(args, "--out") else {
        eprintln!("psolve index build: --out <FILE> is required");
        return ExitCode::from(2);
    };
    let max_mag: f32 = match flag(args, "--max-mag").unwrap_or("14").parse() {
        Ok(v) => v,
        Err(_) => {
            eprintln!("psolve index build: --max-mag must be a number");
            return ExitCode::from(2);
        }
    };
    let nside: u32 = match flag(args, "--nside").unwrap_or("64").parse() {
        Ok(v) => v,
        Err(_) => {
            eprintln!("psolve index build: --nside must be an integer");
            return ExitCode::from(2);
        }
    };
    let epoch: f64 = match flag(args, "--epoch").unwrap_or("2016.0").parse() {
        Ok(v) => v,
        Err(_) => {
            eprintln!("psolve index build: --epoch must be a decimal year");
            return ExitCode::from(2);
        }
    };
    // Declination limits: a fixed site never sees the whole sky.
    let parse_dec = |name: &str, default: &str| -> Result<f64, ()> {
        flag(args, name).unwrap_or(default).parse::<f64>().map_err(|_| ())
    };
    let (min_dec, max_dec) = match (parse_dec("--min-dec", "-90"), parse_dec("--max-dec", "90")) {
        (Ok(a), Ok(b)) => (a, b),
        _ => {
            eprintln!("psolve index build: --min-dec/--max-dec must be degrees");
            return ExitCode::from(2);
        }
    };
    let filter = RowFilter { max_mag, min_dec, max_dec };
    if let Err(e) = filter.validate() {
        eprintln!("psolve index build: {e}");
        return ExitCode::from(2);
    }
    let names = match flag(args, "--columns") {
        None => ColumnNames::default(),
        Some(spec) => match ColumnNames::with_overrides(spec) {
            Ok(n) => n,
            Err(e) => {
                eprintln!("psolve index build: {e}");
                return ExitCode::from(2);
            }
        },
    };

    let default_name = format!("gaia-dr3-g{}-nside{}", max_mag as i32, nside);
    let name = flag(args, "--name").unwrap_or(&default_name);
    if let Some(j) = flag(args, "--jobs").and_then(|v| v.parse::<usize>().ok()) {
        let _ = rayon::ThreadPoolBuilder::new().num_threads(j).build_global();
    }

    let dir = Path::new(input);
    let (files, compressed) = match csv_files(dir) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("psolve index build: cannot read {input}: {e}");
            return ExitCode::from(2);
        }
    };
    if compressed > 0 {
        eprintln!(
            "psolve index build: {compressed} compressed file(s) in {input} will be IGNORED \
             -- psolve has no gzip decoder. Decompress them first."
        );
    }
    if files.is_empty() {
        eprintln!("psolve index build: no .csv files in {input}");
        return ExitCode::from(2);
    }

    // Never build deeper or wider than the mirror actually holds: doing so
    // produces an index that is silently short, which looks exactly like a
    // successful build.
    if let Some((m_mag, m_min, m_max)) = read_mirror(dir) {
        if max_mag > m_mag + 1e-6 {
            eprintln!(
                "psolve index build: --max-mag {max_mag} is deeper than this mirror, which \
                 was fetched to {m_mag}. Re-fetch deeper or lower --max-mag; building anyway \
                 would produce a silently shallow index."
            );
            return ExitCode::from(2);
        }
        if min_dec < m_min - 1e-9 || max_dec > m_max + 1e-9 {
            eprintln!(
                "psolve index build: declination range {min_dec}..{max_dec} is wider than this \
                 mirror's {m_min}..{m_max}. Re-fetch wider or narrow the range."
            );
            return ExitCode::from(2);
        }
    }

    let mut builder = match Builder::new(nside, max_mag, epoch, name) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("psolve index build: {e}");
            return ExitCode::from(2);
        }
    };

    let t0 = Instant::now();
    eprintln!(
        "reading {} file(s) from {input} (mag<={max_mag}, dec {min_dec}..{max_dec})",
        files.len()
    );

    // Files are read in parallel; each yields its own row vector, then rows are
    // pushed into the single builder. The sort happens once, in finish().
    let per_file: Vec<Vec<psolve_index::gaia::GaiaRow>> = files
        .par_iter()
        .map(|p| {
            let mut rows = Vec::new();
            match File::open(p) {
                Ok(f) => {
                    if let Err(e) = read_ecsv(BufReader::new(f), &names, &filter, |r| rows.push(r))
                    {
                        eprintln!("  warn: {}: {e}", p.display());
                    }
                }
                Err(e) => eprintln!("  warn: {}: {e}", p.display()),
            }
            rows
        })
        .collect();

    for rows in per_file {
        for r in rows {
            builder.push(r.ra, r.dec, r.mag, r.pmra, r.pmdec);
        }
    }

    let mut f = match File::create(out) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("psolve index build: cannot create {out}: {e}");
            return ExitCode::from(2);
        }
    };
    let stats = match builder.finish(&mut f) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("psolve index build: {e}");
            return ExitCode::from(3);
        }
    };
    drop(f);

    let digest = match psolve_index::reader::Index::open(Path::new(out)) {
        Ok(i) => hex(&i.header().records_sha256),
        Err(e) => {
            eprintln!("psolve index build: wrote an index that will not open: {e}");
            return ExitCode::from(3);
        }
    };

    println!(
        "{{\"n_records\":{},\"clamped\":{},\"skipped\":{},\"nside\":{},\"max_mag\":{},\
\"min_dec\":{},\"max_dec\":{},\"epoch\":{},\"name\":\"{}\",\"sha256\":\"{}\",\
\"seconds\":{:.1}}}",
        stats.written,
        stats.clamped,
        stats.skipped,
        nside,
        max_mag,
        min_dec,
        max_dec,
        epoch,
        name,
        digest,
        t0.elapsed().as_secs_f64()
    );
    ExitCode::SUCCESS
}
```

**AMENDMENT (found by the Task 9 review/implementer — five defects in the Step 4/5 code).**

1. `main.rs` dispatches to `cmd_index::info`, which Step 5 never defines — it does not compile. Add a placeholder that exits 2 until Task 10 replaces it.
2. `--max-mag NaN`/`-inf` parses fine and passes `RowFilter::validate()` (which only range-checks declination), then matches nothing — a **zero-record index at exit 0**. Reject non-finite.
3. `--epoch NaN`/`inf` reaches the JSON as a bare `NaN` token — **invalid JSON at exit 0**. Reject non-finite.
4. `--name` is interpolated unescaped into `"name":"{}"`, so a `"` terminates the string early — again invalid JSON at exit 0. Require 1–32 printable ASCII, no quote or backslash. (It is also a fixed 32-byte header field.)
5. `--jobs` parse failure is swallowed by `.ok()` and silently falls back to the core count. Exit 2 instead.
6. `read_mirror` returns `Option`, so a **present-but-corrupt** `mirror.json` is indistinguishable from an absent one and silently disables the guard — reopening the exact silently-short-index hole it exists to close. Use a three-state `Mirror { Absent, Unreadable, Present {..} }` and exit 2 on `Unreadable`.

Every one of these is the same class: **exit 0 with a wrong or malformed result.** Add regression tests asserting exit 2 for each, plus an assertion that success-path stdout starts `{`, ends `}`, and contains no bare `NaN`/`inf`.

- [ ] **Step 6: Run the tests**

Run: `cargo test -p psolve-cli --test cli_build`
Expected: 10 PASS

- [ ] **Step 7: Commit**

```bash
git add crates/psolve-cli
git commit -m "feat(cli): psolve index build"
```

---

## Task 10: CLI `index info`

**Files:**
- Modify: `crates/psolve-cli/src/cmd_index.rs`
- Test: `crates/psolve-cli/tests/cli_info.rs`

**Interfaces:**
- Consumes: `Index` (Task 7), `flag` (Task 9)
- Produces: `psolve index info <FILE> [--verify]`

- [ ] **Step 1: Write the failing test `crates/psolve-cli/tests/cli_info.rs`**

```rust
use std::process::Command;

fn bin() -> std::path::PathBuf {
    let mut p = std::env::current_exe().unwrap();
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("psolve")
}

const SAMPLE: &str = include_str!("../../psolve-index/tests/fixtures/gaia_sample.csv");

fn built_index(tag: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let d = std::env::temp_dir().join(format!("psolve-info-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    let input = d.join("in");
    std::fs::create_dir_all(&input).unwrap();
    std::fs::write(input.join("a.csv"), SAMPLE).unwrap();
    let out = d.join("i.psidx");
    let o = Command::new(bin())
        .args(["index", "build", "--input"])
        .arg(&input)
        .arg("--out")
        .arg(&out)
        .args(["--max-mag", "20", "--name", "info-test"])
        .output()
        .unwrap();
    assert!(o.status.success());
    (d, out)
}

#[test]
fn info_reports_the_header_as_json() {
    let (_d, idx) = built_index("basic");
    let o = Command::new(bin()).args(["index", "info"]).arg(&idx).output().unwrap();
    assert!(o.status.success());
    let s = String::from_utf8_lossy(&o.stdout);
    assert!(s.trim_start().starts_with('{'), "stdout must be JSON: {s}");
    for key in ["\"name\"", "\"nside\"", "\"n_records\"", "\"epoch\"", "\"sha256\"", "\"occupied_cells\""] {
        assert!(s.contains(key), "missing {key} in {s}");
    }
    assert!(s.contains("info-test"));
}

#[test]
fn info_verify_passes_on_a_good_index() {
    let (_d, idx) = built_index("verify-ok");
    let o = Command::new(bin())
        .args(["index", "info", "--verify"])
        .arg(&idx)
        .output()
        .unwrap();
    assert!(o.status.success(), "stderr: {}", String::from_utf8_lossy(&o.stderr));
    assert!(String::from_utf8_lossy(&o.stdout).contains("\"digest_ok\":true"));
}

#[test]
fn info_verify_fails_on_a_corrupted_index_with_exit_3() {
    let (_d, idx) = built_index("verify-bad");
    let mut b = std::fs::read(&idx).unwrap();
    let last = b.len() - 1;
    b[last] ^= 0xff;
    std::fs::write(&idx, &b).unwrap();
    let o = Command::new(bin())
        .args(["index", "info", "--verify"])
        .arg(&idx)
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(3), "index problems exit 3");
}

#[test]
fn info_on_a_missing_file_exits_3() {
    let o = Command::new(bin())
        .args(["index", "info", "/nonexistent/none.psidx"])
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(3));
}

#[test]
fn info_without_a_path_exits_2() {
    let o = Command::new(bin()).args(["index", "info"]).output().unwrap();
    assert_eq!(o.status.code(), Some(2));
}
```

- [ ] **Step 2: Run it to confirm it fails**

Run: `cargo test -p psolve-cli --test cli_info`
Expected: FAIL — no `info` subcommand

- [ ] **Step 3: Append `info` to `cmd_index.rs`**

```rust
pub fn info(args: &[&str]) -> ExitCode {
    let Some(path) = args.iter().find(|a| !a.starts_with("--")) else {
        eprintln!("psolve index info: <FILE> is required");
        return ExitCode::from(2);
    };
    let verify = args.contains(&"--verify");

    let idx = match psolve_index::reader::Index::open(Path::new(path)) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("psolve index info: {e}");
            return ExitCode::from(3);
        }
    };
    let h = idx.header();

    let npix = psolve_index::healpix::npix(h.nside);
    let mut occupied = 0u64;
    let mut max_cell = 0usize;
    for c in 0..npix {
        let l = idx.cell_len(c);
        if l > 0 {
            occupied += 1;
        }
        if l > max_cell {
            max_cell = l;
        }
    }

    let digest_ok = if verify {
        match idx.verify_digest() {
            Ok(()) => true,
            Err(e) => {
                eprintln!("psolve index info: {e}");
                println!(
                    "{{\"name\":\"{}\",\"digest_ok\":false}}",
                    h.name_str()
                );
                return ExitCode::from(3);
            }
        }
    } else {
        false
    };

    println!(
        "{{\"name\":\"{}\",\"version\":{},\"nside\":{},\"npix\":{},\"epoch\":{},\
\"n_records\":{},\"max_mag\":{},\"occupied_cells\":{},\"max_cell_records\":{},\
\"mean_per_occupied_cell\":{:.1},\"sha256\":\"{}\",\"verified\":{},\"digest_ok\":{}}}",
        h.name_str(),
        h.version,
        h.nside,
        npix,
        h.epoch,
        h.n_records,
        h.mag_limit,
        occupied,
        max_cell,
        if occupied > 0 { h.n_records as f64 / occupied as f64 } else { 0.0 },
        hex(&h.records_sha256),
        verify,
        digest_ok
    );
    ExitCode::SUCCESS
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p psolve-cli`
Expected: all 9 CLI tests PASS

- [ ] **Step 5: Commit**

```bash
git add crates/psolve-cli/src/cmd_index.rs crates/psolve-cli/tests/cli_info.rs
git commit -m "feat(cli): psolve index info with --verify"
```

---

## Task 11: Gaia fetch script and a real end-to-end build

**Files:**
- Create: `scripts/fetch-gaia.sh`
- Create: `docs/index-building.md`

**Interfaces:**
- Consumes: `psolve index build` (Task 9), `psolve index info` (Task 10)
- Produces: a real index on disk; no new code interfaces

**Why a script and not a subcommand:** downloading 701 GB is not the solver's job, it needs no Rust, and keeping it out means `psolve` has no HTTP dependency. The script streams each file, extracts the five columns we keep, and discards the rest, so peak disk stays small.

### The mirror is downloaded once and kept

The 701 GB transfer is the expensive artifact; the index is cheap and derived. So the
reduced shards under `<outdir>/shards/` are a **durable local mirror**, and a change of
camera, lens, or even site is an `index build` re-run against them — minutes, no network.

That only holds if the mirror is fetched **wider and deeper than any index you will build
from it**, because its magnitude and declination cuts are baked in and cannot be widened
without re-downloading. Reduced-shard sizes, at ~68 bytes per row against Gaia DR3's
~1.81 billion sources:

| mirror depth | rows | shards on disk |
|---|---:|---:|
| G<14 | ~76M | ~5 GB |
| G<16 | ~212M | ~14 GB |
| **G<18, full sky** | **~512M** | **~35 GB** |

**Fetch G<18 full sky.** On the workstation's 5.5 TB that is a rounding error, it is deeper than
any plate solver needs, and it means the 701 GB download never happens twice. Narrow at
*build* time, never at fetch time, unless disk is genuinely scarce.

The script writes `<outdir>/shards/mirror.json` recording the cuts it used, and
`index build` refuses to exceed them — otherwise asking for a deeper index than the mirror
holds silently produces a short one, which looks exactly like a successful build.

- [ ] **Step 1: Write `scripts/fetch-gaia.sh`**

```bash
#!/usr/bin/env bash
# Fetch Gaia DR3 gaia_source and reduce it to the columns psolve needs.
#
# The full corpus is 3,386 files / ~701 GB gzipped. We never keep that: each
# file is streamed, filtered to (ra, dec, pmra, pmdec, phot_g_mean_mag) at or
# brighter than MAX_MAG, appended to a compact CSV, and discarded. Peak disk is
# one file plus the output.
#
# A fixed observatory never sees the whole sky, so MIN_DEC/MAX_DEC drop stars
# that cannot appear in any frame it takes -- less to download and a smaller
# index. Defaults keep everything.
#
# Usage: scripts/fetch-gaia.sh <outdir> [max_mag] [parallel] [min_dec] [max_dec]
set -euo pipefail

OUT="${1:?usage: fetch-gaia.sh <outdir> [max_mag] [parallel] [min_dec] [max_dec]}"
MAX_MAG="${2:-14}"
PAR="${3:-8}"
MIN_DEC="${4:--90}"
MAX_DEC="${5:-90}"
BASE="https://cdn.gea.esac.esa.int/Gaia/gdr3/gaia_source"
LIST="https://gaia.eu-1.cdn77-storage.com/?prefix=Gaia/gdr3/gaia_source/&delimiter=/"

mkdir -p "$OUT"
cd "$OUT"

# 1. Build the file list (S3-style XML, 1000 keys per page, paginate by marker).
if [ ! -s filelist.txt ]; then
  echo "listing gaia_source ..." >&2
  : > filelist.txt
  marker=""
  while :; do
    url="$LIST"; [ -n "$marker" ] && url="$url&marker=$marker"
    curl -sS --retry 3 --max-time 120 "$url" -o page.xml
    grep -oE '<Key>[^<]*</Key>' page.xml | sed 's/<[^>]*>//g' | grep 'GaiaSource_' >> filelist.txt || true
    grep -q '<IsTruncated>true' page.xml || break
    marker=$(tail -1 filelist.txt | sed 's|/|%2F|g')
  done
  rm -f page.xml
fi
echo "$(wc -l < filelist.txt) files to process" >&2

# 2. Stream, filter, discard. One output shard per input file so this is
#    restartable and parallel-safe.
mkdir -p shards
fetch_one() {
  key="$1"; mag="$2"; dmin="$3"; dmax="$4"
  name=$(basename "$key" .csv.gz)
  [ -s "shards/$name.csv" ] && return 0
  curl -sS --retry 3 --max-time 900 "$BASE/$(basename "$key")" \
    | gunzip -c \
    | awk -v m="$mag" -v dmin="$dmin" -v dmax="$dmax" -F, '
        /^#/ { next }
        !hdr { for (i=1;i<=NF;i++) c[$i]=i; hdr=1;
               print "ra,dec,pmra,pmdec,phot_g_mean_mag" > ("shards/'"$name"'.tmp"); next }
        $c["phot_g_mean_mag"] != "" && ($c["phot_g_mean_mag"]+0) <= m &&
        $c["dec"] != "" && ($c["dec"]+0) >= dmin && ($c["dec"]+0) <= dmax {
               print $c["ra"] "," $c["dec"] "," $c["pmra"] "," $c["pmdec"] "," $c["phot_g_mean_mag"] \
                     >> ("shards/'"$name"'.tmp") }
      '
  # A file entirely outside the declination range yields a header-only shard;
  # that is a completed file, not a failure, so still mark it done.
  [ -f "shards/$name.tmp" ] || printf 'ra,dec,pmra,pmdec,phot_g_mean_mag\n' > "shards/$name.tmp"
  mv "shards/$name.tmp" "shards/$name.csv"
}
export -f fetch_one
export BASE

xargs -P "$PAR" -I{} bash -c 'fetch_one "$@"' _ {} "$MAX_MAG" "$MIN_DEC" "$MAX_DEC" < filelist.txt

# Record what this mirror actually contains. `psolve index build` reads it and
# refuses to build deeper or wider than the mirror holds -- without this, asking
# for more than was fetched yields a silently short index.
ROWS=$(cat shards/*.csv 2>/dev/null | grep -vc '^ra,' || echo 0)
cat > shards/mirror.json <<JSON
{
  "source": "Gaia DR3 gaia_source",
  "url": "$BASE",
  "fetched_utc": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "max_mag": $MAX_MAG,
  "min_dec": $MIN_DEC,
  "max_dec": $MAX_DEC,
  "epoch": 2016.0,
  "files": $(wc -l < filelist.txt),
  "rows": $ROWS
}
JSON

echo "shards written to $OUT/shards" >&2
cat shards/mirror.json >&2
du -sh shards >&2
```

- [ ] **Step 2: Make it executable and smoke-test on two files only**

```bash
chmod +x scripts/fetch-gaia.sh
mkdir -p /tmp/gaia-smoke
```

Run a deliberately tiny check first — **"run a known-good case first, not third"**:

```bash
./scripts/fetch-gaia.sh /tmp/gaia-smoke 14 2 &
sleep 90 && kill %1 2>/dev/null || true
ls -la /tmp/gaia-smoke/shards | head
head -3 /tmp/gaia-smoke/shards/*.csv | head -10
```

Expected: `filelist.txt` holds **3,386** lines; at least one shard exists with a `ra,dec,pmra,pmdec,phot_g_mean_mag` header and numeric rows.

- [ ] **Step 3: Build an index from the smoke shards and inspect it**

```bash
cargo build --release
./target/release/psolve index build \
  --input /tmp/gaia-smoke/shards --out /tmp/gaia-smoke/smoke.psidx \
  --max-mag 14 --nside 64 --name gaia-dr3-g14-smoke
./target/release/psolve index info --verify /tmp/gaia-smoke/smoke.psidx
```

Expected: build prints JSON with a non-zero `n_records`; `info --verify` prints `"digest_ok":true` and exits 0.

**Column handling is already done (Task 8).** `find_columns` treats `source_id` as optional and accepts `--columns` overrides, so the 5-column shards this script writes build without further change. Confirm with the smoke build above rather than editing `gaia.rs` again.

- [ ] **Step 4: Run the full test suite**

Run: `cargo test`
Expected: every test in both crates passes.

- [ ] **Step 5: Write `docs/index-building.md`**

Document, with the real numbers from this plan's "Verified facts" section: the source URL and listing endpoint, the 3,386 files / 701.3 GB figure, the ECSV shape (1,000 comment lines, by-name column lookup), the magnitude/size table, how to run `fetch-gaia.sh`, how to run `index build`, and how to verify with `index info --verify`. State plainly that a full G<14 fetch is bandwidth-bound and takes hours.

Include a **"build your own index"** section covering the options a user tunes for their own site and sources: `--max-mag` (depth), `--min-dec`/`--max-dec` (what their latitude can actually reach, with the φ±90° rule and a worked example for this rig), `--nside`, `--epoch`, and `--columns` for a non-Gaia catalogue — with the Vizier example `--columns ra=RAJ2000,dec=DEJ2000,mag=Vmag,pmra=pmRA,pmdec=pmDE`. State that `--input` accepts any directory of CSVs, so a user who already has catalogue data does not need `fetch-gaia.sh` at all.

Give the **download-once** rule its own section: the mirror is fetched wide and deep and kept; the index is derived and rebuilt freely. Explain `mirror.json`, why `index build` refuses to exceed it, and that psolve has no gzip decoder so shards must be plain `.csv`. Include this rig's worked profile from Step 8 as the example, showing the latitude/horizon/field-size arithmetic rather than just the final numbers.

- [ ] **Step 6: Commit**

```bash
git add scripts/fetch-gaia.sh docs/index-building.md \
        crates/psolve-index/src/gaia.rs crates/psolve-index/tests/gaia.rs
git commit -m "feat(index): gaia fetch script, optional source_id, build docs"
```

- [ ] **Step 7: Fetch the durable mirror (operator's call — hours, bandwidth-bound)**

Fetch **once**, deep and full-sky, so no future change of camera, lens or site ever needs
the 701 GB again:

```bash
./scripts/fetch-gaia.sh ~/gaia-dr3 18 12
```

`18` is the magnitude limit, `12` the parallel fetchers (the workstation has 18 cores; leaving
headroom keeps the machine usable). Declination arguments are deliberately omitted — the
mirror stays full-sky. Expect ~35 GB of shards and a `mirror.json` beside them.

Per the storage conventions this is archival data: keep a copy on the NAS `astro` share
with a `SHA256SUMS`, so re-fetching is never the recovery path.

- [ ] **Step 8: Build this rig's index from the mirror**

The mirror is generic; the index is tuned. Values below are derived from this rig, not
guessed — site latitude **−38.14°** (`core/site.py` default, no `astroops.toml` override),
measured northern horizon **10.0° at az 0** (`~/astroops/state/horizon.json`, 24 points,
2026-07-30), probe hard floor **15°** (`core/planner/ladder.py`), and the frame's
**1.507°** half-diagonal at 2.626° × 1.477°.

| limit | value | why |
|---|---|---|
| `--min-dec` | `-90` | the south celestial pole sits 38.14° up — everything south of dec −51.86° is circumpolar and always available |
| `--max-dec` | `45` | φ+90−floor gives dec +41.86° at the measured 10° northern horizon; +1.51° for the frame's half-diagonal → +43.37°, rounded up |
| `--max-mag` | `14` | records are magnitude-sorted, so extra depth costs disk but **not** solve time — the solver reads only the brightest ~300 per field |
| `--nside` | `64` | 0.92° cells against a 2.63° × 1.48° field → 9–16 cells per query |
| `--epoch` | `2016.0` | Gaia DR3 reference epoch |

```bash
cargo build --release
./target/release/psolve index build \
  --input ~/gaia-dr3/shards \
  --out ~/astroops/data/gaia-dr3-g14-dec45-nside64.psidx \
  --max-mag 14 --min-dec -90 --max-dec 45 --nside 64 --epoch 2016.0 \
  --name gaia-dr3-g14-dec45-nside64
./target/release/psolve index info --verify ~/astroops/data/gaia-dr3-g14-dec45-nside64.psidx
```

The `--max-dec 45` cut removes 14.6% of the celestial sphere — worth taking since those
stars can never appear in a frame, but it is not transformative, and the honest number
belongs in the docs rather than a flattering one.

**A different camera or lens changes only the last command.** A longer focal length wants
a deeper `--max-mag`; a much wider field may prefer `--nside 32`. Both are a rebuild from
the existing mirror, and the mirror's G<18 depth leaves headroom for either.

Record the resulting `n_records`, file size and wall clock in `docs/index-building.md`.
**These are the first real numbers for spec §12.1 (index depth), which the spec explicitly
defers to measurement.**

---

## Definition of done for M1

- [ ] `cargo test` passes with no network access
- [ ] `cargo clippy --all-targets -- -D warnings` is clean (**`--all-targets`**: without it `#[cfg(test)]` code is never linted, so a bare `cargo clippy` is a weaker gate than it looks)
- [ ] `psolve index build` produces an index from Gaia shards
- [ ] `psolve index build` also builds from a non-Gaia CSV via `--columns`, and `--min-dec`/`--max-dec` measurably shrink the result
- [ ] `psolve index info --verify` reports `digest_ok:true` and exits 0
- [ ] A corrupted index exits 3, a missing file exits 3, a usage error exits 2
- [ ] HEALPix agrees with Gaia's `source_id` encoding across all 12 base faces
- [ ] `docs/index-building.md` records the real measured build

**M1 does not solve anything.** That is M2. The gate for starting M2 is: an index exists on disk, `brightest_in_disc` returns plausible stars for a known field, and the digest verifies.
