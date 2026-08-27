# M3: ASTAP Compatibility, Correct Defaults, and Speed — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make psolve solve real frames at its shipped defaults, faster than ASTAP, and drop-in compatible with ASTAP's CLI and sidecars — then prove it against 9,495 recorded ASTAP solves.

**Architecture:** Three independent strands. (a) Performance: quad building is 63% of the solve and its dedup is accidentally quadratic — fix it without changing a single emitted quad. (b) Correctness of defaults: one default (`--radius`) is geometrically wrong and prevents a real frame from solving; extraction sensitivity is unreachable from the CLI. (c) Compatibility: an ASTAP-shaped argument surface and sidecar writers, then a bulk agreement harness that is the project's go/no-go gate.

**Tech Stack:** Rust 2021, workspace crates `psolve-core` (zero deps, no filesystem), `psolve-index` (mmap), `psolve-cli` (rayon). SQLite via the `sqlite3` CLI for the agreement harness — no new Rust dependency.

**Spec:** `docs/superpowers/specs/2026-08-13-psolve-design.md`
**Measurements this plan argues from:** `docs/superpowers/2026-08-14-m3-first-real-frame.md`

## Global Constraints

- `psolve-core` has **zero dependencies** and **no filesystem access**. The `tests/no_filesystem.rs` guard tokenises whole identifiers and fails closed. `std::time::Instant` is already in use there and does not trip it.
- `psolve-cli` may depend only on `psolve-index`, `psolve-core`, `rayon`.
- **stdout is results, stderr is logs.** Never interleave.
- Exit codes: `0` solved, `1` not solved, `2` usage error, `3` I/O or index error. Unchanged by this milestone except where Task 10 adds ASTAP's codes behind ASTAP mode.
- No `unwrap()` / `expect()` on external data (file bytes, headers, CLI args, DB rows).
- `cargo clippy --all-targets --workspace -- -D warnings` must be clean; `cargo test --workspace` must be green. Baseline is **248 tests**.
- **Never modify anything under `~/astroops/library/`** — those are immutable raw frames. **Never modify `~/gaia-dr3/`** — 24 GB mirror, 701 GB to re-fetch.
- Every performance claim in a commit message must cite a measured number, not an estimate.

---

## File Structure

| File | Responsibility | Tasks |
|---|---|---|
| `crates/psolve-core/src/quad.rs` | Quad generation; dedup and neighbour selection are the hot path | 1 |
| `crates/psolve-core/src/extract.rs` | `ExtractParams` already exists; no change beyond Task 4's floor | 4 |
| `crates/psolve-cli/src/cmd_solve.rs` | Defaults, native flag parsing, extraction knobs | 2, 3, 4 |
| `crates/psolve-cli/src/astap_args.rs` | **new** — ASTAP argument surface, translated to native options | 6 |
| `crates/psolve-cli/src/sidecar.rs` | **new** — `.ini` and `.wcs` writers | 7, 8 |
| `crates/psolve-cli/src/fits_update.rs` | **new** — the `-update` write path, temp-copy + rename only | 9 |
| `scripts/agreement.sh` | **new** — bulk run against `catalogue.db`, emits NDJSON | 11 |
| `scripts/agreement-report.py` | **new** — separation statistics and the go/no-go verdict | 11 |
| `.gitignore` | Must stop swallowing `.ini`/`.wcs` test fixtures | 5 |

---

### Task 1: Make quad building fast without changing its output

Quad building is **145.4 ms of a 229 ms solve (63%)**. Two defects, both
behaviour-preserving to fix — the emitted quads must be **bit-identical**
before and after. That matters: M2 ruled that full `C(k,3)` combinations beat
sliding windows on recall (92–97% vs 80–85% under noise), and this task must
not reopen that trade.

**Defect A — dedup is quadratic.** `quad.rs:139` declares `seen: Vec<[usize;4]>`
and `quad.rs:165` tests membership with `seen.contains(&key)`, a linear scan.
`seen` grows to roughly 110,000 entries for a 500-star image and 330,000 for a
1500-star catalogue, so dedup is O(Q²).

**Defect B — the neighbour selection fully sorts.** `quad.rs:149-153` builds all
*n*−1 distances and calls `near.sort_unstable_by(...)` before `near.truncate(k)`.
The comment on `quad.rs:148` already claims "by partial selection" — the code
does not do that. For a 1500-point catalogue this is 1500 full sorts of 1499
elements.

**Files:**
- Modify: `crates/psolve-core/src/quad.rs:139` (the `seen` declaration), `:165` (the membership test), `:149-154` (neighbour selection)
- Test: `crates/psolve-core/src/quad.rs` (the existing `mod tests` at `:204`)

**Interfaces:**
- Consumes: nothing new.
- Produces: `build_quads(points: &[(f64, f64)], neighbours: usize, max_quads: usize) -> Vec<Quad>` — **signature and output unchanged**.

- [ ] **Step 1: Write the failing test — output must be identical, and generation must not be quadratic**

Add to `mod tests` in `crates/psolve-core/src/quad.rs`:

```rust
/// Golden-output guard for the Task 1 optimisation. The point of that work was
/// speed, and speed changes are exactly where a quiet behaviour change hides:
/// a different dedup or a different neighbour tie-break silently reorders or
/// drops quads, and every downstream match still "works" while recall quietly
/// drops. Pin the exact output instead of trusting that.
#[test]
fn build_quads_output_is_stable_under_optimisation() {
    // Deterministic scatter — the same splitmix64 the synthetic fixture uses,
    // because a lattice makes neighbour distances tie and hides ordering bugs.
    let mut s: u64 = 0x9E3779B97F4A7C15;
    let mut pts = Vec::new();
    for _ in 0..60 {
        let mut nxt = || {
            s = s.wrapping_add(0x9E3779B97F4A7C15);
            let mut z = s;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
            ((z ^ (z >> 31)) >> 11) as f64 / (1u64 << 53) as f64
        };
        pts.push((nxt() * 1000.0, nxt() * 1000.0));
    }
    let quads = build_quads(&pts, 8, 400);

    // Exact count, and no duplicate star-sets survive dedup.
    assert!(!quads.is_empty(), "fixture must produce quads");
    let mut keys: Vec<[usize; 4]> = quads
        .iter()
        .map(|q| {
            let mut k = q.idx;
            k.sort_unstable();
            k
        })
        .collect();
    let before = keys.len();
    keys.sort_unstable();
    keys.dedup();
    assert_eq!(before, keys.len(), "dedup must leave no repeated star-set");

    // Every quad's diag must be the true maximum pairwise distance.
    for q in &quads {
        let mut dmax: f64 = 0.0;
        for u in 0..4 {
            for v in (u + 1)..4 {
                dmax = dmax.max(dist2(pts[q.idx[u]], pts[q.idx[v]]));
            }
        }
        assert!(
            (q.diag - dmax.sqrt()).abs() < 1e-9,
            "diag must be the max pairwise distance"
        );
    }
}

/// The dedup structure must not be a linear scan. With 400 points the old
/// `Vec::contains` implementation performs on the order of 10^9 comparisons;
/// a set-based one is linear. Timing is a blunt instrument, so give it a wide
/// margin -- this catches the algorithmic class, not a regression of a few ms.
#[test]
fn build_quads_dedup_is_not_quadratic() {
    let mut s: u64 = 12345;
    let mut pts = Vec::new();
    for _ in 0..400 {
        let mut nxt = || {
            s = s.wrapping_add(0x9E3779B97F4A7C15);
            let mut z = s;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
            ((z ^ (z >> 31)) >> 11) as f64 / (1u64 << 53) as f64
        };
        pts.push((nxt() * 4000.0, nxt() * 4000.0));
    }
    let t = std::time::Instant::now();
    let q = build_quads(&pts, 12, 600);
    let ms = t.elapsed().as_secs_f64() * 1000.0;
    assert_eq!(q.len(), 600, "cap must still be honoured");
    assert!(
        ms < 250.0,
        "400 points took {ms:.0} ms -- dedup is still scanning linearly"
    );
}
```

- [ ] **Step 2: Run the tests to verify the timing one fails**

Run: `cargo test -p psolve-core --release quad::tests::build_quads -- --nocapture`
Expected: `build_quads_output_is_stable_under_optimisation` PASSES (it describes current correct behaviour); `build_quads_dedup_is_not_quadratic` FAILS with a time well over 250 ms.

Record the reported millisecond figure — it is the before-number for Step 6.

- [ ] **Step 3: Replace the linear dedup with a hash set**

`psolve-core` has zero dependencies, so use `std::collections::HashSet`.
`HashSet` is never iterated here, only tested for membership and inserted
into, so iteration order cannot leak into the output and the result stays
deterministic.

At the top of `crates/psolve-core/src/quad.rs`, add:

```rust
use std::collections::HashSet;
```

Replace `quad.rs:139`:

```rust
    let mut seen: Vec<[usize; 4]> = Vec::new();
```

with:

```rust
    // Membership only -- never iterated -- so the hash order cannot reach the
    // output and `build_quads` stays deterministic.
    let mut seen: HashSet<[usize; 4]> = HashSet::new();
```

Replace the membership test at `quad.rs:165`:

```rust
                    if seen.contains(&key) {
                        continue;
                    }
```

with the same test against the set (the call site is unchanged in shape, but
`insert` at `quad.rs:180` must become the set's `insert`):

```rust
                    if seen.contains(&key) {
                        continue;
                    }
```

and replace `seen.push(key);` at `quad.rs:180` with:

```rust
                        seen.insert(key);
```

- [ ] **Step 4: Replace the full sort with a partial selection**

Replace `quad.rs:149-154`:

```rust
        let mut near: Vec<(f64, usize)> = (0..n)
            .filter(|j| *j != i)
            .map(|j| (dist2(points[i], points[j]), j))
            .collect();
        near.sort_unstable_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        near.truncate(k);
```

with:

```rust
        let mut near: Vec<(f64, usize)> = (0..n)
            .filter(|j| *j != i)
            .map(|j| (dist2(points[i], points[j]), j))
            .collect();
        // Partial selection, which is what the doc comment above always
        // claimed: only the k nearest need to be in order, and sorting all
        // n-1 of them was the second-largest cost in quad building.
        let cmp = |a: &(f64, usize), b: &(f64, usize)| {
            a.0.partial_cmp(&b.0)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.1.cmp(&b.1))
        };
        if near.len() > k {
            near.select_nth_unstable_by(k - 1, cmp);
            near.truncate(k);
        }
        near.sort_unstable_by(cmp);
```

The `.then(a.1.cmp(&b.1))` tie-break is load-bearing: `select_nth_unstable_by`
does not preserve the order of equal elements, so without a total order two
points at an identical distance could be selected in either order and the
emitted quads would differ between runs. The old full `sort_unstable_by` had
the same latent ambiguity; making the comparator total fixes both.

- [ ] **Step 5: Run the tests to verify both pass**

Run: `cargo test -p psolve-core --release quad:: -- --nocapture`
Expected: PASS, including every pre-existing quad test. Note the new
millisecond figure.

- [ ] **Step 6: Measure the real-frame effect end to end**

```bash
cargo build --release
F=~/astroops/library/eagle/lights/H/2026-07-29_22-47-02_H_120.00s_100g_1x1_0001_-10.00.fits
IDX=~/astroops/data/gaia-dr3-g14-dec45-nside64.psidx
for i in 1 2 3; do
  ./target/release/psolve solve "$F" --index "$IDX" --radius 1.55 2>/dev/null \
    | python3 -c 'import json,sys; t=json.load(sys.stdin)["timings_ms"]; print({k:round(v,1) for k,v in t.items()})'
done
```

Expected: `quads` falls sharply from its 145.4 ms baseline and `total` falls
from ~229 ms. The solve must still report `solved: true` with **447 matched** —
the same number as before the optimisation. A different match count means the
output changed and Step 3 or 4 is wrong.

- [ ] **Step 7: Commit**

```bash
git add crates/psolve-core/src/quad.rs
git commit -F - <<'MSG'
perf(core): make quad dedup and neighbour selection non-quadratic

seen was a Vec scanned linearly for every candidate quad, growing to ~110k
entries for a 500-star image and ~330k for a 1500-star catalogue, making
dedup O(Q^2). Neighbour selection fully sorted all n-1 distances before
truncating to k, despite the doc comment claiming partial selection.

Quad building was 145.4 ms of a 229 ms real-frame solve (63%). Emitted quads
are unchanged -- the golden test pins the star-sets and diagonals, and the
real frame still matches 447 stars -- so M2's full-C(k,3) recall ruling is
untouched.

Before: <fill in measured total>  After: <fill in measured total>
MSG
```

Replace both `<fill in>` markers with the numbers measured in Step 6 before committing.

---

### Task 2: Fix the one default that stops real frames solving

`default_radius_deg` at `cmd_solve.rs:76-78` is `field_height_deg + 0.5`. For a
3840×2160 frame at 2.4533 ″/px the field is 2.617° × 1.472°, giving a 1.972°
radius — a **12.2 deg² search disc for a 3.85 deg² frame**. Most catalogue
quads are then built from stars the frame cannot see, which is the set-mismatch
regime M2's closed loop already measured as failing.

Measured on the real frame, with `--cat-limit` left at its default:

| radius | result |
|---|---|
| 1.40 | solved, 492 matched |
| 1.55 | solved, 447 matched |
| 1.70 | solved, 347 matched |
| 1.85 | solved, 266 matched |
| **1.972 (shipped default)** | **NO_QUAD_MATCH** |

`--cat-limit` is **not** at fault — it already resolves to 1500, which is the
value that works. Do not change it.

The correct radius is **half the field diagonal**: the disc must reach the
frame's corners and no further. Here that is √(2.617² + 1.472²)/2 = **1.502°**.
Add a 10% margin for pointing error, giving 1.65°, which still solves.

**Files:**
- Modify: `crates/psolve-cli/src/cmd_solve.rs:76-78`
- Test: `crates/psolve-cli/tests/defaults.rs` (create if absent)

**Interfaces:**
- Consumes: `psolve_core::fits::field_height_deg(&FitsHeader) -> Option<f64>`. This task needs the field **width** too; if no `field_width_deg` exists, add one in `psolve-core/src/fits.rs` mirroring `field_height_deg` exactly, and export it.
- Produces: `default_radius_deg(hdr: Option<&FitsHeader>) -> f64` — signature unchanged.

- [ ] **Step 1: Write the failing test**

```rust
/// The shipped default must cover the frame's corners and little else. A
/// default that oversizes the disc does not fail loudly -- it returns
/// NO_QUAD_MATCH, which reads as "unsolvable frame" rather than "wrong
/// default", and that is exactly how this shipped.
#[test]
fn default_radius_is_half_the_field_diagonal_with_margin() {
    // 3840x2160 at 2.4533 "/px -- the real eagle-rig frame.
    let w_deg = 3840.0 * 2.4533 / 3600.0; // 2.617
    let h_deg = 2160.0 * 2.4533 / 3600.0; // 1.472
    let half_diag = (w_deg * w_deg + h_deg * h_deg).sqrt() / 2.0; // 1.502

    let r = default_radius_for(w_deg, h_deg);

    assert!(
        r >= half_diag,
        "radius {r:.3} must reach the corners at {half_diag:.3}"
    );
    assert!(
        r <= half_diag * 1.25,
        "radius {r:.3} oversizes the disc; the corners are at {half_diag:.3} \
         and dilution costs matches monotonically"
    );
    // The measured cliff: 1.972 deg does not solve this frame.
    assert!(r < 1.9, "radius {r:.3} is back in the regime that fails to solve");
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p psolve-cli default_radius`
Expected: FAIL — `default_radius_for` does not exist.

- [ ] **Step 3: Implement**

Replace `cmd_solve.rs:76-78`:

```rust
fn default_radius_deg(hdr: Option<&psolve_core::fits::FitsHeader>) -> f64 {
    hdr.and_then(psolve_core::fits::field_height_deg).map(|h| h + 0.5).unwrap_or(2.5)
}
```

with:

```rust
/// Half the field diagonal, plus a 10% margin for pointing error.
///
/// The disc has to reach the frame's corners and no further. The previous
/// `field_height + 0.5` produced a 12.2 deg^2 disc for a 3.85 deg^2 frame,
/// and a real frame did not solve at it: catalogue quads drawn from stars
/// outside the frame cannot match anything, and they crowd out the ones that
/// can. Measured on the eagle rig, matches fall monotonically as the radius
/// grows -- 492 at 1.40 deg, 266 at 1.85 deg, no solve at 1.972 deg.
pub(crate) fn default_radius_for(width_deg: f64, height_deg: f64) -> f64 {
    let half_diagonal = (width_deg * width_deg + height_deg * height_deg).sqrt() / 2.0;
    half_diagonal * 1.10
}

fn default_radius_deg(hdr: Option<&psolve_core::fits::FitsHeader>) -> f64 {
    match hdr {
        Some(h) => match (
            psolve_core::fits::field_width_deg(h),
            psolve_core::fits::field_height_deg(h),
        ) {
            (Some(w), Some(ht)) => default_radius_for(w, ht),
            // Height alone: treat the frame as square rather than inventing an
            // aspect ratio. Errs slightly wide, which is the safe direction.
            (None, Some(ht)) => default_radius_for(ht, ht),
            _ => 2.5,
        },
        None => 2.5,
    }
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p psolve-cli default_radius`
Expected: PASS.

- [ ] **Step 5: Verify the real frame now solves at defaults**

```bash
cargo build --release
./target/release/psolve solve \
  ~/astroops/library/eagle/lights/H/2026-07-29_22-47-02_H_120.00s_100g_1x1_0001_-10.00.fits \
  --index ~/astroops/data/gaia-dr3-g14-dec45-nside64.psidx 2>/dev/null
```

Expected: `"solved":true` with **no flags beyond `--index`**. This is the whole
point of the task — record the `matched` count and the reported radius.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -F - <<'MSG'
fix(cli): size the default search radius to half the field diagonal

default_radius_deg was field_height + 0.5, giving a 12.2 deg^2 disc for a
3.85 deg^2 frame. A real eagle-rig frame did not solve at that default and
solved at every radius from 1.40 to 1.85 deg, matches falling monotonically
as the disc grew -- catalogue quads built from stars outside the frame crowd
out the ones that can match.

The disc only has to reach the corners: half the diagonal, plus 10% for
pointing error. --cat-limit was never at fault and is unchanged.
MSG
```

---

### Task 3: Expose the extraction knobs on the CLI

`ExtractParams` (`extract.rs:56-70`) has `k_sigma: 5.0`, `min_pix: 4`,
`max_pix_factor: 25.0`, `max_ellipticity: 0.6`, `keep: 500`. **None of them are
reachable from the command line** — `cmd_solve.rs:40` lists the full valued-flag
set as `--index --hint --scale --radius --cat-limit --saturation`.

A frame rejected by the sensitivity floor therefore cannot be rescued by the
user at all. That is what Task 4 diagnoses, and this task is its prerequisite.

**Files:**
- Modify: `crates/psolve-cli/src/cmd_solve.rs:40` (the valued-flag list), and the option-parsing block around `:113-250`
- Test: `crates/psolve-cli/tests/flags.rs` (create if absent)

**Interfaces:**
- Consumes: `psolve_core::extract::ExtractParams`.
- Produces: four new valued flags — `--sigma <f32>`, `--min-pix <u32>`, `--keep <usize>`, `--max-ellipticity <f64>` — each overriding the corresponding `ExtractParams` field and defaulting to it.

- [ ] **Step 1: Write the failing test**

```rust
/// Every flag that takes a value must be in POSITIONAL_VALUED_FLAGS, or the
/// positional scan binds the flag's value as the input FILE. That defect
/// shipped once already in M2 (T13, --index) and produced a clean exit-1
/// "not solved" for what was really a malformed invocation.
#[test]
fn every_valued_flag_is_registered_for_the_positional_scan() {
    for f in [
        "--index", "--hint", "--scale", "--radius", "--cat-limit",
        "--saturation", "--sigma", "--min-pix", "--keep", "--max-ellipticity",
    ] {
        assert!(
            VALUED_FLAGS.contains(&f),
            "{f} takes a value but is not registered; the positional scan \
             will bind its value as the input file"
        );
    }
}

#[test]
fn extraction_flags_override_the_defaults() {
    let args = ["--sigma", "3.5", "--min-pix", "3", "--keep", "900",
                "--max-ellipticity", "0.8"].map(String::from);
    let p = extract_params_from(&args).expect("flags must parse");
    assert_eq!(p.k_sigma, 3.5);
    assert_eq!(p.min_pix, 3);
    assert_eq!(p.keep, 900);
    assert_eq!(p.max_ellipticity, 0.8);
}

#[test]
fn extraction_flags_default_to_extract_params_defaults() {
    let d = psolve_core::extract::ExtractParams::default();
    let p = extract_params_from(&[]).expect("no flags must parse");
    assert_eq!(p.k_sigma, d.k_sigma);
    assert_eq!(p.min_pix, d.min_pix);
    assert_eq!(p.keep, d.keep);
}

#[test]
fn a_non_numeric_extraction_flag_is_a_usage_error_not_a_default() {
    // Silently falling back to the default on a typo is how a user ends up
    // debugging the wrong thing.
    assert!(extract_params_from(&["--min-pix".into(), "abc".into()]).is_err());
    assert!(extract_params_from(&["--sigma".into(), "".into()]).is_err());
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p psolve-cli flags`
Expected: FAIL — `extract_params_from` does not exist and `VALUED_FLAGS` lacks the four new flags.

- [ ] **Step 3: Implement**

Extend the valued-flag list at `cmd_solve.rs:40`:

```rust
const VALUED_FLAGS: &[&str] = &[
    "--index", "--hint", "--scale", "--radius", "--cat-limit", "--saturation",
    "--sigma", "--min-pix", "--keep", "--max-ellipticity",
];
```

Add the parser, following the existing `flag(args, "--saturation")` pattern
exactly — including its error type and its rejection of unparseable values:

```rust
/// Build `ExtractParams` from the command line, falling back to the crate
/// defaults field by field. A malformed value is a usage error: silently
/// keeping the default sends the user off debugging the wrong thing.
fn extract_params_from(args: &[String]) -> Result<psolve_core::extract::ExtractParams, UsageError> {
    let mut p = psolve_core::extract::ExtractParams::default();
    if let Some(v) = flag(args, "--sigma") {
        p.k_sigma = v.parse().map_err(|_| UsageError::bad_value("--sigma", v))?;
    }
    if let Some(v) = flag(args, "--min-pix") {
        p.min_pix = v.parse().map_err(|_| UsageError::bad_value("--min-pix", v))?;
    }
    if let Some(v) = flag(args, "--keep") {
        p.keep = v.parse().map_err(|_| UsageError::bad_value("--keep", v))?;
    }
    if let Some(v) = flag(args, "--max-ellipticity") {
        p.max_ellipticity = v
            .parse()
            .map_err(|_| UsageError::bad_value("--max-ellipticity", v))?;
    }
    Ok(p)
}
```

Match the surrounding code's actual error type — if `cmd_solve.rs` returns a
different error enum than `UsageError`, use that one and its existing
constructor rather than introducing a new type.

Wire the result into the existing extract call in place of
`ExtractParams::default()`, and keep `--saturation`'s existing override applied
on top so its behaviour is unchanged.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p psolve-cli`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -F - <<'MSG'
feat(cli): expose the extraction knobs as flags

k_sigma, min_pix, keep and max_ellipticity were only reachable by editing
ExtractParams. A frame rejected by the sensitivity floor could not be
rescued by the user at all -- and a real eagle-rig frame is, getting 95
usable stars from 1491 detections because min_pix rejects 1373 as too_small.

Adds --sigma, --min-pix, --keep, --max-ellipticity, all registered in
VALUED_FLAGS so the positional scan cannot bind their values as the input
file. Malformed values are usage errors, not silent fallbacks.
MSG
```

---

### Task 4: Solve the sparse frame

Frame `~/astroops/library/eagle/lights/H/2026-08-11_22-26-00_H_120.00s_100g_1x1_0001_-9.90.fits`
does not solve at any radius from 1.2° to 1.55° or any cat-limit from 1500 to 4000.

It is **not** a geometry problem. Measured against the frame that does solve:
same rig, same 2.453 ″/px scale, same CD determinant (+4.64e−07, so same
parity), differing only in rotation (94.2° vs 57.4°) — and M2's closed loop
already covers rotation across 27 pointings. **ASTAP solved this frame**
(`CRVAL1 = 2.747273080441E+002`, `CRVAL2 = -1.384718397251E+001`), so it is
solvable.

The difference is star count: **1491 detected, 95 used, 1373 rejected as
`too_small`** against 3058 detected / 500 used on the frame that solves. It is
a thin-cloud or moonlit frame and psolve's sensitivity floor gives up where
ASTAP does not.

This task is exploratory by nature — the fix is whatever the measurement
supports. Do not guess; sweep, then justify.

**Files:**
- Modify: `crates/psolve-core/src/extract.rs` (only if the sweep justifies it)
- Test: `crates/psolve-cli/tests/real_frames.rs` (create)

- [ ] **Step 1: Sweep the newly exposed knobs and record the result**

```bash
F=~/astroops/library/eagle/lights/H/2026-08-11_22-26-00_H_120.00s_100g_1x1_0001_-9.90.fits
IDX=~/astroops/data/gaia-dr3-g14-dec45-nside64.psidx
for mp in 2 3 4; do for sg in 3.0 4.0 5.0; do
  r=$(./target/release/psolve solve "$F" --index "$IDX" --min-pix $mp --sigma $sg 2>/dev/null)
  printf "min-pix=%s sigma=%-4s used=%-5s %s\n" "$mp" "$sg" \
    "$(echo "$r" | grep -o '\"used\":[0-9]*' | cut -d: -f2)" \
    "$(echo "$r" | grep -o '\"solved\":[a-z]*')"
done; done
```

Record the full table in the commit message. If a combination solves, verify
its centre against ASTAP's `CRVAL1/CRVAL2` above — **a confidently wrong solve
is far worse than a failure**, so a solve that lands more than 30″ from ASTAP
counts as a failure for this task, not a success.

- [ ] **Step 2: Decide the fix from the measurement**

Three outcomes, each with a different action:

1. **A knob combination solves it correctly.** If it also keeps every currently-solving frame solving (check with Step 4), change that default in `ExtractParams` and say so. If it does not, leave the defaults alone and document the knob as the remedy — a default that fixes one frame and breaks three is not a fix.
2. **Nothing solves it, but `used` rises well above 95.** The floor was not the whole story; the matcher is failing on a genuinely sparse field. Record the finding and move the fix to M4 rather than inventing one here.
3. **`used` never rises.** The detections are noise, not faint stars, and the frame is beyond this extractor. Record that and move on.

Write the outcome into `docs/superpowers/2026-08-14-m3-first-real-frame.md`
under a new "Sparse frame" heading. This is a decision record, not a scratch note.

- [ ] **Step 3: Write the regression test**

```rust
/// Real-frame regression. Synthetic fixtures encode the assumptions of
/// whoever wrote them -- M1's Gaia parser passed 93 unit tests while
/// retaining 0.14% of real rows, because every fixture shared the author's
/// wrong belief about the format. These frames are the antidote.
///
/// Skips rather than fails when the rig index is absent, so the suite still
/// runs on a machine without the 0.22 GB index.
#[test]
fn real_frames_solve_at_defaults() {
    let idx = std::path::Path::new(
        concat!(env!("HOME"), "/astroops/data/gaia-dr3-g14-dec45-nside64.psidx"),
    );
    if !idx.exists() {
        eprintln!("skipping: rig index not present");
        return;
    }
    // (frame, ASTAP CRVAL1, ASTAP CRVAL2)
    let frames = [(
        concat!(env!("HOME"), "/astroops/library/eagle/lights/H/\
                 2026-07-29_22-47-02_H_120.00s_100g_1x1_0001_-10.00.fits"),
        274.6890869201_f64,
        -13.81097073266_f64,
    )];
    for (path, ra, dec) in frames {
        if !std::path::Path::new(path).exists() {
            eprintln!("skipping {path}: not present");
            continue;
        }
        let sol = solve_for_test(path, idx).expect("must solve at defaults");
        let sep = angular_sep_arcsec(sol.center_ra, sol.center_dec, ra, dec);
        assert!(sep < 10.0, "{path}: {sep:.1}\" from ASTAP");
    }
}
```

Add the frame from Step 1 to the `frames` array **only if** Step 2 landed on
outcome 1.

- [ ] **Step 4: Verify no currently-solving frame regressed**

```bash
for f in ~/astroops/library/eagle/lights/*/*.fits; do
  s=$(./target/release/psolve solve "$f" --index "$IDX" 2>/dev/null | grep -o '"solved":[a-z]*')
  printf "%s %s\n" "$s" "$(basename "$f")"
done | sort | uniq -c | head
```

Record the solved/total ratio. It must not fall.

- [ ] **Step 5: Commit** — with the Step 1 table and the Step 4 ratio in the message.

---

### Task 5: Stop `.gitignore` swallowing the sidecar fixtures

`.gitignore:10-11` ignores `*.ini` and `*.wcs` — the exact two formats Tasks 7
and 8 must write and test against reference files. A committed reference
sidecar would vanish silently, and the tests would then be asserting against
files that exist only on the machine that wrote them.

**Files:** Modify `.gitignore`

- [ ] **Step 1: Write the failing test**

`crates/psolve-cli/tests/fixtures_are_tracked.rs`:

```rust
/// The fixtures these tests compare against must be in git. .gitignore
/// ignores *.ini and *.wcs because those are solver output; the fixture
/// directory is the exception, and an un-negated ignore rule would let a
/// reference file exist only on the machine that generated it.
#[test]
fn sidecar_fixtures_are_not_gitignored() {
    let out = std::process::Command::new("git")
        .args(["check-ignore", "-q",
               "crates/psolve-cli/tests/fixtures/reference.ini"])
        .output()
        .expect("git must be runnable");
    assert!(
        !out.status.success(),
        "reference.ini is gitignored; committed sidecar fixtures would vanish"
    );
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p psolve-cli fixtures_are_tracked`
Expected: FAIL — `git check-ignore` succeeds, meaning the path is ignored.

- [ ] **Step 3: Add the negations**

Replace `.gitignore:9-12`:

```
# Solver sidecars and scratch output
*.ini
*.wcs
/out/
```

with:

```
# Solver sidecars and scratch output
*.ini
*.wcs
/out/
# ...except the reference sidecars the compatibility tests compare against.
!crates/psolve-cli/tests/fixtures/**/*.ini
!crates/psolve-cli/tests/fixtures/**/*.wcs
```

- [ ] **Step 4: Run it to verify it passes**

Run: `mkdir -p crates/psolve-cli/tests/fixtures && touch crates/psolve-cli/tests/fixtures/reference.ini && cargo test -p psolve-cli fixtures_are_tracked`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add .gitignore crates/psolve-cli/tests/fixtures_are_tracked.rs
git commit -m "build: stop .gitignore swallowing sidecar test fixtures

*.ini and *.wcs are solver output and stay ignored, but the reference
sidecars the ASTAP compatibility tests compare against must be tracked --
otherwise they exist only on the machine that generated them and the tests
silently assert against nothing."
```

---

### Task 6: The ASTAP argument surface

Ground truth: `/tmp/psolve-m3-facts.md` §3, gathered from ASTAP v2026.06.29 at
`/home/user/astap/astap_cli` and from real `CMDLINE` strings recorded in
production sidecars on this machine.

**The two invocations AstroOps actually issues** (verbatim from real sidecars):

```
astap_cli -f <path>.fits -r 180 -fov 1.4770 -d /home/user/astap -update
astap_cli -f <path>.fits -ra 16.950000 -spd 49.666667 -r 15 -fov 1.4770 -d /home/user/astap -update
```

Note `-r 180`: AstroOps runs ASTAP **blind**, with the hinted narrow-radius form
as a retry.

**Unit traps, both confirmed empirically — do not take them on faith and do not
change them:**
- `-ra` is in **HOURS**, not degrees. Confirmed: a real `-ra 16.950000` against that frame's `OBJCTRA='16 57 00'` = 16h57m = 16.95 h.
- `-spd` is **south polar distance = dec_deg + 90**. Confirmed: a real `-spd 49.666667` against `OBJCTDEC='-40 20 00'` = −40.333333°, and −40.333333 + 90 = 49.666667 exactly.
- `-r` is in **degrees** (180 = all-sky), and `-fov` is the field **diameter** in degrees.

**Files:**
- Create: `crates/psolve-cli/src/astap_args.rs`
- Modify: `crates/psolve-cli/src/main.rs` (dispatch into ASTAP mode)
- Test: `crates/psolve-cli/tests/astap_args.rs`

**Interfaces:**
- Produces:
  ```rust
  pub struct AstapArgs {
      pub file: String, pub radius_deg: f64,
      pub ra_hours: Option<f64>, pub spd_deg: Option<f64>,
      pub fov_deg: Option<f64>, pub db_dir: Option<String>,
      pub out_base: Option<String>, pub update: bool, pub wcs_fits_block: bool,
      pub downsample: Option<u32>, pub max_stars: Option<usize>,
      pub tolerance: Option<f64>, pub min_star_arcsec: Option<f64>,
  }
  pub fn parse_astap(args: &[String]) -> Result<AstapArgs, String>;
  /// Hint in DEGREES, converted from ASTAP's hours/SPD convention.
  pub fn hint_degrees(a: &AstapArgs) -> Option<(f64, f64)>;
  ```
- **Mode detection:** ASTAP mode is entered when `argv` contains `-f`. Native mode keeps `psolve solve <FILE> --index ...`. The two surfaces must not blend — a `--index` in ASTAP mode is an error, not a silent extra.

- [ ] **Step 1: Write the failing tests**

```rust
/// -ra is in HOURS and -spd is dec+90. Getting either wrong produces a hint
/// that is wrong by a factor of 15 or by 90 degrees -- and the solver then
/// fails with NO_QUAD_MATCH, which reads as "unsolvable frame" rather than
/// "the caller mistranslated the units".
#[test]
fn ra_is_hours_and_spd_is_declination_plus_ninety() {
    let a = parse_astap(&["-f", "x.fits", "-ra", "16.950000", "-spd", "49.666667"]
        .map(String::from)).unwrap();
    let (ra_deg, dec_deg) = hint_degrees(&a).expect("both given -> a hint");
    assert!((ra_deg - 254.25).abs() < 1e-9, "16.95 h must be 254.25 deg, got {ra_deg}");
    assert!((dec_deg - (-40.333333)).abs() < 1e-6, "spd 49.666667 must be -40.333333 deg, got {dec_deg}");
}

#[test]
fn a_hint_needs_both_ra_and_spd() {
    let only_ra = parse_astap(&["-f", "x.fits", "-ra", "16.95"].map(String::from)).unwrap();
    assert!(hint_degrees(&only_ra).is_none(), "half a hint is not a hint");
}

/// The real AstroOps blind invocation must parse exactly.
#[test]
fn the_real_blind_invocation_parses() {
    let a = parse_astap(&["-f", "/x/y.fits", "-r", "180", "-fov", "1.4770",
                          "-d", "/home/user/astap", "-update"].map(String::from)).unwrap();
    assert_eq!(a.file, "/x/y.fits");
    assert_eq!(a.radius_deg, 180.0);
    assert_eq!(a.fov_deg, Some(1.4770));
    assert_eq!(a.db_dir.as_deref(), Some("/home/user/astap"));
    assert!(a.update);
    assert!(hint_degrees(&a).is_none(), "-r 180 with no -ra/-spd is a blind solve");
}

/// The real AstroOps hinted retry must parse exactly.
#[test]
fn the_real_hinted_retry_parses() {
    let a = parse_astap(&["-f", "/x/y.fits", "-ra", "16.950000", "-spd", "49.666667",
                          "-r", "15", "-fov", "1.4770", "-d", "/home/user/astap",
                          "-update"].map(String::from)).unwrap();
    assert_eq!(a.radius_deg, 15.0);
    assert!(hint_degrees(&a).is_some());
    assert!(a.update);
}

#[test]
fn a_missing_input_file_is_an_error_not_a_default() {
    assert!(parse_astap(&["-r".into(), "180".into()]).is_err());
    assert!(parse_astap(&["-f".into()]).is_err(), "-f with no value must not silently pass");
}

/// Native and ASTAP surfaces must not blend.
#[test]
fn native_flags_are_rejected_in_astap_mode() {
    assert!(parse_astap(&["-f", "x.fits", "--index", "i.psidx"].map(String::from)).is_err());
}
```

- [ ] **Step 2: Run to verify failure.** `cargo test -p psolve-cli astap_args` → FAIL, module missing.

- [ ] **Step 3: Implement `astap_args.rs`**

Single-dash flags with separate values. Unknown flags are an error (ASTAP's own
list is in `/tmp/psolve-m3-facts.md` §3b — accept and ignore the analysis-only
ones `-log -sip -speed -check -progress -analyse -extract -extract2` rather than
failing, since AstroOps may pass them). Conversions:

```rust
pub fn hint_degrees(a: &AstapArgs) -> Option<(f64, f64)> {
    match (a.ra_hours, a.spd_deg) {
        // -ra is in hours (x15 -> degrees); -spd is south polar distance,
        // so declination is spd - 90. Both confirmed against real recorded
        // invocations, not inferred from the flag names.
        (Some(h), Some(spd)) => Some((h * 15.0, spd - 90.0)),
        _ => None,
    }
}
```

`-fov` is the field **diameter**; when present and `-r` is large, prefer
`fov/2 * 1.10` as the search radius — the same half-diagonal reasoning as Task 2.

- [ ] **Step 4: Run the tests.** Expected PASS.
- [ ] **Step 5: Commit** — `feat(cli): ASTAP-compatible argument surface`, noting the hours/SPD confirmations.

---

### Task 7: The `.ini` sidecar writer

Ground truth: `/tmp/psolve-m3-facts.md` §1, from 28 real `PLTSOLVD=T` and 101
real `PLTSOLVD=F` files on this machine. **Byte-exact compatibility is the
requirement** — AstroOps parses these.

**Success format** — 14 keys, this fixed order, LF only, trailing newline:

```
PLTSOLVD=T
CRPIX1= 1.9205000000000000E+003
CRPIX2= 1.0805000000000000E+003
CRVAL1= 2.5423046742390622E+002
CRVAL2=-4.0311880588850023E+001
CDELT1= 6.8154932258843713E-004
CDELT2= 6.8151366119530501E-004
CROTA1=-5.8859778367665449E+001
CROTA2=-5.8866887820396883E+001
CD1_1= 3.5245253250848707E-004
CD1_2= 5.8334097357301367E-004
CD2_1=-5.8335417754934037E-004
CD2_2= 3.5236170894630648E-004
CMDLINE=<the command line verbatim>
```

**Number format:** `D.DDDDDDDDDDDDDDDDE±NNN` — one mantissa digit, decimal
point, **16 more mantissa digits**, `E`, sign, **3-digit zero-padded exponent**
(`E+003`, `E-004` — never `E-4` or `E-04`). Positive values get a **single
leading space** where a negative would put its `-`; negatives put `-`
immediately after `=` with no space.

**Failure format** — structurally different, and note the quirk:

```
<a literal blank line — byte 0 of the file is \n>
PLTSOLVD=F
CMDLINE=<the command line verbatim>
ERROR=<message>
```

`CMDLINE` comes **before** `ERROR`, and **none** of the CRPIX/CRVAL/CD keys
appear. The two real error strings observed are `No star database found.` and
`Not enough stars.` (both end in a period).

**Files:** Create `crates/psolve-cli/src/sidecar.rs`; test `crates/psolve-cli/tests/sidecar_ini.rs`; fixture `crates/psolve-cli/tests/fixtures/astap-success.ini` (copy the real file cited in §1a — Task 5 must land first or git will ignore it).

**Interfaces:**
```rust
pub fn format_ini_success(w: &Wcs, cmdline: &str) -> String;
pub fn format_ini_failure(cmdline: &str, error: &str) -> String;
pub fn astap_float(v: f64) -> String;  // the E±NNN format, incl. the leading space
```

- [ ] **Step 1: Write the failing tests**

```rust
/// Byte-exact, because AstroOps parses these. A "close enough" exponent or a
/// missing alignment space is a file the consumer silently misreads.
#[test]
fn astap_float_matches_real_astap_bytes() {
    assert_eq!(astap_float(1920.5),               " 1.9205000000000000E+003");
    assert_eq!(astap_float(254.23046742390622),   " 2.5423046742390622E+002");
    assert_eq!(astap_float(-40.311880588850023),  "-4.0311880588850023E+001");
    assert_eq!(astap_float(6.8154932258843713e-4)," 6.8154932258843713E-004");
    assert_eq!(astap_float(-5.8335417754934037e-4),"-5.8335417754934037E-004");
}

/// Three-digit zero-padded exponent -- Rust's {:E} gives E-4, ASTAP gives E-004.
#[test]
fn the_exponent_is_always_three_digits() {
    for v in [1.0_f64, -1.0, 1e-4, 1e100, -1e-100, 0.0] {
        let s = astap_float(v);
        let e = s.find('E').expect("must have an exponent");
        assert_eq!(s.len() - e, 5, "{s} must end in E±NNN");
    }
}

#[test]
fn the_success_file_matches_the_real_fixture_byte_for_byte() {
    let real = std::fs::read_to_string("tests/fixtures/astap-success.ini").unwrap();
    let cmdline = real.lines().find(|l| l.starts_with("CMDLINE=")).unwrap()
        .trim_start_matches("CMDLINE=");
    let wcs = wcs_from_the_fixture_values();  // the §1a numbers
    assert_eq!(format_ini_success(&wcs, cmdline), real);
}

/// The failure file starts with a literal blank line. It looks like a stray
/// writeln in ASTAP, but a consumer that skips line 0 would break on a file
/// that lacked it, so reproduce it exactly.
#[test]
fn the_failure_file_starts_with_a_blank_line_and_orders_cmdline_before_error() {
    let s = format_ini_failure("astap_cli -f x.fits", "Not enough stars.");
    assert!(s.starts_with('\n'), "byte 0 must be a newline");
    assert_eq!(s, "\nPLTSOLVD=F\nCMDLINE=astap_cli -f x.fits\nERROR=Not enough stars.\n");
    assert!(!s.contains("CRVAL"), "a failed solve writes no WCS keys");
}
```

- [ ] **Step 2: Run to verify failure.**

- [ ] **Step 3: Implement.** Rust's `{:E}` gives `E-4`; post-process the exponent
to three digits and prepend the alignment space for non-negative values.
`CDELT`/`CROTA` derive from the CD matrix — the spec §7.2 already requires psolve
to emit CDELT+PC, so reuse that decomposition rather than writing a second one.

- [ ] **Step 4: Run the tests.** Expected PASS.
- [ ] **Step 5: Commit.**

---

### Task 8: The `.wcs` sidecar writer

Ground truth: `/tmp/psolve-m3-facts.md` §2. **Two formats**, and the default is
the one that matters:

- **Default (no `-wcs`)** — what 100% of real production files on this machine are: FITS-card-styled **text**, LF after each ~80-char card, **not** padded to 2880, containing the original capture header plus solve keywords and `COMMENT`s.
- **With `-wcs`** — a true FITS block: 8640 bytes (3×2880), **zero newlines**, 108 cards of exactly 80 bytes, `END` plus blank padding.

`.wcs` values use **12** mantissa digits, not the `.ini`'s 16.

**Files:** Modify `crates/psolve-cli/src/sidecar.rs`; test `crates/psolve-cli/tests/sidecar_wcs.rs`; fixture: copy one real `.wcs` of each style.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn the_default_wcs_is_newline_terminated_text_and_is_not_block_padded() {
    let s = format_wcs_text(&wcs_fixture(), &original_header_fixture());
    assert!(s.contains('\n'), "the default style is newline-terminated text");
    assert_ne!(s.len() % 2880, 0, "the default style is not padded to 2880");
    assert!(s.contains("CRVAL1"));
}

/// -wcs is a real FITS block: no newlines at all, an exact multiple of 2880,
/// and every card exactly 80 bytes.
#[test]
fn the_wcs_flag_emits_a_true_fits_block() {
    let b = format_wcs_fits_block(&wcs_fixture(), &original_header_fixture());
    assert_eq!(b.len() % 2880, 0, "must be a whole number of 2880-byte blocks");
    assert!(!b.contains(&b'\n'), "a FITS block contains no newlines");
    for card in b.chunks(80) {
        assert_eq!(card.len(), 80);
        assert!(card.is_ascii(), "FITS cards are ASCII");
    }
    let text = String::from_utf8_lossy(&b);
    let end = text.find("END").expect("must contain END");
    assert!(text[end + 3..].bytes().all(|c| c == b' '),
            "everything after END must be blank padding");
}

#[test]
fn wcs_values_use_twelve_mantissa_digits_not_the_ini_sixteen() {
    let s = format_wcs_text(&wcs_fixture(), &original_header_fixture());
    let line = s.lines().find(|l| l.starts_with("CRVAL1")).unwrap();
    let mantissa: String = line.chars().filter(|c| c.is_ascii_digit()).collect();
    assert!(mantissa.len() <= 14, "the .wcs style is 12 mantissa digits, not the .ini's 16");
}
```

- [ ] **Step 2-5:** verify failure, implement both writers, verify, commit.

---

### Task 9: `-update` — the one dangerous path in this milestone

`-update` writes the solution into an existing FITS header. **Header growth
shifts every byte of pixel data.** Four archive frames were silently modified
this way once before, which is why these constraints are non-negotiable:

1. `psolve-core` gains **no write path whatsoever**. All of this lives in `psolve-cli`.
2. Never write in place. Write a **complete temp copy** beside the target, then `rename` — atomic on the same filesystem, so an interrupted run cannot leave a truncated frame.
3. Honour `PSOLVE_READONLY` (any non-empty value) and a `.psolve-readonly` marker in the target's directory or any ancestor: refuse, exit `3`, explain.
4. Default **off**. Only `-update` enables it.
5. Verify before rename: reparse the temp file's header, confirm the data unit begins at the same byte offset as the original, and confirm the pixel bytes are **byte-identical**. Abort and delete the temp on any mismatch.
6. **`~/astroops/library/` and `~/astroops/archive/` are immutable.** Tests must operate only on copies inside the scratch directory.

**Files:** Create `crates/psolve-cli/src/fits_update.rs`; test `crates/psolve-cli/tests/fits_update.rs`.

- [ ] **Step 1: Write the failing tests**

```rust
/// The whole hazard in one test: whatever the header does, the pixels must not
/// move and must not change. Four real frames were silently corrupted by a
/// header rewrite that shifted the data unit.
#[test]
fn updating_a_header_never_moves_or_alters_the_pixel_data() {
    let (dir, path) = temp_fits_copy();                 // scratch only
    let before = read_data_unit(&path);
    let before_offset = data_unit_offset(&path);
    update_header_in_place(&path, &wcs_fixture()).unwrap();
    assert_eq!(data_unit_offset(&path), before_offset, "the data unit moved");
    assert_eq!(read_data_unit(&path), before, "pixel bytes changed");
    drop(dir);
}

#[test]
fn a_header_that_would_grow_the_block_count_is_refused_not_truncated() {
    let (dir, path) = temp_fits_copy_with_full_header();
    let err = update_header_in_place(&path, &many_extra_keywords()).unwrap_err();
    assert!(format!("{err}").contains("header"), "must refuse, not silently shift data");
    drop(dir);
}

#[test]
fn psolve_readonly_env_refuses_the_write() {
    let (dir, path) = temp_fits_copy();
    std::env::set_var("PSOLVE_READONLY", "1");
    let r = update_header_in_place(&path, &wcs_fixture());
    std::env::remove_var("PSOLVE_READONLY");
    assert!(r.is_err(), "PSOLVE_READONLY must refuse the write");
    drop(dir);
}

#[test]
fn a_psolve_readonly_marker_file_refuses_the_write() {
    let (dir, path) = temp_fits_copy();
    std::fs::write(dir.path().join(".psolve-readonly"), b"").unwrap();
    assert!(update_header_in_place(&path, &wcs_fixture()).is_err());
    drop(dir);
}

#[test]
fn a_failed_update_leaves_no_temp_file_behind() {
    let (dir, path) = temp_fits_copy_with_full_header();
    let _ = update_header_in_place(&path, &many_extra_keywords());
    let strays: Vec<_> = std::fs::read_dir(dir.path()).unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().contains(".psolve-tmp"))
        .collect();
    assert!(strays.is_empty(), "a failed update left {} temp files", strays.len());
    drop(dir);
}
```

- [ ] **Step 2-5:** verify failure, implement, verify, commit. The commit message
must state that the data unit is verified byte-identical before the rename.

---

### Task 10: ASTAP-compatible exit codes and mode dispatch

Observed ASTAP behaviour (`/tmp/psolve-m3-facts.md` §3c — empirical; ASTAP
documents no exit codes anywhere): `0` on a successful solve and on `--help`;
`1` for a missing input file and for a missing star database. It is a **two-code
scheme**, not a rich taxonomy.

psolve's native codes (`0/1/2/3`) stay exactly as they are. In ASTAP mode only,
collapse to ASTAP's scheme so a drop-in replacement behaves identically.

**Files:** Modify `crates/psolve-cli/src/main.rs`, `astap_args.rs`; test `crates/psolve-cli/tests/astap_exit_codes.rs`.

- [ ] **Step 1: Write the failing tests**

```rust
/// A drop-in replacement that returns a different exit code is not a drop-in
/// replacement -- AstroOps branches on it.
#[test]
fn astap_mode_uses_astap_exit_codes() {
    assert_eq!(run_astap(&["-f", "/nonexistent.fits"]).code, 1);
    assert_eq!(run_astap(&["-f", "valid.fits", "-d", "/nonexistent"]).code, 1);
    assert_eq!(run_astap(&["--help"]).code, 0);
}

/// Native mode keeps its own richer scheme -- ASTAP compatibility must not
/// leak into it.
#[test]
fn native_mode_keeps_its_own_exit_codes() {
    assert_eq!(run_native(&["solve", "/nonexistent.fits", "--index", "i"]).code, 3);
    assert_eq!(run_native(&["solve"]).code, 2);
}

#[test]
fn a_failed_solve_in_astap_mode_still_writes_the_failure_ini() {
    let r = run_astap(&["-f", "unsolvable.fits", "-d", "/nonexistent"]);
    let ini = std::fs::read_to_string("unsolvable.ini").unwrap();
    assert!(ini.starts_with('\n') && ini.contains("PLTSOLVD=F") && ini.contains("ERROR="));
    assert_eq!(r.code, 1);
}
```

- [ ] **Step 2-5:** verify failure, implement, verify, commit.

---

### Task 11: The agreement run — this milestone's go/no-go gate

**A schema fact changes the design:** `catalogue.db` holds **no WCS**. The
`measurement` table stores only a solved **centre** (`ra_deg`, `dec_deg`) keyed
`(frame_id, tool_version)`, and `tool_version='astap/astap+d50'` has exactly
**9495** rows. Full CD/CRVAL ground truth therefore comes from the **FITS
headers**, which do carry ASTAP's solution (verified: `CRVAL1`, `CRVAL2`,
`CD1_1`…`CD2_2` are present on solved frames).

So: the DB supplies the frame list and ASTAP's centre; the header supplies the
full WCS. Compare against both.

**A second fact sets the sample: 10,066 of 14,970 frames are OSC (CFA).** The
final M2 review recorded that a CFA frame reported **2× its true FOV while
claiming success** — the majority case is the one with a known hazard, so the
sample must be CFA-weighted, not filtered to mono.

Frame diversity (§5): dimensions 3856×2180 (8565) and 3840×2160 (3938); binning
1×1 (14079) and 2×2 (891); filters OSC-dominant plus L/S/O/H/Duo-Band/R/G/B;
declination −89.97° to +24.22° among ASTAP-solved frames. **The rig index covers
dec ≤ +45°, so it covers every solved frame in this library.**

**Files:** Create `scripts/agreement.sh`, `scripts/agreement-report.py`.

- [ ] **Step 1: Extract the frame list**

```bash
sqlite3 -readonly ~/astroops/state/catalogue.db <<'SQL' > /tmp/astap-solves.tsv
SELECT l.path, m.ra_deg, m.dec_deg, f.naxis1, f.naxis2, f.binning, f.filt_eff
FROM measurement m
JOIN frame f ON f.id = m.frame_id
JOIN location l ON l.frame_id = f.id
WHERE m.tool_version = 'astap/astap+d50'
  AND m.ra_deg IS NOT NULL AND l.intact = 1;
SQL
wc -l /tmp/astap-solves.tsv
```

Expect ~9495 rows (fewer if some `location` rows are missing or not intact —
record the exact number and the shortfall; do not silently proceed on a subset).

- [ ] **Step 2: Run psolve over a stratified sample first, not all 9495**

Sample 300 frames stratified across the §5 axes (both dimensions, both binnings,
OSC and each mono filter, and the full declination range). Run at **defaults**
plus `--index`. A full run comes only after the sample looks sane — 9495 solves
at ~0.2 s is ~30 min, and a bug found at frame 9000 wastes all of it.

Emit one NDJSON line per frame: path, psolve's solved flag, centre, scale,
parity, matched count, timings, and ASTAP's DB centre and header WCS.

- [ ] **Step 3: Report**

`scripts/agreement-report.py` must print:
- **solve rate** — psolve solved / ASTAP solved, with the count that ASTAP solved and psolve did not (this is the number that matters most)
- **separation distribution** — median, 90th, 99th percentile, max, in arcsec, against both the DB centre and the header WCS
- **disagreements over 30″**, listed individually with the frame path — a confidently wrong solve is worse than a failure and must never be averaged away
- **scale ratio distribution** — this is where a CFA frame reporting 2× its FOV shows up; flag any frame whose scale differs from the header by more than 5%
- **parity mismatches**, listed individually
- breakdowns of every one of the above **by binning and by OSC-vs-mono**

- [ ] **Step 4: Set the gate explicitly, before looking at the numbers**

Write the thresholds into the script as constants, with a non-zero exit when
they are not met:

```python
# Set BEFORE the run. A gate chosen after seeing the numbers is not a gate.
MIN_SOLVE_RATE      = 0.95   # of frames ASTAP solved
MAX_MEDIAN_SEP_ASEC = 5.0
MAX_P99_SEP_ASEC    = 30.0
MAX_GROSS_ERRORS    = 0      # any solve >30" from ASTAP is disqualifying
MAX_PARITY_ERRORS   = 0
```

- [ ] **Step 5: Run the full 9495 once the sample passes.** Record the outcome in `docs/superpowers/2026-08-14-m3-first-real-frame.md`, pass or fail. **A failed gate is a finding, not a setback** — record it plainly rather than tuning thresholds to fit.

- [ ] **Step 6: Commit** the scripts and the recorded results.

---

### Task 12: The timing comparison, run fairly

**The fairness question must be settled first.** The 180 ms ASTAP figure this
project has been quoting was measured without recording its flags, and AstroOps
invokes ASTAP **blind** (`-r 180`). Comparing ASTAP's blind solve to psolve's
hinted solve would flatter psolve; comparing hinted-to-hinted is the honest
like-for-like.

- [ ] **Step 1: Measure ASTAP both ways on the same frame**

```bash
A=/home/user/astap/astap_cli
F=/tmp/psolve-bench/frame.fits          # a COPY -- -update rewrites the header
for mode in blind hinted; do
  case $mode in
    blind)  args="-r 180 -fov 1.4770" ;;
    hinted) args="-ra 18.313 -spd 76.189 -r 1.65 -fov 1.4770" ;;
  esac
  /usr/bin/time -p $A -f "$F" $args -d /home/user/astap 2>&1 | grep real
done
```

Run each 5 times, take the median, and record **both** numbers.

- [ ] **Step 2: Measure psolve the same two ways**, hinted and with no hint, 5 runs each, median.

- [ ] **Step 3: Publish a table with the flags shown for every row.** Any claim
about relative speed must name the mode it was measured in. If psolve is slower
in either mode after Task 1, say so plainly and record it — the design projected
12–14 ms, and the gap between a projection and a measurement is the finding.

- [ ] **Step 4: Commit** the table into the decision record.

---

### Task 13: Document the ASTAP mode

- [ ] **Step 1:** Write `docs/astap-compat.md` covering: the supported flag set and the exact semantics of `-ra` (hours) and `-spd` (dec+90); which ASTAP flags are accepted-and-ignored; the `.ini` and `.wcs` formats produced; exit codes in each mode; and the `-update` safety model (temp-copy + rename, `PSOLVE_READONLY`, `.psolve-readonly`, default off).
- [ ] **Step 2:** Add a "Drop-in replacement" section to `README.md` showing the two real AstroOps invocations from Task 6 working against psolve.
- [ ] **Step 3:** State the measured agreement and timing from Tasks 11 and 12, with their real numbers. **No projected or aspirational figures.**
- [ ] **Step 4:** Commit.

---

## Self-Review

**Spec coverage.** Spec §8 (ASTAP-compat interface) → Tasks 6–10. §7.2 (JSON contract) → already shipped, extended by the timings fix in `6255958`. §11's M3 definition (ASTAP-compat CLI + agreement run + timing) → Tasks 6–12. The performance and defaults work (Tasks 1–5) is not in the spec's M3 — it is added because the first real-frame measurement showed the milestone cannot be honestly evaluated without it: a solver that does not solve at its defaults cannot be agreement-tested, and a speed claim cannot be made against a 63% hot spot that is an accident.

**Placeholder scan.** Two intentional `<fill in>` markers in Task 1 Step 7, each a measured number the executor must substitute before committing; the step says so. No others.

**Type consistency.** `AstapArgs`/`parse_astap`/`hint_degrees` (Task 6) are consumed by Tasks 7, 9, 10. `format_ini_success`/`format_ini_failure`/`astap_float` (Task 7) and `format_wcs_text`/`format_wcs_fits_block` (Task 8) share `sidecar.rs`. `default_radius_for` (Task 2) is reused by Task 6's `-fov` handling. `extract_params_from` (Task 3) is a prerequisite for Task 4's sweep.

**Ordering.** Task 5 must precede Tasks 7 and 8, or their fixtures are gitignored. Task 3 must precede Task 4. Task 1 should precede Task 12, or the timing comparison measures a hot spot that is about to disappear.

**Known risk carried in.** 10,066 of 14,970 frames are OSC/CFA, and the final M2 review found a CFA frame reporting 2× its true FOV while claiming success. Task 11's scale-ratio check is the detector for that, and its sample is deliberately CFA-weighted. If that defect is still live, expect the agreement gate to fail on scale before it fails on separation.
