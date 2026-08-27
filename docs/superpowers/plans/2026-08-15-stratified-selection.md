# Spatially Stratified Star Selection — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Select stars spatially rather than purely by brightness, so dense fields stop failing to match.

**Architecture:** Two symmetric changes — one in `psolve-core`'s extraction (image side), one in `psolve-index`'s reader (catalogue side) — plus wiring at **both** CLI entry points. Each side partitions space, gives every cell a budget, fills it with that cell's brightest, then redistributes unused budget so sparse frames keep every star they had.

**Tech Stack:** Rust 2021. `psolve-core` (zero deps, no filesystem), `psolve-index` (mmap), `psolve-cli` (`psolve-index`, `psolve-core`, `rayon`).

**Spec:** `docs/superpowers/specs/2026-08-15-stratified-selection-design.md`

## Global Constraints

- `psolve-core`: **zero dependencies, no filesystem access.** `tests/no_filesystem.rs` tokenises whole words **including inside comments** — avoid the bare tokens `fs`, `net`, `process`, `env`, `File`, `OpenOptions`, `PathBuf` even in prose.
- `psolve-cli` may depend only on `psolve-index`, `psolve-core`, `rayon`. No new dependency.
- No `unwrap()`/`expect()` on external data.
- `cargo clippy --all-targets --workspace -- -D warnings` clean; `cargo test --workspace` green. **Baseline 414 tests**, count must not drop.
- **`~/astroops/` and `~/mnt/astro/` are STRICTLY READ-ONLY.** Never pass `-update`; prefer the native `psolve solve` (writes no sidecars) or `-o` into scratch. Hash frames before/after with `shasum -a 256`. Verify non-modification with an absolute `touch -t` reference — **never `find -newermt`** (this machine's `find` is `bfs`; it rejects relative timestamps, prints nothing, and exits non-zero, so it proves nothing).
- Query the database with `sqlite3 -readonly`.
- Commit messages cite measured numbers, never estimates.

---

## File Structure

| File | Responsibility | Tasks |
|---|---|---|
| `crates/psolve-core/src/extract.rs` | image-side stratified keep | 1 |
| `crates/psolve-index/src/reader.rs` | catalogue-side stratified fetch | 2 |
| `crates/psolve-cli/src/cmd_solve.rs` | native entry point wiring | 3 |
| `crates/psolve-cli/src/main.rs` | **ASTAP entry point wiring** | 3 |
| `scripts/` + decision record | measurement and results | 4, 5 |

---

### Task 1: Image-side stratified selection

`extract.rs:232-233` currently ends with:

```rust
    stars.sort_unstable_by(|p, q| q.flux.partial_cmp(&p.flux).unwrap_or(std::cmp::Ordering::Equal));
    stars.truncate(p.keep);
```

That takes the globally brightest `keep`. In Omega Centauri all 500 come from the cluster core.

**Files:** Modify `crates/psolve-core/src/extract.rs`. Tests co-located in its existing `mod tests`.

**Interfaces:**
- Produces: `fn stratified_keep(stars: Vec<Star>, nx: usize, ny: usize, keep: usize) -> Vec<Star>` — `pub(crate)` is enough; `extract()`'s signature does not change.
- `Star` has `x: f64`, `y: f64`, `flux: f64`. `Image` has `nx`, `ny`.

- [ ] **Step 1: Write the failing tests**

```rust
/// The whole point: a clump must not consume the entire budget.
#[test]
fn a_dense_clump_does_not_crowd_out_the_rest_of_the_frame() {
    let mut stars = Vec::new();
    // 400 bright stars packed into a 40x40 px corner -- the "cluster core".
    for i in 0..400 {
        stars.push(mk_star((i % 20) as f64, (i / 20) as f64, 10_000.0 - i as f64));
    }
    // 100 fainter stars spread across a 1000x1000 frame.
    for i in 0..100 {
        stars.push(mk_star((i * 7 % 1000) as f64, (i * 13 % 1000) as f64, 100.0));
    }
    let kept = stratified_keep(stars, 1000, 1000, 100);
    assert_eq!(kept.len(), 100, "the budget must be filled");
    let in_clump = kept.iter().filter(|s| s.x < 40.0 && s.y < 40.0).count();
    assert!(
        in_clump < 60,
        "{in_clump}/100 kept stars came from the clump; brightest-N would give 100"
    );
}

/// Surplus redistribution: a frame whose signal occupies a minority of cells
/// must not silently lose stars. A naive per-cell cap returns keep/ncells
/// times the occupied fraction, which is the sparse-field regression.
#[test]
fn an_unfilled_cell_donates_its_budget_rather_than_wasting_it() {
    // 300 stars all inside one quadrant of a 1000x1000 frame.
    let mut stars = Vec::new();
    for i in 0..300 {
        stars.push(mk_star((i % 300) as f64, (i / 3) as f64, 1000.0 - i as f64));
    }
    let kept = stratified_keep(stars, 1000, 1000, 200);
    assert_eq!(kept.len(), 200, "empty cells must donate budget to occupied ones");
}

#[test]
fn fewer_stars_than_the_budget_keeps_all_of_them() {
    let stars: Vec<Star> = (0..30).map(|i| mk_star(i as f64 * 3.0, i as f64 * 7.0, 500.0)).collect();
    let kept = stratified_keep(stars, 1000, 1000, 500);
    assert_eq!(kept.len(), 30);
}

#[test]
fn the_result_is_sorted_brightest_first() {
    let stars: Vec<Star> = (0..200).map(|i| mk_star((i * 5 % 900) as f64, (i * 11 % 900) as f64, i as f64)).collect();
    let kept = stratified_keep(stars, 1000, 1000, 50);
    for w in kept.windows(2) {
        assert!(w[0].flux >= w[1].flux, "downstream code assumes brightest-first order");
    }
}

#[test]
fn a_degenerate_frame_size_does_not_panic() {
    let stars: Vec<Star> = (0..10).map(|i| mk_star(0.0, i as f64, 1.0)).collect();
    assert!(stratified_keep(stars.clone(), 0, 0, 5).len() <= 5);
    assert!(stratified_keep(stars, 1, 1, 5).len() <= 5);
}
```

Add a `mk_star` helper in the test module building a `Star` with the given `x`, `y`, `flux` and any plausible values for the remaining fields.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p psolve-core extract::tests::` — expect FAIL, `stratified_keep` not found.

- [ ] **Step 3: Implement**

```rust
/// Keep `keep` stars chosen spatially first, brightest-within-cell second.
///
/// Brightness is spatially correlated: in a cluster the brightest 500
/// detections are all core members, packed into a few arcminutes, while the
/// catalogue's brightest spread across the whole field. The two sets then
/// barely overlap geometrically and no consistent transform exists to find.
/// Selecting per cell fixes that, and incidentally improves quad geometry,
/// since quads drawn from one corner constrain a fit weakly.
///
/// The grid adapts to crowding so that a sparse frame is essentially
/// unaffected, and unfilled cells donate their budget — without that a frame
/// whose signal occupies a minority of cells would silently return far fewer
/// than `keep` stars.
pub(crate) fn stratified_keep(mut stars: Vec<Star>, nx: usize, ny: usize, keep: usize) -> Vec<Star> {
    stars.sort_unstable_by(|p, q| q.flux.partial_cmp(&p.flux).unwrap_or(std::cmp::Ordering::Equal));
    if keep == 0 || stars.len() <= keep || nx == 0 || ny == 0 {
        stars.truncate(keep);
        return stars;
    }

    // g^2 cells. sqrt(detected/keep) grows with crowding; the factor of 2 and
    // the cap of 16 keep the cell count sane at both extremes. At 500
    // detections against keep 500 this is g = 2 (near-identity); at 20,575
    // detections it is g = 12, i.e. 144 cells of ~3 stars each.
    let g = (((stars.len() as f64 / keep as f64).sqrt() * 2.0).round() as usize).clamp(1, 16);
    let ncells = g * g;

    // Bucket, preserving the brightest-first order within each cell.
    let mut cells: Vec<Vec<Star>> = vec![Vec::new(); ncells];
    for s in stars {
        let cx = ((s.x / nx as f64) * g as f64) as usize;
        let cy = ((s.y / ny as f64) * g as f64) as usize;
        cells[cy.min(g - 1) * g + cx.min(g - 1)].push(s);
    }

    // Round-robin over cells: every pass takes one star from each cell that
    // still has one. This fills the budget, spreads the selection, and makes
    // redistribution automatic rather than a separate pass.
    let mut out: Vec<Star> = Vec::with_capacity(keep);
    let mut cursor = vec![0usize; ncells];
    'outer: loop {
        let mut progressed = false;
        for (ci, cell) in cells.iter().enumerate() {
            if let Some(s) = cell.get(cursor[ci]) {
                cursor[ci] += 1;
                out.push(*s);
                progressed = true;
                if out.len() >= keep {
                    break 'outer;
                }
            }
        }
        if !progressed {
            break;
        }
    }

    out.sort_unstable_by(|p, q| q.flux.partial_cmp(&p.flux).unwrap_or(std::cmp::Ordering::Equal));
    out
}
```

Then replace `extract.rs:232-233` with:

```rust
    let stars = stratified_keep(stars, img.nx, img.ny, p.keep);
```

`extract()` must already have `img` in scope; if it does not, thread it in.

If `Star` is not `Copy`, use `std::mem::take` or clone rather than changing the type.

- [ ] **Step 4: Run the tests** — expect PASS, and every pre-existing extraction test still green.

- [ ] **Step 5: Commit**

---

### Task 2: Catalogue-side stratified fetch

`reader.rs` has `brightest_in_disc` (brightest N, used for solving) and `stars_in_disc` (everything to a magnitude cap, used by `index query`). Neither spreads a bounded selection spatially.

**Files:** Modify `crates/psolve-index/src/reader.rs`. Tests in its existing test module.

**Interfaces:**
- Produces: `pub fn stratified_in_disc(&self, ra_deg: f64, dec_deg: f64, radius_deg: f64, limit: usize) -> Vec<StarRecord>`
- **Reuse `cells_in_disc` and `angsep_deg`.** A second copy of either will drift. `brightest_in_disc` and `stars_in_disc` must be left byte-for-byte unchanged.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn stratified_in_disc_spreads_across_cells_rather_than_taking_the_brightest() {
    // Build a synthetic index where one HEALPix cell holds all the bright
    // stars and several others hold fainter ones.
    let idx = build_lopsided_index();
    let got = idx.stratified_in_disc(CENTRE_RA, CENTRE_DEC, 2.0, 40);
    let cells_hit = distinct_cells(&idx, &got);
    assert!(cells_hit > 1, "all {} stars came from one cell", got.len());
}

#[test]
fn stratified_in_disc_returns_the_full_limit_when_stars_exist() {
    let idx = build_lopsided_index();
    let got = idx.stratified_in_disc(CENTRE_RA, CENTRE_DEC, 2.0, 40);
    assert_eq!(got.len(), 40, "sparse cells must donate their budget");
}

#[test]
fn stratified_in_disc_never_returns_a_star_outside_the_radius() {
    let idx = build_lopsided_index();
    for s in idx.stratified_in_disc(CENTRE_RA, CENTRE_DEC, 1.0, 100) {
        assert!(angsep_deg(CENTRE_RA, CENTRE_DEC, s.ra_deg(), s.dec_deg()) <= 1.0);
    }
}

#[test]
fn stratified_in_disc_is_a_subset_of_stars_in_disc() {
    let idx = build_lopsided_index();
    let all: std::collections::HashSet<(u32, i32)> = idx
        .stars_in_disc(CENTRE_RA, CENTRE_DEC, 2.0, f32::MAX)
        .iter().map(|s| (s.ra_scaled, s.dec_scaled)).collect();
    for s in idx.stratified_in_disc(CENTRE_RA, CENTRE_DEC, 2.0, 40) {
        assert!(all.contains(&(s.ra_scaled, s.dec_scaled)));
    }
}

#[test]
fn a_limit_larger_than_the_disc_returns_everything_in_it() {
    let idx = build_lopsided_index();
    let a = idx.stratified_in_disc(CENTRE_RA, CENTRE_DEC, 2.0, usize::MAX).len();
    let b = idx.stars_in_disc(CENTRE_RA, CENTRE_DEC, 2.0, f32::MAX).len();
    assert_eq!(a, b);
}
```

Reuse whatever synthetic-index builder the existing reader tests already use rather than writing a new one.

- [ ] **Step 2: Run to verify failure.**

- [ ] **Step 3: Implement** — round-robin across the HEALPix cells intersecting the disc, taking each cell's next-brightest in turn, skipping records outside the true radius, until `limit` is reached or every cell is exhausted. That is the same round-robin shape as Task 1, and it gives redistribution for free. Sort the result brightest-first before returning, since callers assume that ordering.

- [ ] **Step 4: Run the tests.**

- [ ] **Step 5: Commit.**

---

### Task 3: Wire both entry points

**This is where the previous fix of this shape went wrong.** On 2026-08-14 a scale retry was added to `cmd_solve.rs` only, and the ASTAP dispatch path — the one AstroOps actually calls — kept the old behaviour. Review caught it by running the same frame both ways.

There are **two** catalogue-fetch call sites and both must change:

- `crates/psolve-cli/src/cmd_solve.rs:426` (native)
- `crates/psolve-cli/src/main.rs:238` (ASTAP mode)

**Files:** Modify both. Tests in `crates/psolve-cli/tests/`.

- [ ] **Step 1: Write the failing test**

```rust
/// Both entry points must use the stratified fetch. A fix that reaches only
/// the native path leaves the drop-in interface -- the one AstroOps calls --
/// on the old behaviour, which has happened before in this repo.
#[test]
fn both_entry_points_use_the_same_catalogue_selection() {
    let frame = dense_field_fixture();     // or skip if the real frame is absent
    let native = run_native(&["solve", &frame, "--index", IDX]);
    let astap  = run_astap(&["-f", &frame, "-d", DB, "-o", &scratch()]);
    assert_eq!(native.solved, astap.solved,
        "native and ASTAP mode disagree on the same frame");
}
```

- [ ] **Step 2: Run to verify failure.**

- [ ] **Step 3: Implement** — replace `brightest_in_disc` with `stratified_in_disc` at both sites, keeping the existing limit computation (`cat_limit_for`) unchanged. Do not alter `--cat-limit` or `--keep` semantics; an explicit value is still honoured exactly.

- [ ] **Step 4: Run the tests.**

- [ ] **Step 5: Commit.**

---

### Task 4: Measure it — the whole point of the change

The spec names four acceptance measures. Report **all four**, whatever they show. A change that improves solve rate while worsening separation is a regression, not a trade — the CFA attempt failed exactly that way and was withdrawn.

- [ ] **Step 1: Omega Centauri at defaults**

```bash
# Copy nothing; read only. Frame paths from catalogue.db, `sqlite3 -readonly`.
psolve solve <omega_cen_frame> --index ~/astroops/data/gaia-dr3-g14-dec45-nside64.psidx
```
Must solve with **no** `--cat-limit`, `--keep` or `--radius`. Verify the result lands within 30″ of an independent check; a confident wrong solve is worse than a failure.

- [ ] **Step 2: The 276 misses**

Re-solve the frames that failed in the committed baseline
(`docs/superpowers/data/task-11-agreement-full-9495.ndjson.gz`, records where
`psolve.solved` is false — 276 of them, 267 `NO_QUAD_MATCH`). Report how many
now solve, broken down by target. The dense-field concentration is what should
flatten: C_76 was 10.8%, HD_93308 7.6%, Eta_Carina 6.3%.

- [ ] **Step 3: The full agreement corpus must not regress**

Re-run `scripts/agreement.sh` over the corpus and compare against the committed
baseline: separation median **0.531″**, p90 0.947″, p99 3.128″, 0 scale
outliers, 0 parity mismatches, solve rate 9219/9495.

**Note the corpus has grown** — another team's sweep added ~791 `binning=2`
rows since that baseline, so report the new total and compare like with like
rather than assuming 9,495.

- [ ] **Step 4: The sham-rate floor**

The `astroops-ai` session measured 0.046 / 0.065 / 0.205 / 0.646 on
IC 4592 / HD 37805 / Eta Carinae / Omega Centauri. Ask them to re-run it, or
reproduce it, and report whether the crowded-field figures drop. It is an
independent instrument built by another team, which is exactly what makes it
worth having.

- [ ] **Step 5: Record all four in `docs/superpowers/2026-08-15-stratified-selection-results.md`** — pass or fail.

---

### Task 5: Documentation

- [ ] **Step 1:** Update `docs/astap-compat.md` if any user-visible behaviour changed (defaults, reason codes, flag meanings).
- [ ] **Step 2:** Note in the decision record that clusters previously required three flags and now do not — and if they still do, say so plainly.
- [ ] **Step 3:** Commit.

---

## Self-Review

**Spec coverage.** §3.1 image side → Task 1. §3.2 catalogue side → Task 2. §3.3 "ASTAP path gets it for free" → Task 3 makes that explicit rather than assumed, because the assumption failed once. §4 four acceptance measures → Task 4. §5 sparse-field regression risk → Task 1's redistribution test. §5 `min_pix` interaction → Task 4 Step 2 reports rejection breakdowns.

**Ordering.** Tasks 1 and 2 are independent; 3 needs both; 4 needs 3.

**Risk carried in.** Star selection is upstream of everything measured in this project. Task 4 Step 3 is not optional, and the corpus has changed size since the baseline was taken.
