# Binning-Retry Catalogue Refetch Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** When the XPIXSZ/XBINNING scale retry fires, refetch the catalogue at the corrected search radius so the 791 bin-2 CFA frames that currently solve at 0% can solve.

**Architecture:** `solve_with_binning_retry` in `crates/psolve-cli/src/cmd_solve.rs` currently corrects the plate scale on a failed binned solve but re-solves against the catalogue the caller already fetched at the *uncorrected* radius — twice too wide, four times the sky area for the same star budget. It gains an optional `CatalogRefetch` parameter carrying everything needed to redo the disc query at `radius_header / XBINNING`, and returns the refetched catalogue's diagnostics so the reported JSON describes the catalogue that actually produced the solve. Both hinted entry points pass it; the blind path passes `None` and stays bit-identical.

**Tech Stack:** Rust 2021, no new dependencies (`psolve-core` has none by construction, `psolve-index` has memmap2, `psolve-cli` adds nothing). Tests are `cargo test` integration tests that shell out to the compiled binary. Bash + Python 3 for the measurement scripts.

**Spec:** `docs/superpowers/specs/2026-08-22-binning-retry-catalogue-refetch-design.md`

## Global Constraints

- **`psolve-core` must not gain a filesystem dependency.** Every change in this plan is in `psolve-cli`. `crates/psolve-core/tests/no_filesystem.rs` token-scans `psolve-core/src` for `fs`, `net`, `process`, `env`, `File`, `OpenOptions`, `PathBuf` — **including inside comments** — and fails the build if any appears.
- **No new dependencies in any crate.**
- **`~/astroops/` is strictly read-only.** No `-update` against any path inside it, no sidecar written beside its frames, `sqlite3 -readonly` for every query against `~/astroops/state/catalogue.db`. Copy to a scratch directory first if a test needs to mutate a frame.
- **Any behaviour change must be wired through BOTH entry points** — native `psolve solve` and ASTAP-compatible `psolve -f ...`. A fix of exactly this shape reached `cmd_solve.rs` alone on 2026-08-14 and left ASTAP dispatch stale.
- **Acceptance bar: net-positive AND regression-free.** No frame that solves today may stop solving. Compared per frame, not in aggregate.
- **Tests requiring real rig data skip with an `eprintln!` rather than fail** when the data is absent, matching `real_frames.rs`.
- Commit subjects are `type(scope): summary`. ASCII `--`, never em dashes, in code and commit prose.
- `cargo clippy --workspace --all-targets` must stay clean; `cargo test --workspace` must stay green (baseline: 585 tests passing).

---

## File Structure

| File | Responsibility | Change |
|---|---|---|
| `crates/psolve-cli/src/cmd_solve.rs` | The retry's contract, the refetch, both native call sites | Modify: `solve_with_binning_retry` (`:396-447`), native hinted call site (`:1126`), blind call site (`:795`), `--radius` parsing (`:1039`) |
| `crates/psolve-cli/src/main.rs` | ASTAP-mode dispatch | Modify: call site at `:320` |
| `crates/psolve-cli/src/astap_args.rs` | ASTAP radius rule; supplies the uncapped header radius and the `-r` cap | Modify: add `header_radius_and_cap` |
| `crates/psolve-cli/tests/cli_solve_binning_retry_refetch.rs` | The defect reproduction: an index holding out-of-field decoy stars, so an over-wide disc actually dilutes | Create |
| `scripts/agreement.sh` | The measurement instrument; its sampler is currently blind to bin-2 frames | Modify: stratify on binning, correct the stale comment |
| `docs/superpowers/2026-08-22-binning-retry-refetch-results.md` | Acceptance measurement | Create |

The existing `crates/psolve-cli/tests/cli_solve_binning_retry.rs` is **not** modified. It asserts today's scale-only retry behaviour on an index containing only in-field stars, and must keep passing unchanged — that is part of the regression evidence.

---

### Task 1: Reproduce the defect in a test

The existing retry test passes today because its index contains only the 60 in-field stars, so a disc twice too wide costs nothing — there is nothing else to fetch. This task builds the fixture that makes the over-wide disc bite: bright decoy stars placed outside the true field but inside the inflated disc, plus a `--cat-limit` small enough that the decoys exhaust it.

**Files:**
- Test: `crates/psolve-cli/tests/cli_solve_binning_retry_refetch.rs` (create)

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces: the failing test `a_binned_frame_solves_when_decoys_would_swamp_an_overwide_disc`, which Task 2 makes pass.

- [ ] **Step 1: Write the failing test**

Create `crates/psolve-cli/tests/cli_solve_binning_retry_refetch.rs`:

```rust
//! The catalogue half of the XPIXSZ/XBINNING retry.
//!
//! `cli_solve_binning_retry.rs` already proves the retry corrects the SCALE.
//! It cannot catch this defect: its index holds only the 60 in-field stars,
//! so the disc being twice too wide costs nothing -- there is no other star
//! to fetch instead. Real frames are not like that. This fixture puts bright
//! decoy stars OUTSIDE the true field but INSIDE the inflated disc, so a
//! disc derived from the doubled scale spends the catalogue budget on stars
//! that cannot appear in the frame, exactly as measured on 791 real bin-2
//! sv405 frames (all 0% before this fix).

use psolve_core::fit::Wcs;
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
    let d = std::env::temp_dir()
        .join(format!("psolve-retry-refetch-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

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

const NX: usize = 640;
const NY: usize = 480;
const FOCALLEN_MM: f64 = 243.0;
const PHYSICAL_PIX_UM: f64 = 2.9;
const XBINNING: u32 = 2;
/// The TRUE per-file-pixel scale: the frame's grid is already on-chip binned.
const TRUE_SCALE_ARCSEC: f64 = 206.265 * PHYSICAL_PIX_UM * XBINNING as f64 / FOCALLEN_MM;
/// What this rig's driver writes: the pixel size ALREADY multiplied by
/// binning. `pixel_scale_arcsec` multiplies by XBINNING again, so the
/// header-derived scale is 2x too coarse and the derived radius 2x too wide.
const WRITTEN_XPIXSZ_UM: f64 = PHYSICAL_PIX_UM * XBINNING as f64;

fn truth_wcs(ra0: f64, dec0: f64) -> Wcs {
    let s = TRUE_SCALE_ARCSEC / 3600.0;
    Wcs { crval: [ra0, dec0], crpix: [NX as f64 / 2.0, NY as f64 / 2.0], cd: [[-s, 0.0], [0.0, s]] }
}

/// True field half-diagonal in degrees, from the TRUE scale.
fn true_half_diagonal_deg() -> f64 {
    let w = NX as f64 * TRUE_SCALE_ARCSEC / 3600.0;
    let h = NY as f64 * TRUE_SCALE_ARCSEC / 3600.0;
    (w * w + h * h).sqrt() / 2.0
}

/// The frame, plus a catalogue CSV holding both the in-field stars and a
/// ring of BRIGHTER decoys outside the true field.
///
/// Decoys sit between 1.4x and 1.9x the true half-diagonal: outside the
/// correct disc (half-diagonal x 1.10) and inside the doubled one, so they
/// are fetched only when the radius is wrong. They are brighter than every
/// real star, so `brightest_in_disc` prefers them and the budget is spent
/// before a single in-field star is reached.
fn build_fixture(ra0: f64, dec0: f64, n_field: usize, n_decoy: usize) -> (Vec<u8>, String) {
    let w = truth_wcs(ra0, dec0);
    let margin = 40.0;
    let mut pix = Vec::new();
    for i in 0..n_field {
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
        // Real stars: mag 12.0-12.9.
        csv.push_str(&format!("{ra:.8},{dec:.8},0,0,{:.2}\n", 12.0 + (k % 10) as f64 * 0.1));
    }

    // Decoys: brighter (mag 6-8), outside the true field, inside the doubled disc.
    let hd = true_half_diagonal_deg();
    for i in 0..n_decoy {
        let (u, v) = scatter(100_000 + i);
        let theta = u * std::f64::consts::TAU;
        let rho = hd * (1.4 + 0.5 * v);
        let dec = dec0 + rho * theta.sin();
        let ra = ra0 + rho * theta.cos() / dec0.to_radians().cos().abs().max(1e-6);
        csv.push_str(&format!("{ra:.8},{dec:.8},0,0,{:.2}\n", 6.0 + (i % 20) as f64 * 0.1));
    }

    let cards = [
        "SIMPLE  =                    T".to_string(),
        "BITPIX  =                   16".to_string(),
        "NAXIS   =                    2".to_string(),
        format!("NAXIS1  = {NX:>20}"),
        format!("NAXIS2  = {NY:>20}"),
        "BZERO   =                32768".to_string(),
        format!("FOCALLEN= {FOCALLEN_MM:>20.4}"),
        format!("XPIXSZ  = {WRITTEN_XPIXSZ_UM:>20.4}"),
        format!("XBINNING= {XBINNING:>20}"),
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

fn setup(d: &std::path::Path, ra0: f64, dec0: f64) -> (std::path::PathBuf, std::path::PathBuf) {
    let (fits_bytes, csv) = build_fixture(ra0, dec0, 60, 4000);
    let f = d.join("field.fits");
    std::fs::write(&f, &fits_bytes).unwrap();
    let input = d.join("cat");
    std::fs::create_dir_all(&input).unwrap();
    std::fs::write(input.join("a.csv"), csv).unwrap();
    let idx = d.join("t.psidx");
    let o = Command::new(bin())
        .args(["index", "build", "--input"])
        .arg(&input)
        .arg("--out")
        .arg(&idx)
        .args(["--max-mag", "20", "--nside", "64"])
        .output()
        .unwrap();
    assert!(o.status.success(), "index build failed: {}", String::from_utf8_lossy(&o.stderr));
    (f, idx)
}

#[test]
fn a_binned_frame_solves_when_decoys_would_swamp_an_overwide_disc() {
    let d = tmpdir("native");
    let (ra0, dec0) = (150.0, -10.0);
    let (f, idx) = setup(&d, ra0, dec0);

    // No --scale and no --radius: the CLI derives the wrong (doubled) scale,
    // derives a doubled radius from it, fetches a disc full of decoys, fails,
    // and must then refetch at the corrected radius rather than retrying the
    // corrected scale against the same swamped catalogue.
    let o = Command::new(bin())
        .args(["solve"])
        .arg(&f)
        .arg("--index")
        .arg(&idx)
        .args(["--hint", &format!("{ra0},{dec0}")])
        .args(["--cat-limit", "300"])
        .args(["--saturation", "60000"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&o.stdout).to_string();
    let stderr = String::from_utf8_lossy(&o.stderr).to_string();

    assert_eq!(o.status.code(), Some(0), "stderr: {stderr}\nstdout: {stdout}");
    assert!(stdout.contains("\"solved\":true"), "stdout: {stdout}");
    assert!(
        stdout.contains("\"scale_source\":\"header/binning-retry\""),
        "the retried scale must be what solved it: {stdout}"
    );
    let _ = std::fs::remove_dir_all(&d);
}
```

- [ ] **Step 2: Run the test to verify it fails for the right reason**

Run: `cargo test -p psolve-cli --test cli_solve_binning_retry_refetch -- --nocapture`

Expected: FAIL. The assertion that trips is `o.status.code(), Some(0)` (exit 1) or `"solved":true`, and the captured stdout contains `"reason":"NO_QUAD_MATCH"`.

**If it PASSES, stop.** The fixture is not reproducing the defect — most likely the decoys are not actually inside the inflated disc, or `--cat-limit 300` is large enough to reach the in-field stars anyway. Print the stderr line `N catalogue stars within R deg of ...`, confirm `R` is ~2x `true_half_diagonal_deg() * 1.10`, and raise `n_decoy` until the in-field stars are excluded. A test that passes before the fix proves nothing.

- [ ] **Step 3: Commit the failing test**

```bash
git add crates/psolve-cli/tests/cli_solve_binning_retry_refetch.rs
git commit -m "test(cli): reproduce the binning retry's swamped-catalogue defect

The existing retry test cannot catch this: its index holds only the
in-field stars, so a disc twice too wide fetches the same 60 stars
either way. This fixture adds 4000 brighter decoys outside the true
field and inside the doubled disc, so the wrong radius spends the
whole --cat-limit budget before reaching a star that appears in the
frame -- the measured behaviour of all 791 real bin-2 sv405 frames.

Fails today with NO_QUAD_MATCH, by design."
```

---

### Task 2: Refetch the catalogue inside the retry

**Files:**
- Modify: `crates/psolve-cli/src/cmd_solve.rs` — `solve_with_binning_retry` (`:396-447`), blind call site (`:795`), native hinted call site (`:1126`), `--radius` parsing (`:1039`)
- Test: `crates/psolve-cli/tests/cli_solve_binning_retry_refetch.rs` (Task 1's, unchanged)

**Interfaces:**
- Consumes: Task 1's failing test. `select_catalog(index: &Index, ra_deg: f64, dec_deg: f64, radius_deg: f64, limit: usize) -> CatalogSelection`, where `CatalogSelection { recs: Vec<psolve_index::record::StarRecord>, concentration: Option<f64>, stratified: bool }` — all already `pub(crate)` in this file.
- Produces:
  - `pub(crate) struct CatalogRefetch<'a>` as defined below.
  - `pub(crate) struct SolveAttempt { outcome: Outcome, scale_source: &'static str, refetched: Option<CatalogSelection> }`.
  - `solve_with_binning_retry(path: &str, prepared: &PreparedFrame, catalog: &[CatalogStar], opts: &SolveOptions, hdr: Option<&FitsHeader>, explicit_scale_given: bool, refetch: Option<CatalogRefetch<'_>>) -> SolveAttempt` — the return type changes from `(Outcome, &'static str)`, so all three call sites must be updated in this task or the crate will not compile.

- [ ] **Step 1: Add the two structs**

In `crates/psolve-cli/src/cmd_solve.rs`, immediately above `solve_with_binning_retry`:

```rust
/// Everything the binning retry needs to redo the catalogue disc query at a
/// corrected radius. Passed by the two HINTED entry points only.
///
/// `radius_header_deg` is deliberately the UNCAPPED header-derived radius,
/// not the radius the first fetch actually used. Dividing an already-capped
/// radius by `XBINNING` and dividing the header value then re-capping are
/// different numbers whenever the cap bound the first fetch, and only the
/// latter is correct. The distinction is invisible in native mode (no cap)
/// and load-bearing in ASTAP mode, whose `-r` is a caller ceiling.
pub(crate) struct CatalogRefetch<'a> {
    pub(crate) index: &'a Index,
    pub(crate) hint_ra: f64,
    pub(crate) hint_dec: f64,
    pub(crate) radius_header_deg: f64,
    /// ASTAP mode's `-r` ceiling. `None` in native mode.
    pub(crate) radius_cap: Option<f64>,
    pub(crate) limit: usize,
    /// Native mode's `--radius`. A caller-supplied radius is an assertion,
    /// exactly as `--scale` is, and is never overridden: the retry falls
    /// back to today's scale-only behaviour.
    pub(crate) explicit_radius: bool,
}

/// One solve attempt's result, including which scale solved it and -- when
/// the retry refetched -- the catalogue that actually produced it.
pub(crate) struct SolveAttempt {
    pub(crate) outcome: Outcome,
    pub(crate) scale_source: &'static str,
    /// `Some` only when a refetch happened. The caller must report THIS
    /// selection's diagnostics rather than its own first fetch's: reporting
    /// the first disc's concentration for a solve produced by the second is
    /// a plausible-looking wrong number, which is the failure shape this
    /// codebase pays for most.
    pub(crate) refetched: Option<CatalogSelection>,
}
```

- [ ] **Step 2: Rewrite the retry body**

Replace the body of `solve_with_binning_retry` (`crates/psolve-cli/src/cmd_solve.rs:396-447`) with:

```rust
pub(crate) fn solve_with_binning_retry(
    path: &str,
    prepared: &psolve_core::solve::PreparedFrame,
    catalog: &[CatalogStar],
    opts: &SolveOptions,
    hdr: Option<&psolve_core::fits::FitsHeader>,
    explicit_scale_given: bool,
    refetch: Option<CatalogRefetch<'_>>,
) -> SolveAttempt {
    let mut result = psolve_core::solve::solve_prepared(prepared, catalog, opts);
    let mut scale_source = if explicit_scale_given { "explicit" } else { "header" };
    let mut refetched = None;

    // XPIXSZ is ambiguous when XBINNING > 1: `pixel_scale_arcsec` assumes
    // XPIXSZ is the PHYSICAL pixel and multiplies it by the binning factor,
    // which is correct for most rigs -- but this project's sv405 rig's
    // driver already writes the ALREADY-BINNED pixel into XPIXSZ, so
    // multiplying by binning again overstates the plate scale by another
    // factor of `binning`. There is no reliable way to tell which convention
    // a single header used, so this does not guess: it solves at the
    // header-derived scale first and, only on failure, retries ONCE at
    // scale/binning.
    //
    // The retry must also redo the CATALOGUE, not just the scale. The disc
    // radius is derived from the same inflated scale, so it comes out
    // `xbinning` times too wide -- 6.02 deg where the frame needs 3.01 --
    // and the star budget is spent across `xbinning^2` times too much sky.
    // Measured 2026-08-22: 0 of 791 real bin-2 frames solved with the scale
    // corrected but the catalogue left alone; 790 of 791 solve once the disc
    // is right. Correcting the scale against a swamped catalogue reports
    // NO_QUAD_MATCH, which reads as "unsolvable frame" rather than "the disc
    // was twice too wide".
    if matches!(result, Outcome::Failed { .. }) && !explicit_scale_given {
        let xbinning = hdr.and_then(|h| h.num("XBINNING")).unwrap_or(1.0);
        let header_scale = hdr.and_then(psolve_core::fits::pixel_scale_arcsec).map(|s| {
            let cfa_binning = hdr.map(psolve_core::fits::binning_factor).unwrap_or(1);
            s * cfa_binning as f64
        });
        if let (true, Some(header_scale)) = (xbinning > 1.0, header_scale) {
            let alt_scale = header_scale / xbinning;
            eprintln!(
                "solving {path}: header scale {header_scale:.4}\"/px did not solve; \
retrying once at {alt_scale:.4}\"/px (scale / XBINNING {xbinning:.0}, in case XPIXSZ was \
already binned)"
            );
            let mut retry_opts = *opts;
            retry_opts.scale_arcsec = Some(alt_scale);

            // Refetch at the corrected radius when the caller supplied the
            // means and did not assert a radius of its own. Skipped when the
            // corrected radius equals what was already fetched (a cap
            // binding both times) -- no query is issued to arrive at the
            // same disc.
            let alt_catalog: Option<(Vec<CatalogStar>, CatalogSelection)> = refetch
                .as_ref()
                .filter(|r| !r.explicit_radius)
                .and_then(|r| {
                    let first = r.radius_cap.map_or(r.radius_header_deg, |c| r.radius_header_deg.min(c));
                    let corrected = r.radius_header_deg / xbinning;
                    let corrected = r.radius_cap.map_or(corrected, |c| corrected.min(c));
                    if (corrected - first).abs() < 1e-9 {
                        return None;
                    }
                    eprintln!(
                        "solving {path}: refetching the catalogue at {corrected:.4} deg \
(was {first:.4}) -- the first disc was derived from the uncorrected scale"
                    );
                    let sel = select_catalog(r.index, r.hint_ra, r.hint_dec, corrected, r.limit);
                    let stars: Vec<CatalogStar> = sel
                        .recs
                        .iter()
                        .map(|s| CatalogStar {
                            ra: s.ra_deg(),
                            dec: s.dec_deg(),
                            mag: s.mag(),
                            pmra: s.pmra_mas_yr(),
                            pmdec: s.pmdec_mas_yr(),
                        })
                        .collect();
                    Some((stars, sel))
                });

            let retry_result = match &alt_catalog {
                Some((stars, _)) => psolve_core::solve::solve_prepared(prepared, stars, &retry_opts),
                None => psolve_core::solve::solve_prepared(prepared, catalog, &retry_opts),
            };
            if matches!(retry_result, Outcome::Solved(_)) {
                scale_source = "header/binning-retry";
                refetched = alt_catalog.map(|(_, sel)| sel);
            }
            result = retry_result;
        }
    }

    SolveAttempt { outcome: result, scale_source, refetched }
}
```

- [ ] **Step 3: Update the blind call site (no refetch)**

At `crates/psolve-cli/src/cmd_solve.rs:795`, replace:

```rust
        let (result, scale_source) =
            solve_with_binning_retry(path, prepared, &catalog, &attempt_opts, hdr, explicit_scale_given);
```

with:

```rust
        // `None`: the blind path's disc is centred on a CANDIDATE CLUSTER,
        // not on a hint, and its radius is not the header-derived one this
        // correction divides. Refetching here is unmeasured, and the blind
        // path was proved bit-identical to the hinted one's behaviour in the
        // blind-solve milestone's Task 6 -- it stays that way until someone
        // measures it. Deliberate omission, not an oversight.
        let attempt =
            solve_with_binning_retry(path, prepared, &catalog, &attempt_opts, hdr, explicit_scale_given, None);
        let (result, scale_source) = (attempt.outcome, attempt.scale_source);
```

- [ ] **Step 4: Track whether `--radius` was explicit**

At `crates/psolve-cli/src/cmd_solve.rs:1039`, replace:

```rust
    let radius_deg = match flag(args, "--radius") {
```

with:

```rust
    let explicit_radius = flag(args, "--radius").is_some();
    let radius_deg = match flag(args, "--radius") {
```

- [ ] **Step 5: Update the native hinted call site**

At `crates/psolve-cli/src/cmd_solve.rs:1126`, replace:

```rust
                    let (result, source) = solve_with_binning_retry(
                        path,
                        &prepared,
                        &catalog,
                        &opts,
                        hdr.as_ref(),
                        scale_arcsec_raw.is_some(),
                    );
                    scale_source = source;
                    result
```

with:

```rust
                    let attempt = solve_with_binning_retry(
                        path,
                        &prepared,
                        &catalog,
                        &opts,
                        hdr.as_ref(),
                        scale_arcsec_raw.is_some(),
                        Some(CatalogRefetch {
                            index: &index,
                            hint_ra: hra,
                            hint_dec: hdec,
                            // Native mode has no cap, so the header-derived
                            // radius IS what was fetched unless --radius was
                            // given, in which case `explicit_radius`
                            // suppresses the refetch entirely.
                            radius_header_deg: header_radius_deg(hdr.as_ref()).unwrap_or(radius_deg),
                            radius_cap: None,
                            limit,
                            explicit_radius,
                        }),
                    );
                    scale_source = attempt.scale_source;
                    // Report the catalogue that actually produced the solve.
                    if let Some(sel) = &attempt.refetched {
                        cat_concentration = sel.concentration;
                        cat_stratified = sel.stratified;
                    }
                    attempt.outcome
```

- [ ] **Step 6: Run the new test**

Run: `cargo test -p psolve-cli --test cli_solve_binning_retry_refetch -- --nocapture`
Expected: PASS, with stderr showing both the `retrying once at ...` line and the new `refetching the catalogue at ...` line.

- [ ] **Step 7: Run the whole suite and clippy**

Run: `cargo test --workspace 2>&1 | grep -E '^test result|FAILED'`
Expected: every line `ok`, 0 failed. In particular `cli_solve_binning_retry.rs` must still pass unchanged.

Run: `cargo clippy --workspace --all-targets`
Expected: no warnings.

- [ ] **Step 8: Commit**

```bash
git add crates/psolve-cli/src/cmd_solve.rs crates/psolve-cli/tests/cli_solve_binning_retry_refetch.rs
git commit -m "fix(cli): refetch the catalogue when the binning retry fires

The retry corrected the plate scale and then re-solved against the
catalogue the caller had already fetched at the UNCORRECTED radius --
6.02 deg where the frame needs 3.01, four times the sky area for the
same star budget -- so it reported NO_QUAD_MATCH on frames whose only
problem was the disc. Measured: 0 of 791 real bin-2 sv405 frames solved
with the scale corrected alone; 790 of 791 solve once the disc is right.

The retry now redoes the disc query at radius_header/XBINNING, re-caps
it against ASTAP mode's -r, and reports the refetched catalogue's own
concentration/stratified diagnostics rather than the first disc's. A
caller-supplied --radius suppresses the refetch, as --scale already
suppresses the whole retry. The blind path passes None deliberately.

Reachable only after a first attempt has already failed, so no frame
that solves today can change: all 9342 currently-solving corpus frames
report scale_source \"header\"."
```

---

### Task 3: Wire and prove the ASTAP entry point

**Files:**
- Modify: `crates/psolve-cli/src/astap_args.rs` (add `header_radius_and_cap`)
- Modify: `crates/psolve-cli/src/main.rs:320`
- Test: `crates/psolve-cli/tests/cli_solve_binning_retry_refetch.rs` (append)

**Interfaces:**
- Consumes: `CatalogRefetch`, `SolveAttempt`, and the new `solve_with_binning_retry` signature from Task 2. `astap_args::search_radius_deg(a: &AstapArgs, hdr: Option<&FitsHeader>) -> f64` and `cmd_solve::header_radius_deg(hdr: Option<&FitsHeader>) -> Option<f64>`, both existing.
- Produces: `astap_args::header_radius_and_cap(a: &AstapArgs, hdr: Option<&FitsHeader>) -> (Option<f64>, f64)` — the uncapped header-derived radius and the `-r` ceiling, for `CatalogRefetch`.

- [ ] **Step 1: Write the failing ASTAP-mode test**

Append to `crates/psolve-cli/tests/cli_solve_binning_retry_refetch.rs`:

```rust
/// The same frame through the ASTAP-compatible surface. A fix of exactly
/// this shape reached `cmd_solve.rs` alone on 2026-08-14 and left this
/// dispatch stale; `ingest.identify.astap_solve` calls THIS path, so a fix
/// that lands only in native mode does not reach production at all.
#[test]
fn the_astap_entry_point_refetches_too() {
    let d = tmpdir("astap");
    let (ra0, dec0) = (150.0, -10.0);
    let (f, idx) = setup(&d, ra0, dec0);

    // ASTAP mode resolves its index from the directory holding the .psidx.
    let db_dir = idx.parent().unwrap().to_path_buf();
    // -ra is HOURS, -spd is dec + 90. -r 180 is AstroOps' own blind form;
    // the header narrows it, so the cap does not bind here.
    let o = Command::new(bin())
        .arg("-f")
        .arg(&f)
        .args(["-ra", &format!("{}", ra0 / 15.0)])
        .args(["-spd", &format!("{}", dec0 + 90.0)])
        .args(["-r", "180"])
        .arg("-d")
        .arg(&db_dir)
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&o.stderr).to_string();
    assert_eq!(o.status.code(), Some(0), "ASTAP mode must solve it too. stderr: {stderr}");

    let ini = f.with_extension("ini");
    let text = std::fs::read_to_string(&ini).unwrap_or_else(|e| panic!("reading {ini:?}: {e}"));
    assert!(text.contains("PLTSOLVD=T"), "the .ini must record a solve: {text}");
    assert!(
        stderr.contains("refetching the catalogue at"),
        "the refetch must fire on this path, not just in native mode: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&d);
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p psolve-cli --test cli_solve_binning_retry_refetch the_astap_entry_point -- --nocapture`
Expected: FAIL — exit code 1, and stderr has no `refetching the catalogue at` line, because `main.rs` still passes no refetch.

- [ ] **Step 3: Add the radius/cap accessor**

In `crates/psolve-cli/src/astap_args.rs`, directly below `search_radius_deg`:

```rust
/// The two numbers [`search_radius_deg`] combines, kept separate for the
/// binning retry's refetch: the UNCAPPED header-derived radius (`None` when
/// the header lacks the optics keywords) and the caller's `-r` ceiling.
///
/// The retry divides the header-derived radius by `XBINNING` and re-applies
/// the ceiling. Dividing `search_radius_deg`'s already-capped output instead
/// would produce a different, wrong number whenever `-r` bound the first
/// fetch.
pub fn header_radius_and_cap(
    a: &AstapArgs,
    hdr: Option<&psolve_core::fits::FitsHeader>,
) -> (Option<f64>, f64) {
    (crate::cmd_solve::header_radius_deg(hdr), a.radius_deg)
}
```

- [ ] **Step 4: Pass the refetch from ASTAP dispatch**

At `crates/psolve-cli/src/main.rs:320`, replace:

```rust
                    let (outcome, _scale_source) = cmd_solve::solve_with_binning_retry(
                        &parsed.file,
                        &prepared,
                        &catalog,
                        &opts,
                        hdr.as_ref(),
                        false,
                    );
                    outcome
```

with:

```rust
                    let (header_radius, radius_cap) =
                        astap_args::header_radius_and_cap(&parsed, hdr.as_ref());
                    let attempt = cmd_solve::solve_with_binning_retry(
                        &parsed.file,
                        &prepared,
                        &catalog,
                        &opts,
                        hdr.as_ref(),
                        false,
                        header_radius.map(|r| cmd_solve::CatalogRefetch {
                            index: &index,
                            hint_ra: hra,
                            hint_dec: hdec,
                            radius_header_deg: r,
                            radius_cap: Some(radius_cap),
                            limit,
                            // ASTAP's grammar has no radius-assertion flag:
                            // `-r` is a ceiling, applied above, not a
                            // caller-chosen disc. So the refetch is never
                            // suppressed here.
                            explicit_radius: false,
                        }),
                    );
                    attempt.outcome
```

- [ ] **Step 5: Run the ASTAP test**

Run: `cargo test -p psolve-cli --test cli_solve_binning_retry_refetch -- --nocapture`
Expected: both tests PASS.

- [ ] **Step 6: Run the full suite and clippy**

Run: `cargo test --workspace 2>&1 | grep -E '^test result|FAILED'`
Expected: all `ok`, 0 failed. `astap_cli.rs`, `astap_exit_codes.rs` and `astap_binning_retry.rs` must be unchanged and passing.

Run: `cargo clippy --workspace --all-targets`
Expected: clean.

- [ ] **Step 7: Commit**

```bash
git add crates/psolve-cli/src/main.rs crates/psolve-cli/src/astap_args.rs crates/psolve-cli/tests/cli_solve_binning_retry_refetch.rs
git commit -m "fix(cli): wire the catalogue refetch into ASTAP dispatch too

ingest.identify.astap_solve calls this path, not native solve_cmd, so a
refetch that landed only in cmd_solve.rs would not reach production --
the exact omission the 2026-08-14 scale retry made here.

-r is a ceiling, not a caller-chosen disc, so it is re-applied to the
corrected radius rather than suppressing the refetch; header_radius_and_cap
keeps the uncapped header value separate for that reason."
```

---

### Task 4: Un-blind the measurement instrument

`scripts/agreement.sh`'s sampler asserts there are no bin-2 rows to draw from. There are 791. Until this is fixed, `agreement.sh sample` — the cheap iteration path — structurally cannot see the population this whole plan is about.

**Files:**
- Modify: `scripts/agreement.sh` (the stratified sampler, around the `Binning is NOT stratified` comment at `:100`)

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces: a sampler that draws bin-2 frames, and fails loudly rather than silently omitting a stratum.

- [ ] **Step 1: Confirm the claim is false before changing anything**

Run:

```bash
sqlite3 -readonly ~/astroops/state/catalogue.db "
SELECT f.binning, COUNT(*) FROM measurement m JOIN frame f ON f.id=m.frame_id
WHERE m.tool_version='astap/astap+d50' AND m.ra_deg IS NOT NULL GROUP BY 1;"
```

Expected: two rows, `1|9587` and `2|791` (counts may drift; the point is that row `2` exists and is non-zero).

- [ ] **Step 2: Replace the stale comment and stratify on binning**

In `scripts/agreement.sh`, replace the comment block beginning `# Stratified sample across the axes task-11-brief.md section 5 calls out:` through the end of the sentence ending `rather than a real cross-section.` with:

```
# Stratified sample across the axes task-11-brief.md section 5 calls out:
# both frame-dimension buckets, OSC vs each mono filter, and the full
# declination range -- AND binning.
#
# Binning was originally NOT stratified, on the stated grounds that "every
# ASTAP-solved frame in this database is binning=1 ... there are no 2x2 rows
# to draw from". That was true when written and expired silently: as of
# 2026-08-22 there are 791 binning=2 rows, and every one of them failed to
# solve. A sampler that omits an axis it assumes is empty cannot see a
# population that appears later, which is exactly how a 0%-solving group of
# 791 frames sat behind a reported 97.6% headline. Binning is now a real
# stratum, and an assumed-empty stratum that turns out to be populated is a
# loud failure below, never a silent omission.
```

- [ ] **Step 3: Add binning to the stratification key and assert coverage**

In the Python heredoc that follows, find where the stratification key is built from the frame-dimension bucket, filter and declination band, and add the binning column (index 5) to that tuple. Then, immediately after the sample is selected and before it is written out, add:

```python
# A stratum that exists in the population must appear in the sample. The
# sampler used to assume binning=2 did not exist; when that assumption
# expired, nothing said so. This makes the same class of mistake loud.
pop_binning = {r[5] for r in rows}
sample_binning = {r[5] for r in selected}
missing = pop_binning - sample_binning
if missing:
    raise SystemExit(
        f"agreement.sh: sample omits binning stratum/strata {sorted(missing)} "
        f"present in the population -- raise N or fix the sampler; a silently "
        f"omitted stratum is how the 791 bin-2 frames went unseen"
    )
```

- [ ] **Step 4: Verify the sampler now draws bin-2 frames**

Run:

```bash
cargo build --release
scripts/agreement.sh sample 300 /tmp/psolve-sample-check.ndjson
python3 -c "
import json,collections
c=collections.Counter(json.loads(l).get('binning') for l in open('/tmp/psolve-sample-check.ndjson'))
print('sample by binning:', dict(c))
assert c.get(2,0) > 0, 'sampler still blind to bin-2 frames'
print('OK')
"
```

Expected: `sample by binning: {1: ..., 2: ...}` with a non-zero bin-2 count, then `OK`.

- [ ] **Step 5: Commit**

```bash
git add scripts/agreement.sh
git commit -m "fix(scripts): stratify agreement sampling on binning

The sampler asserted 'there are no 2x2 rows to draw from'. True when
written, false by 791 rows now, and the consequence was that the cheap
iteration path could not see the only population that was failing --
a 0%-solving group of 791 frames behind a 97.6% headline.

Binning is a stratum now, and a populated stratum missing from the
sample raises instead of being silently dropped."
```

---

### Task 5: Acceptance measurement

**Files:**
- Create: `docs/superpowers/2026-08-22-binning-retry-refetch-results.md`

**Interfaces:**
- Consumes: the fixed binary from Tasks 2-3 and the fixed sampler from Task 4.
- Produces: the measured record the spec's §8 criteria are judged against.

- [ ] **Step 1: Build the release binary and capture the baseline**

```bash
cargo build --release
git stash list  # ensure a clean tree; the build id must not say -dirty
./target/release/psolve solve --help >/dev/null && echo built
```

The pre-fix baseline is already recorded and does not need re-running: 0/791 bin-2, 9342/10373 overall, all solving frames `scale_source: "header"` (see the spec §1, §2).

- [ ] **Step 2: Measure the bin-2 population**

```bash
sqlite3 -readonly ~/astroops/state/catalogue.db <<'SQL' > /tmp/psolve-bin2.tsv
SELECT l.path, m.ra_deg, m.dec_deg, f.id, f.ra_deg, f.dec_deg, f.binning
FROM measurement m JOIN frame f ON f.id=m.frame_id
JOIN location l ON l.frame_id=f.id AND l.path=(
  SELECT l2.path FROM location l2 WHERE l2.frame_id=f.id AND l2.intact=1
  ORDER BY (l2.tree='library') DESC, l2.path ASC LIMIT 1)
WHERE m.tool_version='astap/astap+d50' AND m.ra_deg IS NOT NULL AND l.intact=1
  AND f.binning=2;
SQL
wc -l < /tmp/psolve-bin2.tsv   # expect 791
```

Then run every frame through the release binary with the same invocation `agreement.sh` uses (`psolve solve <path> --index <g14 psidx> --hint <ra>,<dec>`, no other flags), record solved/reason/separation per frame to NDJSON, and report: solved count, rate, separation median/p90/max, and the count over 30″.

**Criterion 1 passes at ≥ 785 of 791 solved with median separation ≤ 1.0″.** The measured ceiling for this approach is 790/791 at 0.707″.

- [ ] **Step 3: Measure the full corpus and diff per frame**

```bash
scripts/agreement.sh full /tmp/psolve-agreement-postfix.ndjson
```

Compare against `.scratch/agreement-full-current.ndjson` **per frame_id**, not in aggregate:

```python
import json
def load(p): return {r["frame_id"]: r for r in (json.loads(l) for l in open(p))}
pre  = load(".scratch/agreement-full-current.ndjson")
post = load("/tmp/psolve-agreement-postfix.ndjson")
sv = lambda d, i: bool((d[i].get("psolve") or {}).get("solved"))
ids = set(pre) & set(post)
regressed = sorted(i for i in ids if sv(pre, i) and not sv(post, i))
newly     = sorted(i for i in ids if not sv(pre, i) and sv(post, i))
print(f"regressed {len(regressed)}  newly {len(newly)}  net {len(newly)-len(regressed):+d}")
print("regressed ids:", regressed[:50])
```

**Criterion 2 requires `regressed == 0`.** Any non-zero count contradicts §7's structural argument and must be investigated before anything ships — a regressed frame means something reaches the refetch that should not.

Note the pre-fix file is from a superseded binary on a slightly smaller corpus; frames present in only one run are excluded by the `set(pre) & set(post)` intersection, and the excluded count must be reported rather than passed over.

**Criterion 3 requires an overall solve rate ≥ 97.5%.**

- [ ] **Step 4: Arbitrate the 38.2″ outlier**

The corrected-radius run produces one solve disagreeing with ASTAP's recorded centre by 38.2″, over the 30″ bar. It is not a regression — the frame does not solve at all today — but it gets the same treatment the NGC 3372 case got in `docs/superpowers/2026-08-14-m3-first-real-frame.md`: reproject Gaia catalogue stars through each candidate WCS, measure pixel flux at the predicted positions, and check the metric against two clean session-neighbour control frames so its reliability is visible rather than assumed. Report the finding whichever way it lands.

Also record the reason code of the one frame that still fails with the corrected radius. Both go in the results document; neither is waved away.

- [ ] **Step 5: Write the results document**

Create `docs/superpowers/2026-08-22-binning-retry-refetch-results.md` covering, with the numbers actually measured and never a projection:

1. Bin-2 population before and after, with separation distribution.
2. Full-corpus solve rate before and after, and the **per-frame** regression count.
3. Both entry points confirmed on the same frame.
4. The 38.2″ arbitration and the one remaining failure's reason code.
5. Each of the spec's six acceptance criteria marked PASS or FAIL **on its own terms** — a criterion that fails is reported as failing even when the explanation is benign, following `2026-08-15-blind-solve-results.md`'s criterion 5.
6. **The CFA decode fix's status**, recorded so it is not re-litigated: `7ebda12` cherry-picks cleanly onto HEAD, is correct in isolation (double-binning a hardware-binned CFA frame is simply wrong), and costs 14 net frames on top of a corrected radius because `extract.rs`'s fixed `min_pix = 4` is tuned around the coarser double-binned plate scale. It stays unmerged pending scale-aware extraction.

- [ ] **Step 6: Update the headline docs**

If criteria 1-3 pass, update the solve-rate figures in `README.md` ("Measured, not projected") and `docs/astap-compat.md` (the agreement table), following that table's existing two-column convention: keep the prior column, add the new one, and state what changed and why. Do not overwrite a measurement — this repository retracts in place.

- [ ] **Step 7: Commit**

```bash
git add docs/superpowers/2026-08-22-binning-retry-refetch-results.md README.md docs/astap-compat.md
git commit -m "docs: binning-retry refetch acceptance measurement

<measured bin-2 rate>, <measured corpus rate>, <regression count> per-frame
regressions. Criterion-by-criterion verdicts, the 38.2\" arbitration, and
the decode fix's recorded status."
```

---

## Self-Review

**Spec coverage.** §1-2 (problem, root cause) → Tasks 1-2. §3 (decode fix is not the fix) → Task 5 Step 5 item 6, recorded rather than implemented. §4 rules 1-2 (corrected radius, explicit radius never overridden) → Task 2 Steps 2, 4, 5. §4 rule 3 (no optics keywords → retry never fires) → unchanged code path, asserted by the existing suite staying green in Task 2 Step 7. §4 rule 4 + §5 (shared function, both entry points) → Tasks 2-3. §6 (cost) → no task; it is an argument about a ~1 ms query on already-failed frames, and Task 5's timing is unaffected. §7 (regression safety) → Task 5 Step 3. §8 criteria 1-6 → Task 5 Steps 2-5. §9 (instrument) → Task 4. §10-11 (risks, deferrals) → Task 5 Step 5 item 6.

**Type consistency.** `CatalogRefetch` fields are identical in Task 2 Step 1, Task 2 Step 5 and Task 3 Step 4. `SolveAttempt` is constructed once (Task 2 Step 2) and destructured in three places (Task 2 Steps 3, 5; Task 3 Step 4). `select_catalog` and `CatalogSelection` are used exactly as they exist today. `header_radius_deg` returns `Option<f64>` and is unwrapped with a fallback at both hinted call sites.

**Known gap, deliberate.** §6's cost claim gets no measurement task. One extra ~1 ms disc query on frames that have already failed is not worth a benchmark, and Task 5 Step 3's full-corpus run would surface any real regression in wall clock.

---

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-08-22-binning-retry-catalogue-refetch.md`. Two execution options:

**1. Subagent-Driven (recommended)** — a fresh subagent per task, review between tasks, fast iteration.

**2. Inline Execution** — execute tasks in this session using executing-plans, batch execution with checkpoints.
