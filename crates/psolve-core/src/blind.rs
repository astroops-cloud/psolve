//! Turning one quad-code match into a candidate TAN WCS -- the step between
//! "a code matched" (`psolve-index`'s `.psqidx` lookup, outside this crate)
//! and "here is a transform worth verifying" (`fit.rs`, reused wholesale).
//!
//! This crate has no filesystem access and no dependencies (`lib.rs`'s own
//! doc), so it cannot open a `.psqidx` and cannot know what a candidate
//! quad even is beyond the geometry handed to it. A caller (the CLI, in the
//! blind-solve milestone's Task 7) does the code-space lookup with
//! `psolve-index::quad_reader::QuadIndex::candidates`, resolves each
//! candidate's `star_idx` references to sky positions via the paired
//! `.psidx`, and passes THOSE positions in here -- exactly the same
//! boundary the hinted path already draws for the star catalogue itself
//! (`solve.rs`'s module doc: "The catalogue is PASSED IN rather than looked
//! up").
//!
//! ## The four-star correspondence, and why this module does not re-derive it
//!
//! A quad's four stars are matched to a candidate's four stars POSITION FOR
//! POSITION: `image_points[image_quad.idx[k]]` corresponds to
//! `cat_quad_sky[k]`, for every `k` in `0..4`. This module trusts that
//! correspondence rather than re-deriving it (nearest-star matching, say) --
//! that trust is not new here. It is exactly what the existing, working
//! hinted-solve path (`match_::match_quads`) already relies on for its own
//! per-quad-pair correspondences (see its "Gather star correspondences"
//! section), and this module reuses the same premise for a candidate quad
//! that happened to arrive via an index lookup instead of a live
//! `quad::build_quads` call on both sides.
//!
//! **What actually makes that premise true is worth being precise about,
//! because `Quad.idx`'s own doc comment overstates it.** `quad.rs` documents
//! `Quad.idx` as "`[A, B, C, D]` in canonical order" -- but reading
//! `build_quads`, `idx` is stored exactly as assembled BEFORE
//! `quad_code`'s internal canonicalisation runs: `idx = [seed, near_a,
//! near_b, near_c]`, and `quad_code` itself does not return which of its
//! four internal branches (which pair became "A/B", which of C/D printed
//! first) it chose -- only the numeric code. `psolve-cli`'s
//! `cmd_quadindex.rs` independently documents the true mechanism, in
//! `select_tile_quads`'s own doc: `idx[0]` is "set from the ORIGINAL
//! `[seed, ...]` index array before `quad_code`'s own A/B/C/D
//! canonicalisation runs, so it survives intact". So `idx` is SEED-AND-
//! NEAREST-NEIGHBOUR order, not the geometric A/B/C/D role order the doc
//! comment names.
//!
//! That distinction does not break the premise, because the actual
//! mechanism is stronger than "canonical order" would have been anyway:
//! nearest-neighbour RANK order (which physical star is 1st-, 2nd-,
//! 3rd-nearest to a given seed) is preserved by any similarity transform,
//! reflections included -- distances (and therefore their rank order) do
//! not care about rotation, uniform scale, translation, or mirroring. So
//! for the SAME seed star used on both sides, `build_quads`'s deterministic
//! `a<b<c` loop over neighbour ranks enumerates the same rank-combinations
//! in the same order on both sides, and position `k` of one quad's `idx`
//! and position `k` of its true counterpart's `idx` name the same physical
//! star's rank -- which is what position-for-position pairing actually
//! needs. See this module's own tests for a direct check of that premise
//! using this crate's real `quad::build_quads`, and the task report for
//! whether it survives the `.psqidx` write/read round trip specifically
//! (short answer: yes -- `psolve-index`'s `quad_builder.rs`/
//! `quad_format.rs`/`quad_reader.rs` copy `Quad.idx` into `QuadRecord.
//! star_idx` position-for-position and never reorder it, so whatever this
//! premise is worth, the index format does not spend any of it).
//!
//! ## Verifying the pairing here, not just fitting it
//!
//! A geometric fit alone cannot catch every wrong pairing at this stage:
//! with EXACTLY four correspondences, `fit::fit_tan`'s six-parameter affine
//! model is only two equations over-determined, and any internally
//! consistent relabelling of the four points -- including a full mirror
//! reflection -- is still something SOME affine map fits with near-zero
//! residual. A residual check alone would silently accept a wrong-but-
//! self-consistent pairing.
//!
//! So before fitting, this module re-derives the candidate's OWN quad code
//! from `cat_quad_sky` (projected to a local tangent plane about its own
//! first star -- no external tangent point is needed, which matters
//! because blind solving has no hint to supply one) and requires it to
//! reproduce `image_quad.code`, at EITHER parity -- `quad::quad_code` is
//! proven invariant under translation/rotation/scale but NOT under
//! reflection (`quad.rs`'s own `a_mirrored_quad_gives_a_different_code`
//! test), which is exactly the property that makes this check discriminate
//! a genuine match (of either handedness) from a coincidental or corrupted
//! one. Both parities are tried -- not just the direct one -- because a
//! real mirrored optical train is a real, expected case (`quad.rs`'s
//! module doc), and only checking one parity would silently reject half of
//! it rather than silently accept the wrong thing; either failure is a
//! regression to the class M3's agreement run measured at zero.
//! `fit::Wcs::parity()` then reports, correctly, whichever handedness the
//! four accepted correspondences actually have -- this module does not
//! need to (and does not) force one.

use crate::fit::{self, FitResult};
use crate::project;
use crate::quad::{self, Quad};

/// Passed through to `fit::fit_tan`. With exactly four correspondences,
/// sigma-clipping never removes a point -- `fit_tan` refuses to clip below
/// its own four-point floor -- so this constant has no observable effect on
/// this module's output; it exists only because `fit_tan`'s signature takes
/// one and this crate does not invent a second, differently-named "this
/// doesn't matter" constant elsewhere.
const CLIP_SIGMA: f64 = 3.0;

/// Tolerance, in quad-code units, for the parity/mismatch check described
/// in this module's doc.
///
/// **Re-measured against the real G<=16 index for Task 7, twice.** The
/// original 0.15 was chosen and validated only against a synthetic fixture
/// (Task 5's own report flagged it for re-measurement). The first
/// re-measurement (three frames) transcribed each frame's ground-truth WCS
/// by hand from its header's `CRPIX1`/`CRPIX2` -- FITS's 1-based
/// convention, silently mismatched against this crate's 0-based pixel grid
/// (the same crossing `cmd_solve.rs`'s `(nx-1)/2.0` exists to get right,
/// and the third time on this branch the mistake was made). A one-pixel
/// CRPIX error is 3.48" at this rig's 2.46"/px scale -- enough to mislabel
/// genuine matches as false and pick a threshold (0.005) that discarded
/// ~40% of them.
///
/// The corrected re-measurement (`psolve-cli`'s
/// `tests/blind_measure_tolerances.rs`) never transcribes a WCS: it takes
/// `Solution.wcs` from a REAL hinted solve as ground truth, which is
/// already in the crate's own 0-based convention by construction. Run over
/// eleven real frames (eight yielding a usable ground truth; 600 image
/// quads each, every code-space candidate offered across every selected
/// band): 67 genuine-match samples (a true correspondence can be offered by
/// more than one image quad, or more than once per quad, so this is not
/// "one per frame"), against 63,228 false ones.
///
/// | `SHAPE_TOL` | true survive | false survive |
/// |---|---|---|
/// | 0.005 (the first, miscalibrated value) | 55/67 (82%) | 290/63,228 (0.46%) |
/// | 0.01 | 63/67 (94%) | 4,046/63,228 (6.4%) |
/// | 0.015 | 67/67 (100%) | 20,110/63,228 (31.8%) |
/// | 0.02 (`psolve-index`'s own `BLIND_CODE_TOL`) | 67/67 (100%) | 63,153/63,228 (99.9%) |
///
/// At 0.02 the check again provides no discrimination at all -- the
/// code-space lookup's own tolerance is already doing all the filtering by
/// itself at that width. **0.01** keeps 94% of genuine matches (the
/// remaining 6% sit at `code_dist` 0.0106-0.0112, just past a threshold
/// that must also reject the false population growing sharply through this
/// same range) while cutting false survivors by >93%. Confirmed end to end,
/// not just on this table: at 0.01, four of five real frames that solve
/// hinted but previously failed blind now solve blind (see `psolve-cli`'s
/// `solve_blind` doc and the M3 progress ledger for the corpus run), each
/// landing within ~0.01" of its own hinted centre.
///
/// **Recorded honestly, not hidden: still not a large sample** (eleven
/// frames, one rig, one catalogue depth) against, say,
/// `CATALOG_CONCENTRATION_THRESHOLD`'s 300-frame corpus. The direction (a
/// real, measurable separation exists, and 0.02 provides none) and the
/// chosen value's own end-to-end confirmation are solid; a future
/// re-measurement across more rigs/scales/depths may still move 0.01.
const SHAPE_TOL: f64 = 0.01;

/// Maximum fractional disagreement allowed between the fitted WCS's own
/// `scale_arcsec()` and the scale implied directly by `image_quad.diag`
/// (pixels) against the candidate's own widest sky separation (degrees).
/// See `candidate_transform`'s post-fit check for why this catches a class
/// of near-collinear four-point fit that `fit_tan`'s own singular-matrix
/// guard does not.
///
/// **Re-measured against the real G<=16 index for Task 7** -- same
/// corrected methodology as `SHAPE_TOL`'s doc (real hinted-solve ground
/// truth, not a transcribed header; see that doc for why the first attempt
/// at this measurement was wrong). Measured directly against
/// `fit::fit_tan`, not through `candidate_transform` (which would apply
/// whatever is already compiled in and make every threshold at or above
/// the shipped value report "100% survive" by construction). Chained
/// after `SHAPE_TOL = 0.01` -- the number that matters in production, both
/// checks together, not either in isolation -- over the same 67 true /
/// 63,228 false samples:
///
/// | `SCALE_CONSISTENCY_FRAC` (after `SHAPE_TOL = 0.01`) | true survive | false survive |
/// |---|---|---|
/// | 0.003 | 61/67 (91%) | 1,001/63,228 (1.58%) |
/// | **0.005** | 63/67 (94%) | 1,493/63,228 (2.36%) |
///
/// **0.005** keeps exactly as many true matches as `SHAPE_TOL = 0.01` alone
/// admits (this check costs nothing further on top of the shape gate for
/// this sample) while still cutting the false-survivor rate by more than
/// 97% relative to no scale check at all. The old 3% was ~30-100x looser
/// than the real matches' own measured disagreement (0.024%-0.103% in the
/// first, miscalibrated round; the corrected measurement's true-match
/// fractions run up to ~0.9%) and, chained after the also-corrected
/// `SHAPE_TOL`, would have admitted the great majority of what that check
/// alone lets through.
const SCALE_CONSISTENCY_FRAC: f64 = 0.005;

fn code_dist2(a: &[f64; 4], b: &[f64; 4]) -> f64 {
    a.iter().zip(b.iter()).map(|(x, y)| (x - y) * (x - y)).sum()
}

/// Derive a candidate TAN WCS from one image quad matched, by code, against
/// one candidate catalogue quad.
///
/// `image_quad.idx` gives the four detections' positions in `image_points`.
/// `cat_quad_sky` is the candidate's four stars' `(ra, dec)` in DEGREES, in
/// the SAME position order -- see the module doc for why that
/// correspondence is trusted rather than re-derived, and for the parity
/// check this function performs before ever calling `fit::fit_tan`.
///
/// Returns `None` for: an out-of-bounds or degenerate `image_quad`
/// (duplicate or out-of-range indices), a non-finite input, a degenerate
/// four-point configuration on either side (collinear close enough to be
/// numerically singular, or two of the four points coinciding --
/// `quad::quad_code` already refuses to assign a code to either, and this
/// function reuses that refusal rather than re-deriving it), a candidate
/// whose own shape does not reproduce `image_quad.code` at either parity,
/// or (rare, once the shape check has passed) a fit whose own normal matrix
/// is singular. A shape match that is not numerically singular still
/// returns `Some` -- the returned `FitResult`'s `rms_deg`/
/// `max_residual_deg` are the caller's signal for "fits, but not well",
/// exactly as `verify::accept` already uses residuals for the hinted path.
pub fn candidate_transform(
    image_quad: &Quad,
    image_points: &[(f64, f64)],
    cat_quad_sky: [(f64, f64); 4],
) -> Option<FitResult> {
    let idx = image_quad.idx;
    let mut sorted = idx;
    sorted.sort_unstable();
    if sorted.windows(2).any(|w| w[0] == w[1]) {
        return None;
    }
    if idx.iter().any(|&i| i >= image_points.len()) {
        return None;
    }

    let img_pts: [(f64, f64); 4] = [
        image_points[idx[0]],
        image_points[idx[1]],
        image_points[idx[2]],
        image_points[idx[3]],
    ];
    if !img_pts.iter().all(|p| p.0.is_finite() && p.1.is_finite()) {
        return None;
    }
    if !cat_quad_sky.iter().all(|p| p.0.is_finite() && p.1.is_finite()) {
        return None;
    }

    // A degenerate image-side configuration -- collinear-to-numerical-
    // singularity, or two coincident points -- is exactly what
    // `quad_code` already declines to assign a code to.
    quad::quad_code(img_pts[0], img_pts[1], img_pts[2], img_pts[3])?;

    // Reproject the candidate's own four stars to a local tangent plane
    // about its own first star, and recompute its shape. No external
    // tangent point is available or needed -- see the module doc.
    let (ra0, dec0) = cat_quad_sky[0];
    let mut proj = [(0.0, 0.0); 4];
    for (k, slot) in proj.iter_mut().enumerate() {
        *slot = project::radec_to_tangent(cat_quad_sky[k].0, cat_quad_sky[k].1, ra0, dec0)?;
    }
    let cat_code = quad::quad_code(proj[0], proj[1], proj[2], proj[3])?;

    let mirrored_proj: [(f64, f64); 4] =
        [0, 1, 2, 3].map(|k| (-proj[k].0, proj[k].1));
    let cat_code_mirrored =
        quad::quad_code(mirrored_proj[0], mirrored_proj[1], mirrored_proj[2], mirrored_proj[3]);

    let tol2 = SHAPE_TOL * SHAPE_TOL;
    let shape_matches = code_dist2(&cat_code, &image_quad.code) <= tol2
        || cat_code_mirrored.is_some_and(|c| code_dist2(&c, &image_quad.code) <= tol2);
    if !shape_matches {
        return None;
    }

    let pairs: Vec<fit::Correspondence> = (0..4).map(|k| (img_pts[k], cat_quad_sky[k])).collect();
    let result = fit::fit_tan(&pairs, ra0, dec0, CLIP_SIGMA)?;

    // `fit_tan`'s own singular-matrix guard (a 1e-12 pivot floor in
    // `solve3`) is tuned for the hinted path's 10-40+ point fits, where a
    // handful of near-collinear rows are drowned out by the rest. At
    // EXACTLY four correspondences there is no "rest": three of the four
    // points landing close to one line -- measured directly, this happens
    // for a real fraction of the quads a synthetic scatter produces, not
    // just contrived inputs -- leaves the normal matrix ill-conditioned but
    // not always below that floor, and `fit_tan` returns a technically
    // non-singular but numerically unreliable answer (see this module's
    // tests: `scale_arcsec` off by double digits of percent while
    // `rms_deg` still looks small, because the fit residual at the four
    // input points says nothing about how badly the solution extrapolates).
    // A quad's own `diag` (the widest pairwise pixel distance, independent
    // of this fit) implies a scale directly from the matched pair's sky
    // separation; a fit whose own scale disagrees with that by more than a
    // few percent is exactly this failure mode, not ordinary noise.
    let cat_diag_deg = {
        let mut m = 0.0f64;
        for i in 0..4 {
            for j in (i + 1)..4 {
                let d = project::angsep_deg(
                    cat_quad_sky[i].0, cat_quad_sky[i].1,
                    cat_quad_sky[j].0, cat_quad_sky[j].1,
                );
                if d > m {
                    m = d;
                }
            }
        }
        m
    };
    if image_quad.diag <= 0.0 || !cat_diag_deg.is_finite() || cat_diag_deg <= 0.0 {
        return None;
    }
    let expected_scale_arcsec = cat_diag_deg * 3600.0 / image_quad.diag;
    let got_scale_arcsec = result.wcs.scale_arcsec();
    if !got_scale_arcsec.is_finite()
        || (got_scale_arcsec - expected_scale_arcsec).abs() > SCALE_CONSISTENCY_FRAC * expected_scale_arcsec
    {
        return None;
    }

    Some(result)
}

/// `candidate_transform` applied to every entry of `candidates`, keeping
/// only the ones that produced a fit. This is the shape a caller with N
/// code-space matches for ONE image quad actually has (Task 4's measured
/// ~21 candidates per lookup): one image quad, many candidate catalogue
/// quads, most of which -- for anything but the true match and its rare
/// code-space neighbours -- fail the shape check in `candidate_transform`
/// outright rather than producing a close-but-wrong fit.
pub fn candidate_transforms(
    image_quad: &Quad,
    image_points: &[(f64, f64)],
    candidates: &[[(f64, f64); 4]],
) -> Vec<FitResult> {
    candidates
        .iter()
        .filter_map(|&c| candidate_transform(image_quad, image_points, c))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fit::{Parity, Wcs};
    use crate::project::angsep_deg;
    use crate::quad::build_quads;

    /// Same synthetic truth WCS shape `fit.rs`'s own tests use, so numbers
    /// are directly comparable to that module's baseline.
    fn truth(rot_deg: f64, mirrored: bool) -> Wcs {
        let s = 2.4614 / 3600.0;
        let r = rot_deg.to_radians();
        let (c, si) = (r.cos(), r.sin());
        let m = if mirrored { -1.0 } else { 1.0 };
        Wcs {
            crval: [274.689087, -13.810971],
            crpix: [1920.5, 1080.5],
            cd: [[-s * c * m, s * si], [s * si * m, s * c]],
        }
    }

    /// Pixel positions paired with their sky positions under some `Wcs`,
    /// index-for-index.
    type PxSky = (Vec<(f64, f64)>, Vec<(f64, f64)>);
    /// A hand-built quad plus its four pixel positions and their true sky
    /// positions, index-for-index.
    type QuadFixture = (Quad, [(f64, f64); 4], [(f64, f64); 4]);

    /// A deterministic scatter of pixel positions across a realistic frame
    /// and their sky positions under `w` -- index `i` of one corresponds,
    /// by construction, to index `i` of the other.
    fn field(w: &Wcs, n: usize) -> PxSky {
        let mut px = Vec::new();
        let mut sky = Vec::new();
        for i in 0..n {
            let t = i as f64;
            let x = (t * 173.0) % 3800.0 + 20.0;
            let y = (t * 97.0) % 2140.0 + 10.0;
            px.push((x, y));
            sky.push(w.pix_to_radec(x, y));
        }
        (px, sky)
    }

    /// Four well-spread, non-collinear, non-coincident pixel positions and
    /// their true sky positions under `w` -- built by hand rather than via
    /// `build_quads`, so the fixture is exactly the shape the task brief
    /// asks for ("Build the fixture by projecting known sky positions
    /// through a known WCS") with no dependence on which quad a scatter
    /// happens to produce.
    fn hand_built_quad(w: &Wcs) -> QuadFixture {
        // A realistic quad footprint -- comparable in size to what
        // `quad::build_quads` actually forms from nearest neighbours (a
        // few hundred pixels across), not a span approaching the whole
        // frame. This matters for the accuracy this fixture can promise:
        // `candidate_transform` fits a LOCAL affine about the candidate's
        // own first star, so its accuracy away from that neighbourhood
        // degrades with gnomonic projection curvature -- a real effect,
        // not a bug (see
        // `a_locally_fit_wcs_is_not_expected_to_extrapolate_to_sub_arcsecond_across_the_whole_frame`
        // below for that measured directly). A few-hundred-pixel quad
        // keeps this fixture inside the regime the "sub-arcsecond" claim is
        // actually about.
        let img_pts = [(1700.0, 950.0), (2050.0, 1180.0), (1780.0, 1210.0), (1950.0, 990.0)];
        let cat_sky: [(f64, f64); 4] = img_pts.map(|(x, y)| w.pix_to_radec(x, y));
        let code = quad::quad_code(img_pts[0], img_pts[1], img_pts[2], img_pts[3])
            .expect("a well-spread quad must produce a code");
        let mut diag = 0.0f64;
        for i in 0..4 {
            for j in (i + 1)..4 {
                let dx = img_pts[j].0 - img_pts[i].0;
                let dy = img_pts[j].1 - img_pts[i].1;
                diag = diag.max((dx * dx + dy * dy).sqrt());
            }
        }
        (Quad { code, idx: [0, 1, 2, 3], diag }, img_pts, cat_sky)
    }

    #[test]
    fn recovers_a_known_wcs_from_the_true_matching_quad() {
        let w = truth(31.0, false);
        let (q, img_pts, cat_sky) = hand_built_quad(&w);

        let fit = candidate_transform(&q, &img_pts, cat_sky).expect("the true match must fit");
        assert_eq!(fit.used, 4);

        // "Recovers the known WCS to sub-arcsecond": the fitted transform,
        // applied back to the very pixels it was derived from, reproduces
        // the KNOWN true sky positions to sub-arcsecond -- `fit_tan`'s own
        // residual statistics say exactly this, over all four points at
        // once.
        assert!(fit.rms_deg * 3600.0 < 0.05, "rms {} arcsec -- not sub-arcsecond", fit.rms_deg * 3600.0);
        assert!(
            fit.max_residual_deg * 3600.0 < 0.1,
            "max residual {} arcsec -- not sub-arcsecond",
            fit.max_residual_deg * 3600.0
        );
        for k in 0..4 {
            let (ra, dec) = fit.wcs.pix_to_radec(img_pts[k].0, img_pts[k].1);
            let sep = angsep_deg(ra, dec, cat_sky[k].0, cat_sky[k].1) * 3600.0;
            assert!(sep < 0.1, "star {k} off by {sep} arcsec");
        }

        assert!(
            (fit.wcs.scale_arcsec() - w.scale_arcsec()).abs() < 1e-2,
            "scale {} vs truth {}",
            fit.wcs.scale_arcsec(),
            w.scale_arcsec()
        );
        let d = (fit.wcs.orientation_deg() - w.orientation_deg()).abs() % 360.0;
        assert!(d < 0.05 || (360.0 - d) < 0.05, "orientation drift {d} deg");
        assert_eq!(fit.wcs.parity(), Parity::Normal);
    }

    #[test]
    fn recovers_known_wcs_across_rotations_for_quads_that_fit() {
        // Not every quad a scatter produces is well-conditioned enough to
        // fit (see `the_scale_consistency_check_catches_the_quads_fit_tan_gets_wrong`
        // below for the specific failure mode, and for the measurement showing
        // this check is what refuses them) -- what matters here is that
        // EVERY quad that DOES fit recovers the true scale, across a sweep
        // of rotations, and that a solid majority of the field's quads fit
        // at all (this scatter is not pathological).
        for rot in [0.0, 45.0, 122.6, 300.0] {
            let w = truth(rot, false);
            let (px, sky) = field(&w, 40);
            let quads = build_quads(&px, 6, 200);
            assert!(quads.len() >= 5, "rot {rot}: too few quads to exercise this ({})", quads.len());
            let mut fitted = 0usize;
            for q in &quads {
                let cat_sky = [sky[q.idx[0]], sky[q.idx[1]], sky[q.idx[2]], sky[q.idx[3]]];
                if let Some(fit) = candidate_transform(q, &px, cat_sky) {
                    fitted += 1;
                    assert!(
                        (fit.wcs.scale_arcsec() - w.scale_arcsec()).abs() < 0.02,
                        "rot {rot}, quad {:?}: scale {} vs {}",
                        q.idx,
                        fit.wcs.scale_arcsec(),
                        w.scale_arcsec()
                    );
                }
            }
            assert!(
                fitted * 2 >= quads.len(),
                "rot {rot}: too few of {} quads fit ({fitted})",
                quads.len()
            );
        }
    }

    #[test]
    fn a_quad_whose_declared_diag_disagrees_with_its_points_is_rejected() {
        // This fixture was introduced as a "near-collinear quad" test and
        // its doc claimed `fit_tan` returns a wildly inaccurate scale at
        // four points when three of them are collinear. **Re-measured
        // 2026-08-23: that claim was false.** The rejection came entirely
        // from the fixture declaring `diag: 595.0` for points whose true
        // maximum pairwise distance is 396.676 -- a 1.5x inflation. Given
        // the honest diag the SAME points are ACCEPTED, with a scale
        // accurate to -0.042%. See
        // `a_near_collinear_quad_is_never_confidently_wrong` below for what
        // `fit_tan` actually does with collinear input.
        //
        // The property this fixture does pin is still worth having, so it
        // is kept under an honest name: a `Quad` whose declared `diag`
        // disagrees with the positions its `idx` resolves to is refused
        // rather than turned into a WCS. That is the production symptom of
        // a `.psqidx` resolved against the wrong `.psidx` -- the exact
        // failure `QuadIndex::open`'s fingerprint check exists to prevent,
        // caught here a second time by `SCALE_CONSISTENCY_FRAC` in case it
        // ever is not.
        let w = truth(0.0, false);
        let img_pts = [(20.0, 10.0), (193.0, 107.0), (366.0, 204.0), (199.0, 101.0)];
        let cat_sky: [(f64, f64); 4] = img_pts.map(|(x, y)| w.pix_to_radec(x, y));
        let code = quad::quad_code(img_pts[0], img_pts[1], img_pts[2], img_pts[3]).unwrap();

        let honest_diag = 396.676_f64;
        let inflated = Quad { code, idx: [0, 1, 2, 3], diag: honest_diag * 1.5 };
        assert!(
            candidate_transform(&inflated, &img_pts, cat_sky).is_none(),
            "a quad whose declared diag is 1.5x its points' real extent must not \
produce a confident answer -- that is what a mismatched star index looks like"
        );

        // The control that makes the assertion above mean something: with
        // the honest diag, this very quad IS accepted. Without this the
        // test would pass against a `candidate_transform` that rejects
        // everything.
        let honest = Quad { code, idx: [0, 1, 2, 3], diag: honest_diag };
        assert!(
            candidate_transform(&honest, &img_pts, cat_sky).is_some(),
            "control: the same points with an honest diag must be accepted, \
otherwise the rejection above is not attributable to the diag"
        );
    }

    #[test]
    fn the_scale_consistency_check_catches_the_quads_fit_tan_gets_wrong() {
        // **The test that pins `SCALE_CONSISTENCY_FRAC`.** Measured over
        // this module's own `field(&truth(0.0,false), 40)` scatter, 200
        // quads, the whole point of the constant shows up as a 1,500x
        // difference in the worst answer that escapes:
        //
        // | `SCALE_CONSISTENCY_FRAC` | accepted | worst accepted scale error |
        // |---|---|---|
        // | **0.005** (shipped) | 196/200 | **0.0415%** |
        // | 1000.0 (guard removed) | 200/200 | **62.2550%** |
        //
        // So four of the 200 quads are ones `fit::fit_tan` fits badly --
        // double digits of percent off while its own `rms_deg` still looks
        // small, exactly as this module's doc says -- and this check is what
        // stops them becoming a candidate WCS. An earlier version of this
        // test swept one near-collinear geometry through
        // `candidate_transform` and asserted only "never confidently wrong";
        // it passed with the guard set to 1000.0, i.e. it pinned nothing.
        // Sweeping `candidate_transform` cannot find these: it refuses
        // extreme collinearity earlier, at `SHAPE_TOL` or at `fit_tan`
        // returning `None` (measured: accepted at 1e-2 px lateral offset,
        // refused at 1e-3 and below). The badly-fit quads come from an
        // ordinary scatter, not from a contrived degenerate one -- which is
        // why the fixture is the same helper the accuracy tests use.
        let w = truth(0.0, false);
        let (px, sky) = field(&w, 40);
        let quads = quad::build_quads(&px, 6, 200);
        assert!(
            quads.len() >= 100,
            "fixture drifted: expected a couple of hundred quads, got {}",
            quads.len()
        );

        let mut accepted = 0usize;
        let mut worst = 0.0_f64;
        for q in &quads {
            let cat_sky: [(f64, f64); 4] =
                [sky[q.idx[0]], sky[q.idx[1]], sky[q.idx[2]], sky[q.idx[3]]];
            if let Some(fit) = candidate_transform(q, &px, cat_sky) {
                accepted += 1;
                let err = (fit.wcs.scale_arcsec() / w.scale_arcsec() - 1.0).abs();
                if err > worst {
                    worst = err;
                }
                assert!(
                    err < 0.005,
                    "quad {:?} was accepted with a scale {:.4}% off ({:.6} vs truth \
{:.6}) -- SCALE_CONSISTENCY_FRAC is what is supposed to stop this",
                    q.idx,
                    err * 100.0,
                    fit.wcs.scale_arcsec(),
                    w.scale_arcsec()
                );
            }
        }

        // The guard must actually REFUSE something on this fixture,
        // otherwise the assertion above is vacuous and the test would keep
        // passing with the check removed -- which is precisely how its
        // predecessor failed.
        assert!(
            accepted < quads.len(),
            "the scale-consistency check refused none of {} quads, so this test \
proves nothing about it",
            quads.len()
        );
        // ...and it must not be refusing wholesale, or the "guard fires"
        // signal above would be indistinguishable from a broken pipeline.
        assert!(
            accepted * 10 >= quads.len() * 9,
            "only {accepted} of {} quads survived -- the check has gone from \
catching outliers to rejecting ordinary quads",
            quads.len()
        );
    }

    #[test]
    fn a_locally_fit_wcs_is_not_expected_to_extrapolate_to_sub_arcsecond_across_the_whole_frame() {
        // Documents a real, understood property rather than a bug: a
        // candidate transform is fit from ONE quad's four (nearby) stars,
        // about a tangent point at one of them. Evaluated back at those
        // same four pixels it is sub-arcsecond (the other accuracy tests in
        // this module measure that directly) -- but evaluated far away, on
        // the OTHER side of a multi-degree field, gnomonic projection
        // curvature between the fit's own tangent point and the frame's
        // true centre shows up as a real, non-arcsecond-scale offset. That
        // is expected of a single local quad's fit and is exactly why the
        // full pipeline (Task 6/7) refines with many quads spread across
        // the frame rather than trusting one candidate's extrapolation.
        let w = truth(31.0, false);
        let (q, img_pts, cat_sky) = hand_built_quad(&w);
        let fit = candidate_transform(&q, &img_pts, cat_sky).expect("the true match must fit");

        let c_truth = w.pix_to_radec(3800.0, 2140.0);
        let c_fit = fit.wcs.pix_to_radec(3800.0, 2140.0);
        let sep_arcsec = angsep_deg(c_truth.0, c_truth.1, c_fit.0, c_fit.1) * 3600.0;
        assert!(
            sep_arcsec > 0.1,
            "expected a measurable far-field offset from a single local quad's fit, got {sep_arcsec} arcsec"
        );
        // But it must still be a SMALL, bounded offset, not a diverging or
        // nonsensical one -- gnomonic curvature over a few degrees, not a
        // wrong transform.
        assert!(sep_arcsec < 30.0, "far-field offset {sep_arcsec} arcsec is implausibly large");
    }

    #[test]
    fn a_genuinely_mirrored_optical_train_is_recovered_and_reported_as_mirrored() {
        // The scenario Task 5's brief calls out by name: mirrored frames
        // are real (an odd number of reflections in the optical train),
        // and this must not be silently forced to Normal parity.
        let w = truth(20.0, true);
        assert_eq!(w.parity(), Parity::Mirrored, "fixture sanity check");
        let (q, img_pts, cat_sky) = hand_built_quad(&w);

        let fit = candidate_transform(&q, &img_pts, cat_sky).expect("a genuine mirrored match must fit");
        assert_eq!(
            fit.wcs.parity(),
            Parity::Mirrored,
            "mirrored truth must not be silently reported as Normal parity"
        );
        assert!(fit.rms_deg * 3600.0 < 0.05, "rms {} arcsec -- not sub-arcsecond", fit.rms_deg * 3600.0);
    }

    #[test]
    fn both_parities_recover_the_same_scale_and_accuracy() {
        // Neither handedness gets preferential treatment -- accuracy for a
        // genuine match should not depend on which parity it happens to be.
        for mirrored in [false, true] {
            let w = truth(64.0, mirrored);
            let (q, img_pts, cat_sky) = hand_built_quad(&w);
            let fit = candidate_transform(&q, &img_pts, cat_sky)
                .unwrap_or_else(|| panic!("mirrored={mirrored} must fit"));
            assert_eq!(fit.wcs.parity(), w.parity());
            assert!((fit.wcs.scale_arcsec() - w.scale_arcsec()).abs() < 1e-2);
            assert!(fit.rms_deg * 3600.0 < 0.05, "mirrored={mirrored}: rms {} arcsec", fit.rms_deg * 3600.0);
        }
    }

    #[test]
    fn an_unrelated_quad_is_rejected_rather_than_fit_plausibly() {
        // A mismatched pair -- no consistent transform relates these four
        // catalogue positions to these four image positions at all.
        let w = truth(10.0, false);
        let (px, _sky) = field(&w, 30);
        let quads = build_quads(&px, 6, 100);
        let q = quads.first().expect("must produce a quad");

        // An unrelated field, in the same sky region so it is a fair
        // comparison rather than one rejected purely by RA/Dec bounds.
        let unrelated_sky: Vec<(f64, f64)> = (0..30)
            .map(|i| {
                let t = i as f64;
                let dxi = ((t * 0.911) % 1.2) - 0.6;
                let deta = ((t * 0.577) % 0.7) - 0.35;
                crate::project::tangent_to_radec(dxi, deta, w.crval[0], w.crval[1])
            })
            .collect();
        let bad_cat_sky = [
            unrelated_sky[q.idx[0] % unrelated_sky.len()],
            unrelated_sky[q.idx[1] % unrelated_sky.len()],
            unrelated_sky[q.idx[2] % unrelated_sky.len()],
            unrelated_sky[q.idx[3] % unrelated_sky.len()],
        ];

        match candidate_transform(q, &px, bad_cat_sky) {
            None => {}
            Some(fit) => assert!(
                fit.rms_deg * 3600.0 > 1.0,
                "an unrelated quad must not fit plausibly -- rms {} arcsec",
                fit.rms_deg * 3600.0
            ),
        }
    }

    #[test]
    fn a_mismatched_pair_is_rejected_by_many_independent_unrelated_candidates() {
        // Same idea as the single-pair test above, but sweeping many
        // independent false candidates so this is not just one lucky (or
        // unlucky) seed -- the shape check in `candidate_transform` should
        // reject essentially all of them.
        let w = truth(0.0, false);
        let (px, _sky) = field(&w, 30);
        let quads = build_quads(&px, 6, 100);
        let q = quads.first().unwrap();

        let mut accepted_with_small_rms = 0;
        for seed in 0..20u64 {
            let base = seed as f64 * 0.031;
            let unrelated_sky: Vec<(f64, f64)> = (0..30)
                .map(|i| {
                    let t = i as f64 + base * 100.0;
                    let dxi = ((t * 0.733 + base) % 1.2) - 0.6;
                    let deta = ((t * 0.419 + base) % 0.7) - 0.35;
                    crate::project::tangent_to_radec(dxi, deta, w.crval[0], w.crval[1])
                })
                .collect();
            let bad_cat_sky = [
                unrelated_sky[q.idx[0] % unrelated_sky.len()],
                unrelated_sky[q.idx[1] % unrelated_sky.len()],
                unrelated_sky[q.idx[2] % unrelated_sky.len()],
                unrelated_sky[q.idx[3] % unrelated_sky.len()],
            ];
            if let Some(fit) = candidate_transform(q, &px, bad_cat_sky) {
                if fit.rms_deg * 3600.0 < 1.0 {
                    accepted_with_small_rms += 1;
                }
            }
        }
        assert_eq!(
            accepted_with_small_rms, 0,
            "no unrelated candidate should be accepted with sub-arcsecond residual"
        );
    }

    #[test]
    fn coincident_image_points_return_none_rather_than_panicking() {
        let w = truth(0.0, false);
        let (_, sky) = field(&w, 4);
        let cat_sky = [sky[0], sky[1], sky[2], sky[3]];
        let degenerate_px = [(100.0, 100.0), (100.0, 100.0), (50.0, 60.0), (70.0, 20.0)];
        let q = Quad { code: [0.0, 0.5, 0.5, 1.0], idx: [0, 1, 2, 3], diag: 0.0 };
        assert!(candidate_transform(&q, &degenerate_px, cat_sky).is_none());
    }

    #[test]
    fn a_zero_area_collinear_quad_returns_none_rather_than_panicking() {
        let w = truth(0.0, false);
        let (_, sky) = field(&w, 4);
        let cat_sky = [sky[0], sky[1], sky[2], sky[3]];
        let collinear_px = [(0.0, 0.0), (10.0, 0.0), (20.0, 0.0), (30.0, 0.0)];
        let q = Quad { code: [0.0, 0.5, 0.5, 1.0], idx: [0, 1, 2, 3], diag: 30.0 };
        assert!(candidate_transform(&q, &collinear_px, cat_sky).is_none());
    }

    #[test]
    fn coincident_catalogue_points_return_none_rather_than_panicking() {
        let w = truth(0.0, false);
        let (px, sky) = field(&w, 30);
        let quads = build_quads(&px, 6, 100);
        let q = quads.first().unwrap();
        let s0 = sky[q.idx[0]];
        let cat_sky = [s0, s0, sky[q.idx[2]], sky[q.idx[3]]];
        assert!(candidate_transform(q, &px, cat_sky).is_none());
    }

    #[test]
    fn out_of_bounds_indices_return_none_rather_than_panicking() {
        let w = truth(0.0, false);
        let (px, sky) = field(&w, 4);
        let cat_sky = [sky[0], sky[1], sky[2], sky[3]];
        let q = Quad { code: [0.0, 0.5, 0.5, 1.0], idx: [0, 1, 2, 99], diag: 1.0 };
        assert!(candidate_transform(&q, &px, cat_sky).is_none());
    }

    #[test]
    fn duplicate_indices_return_none_rather_than_panicking() {
        let w = truth(0.0, false);
        let (px, sky) = field(&w, 4);
        let cat_sky = [sky[0], sky[1], sky[2], sky[3]];
        let q = Quad { code: [0.0, 0.5, 0.5, 1.0], idx: [0, 1, 1, 3], diag: 1.0 };
        assert!(candidate_transform(&q, &px, cat_sky).is_none());
    }

    #[test]
    fn non_finite_inputs_return_none_rather_than_panicking() {
        let w = truth(0.0, false);
        let (px, sky) = field(&w, 4);
        let q = Quad { code: [0.0, 0.5, 0.5, 1.0], idx: [0, 1, 2, 3], diag: 1.0 };
        let mut bad_sky = [sky[0], sky[1], sky[2], sky[3]];
        bad_sky[1].0 = f64::NAN;
        assert!(candidate_transform(&q, &px, bad_sky).is_none());

        let mut bad_px = px.clone();
        bad_px[0].1 = f64::INFINITY;
        assert!(candidate_transform(&q, &bad_px, [sky[0], sky[1], sky[2], sky[3]]).is_none());
    }

    #[test]
    fn the_function_is_deterministic() {
        let w = truth(50.0, false);
        let (px, sky) = field(&w, 30);
        let quads = build_quads(&px, 6, 100);
        let q = quads.first().unwrap();
        let cat_sky = [sky[q.idx[0]], sky[q.idx[1]], sky[q.idx[2]], sky[q.idx[3]]];
        let a = candidate_transform(q, &px, cat_sky);
        let b = candidate_transform(q, &px, cat_sky);
        assert_eq!(a, b, "identical inputs must produce identical output");
    }

    #[test]
    fn candidate_transforms_collects_only_the_fits_that_succeeded() {
        let w = truth(15.0, false);
        let (px, sky) = field(&w, 30);
        let quads = build_quads(&px, 6, 100);
        let q = quads.first().unwrap();
        let good = [sky[q.idx[0]], sky[q.idx[1]], sky[q.idx[2]], sky[q.idx[3]]];
        let bad = [good[0], good[0], good[2], good[3]]; // coincident -> rejected
        let out = candidate_transforms(q, &px, &[bad, good, bad]);
        assert_eq!(out.len(), 1, "only the genuine match should survive");
        assert_eq!(out[0].used, 4);
    }

}
