# M2 decision record

Rulings and deferred findings from the M2 (solver) implementation run, 2026-08-13/14.
Preserved from the execution ledger, which was scratch.

# SDD ledger — plan: docs/superpowers/plans/2026-08-13-m2-solver.md

Spec: docs/superpowers/specs/2026-08-13-psolve-design.md (§6, §7.2, §8.2)
Branch: m2-solver, off main after M1 merged (8bc76f4) + licence/plan docs.
Context: a 701 GB Gaia fetch is running in the background; M2 needs no Gaia data
(all its tests are synthetic), so the two are independent.

## Pre-flight conflict scan

### Pairwise: tasks sharing a file or interface

| A → B | A produces | B consumes | finding |
|---|---|---|---|
| T1 → all | crate, `SolveError`, `ReasonCode`, guard test | error type | clean |
| T2 → T3 | `FitsHeader`, `SolveError` | same file, appended | clean |
| T2/T3 → T11 | `FitsHeader::parse`, `decode`, `pixel_scale_arcsec`, `hint_radec`, `epoch_years` | pipeline reads them | clean |
| T2/T3 → T13 | same, via `psolve_core::fits` | CLI parses header for dims + hint | clean |
| T3 → T4/T5 | `Image { nx, ny, px, binned }` | mesh + extraction | clean |
| T4 → T5 | `Background::{level_at, noise_at}` | thresholding | clean |
| T5 → T11 | `Star`, `Rejections`, `ExtractParams`, `quality()` | pipeline + Solution | clean |
| T6 → T9/T11 | `Quad`, `build_quads`, `quad_code` (pub) | matcher calls `quad_code` for mirrored codes | clean |
| T7 → T8/T11 | `angsep_deg`, `radec_to_tangent`, `tangent_to_radec`, `apply_proper_motion` | fit + projection | clean |
| T8 → T11/T12 | `Wcs`, `FitResult` (both `Copy`), `Parity`, `fit_tan` | pipeline + closed loop | clean — `FitResult: Copy` means T11's `fit: fitres` after reading `fitres.used` compiles |
| T9 → T11 | `match_quads(image_pts, image_quads, cat_pts, cat_sky, cat_quads, p)` | argument order checked against T11's call site | clean |
| T10 → T11 | `confidence`, `accept`, `AcceptParams`, `Confidence` | acceptance gate | clean |
| T11 → T12/T13 | `solve`, `CatalogStar`, `SolveOptions`, `Outcome`, `Solution` | closed loop + CLI | clean |
| T13 → M1 | — | `Index::open`, `brightest_in_disc`, `header().epoch`, `StarRecord` accessors | clean — verified against the merged M1 API |
| T1 → T13 | guard forbids `std::fs`/`std::path` in **psolve-core/src only** | T13's CLI legitimately uses `std::fs` | clean — guard scans `src/*.rs` of psolve-core, not psolve-cli |

### Per-task internal consistency

| task | finding |
|---|---|
| T1–T11 | tests and code agree; every derive needed by a containing type is present (`Copy` on the param structs, `PartialEq` down the `Outcome` chain incl. `Box<Solution>`) |
| **T12** | **F1: `use psolve_core::project::{angsep_deg, tangent_to_radec};` — `tangent_to_radec` is never used in the file.** Under `-D warnings` an unused import fails the build. |
| T12 | `render()`'s `with_optics` parameter is always passed `true`; the `false` branch is dead but harmless |
| T13 | JSON emits CD **and** CDELT+PC per spec §7.2 (added during plan self-review) |

### Rulings

- **Ruling M2-F1: T12 drops the unused `tangent_to_radec` import.** The Definition of Done
  requires `cargo clippy --all-targets -- -D warnings` clean, and an unused import fails it.
  Cost if wrong: none — if a later edit needs the symbol, re-import it.
- **Ruling M2-F2: carry the `with_optics` dead branch.** It documents an intent (a frame
  lacking optics keywords) that a later test may want, and costs nothing. Cost if wrong: a
  reviewer flags dead flexibility as a minor.
- **Precedent carried from M1:** where the plan's code trips a clippy lint, prefer a
  semantically identical rewrite and report it; never change behaviour to satisfy a lint.
  M1 hit this twice (`manual_range_contains`, `map_or` vs `is_none_or`).

## Progress

Task 1: fix round 1/5 — review found the no-filesystem guard is BYPASSABLE, and proved it
  by appending `use std::{fs, path::Path};` + `fs::read` to lib.rs and watching both guard
  tests pass. Substring-matching "std::fs" misses grouped imports, because the std:: prefix
  attaches only to the first element. Defect is in the plan's own code, not the
  transcription. Fix: strip comments, normalise whitespace/braces, match bare call-site
  tokens (fs::, path::, ,fs, File::, ...) as well as qualified ones — AND test the guard
  itself against realistic bypasses, since a guard nobody has checked is a guard nobody
  should trust. Plan amended so a re-run cannot reintroduce it.
Task 1: minor (deferred): the zero-deps check also fires on [dev-dependencies]; that is
  fail-closed and matches the constraint, but the message would baffle whoever first wants
  proptest — a comment was added rather than a behaviour change.
Task 1: fix round 2/5 — the round-1 repair fixed the grouped-import bypass but introduced
  two worse problems, both demonstrated: (a) comment stripping without string-literal
  awareness let a `//` inside a URL string hide a real std::fs::read on the same line;
  (b) `,path` false-positived on `fn solve(image: &[f32], path_length: f64)`.
  **Ruling M2-F3: abandon substring matching; tokenise and match whole identifiers, and do
  NOT strip comments.** Tokenising makes path_length a single token that cannot match
  `path`. Not stripping comments fails CLOSED (a comment containing bare `fs` trips the
  guard, fixed by rewording) where stripping fails OPEN (a real call gets hidden). That
  trade is not close. Cost if wrong: doc comments must avoid seven bare words.
Task 1: fix round 2/5 (2 addressed, 0 open; commits 385451b..64d224f)
Task 1: complete (commits df85753..64d224f, review clean, 4 tests)
Task 1: minor (deferred): a parameter literally named `fs`/`net`/`process`/`env` would trip
  the guard. That is inherent to a whole-token forbidden list, fails closed, and is a policy
  trade rather than a defect — rename the parameter if it ever arises.
Task 2: fix round 1/5 — review approved but found: (a) num()/int() accept NaN and infinity
  and then SATURATE (NaN->0, inf->i64::MAX), handing a caller a plausible-looking value for
  a corrupt card — now rejected at source; (b) the exponent test never exercised the D
  marker it is named for — zero coverage on the one line it was testing; (c) the report
  described "one clippy-driven change" but the diff also added a buffer-end break that the
  reviewer proved load-bearing (without it the plan's own reference code fails its own
  required test). The fix was right; the omission was the issue. Plan amended for (a) and
  the missing break.
Task 2: minor (deferred): `card.len() > 9` is always true — card is a fixed 80-byte slice
Task 2: minor (deferred): unterminated-quote and >MAX_BLOCKS paths are trace-verified only
Task 2: fix round 1/5 (3 addressed, 0 open; commits 41228ff..2a2a829)
Task 2: complete (commits 9fc1856..2a2a829, review clean)
Task 2: minor (deferred): fix report's test count was stale after the fix — reports keep
  under-disclosing; called out to the implementer twice now
Task 3: complete (commits dee38c5..8f15b36, review clean, 24 tests)
Task 3: minor (deferred): BITPIX goes through int()'s i64->i32 truncating cast, so a corrupt
  value differing from a legal one by a multiple of 2^32 wraps into a legal one instead of
  erroring. Reviewer reproduced it (4294967312i64 as i32 == 16). Inconsistent with num()'s
  own anti-coercion philosophy; one-line fix is to match on i64 literals directly.
  ** flagged for the final review to triage — it is cheap and principled **
Task 3: minor (deferred): odd nx/ny with BAYERPAT silently drops the trailing row/column.
  Physically unreachable for a real Bayer sensor; wants a comment, not code.
Task 3: minor (deferred): no test covers a declination of -00 degrees, the classic
  sexagesimal sign bug. The implementation is correct (reviewer verified independently) but
  the existing test would pass even with a naive sign_of(dd).
Task 3: minor (deferred): BSCALE == 0 is silently overridden to 1 without comment.
Task 4: fix round 1/5 — review found median() sorted in place while robust_sigma() silently
  depended on that ordering, an undocumented coupling with no test protecting it: reordering
  the calls would give a wrong sigma with nothing failing. Fixed by REMOVING the side effect
  (explicit sort_pixels, then median_of_sorted and robust_sigma both take sorted input)
  rather than documenting it, plus a test pinning order-independence and a clamp test that
  checks the value instead of only the absence of a panic. Plan amended.
Task 4: minor (deferred): extended_nebulosity_becomes_background does not distinguish median
  from mean (every contributing tile is homogeneous, so both agree). Plan-mandated. The
  bright-star test carries that load; kept as two tests each proving one thing.
Task 4: minor (deferred): median of an even-length buffer takes the upper middle rather than
  averaging; harmless here, and no test pins it either way.
Task 4: fix round 1/5 (1 addressed, 0 open; commits 007ada7..9d634c9 — side effect removed,
  not documented; clamp test tightened from "does not panic" to an actual value)
Task 4: complete (commits 8f15b36..9d634c9, review clean, 32 tests)
Task 5: fix round 1/5 — two Important, both defects in the plan's own code:
  (a) the saturation threshold was `frame_max * 0.98`, trivially satisfied by the brightest
      legitimate star in any frame (the implementer found this; 6 of 11 tests failed). Their
      replacement, a hardcoded 65535 ceiling, is INERT on BITPIX=-32 Siril frames that
      fits.rs already decodes and tests — trading a loud bug for a silent one. Now a caller-
      supplied ExtractParams.saturation. **T11 must set it from the header.**
  (b) the extended-source cap took the median over ALL blobs including hot pixels. Reviewer
      confirmed numerically: 950 hot pixels + 10 real 100px stars -> median 1 -> cap 25 ->
      every real star rejected as "extended", inverting the filter. Now filtered to
      blobs >= min_pix first.
  Plan amended for both.
Task 5: minor (deferred): unreachable else on ellipticity; unreachable b.sum <= 0 branch;
  sort closure shadows `p`; theta_deg sign convention untested despite being a guiding-drift
  sensor; the saturation test hardcodes the same constant as the implementation.
Task 5: fix round 1/5 (2 addressed, 0 open; commits 4d3a9c5..8246b4e)
Task 5: complete (commits 1128b82..8246b4e, review clean, 41 tests)
  Implementer disclosed a third finding unprompted: the required saturation test was
  unsatisfiable because the shared blank() fixture's fixed +/-2.0 noise swamped a near-zero
  sky. They scaled it to 2% of sky. Reviewer verified independently: bit-identical at
  sky=100, and at sky=50 measured every affected assertion's margin (0.1-1.3% of tolerance)
  to confirm none was noise-limited either way. Good disclosure, properly checked.
Task 6: complete-pending-fix. Implementer found and disclosed FOUR real defects in the
  plan's code: a non-slice dedup in a test; quad_code checking only the MAX pairwise
  distance so a coincident non-A/B pair returned Some instead of None; a quad-count
  explosion; and -- pleasingly -- the plan's own doc comment naming a domain whose second
  half is a forbidden token, tripping the very guard designed in Task 1. That is the
  fail-closed trade working as intended.
Task 6: fix round 1/5 — the quad-count fix went the wrong way and the reviewer MEASURED it:
  sliding-window selection retains 80-85% quad-index overlap under detection-scale noise
  against 92-97% for full C(k,3) combinations. The matcher only works when the same four
  stars are chosen from both point sets, so 12 points of recall were traded for a count
  target that the fix did not even hit (3.9/star produced against a doc comment claiming
  0.75). **Ruling M2-F4: recall beats volume. Restore full combinations, bound with the
  existing max_quads, and interleave the cap across seed points** -- truncating one growing
  list keeps only quads from the first few points, all spatially clustered in one corner,
  which is worse than having fewer. Doc comment and the count-based test assertion both
  corrected, since both encoded a target this algorithm never met.
  Cost if wrong: more quads to compare, which at ~75k comparisons is under a millisecond.
Task 6: minor (deferred): canonical-ordering epsilon 1e-15 is ~4-5 ULP above f64::EPSILON
  with no headroom; fails safe (returns None) if ever hit.
Task 6: minor (deferred): seen-set uses Vec::contains, O(n) per lookup; fine at this scale.
Task 6: fix round 1/5 (1 addressed, 0 open; commits 7415bf8..5e0528c). Implementer measured
  the restored version independently: 891 quads (14.85/star) and 91.8% index-set overlap
  under simulated centroiding noise, matching the reviewer's 92-97% band for full
  combinations against 80-85% for the sliding window. Reviewer separately confirmed the
  interleave discriminates: round-robin touches 60/60 seed stars at a cap of 60, sequential
  truncation only 13/60.
Task 6: complete (commits d10fb8b..5e0528c, review clean, 55 tests)
Task 7: fix round 1/5 — the plan's proper-motion pole guard (clamping cos(dec) at 1e-6)
  silently understated motion by ~2900x at dec 89.99999 while returning a finite number, and
  the only test covering it asserted just is_finite(), so it passed for any wrong answer.
  **Ruling M2-F5: do proper motion in Cartesian.** pmRA* is already cos(dec)-multiplied, so
  both components are arcs; converting to a coordinate-RA increment is what introduces the
  division that diverges at the pole. Adding the offset to a unit vector has no singularity
  at all, and the local east vector stays unit at every declination. The replacement test
  asserts the same arc at 0/45/80/89.9/89.99999 degrees. Cost if wrong: a few more flops per
  catalogue star, on a path that runs a few hundred times per solve.
Task 7: fix round 1/5 (1 addressed, 0 open; commits 016661d..b06156f). Reviewer measured the
  old formula at 9.66e-6 deg against an expected 0.02778 at dec 89.99999 (~2875x, matching
  the estimate) and confirmed the new 2% tolerance catches it. It also corrected MY premise:
  the local north vector does NOT degenerate at the pole -- |n|^2 = sd^2 + cd^2 = 1
  identically -- so dec = +/-90 is genuinely non-degenerate, not merely handled.
Task 7: complete (commits c174bb7..b06156f, review clean, 67 tests)
Task 8: complete (commits 2e11b88..cb81c92, review clean, 80 tests). TWO real defects found
  in the plan's code:
  (a) **parity() was inverted.** A normal sky image has a NEGATIVE CD determinant, because
      the FITS convention is CDELT1 < 0 (east left with north up) and CDELT2 > 0. The plan
      mapped det<0 to Mirrored. The reviewer verified the correction from the FITS convention
      itself and from the fixture's general-rotation algebra (det = -s^2*m), independently of
      the code -- so it is a real correction, not a sign flipped until green. This would have
      made every solve report the wrong handedness while still round-tripping perfectly.
  (b) sigma clipping death-spiralled on noiseless synthetic input: residuals were float
      round-off (~1e-10 arcsec) so 3*rms shrank faster than points stabilised, clipping 4 of
      40 good points. Floored the clip limit at a thousandth of a pixel -- ~2.5e7x above the
      round-off floor and 10-100x below any real centroid precision, and 9 orders of
      magnitude below the injected outlier, so it cannot mask one.
  Plan amended for (a).

**Ruling M2-F6 (Task 8): defer the solve3 pivot-threshold finding.** The reviewer rated it
Important: the singularity check uses an absolute 1e-12 pivot, not one scaled to matrix
magnitude, so a nearly-but-not-exactly-collinear field could pass the check while being
numerically unstable. Deferring because (i) real star fields are 2D-distributed, so a sliver
configuration surviving extraction is implausible, and (ii) T10's confidence gate rejects on
RMS, which is exactly what an unstable fit inflates -- the failure is caught downstream and
loudly rather than silently. Cost if wrong: a fit that should have returned None instead
returns a high-RMS one that the acceptance gate rejects anyway.
Task 8: minor (deferred): scale_arcsec's doc overclaims exactness under shear (it is a
  geometric mean); Correspondence type alias is unlisted public surface; clipping can in
  principle leave a rank-deficient residual set on a legitimate input.
Task 9: THREE real defects found in the plan's code by the implementer, all confirmed by
  review (which re-derived the rotation independently and reverted fix #2 in isolation to
  prove both the bug and that the test discriminates it):
  (a) rotation sign inverted (iang - cang should be cang - iang);
  (b) correspondence pairing was "first vote wins", letting an alias quad silently override
      the correct star pairing -- the confident-wrong-match hazard this module most needs to
      avoid;
  (c) min_votes gated on raw vote count, so a coincidental overlapping-star cluster from two
      unrelated fields cleared the threshold.
Task 9: fix round 1/5 — the implementer's fix for (c), a greedy pairwise-star-disjoint
  independent set, over-corrected. Reviewer instrumented the real algorithm:
      n=12: raw votes=142, independent=3 -> REJECTED
      n=16: raw votes=332, independent=3 -> REJECTED
      n=20: independent=4 (barely passes)
  and confirmed with the gate bypassed that n=12/n=16 recover the EXACT transform to ten
  significant figures with zero conflicting votes. Requiring disjointness imposes a floor of
  4*min_votes distinct stars, so min_votes:4 silently became "needs >= 16 stars".
  **Ruling M2-F7: measure independent evidence, not a perfect packing.** Keep raw min_votes
  and add min_distinct_stars (default 8, two quads' worth) counting DISTINCT image stars
  across the winning cluster. That still separates "many quads over many stars" from "many
  quads over the same six", without the 4x star-count penalty. Sparse fields matter here --
  the target project's horizon probe solves 61-star frames.
  Cost if wrong: if distinct-star count fails to block the unrelated-fields case, the measure
  is wrong and I asked to be told rather than have the threshold raised until green.
Task 9: minor (deferred): module doc says ~75,000 comparisons; the parity loop doubles it.
Task 9: minor (deferred): used_image/used_cat are O(n) Vec scans; fine at this scale.
Task 9: fix round 2/5 — Ruling M2-F7's distinct-star COUNT was the wrong measure, and the
  implementer reported that rather than tuning the threshold (exactly as asked). Measured:
      true positives: n=12 raw=142 distinct=12 ... n=40 raw=1387 distinct=40
      coincidence:              raw=42  distinct=12
  Count cannot separate them because it ignores position: twelve stars spread across the
  frame and twelve clustered in one corner are the same number.
  **Ruling M2-F8: gate on spatial SPREAD instead.** Compare the RMS radius of the matched
  stars about their centroid to that of all detected stars -- scale-free, position-aware,
  and needing no frame dimensions. A true solution spans the frame; a coincidental cluster
  is localised. Default min_spread_frac 0.25.
  The implementer was told explicitly: if this measure also fails to separate, STOP and say
  so rather than tuning 0.25 -- two failed discriminators in a row would mean the decision
  belongs downstream in the WCS fit's residual, which is a larger design change and my call.
  Cost if wrong: a third round on this gate, or deferring rejection to the confidence stage.
Task 9: fix round 3/5 — spread DID separate (true 1.0000, coincidence 0.5329) but the 0.25
  default was too permissive, and the implementer again reported rather than tuning it.
  **Ruling M2-F9: remove the gate entirely. The matcher proposes; the confidence stage
  disposes.** Two reasons:
   (i) the 1.0000 is a fixture artifact -- every star matches in the synthetic field, so
       matched-spread and all-spread are the same set. A real frame matches a fraction (the
       reference is 118 of 502), so a real true positive sits well below 1.0 and a threshold
       picked in the 0.53-1.0 gap would be calibrated on a number that cannot occur in
       production, then start rejecting real frames.
   (ii) T10 computes a log-odds confidence and T11 gates on it together with the fit RMS.
       That is a principled statistical test. A coincidental cluster of 42 votes over 12
       localised stars yields a poor residual and weak log-odds and gets rejected there.
       A second heuristic gate duplicates that judgement with worse machinery and puts a
       magic constant on the critical path.
  MatchResult now REPORTS distinct_stars and spread_frac instead of thresholding them, so
  the confidence stage can use the evidence. two_unrelated_fields_do_not_match is retargeted
  to assert weaker evidence rather than refusal, since refusal is not this module's job.
  **CARRIED TO T11: the pipeline tests must assert that an unrelated catalogue does not
  SOLVE.** That requirement moved, it did not disappear.
  Cost if wrong: a coincidental match reaches the confidence stage, which is designed to
  reject it; if it ever survives there, the fix belongs in the confidence model.
Task 9: fix round 3/5 (all addressed; commits 9faaf18..c93ef2d). Reviewer instrumented a
  scratch copy and reproduced the reported table exactly (real votes=1371 distinct=40
  spread=1.0000; coincidence votes=42 distinct=12 spread=0.5329), and confirmed the rotation
  sign and majority-vote fixes survived the refactor intact.
Task 9: complete (commits 111ac69..c93ef2d, review clean, 86 tests)
Task 9: minor (deferred): the retargeted test's assertions sit inside `if let Some(c)`, so
  they would pass trivially if that fixture ever stopped producing a candidate. Consistent
  with the ruling, but its teeth depend on Some continuing to come back.
Task 9: minor (deferred): one hardcoded fixture pair gives no general protection against a
  wrong-but-confident match on some other field — deferred to T11 by ruling M2-F9.
Task 10: complete (commits 55d17da..6a3832b, review clean, 94 tests). One real defect in the
  plan's code: the log-odds formula ln(k!) + lambda - k*ln(lambda) is U-SHAPED about the
  Poisson mode, so it scored "far FEWER matches than chance would produce" as strong evidence,
  identically to "far more" -- the plan's own test expected <10 decades and the verbatim
  formula gave 125.27. Fixed by applying it only when matched > lambda. The implementer
  verified in Python before touching Rust and fuzzed 13,500 combinations: zero NaN, zero
  negative log-odds. Reviewer hand-computed 125.27 independently and confirmed Stirling's
  error is ~0.018 decades at k=2, three orders below the threshold.

**Ruling M2-F10 (important — partially reverses M2-F9): log-odds alone does NOT reject the
coincidence, so the reprojection count must.** The reviewer computed it: Task 9's coincidence
gives 12 matched against lambda ~= 0.0115, which is ~31.9 decades -- comfortably over the
12-decade threshold and the 10-match floor. It survives confidence entirely. Worse, the
Poisson model assumes each match is an independent uniform coincidence, while the real
failure is ONE correlated geometric accident, so the model overstates the surprise.

The gap is in T11, which fed `fitres.used` into confidence(). Twelve points against six
parameters nearly interpolates, so a clustered coincidence yields a LOW RMS too -- both gates
pass. Amended T11 to compute `matched` by REPROJECTION instead: push every catalogue star
through the fitted WCS and count how many land on a detected star. A true solution predicts
many stars it was never fitted to; a coincidence predicts none. That is the test that
actually separates them, and spec section 6.5 already called for it -- my T11 code did not.
Cost if wrong: a slightly more expensive verification pass over a few hundred catalogue stars.
Task 10: minor (deferred): the k<=lambda clamp creates a jump of ~0.5*log10(2*pi*lambda)
  decades at the boundary (~1.4 at lambda~94); invisible at the reference rig's lambda << 1.
Task 10: minor (deferred): chance_matches is forced to 0.0 when matched==0 even where lambda
  is well defined, so a caller cannot distinguish "none expected" from "not computed".
Task 11: fix round 1/5 — the reprojection implementation is correct and the check-order fix
  was a real defect in the plan (its own third test was unpassable: hint-missing was checked
  before catalogue-empty, so it always returned FovMismatch or IndexTooShallow, never
  NoQuadMatch). But the reviewer REVERTED `matched` from `reprojected` back to `fitres.used`
  and reran an_unrelated_catalogue_does_not_produce_a_solve: it still passed, with identical
  output (NoQuadMatch, "no consistent transform"). The matcher rejects that fixture before
  fit_tan, the reprojection loop or accept ever run — so the test passes for a reason
  unrelated to what it claims, and the exact regression this task exists to prevent would
  slip through it. The carried T9/T10 requirement was therefore still untested.
  Fix: keep the end-to-end test but assert its reason so the gap is visible, and add
  `reprojection_counts_stars_the_fit_never_saw` asserting stars_matched > fit.used on a
  SUCCESSFUL solve — the property a revert to fitres.used would destroy, since there the two
  could only ever be equal. Implementer told to report the two numbers rather than relax to
  >= if it does not hold, because that would mean the gate is not doing independent work.
Task 11: minor (deferred): an_explicit_hint_overrides_the_header is brief-mandated and weak —
  blank_frame has no stars, so it fails at NoStars long before the hint is consulted.
Task 11: minor (deferred): reprojection is O(catalogue x detected) per solve; fine now.
Task 11: fix round 1/5 (1 addressed, 0 open; commits aad107c..75cf221). Reviewer reverted the
  fix locally and confirmed the new test FAILS (38 vs 38) while passing on the real code
  (40 vs 38), then restored the tree clean. The margin of 2 is small — an artifact of a
  fixture that paints 40 stars and matches nearly all of them. On a real frame the catalogue
  is far larger than the fitted set, so the margin should widen; T12's richer fixtures are
  where that gets exercised.
Task 11: complete (commits 2640745..75cf221, review clean, 101 tests)
Task 11: minor (deferred): tol_px is hardcoded at 2.0 in the reprojection check.
Task 11: minor (deferred): the IndexTooShallow / NoQuadMatch boundary for "too little
  catalogue" is a judgement call; do not assume a single uniform reason code downstream.
Task 12: BLOCKED on first run — 8 of 9 closed-loop tests failed identically (0 matched,
  rms 393.83 px). The implementer traced it rather than weakening anything: 9 of 51
  correspondence pairs were genuinely SWAPPED, two real stars 640x320 px apart taking each
  other's identity. Root cause is the fixture, not the solver: base-2/base-3 Halton aliases
  at index differences of 64, so `catalogue_for` generated EXACT TRANSLATED DUPLICATE
  patterns. match_quads' majority tally mitigates but does not survive ~18% contamination.
  **Ruling M2-F11: fix the sampler, not the matcher.** A real star field contains no exact
  duplicates. Hardening the matcher against one would tune it against a pathology of the
  generator, and worse, would leave the capstone test passing for a reason unrelated to the
  geometry it exists to validate. Replaced with the R2 (plastic-constant) Kronecker sequence,
  which has no small-index aliasing. Also corrected my catalogue_for comment, which claimed
  out-of-range samples were rejected when nothing rejected them (sampling beyond the frame is
  intentional -- those stars exercise the extractor's edge filter).
  Implementer asked to report the RESIDUAL SWAP RATE on a non-aliasing field either way,
  since a real matcher weakness is worth knowing even behind a green tick.
  Cost if wrong: if swaps persist at a material rate on a clean field, the weakness is in
  match_quads after all and the fix moves there -- which the next report will show.
  **This is the closed loop doing precisely what it was written for.**
Task 12: BLOCKED again after M2-F11 — and the second measurement overturned my ruling.
  Swap rate against ground truth went 18% (Halton) -> **77% (R2)**, with raw quad votes
  jumping 8x. The implementer reframed it correctly and declined to pick the replacement
  unilaterally.

**Ruling M2-F12 (corrects M2-F11): the defect is LOW-DISCREPANCY sampling itself, not the
particular sequence.** Halton and R2 both equidistribute by construction, so every star's
local neighbourhood resembles every other's and k-nearest-neighbour quads alias across the
whole field. A real star field is Poisson-scattered -- clumps and voids -- and that
irregularity is exactly what makes its quads distinctive. I reached for "well-distributed"
when the fixture needed "realistic", and here those are opposites. Replaced with a
deterministic hashed (splitmix64) scatter: Poisson-like, no dependency, still exactly
reproducible so a failure repeats.
  Implementer instructed NOT to touch assertions, tolerances or MatchParams, and told that a
  third BLOCKED report would be a useful result rather than a failure: if a Poisson-like
  field still mass-swaps, the weakness is genuinely in match_quads and I want that stated.
  Diagnostic to watch: 59 detected, 55 used, **0 reprojected**, rms 1331 px -- the fit is
  being handed thoroughly wrong correspondences, not noisy ones. If the swap rate drops and
  the tests pass, the diagnosis is confirmed. If the swap rate drops but tests still fail,
  the problem is downstream of matching, and that distinction matters.
  Cost if wrong: a third fixture iteration, or the finding moves into match_quads.
Task 12: third attempt (hashed splitmix64 scatter) — 1 pass -> 3 passes, and the picture
  resolved completely. `quality_metrics_come_back_with_the_solution` and
  `the_pixel_scale_is_taken_from_the_header_when_not_supplied` BOTH PASS, and neither can
  without reaching Outcome::Solved. **The solver works: frames solve and the WCS is
  recovered.** M2-F12's diagnosis was right.
  All six remaining failures are ONE assertion, synthetic.rs:198 "the reported mirrored flag
  must agree" -- which sits AFTER assert_eq!(sol.wcs.parity(), want_parity), and that passes.
  So the fitted WCS carries the CORRECT parity; only Solution.mirrored, copied from
  MatchResult.mirrored, disagrees.
  **Ruling M2-F13: derive Solution.mirrored from the fitted WCS, not the matcher.** The CD
  determinant IS the parity -- ground truth about the optical train, established in T8 and
  verified against the FITS convention. The matcher's flag says which catalogue orientation
  won, expressed against the tangent plane's axis convention, so it is offset by the
  projection's handedness and was never the right thing to report. Two sources for one fact
  is one too many. MatchResult.mirrored stays (it is meaningful inside the matcher and its
  own tests pin it); it just stops being the answer.
  Cost if wrong: if recovers_a_mirrored_wcs still fails once mirrored comes from the WCS,
  the fit genuinely recovers the wrong handedness on mirrored input -- a real bug, and the
  implementer is instructed to report that rather than adjust the assertion.
Task 12: GREEN — 9/9 synthetic pass (commits 75cf221..39e9677, four passes, review clean).
  Final numbers: swap rate 18% (Halton) / 77% (R2) / **1.9% (hashed scatter)**;
  centre recovery **0.000440 arcsec** across 27 traced cases; stars_matched vs fit.used
  55 vs 51 (margin 4, against T11's 2 -- a denser irregular field does widen it, as expected).
  recovers_a_mirrored_wcs passes, confirming the FIT never had a handedness bug; only the
  reported flag did. Reviewer diffed the final test file against the brief and confirmed NO
  assertion, tolerance or sweep case was weakened across any of the four passes.
Task 12: minor (deferred, and a real cost of ruling M2-F13): sol.mirrored and
  sol.wcs.parity() are now definitionally the same quantity, so the cross-check that CAUGHT
  this bug -- the matcher's own handedness belief against the fit's -- is retired by
  construction. A future matcher-level handedness regression would be structurally
  unobservable from Solution. Accepted: one source of truth beats two kept in sync by hand.
Task 12: minor (deferred): catalogue_for's pixel positions are independent of the WCS, so all
  nine tests render the same 90 star positions and differ only in the sky coordinates
  assigned to them -- the WCS maths varies, the extraction geometry does not.
Task 12: minor (deferred): the suite checks the aggregate fitted WCS after sigma clipping,
  not per-correspondence integrity, so a future rise in swap rate could hide until it exceeds
  what clipping absorbs.
Task 13: fix round 1/5 — JSON hazards, escaping, CD=CDELT*PC algebra and exit codes all
  verified correct (reviewer parsed real output through a JSON parser and checked the
  identity symbolically). But one real defect in the plan's code, which the implementer's
  "no defects found" report missed and the reviewer reproduced against the built binary:
      psolve solve --index t.psidx blank.fits --hint 100.0,20.0
      -> {"solved":false,"reason":"CANNOT_READ",...}  exit 1
  `args.iter().find(|a| !a.starts_with("--"))` takes the first non-flag token, but a VALUED
  FLAG'S VALUE is also not "--"-prefixed -- so any valued flag before the positional binds
  its value as the FILE, the real image is never opened, and it reports a clean exit-1
  "not solved". That is exactly the failure the exit-code contract exists to prevent: a
  broken invocation disguised as bad weather, the ASTAP missing-flag trap in a new costume.
  All seven tests missed it because every one put FILE first -- a shared blind spot in the
  fixtures rather than a gap in any single test.
  Fix: a `positional()` scan that skips a valued flag's value, plus tests asserting argument
  order does not change the outcome, for every valued flag.
  Cost if wrong: none; the scan is strictly more correct than the find().
Task 13: fix round 1/5 (1 addressed, 0 open; commits f6c131b..e3ae088). Reviewer reproduced
  the fix against the built binary: the progress line now names the real frame and the
  outcome matches file-first ordering byte for byte. Confirmed VALUED_FLAGS is complete
  against every flag() call site, that a trailing valued flag with no value exits cleanly
  rather than overrunning, and that both new tests fail against the old scan.
  cmd_index::info has the identical naive pattern but is currently safe -- its only flag,
  --verify, consumes no value. Latent trap, flagged not fixed. index build has no positional
  scan at all.
Task 13: complete (commits a0b1c2e..e3ae088, review clean)

ALL 13 M2 TASKS COMPLETE. 232 tests across 12 binaries; clippy clean workspace-wide.
Every single task found at least one real defect in the plan's own code.

## Final whole-branch review + fix wave

Reviewed on the most capable model. Found 1 Critical, 5 Important, and triaged all 30
deferred minors. One fix wave applied all of them plus the promoted BITPIX cast; 232 -> 245
tests.

  CRITICAL: a CFA frame's WCS and field of view disagreed by exactly 2x, silently. `decode`
    bins a BAYERPAT frame, so the fit ran on a half-size grid while the CLI computed the FOV
    from the FILE's dimensions times the BINNED scale. Both frames reported "solved":true;
    the CFA one described a pixel grid absent from the file, so a consumer applying that WCS
    would be wrong by up to a degree at the corners. Fixed: Solution.binned, WCS converted to
    file coordinates, --scale multiplied by the binning factor as the header path already did.
  IMPORTANT: orientation_pa was 180 degrees off (reported the -y axis; the spec's own example
    reports +y). Invisible to every test, because both assertions compared fitted-to-truth
    THROUGH THE SAME FUNCTION, so a constant offset cancelled exactly.
  IMPORTANT: the saturation default reintroduced the inert constant Task 5 removed -- on a
    12- or 14-bit frame the threshold landed at 65470 and could never fire while
    rejected.saturated reported 0 as if it had. Now derived from the clipping signature.
  IMPORTANT: --radius and --cat-limit silently defaulted on garbage or negatives.
  IMPORTANT: catalogue selection ignored the frame; field_height_deg was dead code.
  IMPORTANT: no fixture tested catalogue/image set mismatch, and no CLI test ever reached
    "solved":true -- the 30-argument success JSON had never executed in CI.

**Ruling M2-F14 (final adjudication): do not raise max_quads; make the test tell the truth.**
The fix wave's 50%-mismatch test passed only at max_quads=4000. The re-reviewer measured the
boundary rather than accepting the "sampling reliability" explanation:
    600 (production default) FAILED 3/3 deterministically | 1200 FAILED | 1800 ok | 4000 ok
and confirmed no --max-quads flag exists, so every real invocation uses 600. A test that
passes only at an unreachable configuration is worse than none: it claims a property that
does not hold. Raising the default to ~2000 would mean ~8M four-dimensional comparisons per
solve, blowing the 12-14 ms budget, and whether the limit matters at all is a question for
real frames rather than a synthetic fixture.
So the test now pins the measured envelope in both directions and is named for what it
measures. **This is a real limitation of the shipped configuration, recorded rather than
hidden, and it is precisely what M3's real-frame corpus has to measure against actual sky.**
Cost if wrong: if real frames turn out to need a wider envelope, the default moves and the
test's first assertion flips -- which it explicitly tells the reader to do.
