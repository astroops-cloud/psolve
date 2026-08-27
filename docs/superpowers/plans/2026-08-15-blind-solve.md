# Blind Solve — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Solve a frame with no pointing hint, in seconds, without confidently returning a wrong position.

**Architecture:** A second on-disk artefact (`.psqidx`) holding precomputed geometric quad codes for the whole sky in angular scale bands, plus an adaptive code-space search structure. At solve time an image quad's 4-vector is looked up directly; position falls out of the match instead of being required as input. The existing star index and the hinted solve path are untouched.

**Tech Stack:** Rust 2021. `psolve-core` (zero deps, no filesystem), `psolve-index` (mmap), `psolve-cli` (`psolve-index`, `psolve-core`, `rayon`).

**Spec:** `docs/superpowers/specs/2026-08-15-blind-solve-design.md` — read §4a first, the spike results, which corrected two of the spec's own assumptions.

## Global Constraints

- `psolve-core`: **zero dependencies, no filesystem access.** `tests/no_filesystem.rs` tokenises whole words **including inside comments** — avoid bare `fs`, `net`, `process`, `env`, `File`, `OpenOptions`, `PathBuf` even in prose.
- `psolve-index` may use `memmap2` (already present). `psolve-cli` may depend only on `psolve-index`, `psolve-core`, `rayon`. **No new dependency.**
- No `unwrap()`/`expect()` on external data.
- `cargo clippy --all-targets --workspace -- -D warnings` clean; `cargo test --workspace` green. **Baseline 447 tests**, count must not drop.
- **`~/astroops/`, `~/mnt/astro/`, `~/gaia-dr3/` are STRICTLY READ-ONLY.** Never `-update`. Hash frames with `shasum -a 256` before and after. Verify non-modification with an absolute `touch -t` reference — **never `find -newermt`** (this machine's `find` is `bfs`: it rejects relative timestamps, prints nothing, exits non-zero, proves nothing).
- **The existing `.psidx` format must not change.** It is load-bearing for the working hinted path. The quad index is a separate file with its own magic, version and digest.
- **The hinted path must be unaffected** — same solve rate, same separations, same timings on the 9,495-frame corpus. Blind is additive.
- Commit messages cite measured numbers, never estimates.

## Facts established by the spike — do not re-derive

- **Build from G≤16.** At G≤14 the 0.25° band fails on ~20% of sky by area, giving blind solve a ~0.5° minimum field size.
- **Bands 0.25° to 8°, doubling.** 750,998 tiles across six bands, 563k of them in the 0.25° band.
- **A per-tile emission cap is mandatory.** The 1.0° band offers a median 11,183 formable quads per tile against a 10-30 target. Cap at 25; uncapped storage is 860 MB-1.08 GB.
- **Expected size: 18.67 M quads, ~448 MB** at 24 B/quad with a 12-star budget.
- **The code space is clustered, not uniform** — CV 0.74-1.51, hotspots 3-9.3× the mean, up to 39% of a naive equal-width grid empty. A uniform grid hash is **not** sufficient.
- Build is 12-15 min single-threaded and embarrassingly parallel.

## Existing shapes to reuse

```rust
// psolve-core/src/quad.rs
pub struct Quad { pub code: [f64; 4], pub idx: [usize; 4], pub diag: f64 }
pub fn quad_code(p0: (f64,f64), p1: (f64,f64), p2: (f64,f64), p3: (f64,f64)) -> Option<[f64; 4]>

// psolve-index/src/format.rs  — the pattern to mirror, NOT to modify
pub const MAGIC: [u8; 8] = *b"PSIDX\0\0\0";
pub const FORMAT_VERSION: u32 = 1;
pub const HEADER_BYTES: usize = 128;
pub const RECORD_ALIGN: u64 = 4096;

// psolve-core/src/verify.rs
pub struct Confidence { pub log_odds: f64, pub chance_matches: f64, pub matched: usize }
// VerifyParams::default() currently has min_log_odds: 12.0
```

---

## File Structure

| File | Responsibility | Tasks |
|---|---|---|
| `crates/psolve-index/src/quad_format.rs` | **new** — `.psqidx` header, record layout, digest | 1 |
| `crates/psolve-index/src/quad_builder.rs` | **new** — tile/band sweep, per-tile cap, write | 2 |
| `crates/psolve-index/src/quad_reader.rs` | **new** — mmap read, code lookup | 3, 4 |
| `crates/psolve-cli/src/cmd_quadindex.rs` | **new** — `quad-index build` / `info` | 2, 3 |
| `crates/psolve-core/src/blind.rs` | **new** — candidate transform from code matches | 5 |
| `crates/psolve-core/src/verify.rs` | all-sky null gate | 6 |
| `crates/psolve-cli/src/cmd_solve.rs`, `main.rs` | wiring, both entry points | 7 |

---

### Task 1: The `.psqidx` format

**Files:** Create `crates/psolve-index/src/quad_format.rs`; export from `lib.rs`. Tests co-located.

**Interfaces:**
- Produces: `QuadHeader { version, nside, n_quads, n_bands, band_scales, mag_limit, epoch, records_offset, records_sha256, name }`, `QuadRecord`, and `HEADER_BYTES`/`MAGIC`/`FORMAT_VERSION` constants.
- **Magic must differ from `.psidx`'s** — use `*b"PSQIDX\0\0"`. A reader handed the wrong file type must fail cleanly, not misparse.

A `QuadRecord` needs the 4-vector code and enough to recover the four stars' sky positions. Storing the star positions directly (4 × ra/dec) costs more but avoids a second lookup; storing indices into the star index couples the two files and their versions. **Choose, and state the reasoning in a doc comment** — the spike's 24 B/quad estimate assumed a compact form.

- [ ] **Step 1: Write the failing tests** — header round-trips through `to_bytes`/`from_bytes`; wrong magic is rejected; a `.psidx` file offered to the quad reader is rejected rather than misread; version mismatch is rejected; record round-trips; the digest covers the record region and detects a single flipped byte.
- [ ] **Step 2: Run to verify failure.**
- [ ] **Step 3: Implement**, mirroring `format.rs`'s structure. Reuse `psolve-index`'s existing in-crate `sha256.rs` rather than adding a dependency.
- [ ] **Step 4: Run the tests.**
- [ ] **Step 5: Commit.**

---

### Task 2: The quad builder

**Files:** Create `crates/psolve-index/src/quad_builder.rs` and `crates/psolve-cli/src/cmd_quadindex.rs`; route `quad-index build` in `main.rs`.

For each band, tile the sky at that band's scale; for each tile, fetch the brightest N stars from the star index within the tile, form quads with the existing `quad.rs` machinery, and emit **at most 25**, deterministically.

**Which 25 matters and is a real design decision.** The spike found ~11,000 formable per tile at 1.0°, so selection is the whole game. Prefer quads that are (a) well-conditioned — not near-degenerate — and (b) spatially spread within the tile rather than all from its densest corner. That is the same lesson conditional stratification just paid for. State the rule in a doc comment.

- [ ] **Step 1: Write the failing tests** — a synthetic star field produces a deterministic quad set (pin the count and a digest); the per-tile cap is honoured exactly; the same input twice gives byte-identical output; a tile with fewer than 4 stars emits nothing rather than panicking; band assignment puts a quad in the band its diagonal falls in.
- [ ] **Step 2: Run to verify failure.**
- [ ] **Step 3: Implement.** Use `rayon` in the CLI for the tile sweep if it helps — the spike says embarrassingly parallel. Keep the builder itself deterministic regardless of thread count; a nondeterministic index would make every downstream measurement unrepeatable.
- [ ] **Step 4: Build a real index** from `~/astroops/data/gaia-dr3-g16-dec45-nside64.psidx` and report actual quad count, size and wall-clock against the spike's 18.67 M / 448 MB / 12-15 min.
- [ ] **Step 5: Commit** with the measured numbers.

---

### Task 3: `quad-index info`, and the reader

**Files:** Create `crates/psolve-index/src/quad_reader.rs`; extend `cmd_quadindex.rs`.

- [ ] **Step 1: Write the failing tests** — `info` reports header fields and per-band counts; `--verify` recomputes the digest and detects corruption; a truncated file is rejected cleanly.
- [ ] **Step 2-5:** verify failure, implement (mmap, mirroring `reader.rs`), run, commit.

---

### Task 4: The code-space search structure

**This is where the spike's second finding lands.** A uniform grid is measurably wrong: up to 39% of its bins would be empty while hotspots hold 3-9.3× the mean.

**Files:** Extend `quad_reader.rs`. Benchmark harness may live in `tests/`.

**Interfaces:**
- Produces: `fn candidates(&self, code: [f64; 4], tol: f64, band: usize) -> impl Iterator<Item = QuadRecord>`

- [ ] **Step 1: Prototype both** an equal-population (quantile) grid and a kd-tree over the 4-vector, and **measure** lookup latency and candidate-set size on the real index. Do not choose from theory.
- [ ] **Step 2: Write the failing tests** — a code present in the index is found; a code far from any is not; the candidate set is a superset of a brute-force scan within the same tolerance (correctness, not just speed); lookup is deterministic.
- [ ] **Step 3: Implement the winner**, and record in a doc comment what the loser measured, so the choice is not re-litigated from taste later.
- [ ] **Step 4: Benchmark** against the milestone's 5 s target — with the caveat that a full blind solve is many lookups, so per-lookup budget is roughly (5 s / image quad count).
- [ ] **Step 5: Commit** with measured latency.

---

### Task 5: Candidate transforms from code matches

**Files:** Create `crates/psolve-core/src/blind.rs`. `psolve-core` stays dependency-free and filesystem-free — the quad candidates are **passed in**, exactly as the catalogue is for the hinted path.

- [ ] **Step 1: Write the failing tests** — given a synthetic image quad and its true catalogue counterpart, the derived transform recovers the known WCS to sub-arcsecond; a mismatched pair yields either no transform or one that fails verification; the function is pure.
- [ ] **Step 2-5:** verify failure, implement (reuse `fit.rs`), run, commit.

---

### Task 6: The verification gate, re-derived for an all-sky null

**This is the correctness crux of the milestone. Everything else is engineering.**

`verify.rs`'s Poisson log-odds gate was calibrated against a **disc of known position**. The blind null hypothesis is the whole sky, which is enormously larger, so the same threshold will pass coincidences. `VerifyParams::default()` currently uses `min_log_odds: 12.0`.

**Files:** Modify `crates/psolve-core/src/verify.rs`.

- [ ] **Step 1: Derive the blind null explicitly.** The hinted λ is `n_image · n_cat · πr²/A` over the search disc. Write down the all-sky equivalent, including the number of candidate quads examined — a blind solve tests far more hypotheses, and the multiple-comparison term is the whole difference. Put the derivation in a doc comment; a future reader must be able to check it.
- [ ] **Step 2: Write the failing tests** — a genuine match clears the blind gate; a coincidence at the hinted threshold does **not** clear the blind one; the hinted gate is numerically unchanged for hinted solves.
- [ ] **Step 3: Implement** as a distinct threshold or a distinct parameter set. **The hinted path must be bit-identical** — this project has twice shipped a change that perturbed the working path while fixing another.
- [ ] **Step 4: Run the tests**, plus the full agreement corpus to confirm the hinted path did not move.
- [ ] **Step 5: Commit.**

---

### Task 7: Wiring, both entry points

**Files:** `crates/psolve-cli/src/cmd_solve.rs`, `crates/psolve-cli/src/main.rs`.

Native: `psolve solve <FILE> --index <star.psidx> --quad-index <q.psqidx>` with no `--hint`.
ASTAP mode: `-r 180` with no `-ra`/`-spd` selects blind — which is what AstroOps already sends.

**A fix of this shape reached only one entry point on 2026-08-14.** Both must be wired, with a test that runs the same frame through both.

- [ ] **Step 1: Write the failing tests** — a frame with no hint and a quad index solves; the same frame through `-f` with `-r 180` and no `-ra`/`-spd` also solves; without a quad index the behaviour is unchanged (`NO_HINT`, not a crash); a hint plus a quad index still takes the hinted path.
- [ ] **Step 2-5:** verify failure, implement, run, commit.

---

### Task 8: Acceptance

Report all five criteria from spec §6, whatever they show.

- [ ] **Step 1: Solves without a hint** on frames whose hint is currently required, landing within 30″ of the hinted answer. Report the distribution.
- [ ] **Step 2: The null test — zero false positives.** Solve frames against a quad index built from a **different part of the sky**. Every one must fail. A blind solver that confidently returns a wrong position is far worse than one that refuses, and this project has twice been bitten by measures erring in the flattering direction. **This is the acceptance criterion that matters most.**
- [ ] **Step 3: Speed** — under 5 s, i.e. comparable to ASTAP's failing case (5.42 s measured) and within 50× of its solving case (0.10 s). Slower than a positional sweep (132-529 s) is an automatic failure.
- [ ] **Step 4: The hinted path is unchanged** — re-run the 9,495-frame corpus and compare against `docs/superpowers/data/task-11-agreement-full-9495.ndjson.gz`. Current `main`: 9268/9495, median 0.530″, p90 0.946″, p99 3.111″.
- [ ] **Step 5: Sentinel-pointing frames now solve** — the `DEC = -90.` frames that motivated this.
- [ ] **Step 6: Record in `docs/superpowers/2026-08-15-blind-solve-results.md`** and commit. Lead with hours on target, not frame counts.

---

### Task 9: Documentation

- [ ] Update `docs/astap-compat.md` (blind now supported, and what `-r 180` does), `README.md`, and spec §7.2 if the JSON gained fields. State measured numbers only.

---

## Self-Review

**Spec coverage.** §3 approach → Tasks 1-5. §4/§4a bands and depth → Task 2. §5.2 search structure → Task 4, now measurement-led rather than assumption-led. §5.4 verification → Task 6. §5.5 CLI → Task 7. §6 acceptance → Task 8.

**Ordering.** 1 → 2 → 3 → 4 are sequential (format before builder before reader before search). 5 and 6 are independent of each other and of 4. 7 needs 4, 5, 6. 8 needs everything.

**Risk carried in.** Task 6 is the one that can silently fail — a gate that is too loose produces confident wrong answers that look like successes, and only Task 8 Step 2's null test would catch it. If Task 6 and Task 8 Step 2 are both rushed, this milestone ships a liar.
