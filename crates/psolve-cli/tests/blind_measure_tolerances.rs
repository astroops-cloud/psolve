//! A re-derivation, not a passing test: measures the true-vs-false
//! separation `blind.rs`'s `SHAPE_TOL`/`SCALE_CONSISTENCY_FRAC` actually
//! achieve against the real G<=16 index and a broad sample of real frames.
//!
//! `#[ignore]`d like `blind_candidates_real_index.rs`'s full-band
//! benchmark: needs the real multi-GB `.psidx`/`.psqidx` pair and a
//! release build to run in a reasonable time. Run explicitly with:
//!   cargo test --release -p psolve-cli --test blind_measure_tolerances -- --ignored --nocapture
//!
//! ## Ground truth: a REAL hinted solve, not a transcribed header
//!
//! Fix round 1 found the first version of this file transcribed each
//! frame's real ASTAP `CRVAL`/`CRPIX`/`CD` by hand from its header --
//! `CRPIX1`/`CRPIX2` in FITS's 1-based convention, silently mismatched
//! against `PreparedFrame::image_points()`'s 0-based grid (the same
//! convention crossing `cmd_solve.rs`'s own `(nx-1)/2.0` exists to get
//! right, and the third time this exact mistake has been made on this
//! branch). A one-pixel CRPIX error is 3.48" at this rig's 2.46"/px scale,
//! just over the 3" `is_true` threshold below -- enough to silently
//! mislabel genuine matches as false and invert the whole measurement.
//!
//! This version never transcribes a WCS by hand. For every candidate
//! frame it runs a REAL hinted solve (`psolve_core::solve::solve_prepared`,
//! the exact function `psolve solve --hint`/header-hint dispatch calls) via
//! the same real G<=16 star index the blind search itself uses, and takes
//! `Solution.wcs` as ground truth -- already in the exact 0-based
//! convention every downstream comparison needs, by construction, because
//! it came out of the same pipeline. This also broadens the corpus beyond
//! "frames that happen to carry a pre-recorded ASTAP solution in their own
//! header" to any real frame with a usable pointing hint.
//!
//! ## Independent of the compiled-in constants
//!
//! Fix round 1 also found the shape/scale-distance sweep below used to
//! call the real `blind::candidate_transform`, which applies whatever
//! `SHAPE_TOL`/`SCALE_CONSISTENCY_FRAC` are compiled into THIS run --
//! so `false_scale_frac` only ever contained candidates that had ALREADY
//! passed those same gates, and every sweep threshold at or above the
//! compiled value reported "100% of what's left survives" by construction,
//! not because the check does not discriminate at those thresholds. This
//! version recomputes the shape code and the fitted scale directly
//! (`quad::quad_code`/`fit::fit_tan`, not `blind::candidate_transform`), so
//! every row of the sweep table below is independent of whatever is
//! compiled into `blind.rs` at the time this runs.
#![allow(dead_code)]

use psolve_core::fit;
use psolve_core::project::{self, angsep_deg};
use psolve_core::quad;
use psolve_core::solve::{CatalogStar, Outcome, SolveOptions};
use psolve_index::quad_reader::QuadIndex;
use psolve_index::reader::Index;
use std::path::Path;

/// A broad sample: eleven real frames across ten different targets (not
/// eleven exposures of the same field), all from the same rig (243mm focal
/// length, 2.9um pixels, unbinned -- confirmed via each frame's own
/// `FOCALLEN`/`XPIXSZ`/`XBINNING`, not assumed), so a plate-scale-dependent
/// effect would not be hidden by frame-to-frame scale variation. Deliberately
/// includes `ngc6526`'s first frame, named directly in fix round 1's review
/// as a boundary case (its only genuine code-space match sits at
/// `code_dist ~= 0.00604`).
const FRAMES: &[&str] = &[
    "eagle/lights/H/2026-07-29_22-47-02_H_120.00s_100g_1x1_0001_-10.00.fits",
    "eagle/lights/H/2026-08-11_22-26-00_H_120.00s_100g_1x1_0001_-9.90.fits",
    "prawn/lights/S/2026-07-28_23-12-39_S_300.00s_100g_1x1_0023_-9.90.fits",
    "ngc6526/lights/H/2026-08-15_19-13-40_H_120.00s_100g_1x1_0001_-10.00.fits",
    "catspaw/lights/H/2026-07-29_01-56-44_H_120.00s_100g_1x1_0001_-9.90.fits",
    "corona/lights/H/2026-08-11_19-47-16_H_120.00s_100g_1x1_0001_-10.00.fits",
    "dragonsegg/lights/H/2026-07-27_18-52-30_H_300.00s_100g_1x1_0007_-10.00.fits",
    "helix/lights/H/2026-07-29_02-24-35_H_120.00s_100g_1x1_0001_-9.90.fits",
    "ic1274/lights/H/2026-07-29_02-15-38_H_120.00s_100g_1x1_0001_-9.90.fits",
    "ic1284/lights/H/2026-07-29_23-49-49_H_120.00s_100g_1x1_0001_-9.90.fits",
    "ic2872/lights/H/2026-07-29_19-21-16_H_120.00s_100g_1x1_0001_-9.90.fits",
];

/// Catalogue search radius for the hinted ground-truth solve -- generous
/// against this rig's own ~2.617x1.472 deg field (half-diagonal + margin
/// ~1.5 deg, matching `cmd_solve::default_radius_for`'s own formula), not
/// re-derived here since this file cannot reach that `pub(crate)` helper
/// (`psolve-cli` is bin-only).
const TRUTH_RADIUS_DEG: f64 = 2.0;
const TRUTH_CAT_LIMIT: usize = 3000;
const CODE_TOL: f64 = 0.02;
/// How close (arcsec) a candidate's four resolved sky positions must land
/// to the truth WCS's own reprojection of the same four image pixels to be
/// counted as the genuine match, position for position.
const TRUE_MATCH_TOL_ARCSEC: f64 = 3.0;

struct FrameStats {
    path: &'static str,
    image_quads: usize,
    truth_found: bool,
    n_true_offered: usize,
    /// (shape_dist2, is_true) for every candidate whose shape code could be
    /// derived.
    shape: Vec<(f64, bool)>,
    /// (scale_frac, is_true) for every candidate whose fit converged --
    /// UNGATED by any compiled threshold (see module doc).
    scale: Vec<(f64, bool)>,
    /// (shape_dist2, scale_frac, is_true) for every candidate where BOTH
    /// were computable -- lets the sweep below answer the number that
    /// actually matters in production: how many candidates survive the
    /// shape check THEN the scale check, chained, at a given pair of
    /// thresholds. `shape`/`scale` above answer each check in isolation,
    /// which is useful for picking an individual threshold but not for
    /// predicting the combined false-survival rate.
    combined: Vec<(f64, f64, bool)>,
}

fn code_dist2(a: &[f64; 4], b: &[f64; 4]) -> f64 {
    a.iter().zip(b.iter()).map(|(x, y)| (x - y) * (x - y)).sum()
}

/// Re-derive a candidate's own shape code and its distance to the image
/// quad's code, at whichever parity is closer -- the SAME computation
/// `blind::candidate_transform` does internally, reimplemented here so this
/// measurement does not depend on whatever `SHAPE_TOL` happens to be
/// compiled in.
fn shape_dist2(image_code: &[f64; 4], cat_sky: [(f64, f64); 4]) -> Option<f64> {
    let (ra0, dec0) = cat_sky[0];
    let mut proj = [(0.0, 0.0); 4];
    for (k, slot) in proj.iter_mut().enumerate() {
        *slot = project::radec_to_tangent(cat_sky[k].0, cat_sky[k].1, ra0, dec0)?;
    }
    let cat_code = quad::quad_code(proj[0], proj[1], proj[2], proj[3])?;
    let mirrored: [(f64, f64); 4] = [0, 1, 2, 3].map(|k| (-proj[k].0, proj[k].1));
    let cat_code_mirrored = quad::quad_code(mirrored[0], mirrored[1], mirrored[2], mirrored[3]);
    let d_direct = code_dist2(&cat_code, image_code);
    let d_mirror = cat_code_mirrored.map(|c| code_dist2(&c, image_code)).unwrap_or(f64::INFINITY);
    Some(d_direct.min(d_mirror))
}

/// Fractional scale disagreement between a direct 4-point fit and the
/// diagonal-ratio implied scale -- `candidate_transform`'s own post-fit
/// check, reimplemented directly against `fit::fit_tan` (not through
/// `candidate_transform`, which would apply whatever is compiled in).
fn scale_frac(image_quad: &quad::Quad, image_points: &[(f64, f64)], cat_sky: [(f64, f64); 4]) -> Option<f64> {
    let idx = image_quad.idx;
    let img_pts: [(f64, f64); 4] =
        [image_points[idx[0]], image_points[idx[1]], image_points[idx[2]], image_points[idx[3]]];
    let pairs: Vec<fit::Correspondence> = (0..4).map(|k| (img_pts[k], cat_sky[k])).collect();
    let (ra0, dec0) = cat_sky[0];
    let result = fit::fit_tan(&pairs, ra0, dec0, 3.0)?;
    let mut cat_diag_deg = 0.0f64;
    for i in 0..4 {
        for j in (i + 1)..4 {
            let d = angsep_deg(cat_sky[i].0, cat_sky[i].1, cat_sky[j].0, cat_sky[j].1);
            if d > cat_diag_deg {
                cat_diag_deg = d;
            }
        }
    }
    if image_quad.diag <= 0.0 || cat_diag_deg <= 0.0 {
        return None;
    }
    let expected = cat_diag_deg * 3600.0 / image_quad.diag;
    let got = result.wcs.scale_arcsec();
    if expected <= 0.0 || !got.is_finite() {
        return None;
    }
    Some((got - expected).abs() / expected)
}

/// Same band-selection window `cmd_solve::select_bands` uses (a factor of
/// 2 either way of the quad's own implied diagonal, all bands if the scale
/// is unknown or nothing qualifies) -- reimplemented locally since that
/// function is `pub(crate)` to the bin-only `psolve-cli` and this
/// integration test cannot reach it.
fn select_bands(n_bands: usize, band_scales_deg: &[f32], diag_px: f64, scale_arcsec: f64) -> Vec<usize> {
    let diag_deg = diag_px * scale_arcsec / 3600.0;
    let mut bands: Vec<usize> = (0..n_bands)
        .filter(|&b| {
            let bs = band_scales_deg[b] as f64;
            bs > 0.0 && (0.5..=2.0).contains(&(diag_deg / bs))
        })
        .collect();
    if bands.is_empty() {
        bands = (0..n_bands).collect();
    }
    bands
}

fn measure_frame(
    path: &'static str,
    star_index: &Index,
    quad_index: &QuadIndex,
) -> Option<FrameStats> {
    let full = Path::new(env!("HOME")).join("astroops/library").join(path);
    if !full.exists() {
        eprintln!("skipping {path}: not present");
        return None;
    }
    let bytes = std::fs::read(&full).ok()?;
    let hdr = psolve_core::fits::FitsHeader::parse(&bytes).ok()?;
    let hint = psolve_core::fits::hint_radec(&hdr)?;

    let opts = SolveOptions { hint: Some(hint), catalog_epoch: star_index.header().epoch, ..Default::default() };
    let prepared = psolve_core::solve::prepare(&bytes, &opts).ok()?;
    let catalog_recs = star_index.brightest_in_disc(hint.0, hint.1, TRUTH_RADIUS_DEG, TRUTH_CAT_LIMIT);
    let catalog: Vec<CatalogStar> = catalog_recs
        .iter()
        .map(|r| CatalogStar { ra: r.ra_deg(), dec: r.dec_deg(), mag: r.mag(), pmra: r.pmra_mas_yr(), pmdec: r.pmdec_mas_yr() })
        .collect();
    let truth_outcome = psolve_core::solve::solve_prepared(&prepared, &catalog, &opts);
    let Outcome::Solved(truth) = truth_outcome else {
        eprintln!("{path}: hinted ground-truth solve FAILED -- excluded from the corpus");
        return Some(FrameStats {
            path,
            image_quads: 0,
            truth_found: false,
            n_true_offered: 0,
            shape: Vec::new(),
            scale: Vec::new(),
            combined: Vec::new(),
        });
    };
    // All frames in this corpus are confirmed XBINNING=1, so the FILE grid
    // `truth.wcs` is expressed in equals the DECODE grid `image_points()`
    // is in -- no binning conversion needed. Asserted, not assumed.
    assert_eq!(truth.binned, 1, "{path}: corpus assumption (unbinned) violated");

    let image_pts = prepared.image_points();
    let iq = quad::build_quads(&image_pts, 6, opts.max_quads);
    let scale_arcsec = prepared.header_scale_arcsec().unwrap_or_else(|| panic!("{path}: rig always carries FOCALLEN/XPIXSZ"));

    let n_bands = quad_index.header().n_bands as usize;
    let band_scales = quad_index.header().band_scales_deg();

    let mut shape = Vec::new();
    let mut scale = Vec::new();
    let mut combined = Vec::new();
    let mut n_true_offered = 0usize;

    for q in &iq {
        let img_pts4: Vec<(f64, f64)> = q.idx.iter().map(|&i| image_pts[i]).collect();
        let true_sky: Vec<(f64, f64)> = img_pts4.iter().map(|&(x, y)| truth.wcs.pix_to_radec(x, y)).collect();

        for band in select_bands(n_bands, &band_scales, q.diag, scale_arcsec) {
            for c in quad_index.candidates(q.code, CODE_TOL, band) {
                let mut sky = [(0.0, 0.0); 4];
                let mut ok = true;
                for (slot, &gi) in sky.iter_mut().zip(c.star_idx.iter()) {
                    match star_index.star_at(gi) {
                        Some(r) => *slot = (r.ra_deg(), r.dec_deg()),
                        None => ok = false,
                    }
                }
                if !ok {
                    continue;
                }
                let is_true = (0..4).all(|k| {
                    angsep_deg(sky[k].0, sky[k].1, true_sky[k].0, true_sky[k].1) * 3600.0 < TRUE_MATCH_TOL_ARCSEC
                });
                if is_true {
                    n_true_offered += 1;
                }
                let d2 = shape_dist2(&q.code, sky);
                if let Some(d2) = d2 {
                    shape.push((d2, is_true));
                }
                let frac = scale_frac(q, &image_pts, sky);
                if let Some(f) = frac {
                    scale.push((f, is_true));
                }
                if let (Some(d2), Some(f)) = (d2, frac) {
                    combined.push((d2, f, is_true));
                }
            }
        }
    }

    Some(FrameStats { path, image_quads: iq.len(), truth_found: true, n_true_offered, shape, scale, combined })
}

#[test]
#[ignore]
fn measure_shape_and_scale_separation() {
    let psidx = Path::new(concat!(env!("HOME"), "/astroops/data/gaia-dr3-g16-dec45-nside64.psidx"));
    let psqidx = Path::new(concat!(env!("HOME"), "/astroops/data/gaia-dr3-g16-dec45-nside64.psqidx"));
    if !psidx.exists() || !psqidx.exists() {
        eprintln!("skipping: fixtures absent");
        return;
    }
    let star_index = Index::open(psidx).unwrap();
    let quad_index = QuadIndex::open(psqidx, &star_index).unwrap();

    let mut all_shape: Vec<(f64, bool)> = Vec::new();
    let mut all_scale: Vec<(f64, bool)> = Vec::new();
    let mut all_combined: Vec<(f64, f64, bool)> = Vec::new();
    let mut frames_with_truth = 0usize;
    let mut frames_with_a_true_candidate = 0usize;

    for &path in FRAMES {
        let Some(stats) = measure_frame(path, &star_index, &quad_index) else { continue };
        eprintln!(
            "{}: image_quads={} truth_found={} true_offered={}",
            stats.path, stats.image_quads, stats.truth_found, stats.n_true_offered
        );
        if stats.truth_found {
            frames_with_truth += 1;
        }
        if stats.n_true_offered > 0 {
            frames_with_a_true_candidate += 1;
        }
        all_shape.extend(stats.shape);
        all_scale.extend(stats.scale);
        all_combined.extend(stats.combined);
    }

    eprintln!("--- summary ---");
    eprintln!("frames with a ground-truth hinted solve: {frames_with_truth}/{}", FRAMES.len());
    eprintln!("frames offering at least one genuine code-space candidate: {frames_with_a_true_candidate}");

    let n_true = all_shape.iter().filter(|(_, t)| *t).count();
    let n_false = all_shape.len() - n_true;
    eprintln!("total shape-code samples: {n_true} true, {n_false} false");
    for tol in [0.001, 0.002, 0.003, 0.004, 0.005, 0.006, 0.008, 0.01, 0.015, 0.02, 0.03, 0.05, 0.1, 0.15] {
        let t2 = tol * tol;
        let true_survive = all_shape.iter().filter(|(d, is_t)| *is_t && *d <= t2).count();
        let false_survive = all_shape.iter().filter(|(d, is_t)| !*is_t && *d <= t2).count();
        eprintln!(
            "SHAPE_TOL={tol:<6}: true survivors {true_survive}/{n_true}   false survivors {false_survive}/{n_false} ({:.2}%)",
            100.0 * false_survive as f64 / n_false.max(1) as f64
        );
    }

    let n_true_s = all_scale.iter().filter(|(_, t)| *t).count();
    let n_false_s = all_scale.len() - n_true_s;
    eprintln!("total scale-frac samples (ungated): {n_true_s} true, {n_false_s} false");
    for tol in [0.0005, 0.001, 0.002, 0.003, 0.005, 0.01, 0.02, 0.03, 0.05, 0.1] {
        let true_survive = all_scale.iter().filter(|(f, is_t)| *is_t && *f <= tol).count();
        let false_survive = all_scale.iter().filter(|(f, is_t)| !*is_t && *f <= tol).count();
        eprintln!(
            "SCALE_FRAC={tol:<7}: true survivors {true_survive}/{n_true_s}   false survivors {false_survive}/{n_false_s} ({:.2}%)",
            100.0 * false_survive as f64 / n_false_s.max(1) as f64
        );
    }

    let mut true_shape_sorted: Vec<f64> = all_shape.iter().filter(|(_, t)| *t).map(|(d, _)| d.sqrt()).collect();
    true_shape_sorted.sort_by(f64::total_cmp);
    eprintln!("true shape_dist (linear) sorted: {true_shape_sorted:?}");
    let mut true_scale_sorted: Vec<f64> = all_scale.iter().filter(|(_, t)| *t).map(|(f, _)| *f).collect();
    true_scale_sorted.sort_by(f64::total_cmp);
    eprintln!("true scale_frac sorted: {true_scale_sorted:?}");

    // The number that actually matters in production: chained shape THEN
    // scale, at candidate (SHAPE_TOL, SCALE_FRAC) pairs -- not either check
    // in isolation.
    let n_true_c = all_combined.iter().filter(|(_, _, t)| *t).count();
    let n_false_c = all_combined.len() - n_true_c;
    eprintln!("--- chained (shape THEN scale), n_true={n_true_c} n_false={n_false_c} ---");
    for (shape_tol, scale_frac_tol) in [
        (0.005, 0.003),
        (0.01, 0.003),
        (0.01, 0.005),
        (0.015, 0.005),
        (0.02, 0.005),
    ] {
        let t2 = shape_tol * shape_tol;
        let true_survive =
            all_combined.iter().filter(|(d, f, is_t)| *is_t && *d <= t2 && *f <= scale_frac_tol).count();
        let false_survive =
            all_combined.iter().filter(|(d, f, is_t)| !*is_t && *d <= t2 && *f <= scale_frac_tol).count();
        eprintln!(
            "SHAPE_TOL={shape_tol} SCALE_FRAC={scale_frac_tol}: true survivors {true_survive}/{n_true_c}   \
false survivors {false_survive}/{n_false_c} ({:.3}%)",
            100.0 * false_survive as f64 / n_false_c.max(1) as f64
        );
    }
}
