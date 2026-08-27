//! Quad matching and transform voting.
//!
//! Brute force by design. At the measured scale -- ~377 image quads against
//! ~200 catalogue quads, each pair tried at both parities -- this is
//! ~150,000 four-dimensional distance computations, well under a
//! millisecond. A KD-tree here would be a data structure whose failure mode
//! is silent, bought for nothing.
//!
//! A single agreeing quad code is weak evidence: codes are four numbers and
//! collisions happen. The true solution produces MANY matches that all imply
//! the same scale and rotation, while false matches imply scattered ones.
//! Binning on (scale, rotation) and taking the winner is what turns weak
//! evidence into strong.

use crate::quad::Quad;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Correspondence {
    pub image: (f64, f64),
    pub sky: (f64, f64),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MatchParams {
    /// Euclidean tolerance on the 4-vector code.
    pub code_tol: f64,
    /// Fractional tolerance on implied scale against `expected_scale`.
    pub scale_tol: f64,
    /// Degrees per pixel, when known from the optics. This is the sharpest
    /// pruning tool available and the main reason a hinted solve is fast.
    pub expected_scale: Option<f64>,
    /// Minimum number of raw agreeing (image quad, catalogue quad, parity)
    /// votes the winning bin must contain. This is a cheap sanity floor --
    /// not a confidence test. Whether the evidence is actually strong
    /// enough to trust is a statistical question (how likely this many
    /// matches were to arise by chance, combined with the fit's residual),
    /// and that judgement belongs to the confidence stage downstream, not
    /// here: see `MatchResult::distinct_stars` and
    /// `MatchResult::spread_frac`, which this module reports but does not
    /// threshold.
    pub min_votes: usize,
}

impl Default for MatchParams {
    fn default() -> Self {
        MatchParams {
            code_tol: 0.02,
            scale_tol: 0.05,
            expected_scale: None,
            min_votes: 4,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MatchResult {
    pub pairs: Vec<Correspondence>,
    /// The raw number of agreeing (image quad, catalogue quad, parity) votes
    /// in the winning bin -- NOT deduplicated by star or quad identity, and
    /// NOT a measure of how independent or how spatially spread that
    /// evidence is (see `distinct_stars` and `spread_frac` for that). A
    /// single star reused across many quads can inflate this number without
    /// adding independent evidence; treat it as a diagnostic, not a
    /// confidence score.
    pub votes: usize,
    pub scale: f64,
    pub rotation_deg: f64,
    pub mirrored: bool,
    pub quads_compared: usize,
    /// Distinct image stars participating in the winning cluster.
    pub distinct_stars: usize,
    /// Spatial spread of the matched stars as a fraction of the spread of all
    /// detected stars. Near 1.0 when the match spans the frame; well below it
    /// when the agreeing quads are localised, which is what a coincidental
    /// cluster looks like. Reported rather than thresholded here: whether the
    /// evidence is sufficient is decided by the confidence stage, which does it
    /// statistically instead of with a constant.
    pub spread_frac: f64,
}

fn code_dist2(a: &[f64; 4], b: &[f64; 4]) -> f64 {
    let mut s = 0.0;
    for i in 0..4 {
        let d = a[i] - b[i];
        s += d * d;
    }
    s
}

/// RMS distance of a point set from its own centroid.
fn spread(points: &[(f64, f64)]) -> f64 {
    if points.len() < 2 {
        return 0.0;
    }
    let n = points.len() as f64;
    let cx = points.iter().map(|p| p.0).sum::<f64>() / n;
    let cy = points.iter().map(|p| p.1).sum::<f64>() / n;
    (points.iter().map(|p| (p.0 - cx).powi(2) + (p.1 - cy).powi(2)).sum::<f64>() / n).sqrt()
}

/// A vote: one quad match's implied similarity transform.
struct Vote {
    scale: f64,
    rot: f64,
    mirrored: bool,
    iq: usize,
    cq: usize,
}

pub fn match_quads(
    image_pts: &[(f64, f64)],
    image_quads: &[Quad],
    cat_pts: &[(f64, f64)],
    cat_sky: &[(f64, f64)],
    cat_quads: &[Quad],
    p: &MatchParams,
) -> Option<MatchResult> {
    if image_quads.is_empty() || cat_quads.is_empty() || image_pts.is_empty() {
        return None;
    }
    // Catalogue quads are matched in both handednesses. Mirrored frames are
    // real -- an odd number of reflections in the optical train -- and a solver
    // that assumes one handedness fails half the equipment it meets.
    let mirrored_pts: Vec<(f64, f64)> = cat_pts.iter().map(|&(x, y)| (-x, y)).collect();

    let tol2 = p.code_tol * p.code_tol;
    let mut votes: Vec<Vote> = Vec::new();
    let mut compared = 0usize;

    for (ii, iq) in image_quads.iter().enumerate() {
        for (ci, cq) in cat_quads.iter().enumerate() {
            for mirrored in [false, true] {
                compared += 1;
                // A mirrored catalogue quad's code is recomputed from mirrored
                // points, so compare against the appropriate one.
                let cat_code = if mirrored {
                    match crate::quad::quad_code(
                        mirrored_pts[cq.idx[0]],
                        mirrored_pts[cq.idx[1]],
                        mirrored_pts[cq.idx[2]],
                        mirrored_pts[cq.idx[3]],
                    ) {
                        Some(c) => c,
                        None => continue,
                    }
                } else {
                    cq.code
                };
                if code_dist2(&iq.code, &cat_code) > tol2 {
                    continue;
                }
                if iq.diag <= 0.0 || cq.diag <= 0.0 {
                    continue;
                }
                // Implied scale: catalogue degrees per image pixel.
                let scale = cq.diag / iq.diag;
                if let Some(exp) = p.expected_scale {
                    if exp > 0.0 && ((scale - exp).abs() / exp) > p.scale_tol {
                        continue;
                    }
                }
                // Implied rotation, from the two most-separated pair in each.
                let ia = image_pts[iq.idx[0]];
                let ib = image_pts[iq.idx[1]];
                let src = if mirrored { &mirrored_pts } else { cat_pts };
                let ca = src[cq.idx[0]];
                let cb = src[cq.idx[1]];
                let iang = (ib.1 - ia.1).atan2(ib.0 - ia.0);
                let cang = (cb.1 - ca.1).atan2(cb.0 - ca.0);
                // The image is produced by rotating the catalogue by rot_deg, so
                // the image bearing is the catalogue bearing MINUS rot_deg
                // (see the transform in `to_pixels` in the tests below). The
                // recovered rotation is therefore cang - iang, not iang - cang.
                let rot = (cang - iang).to_degrees().rem_euclid(360.0);
                votes.push(Vote { scale, rot, mirrored, iq: ii, cq: ci });
            }
        }
    }

    if votes.is_empty() {
        return None;
    }

    // Bin on (log scale, rotation, parity). Coarse enough that real matches
    // share a bin despite noise, fine enough that chance matches do not.
    const ROT_BIN: f64 = 5.0;
    const SCALE_BIN: f64 = 0.02; // in ln space, ~2%
    let key = |v: &Vote| -> (i64, i64, bool) {
        (
            (v.scale.ln() / SCALE_BIN).round() as i64,
            (v.rot / ROT_BIN).round() as i64,
            v.mirrored,
        )
    };

    let mut bins: Vec<((i64, i64, bool), Vec<usize>)> = Vec::new();
    for (i, v) in votes.iter().enumerate() {
        let k = key(v);
        match bins.iter_mut().find(|(bk, _)| *bk == k) {
            Some((_, list)) => list.push(i),
            None => bins.push((k, vec![i])),
        }
    }
    // Rotation bins wrap: a cluster straddling 359/0 would otherwise split.
    // Merge each bin with its neighbours before choosing a winner.
    let mut best: Option<(usize, Vec<usize>)> = None;
    for (k, list) in &bins {
        let mut merged = list.clone();
        for (k2, l2) in &bins {
            if k2 == k {
                continue;
            }
            let drot = (k.1 - k2.1).abs();
            let wrapped = drot.min(((360.0 / ROT_BIN) as i64) - drot);
            if k.2 == k2.2 && (k.0 - k2.0).abs() <= 1 && wrapped <= 1 {
                merged.extend_from_slice(l2);
            }
        }
        merged.sort_unstable();
        merged.dedup();
        if best.as_ref().is_none_or(|(n, _)| merged.len() > *n) {
            best = Some((merged.len(), merged));
        }
    }

    let (_, winners) = best?;
    if winners.len() < p.min_votes {
        return None;
    }
    // Report -- don't judge -- how independent and how spatially spread this
    // evidence is. A coincidental cluster (the same few stars recombined
    // into many quads, confined to one patch of the field) looks different
    // from a real match on both counts, but deciding how much difference is
    // enough is a statistical question for the confidence stage downstream,
    // not a constant threaded through this module.
    let mut used: Vec<usize> = winners
        .iter()
        .flat_map(|&vi| image_quads[votes[vi].iq].idx)
        .collect();
    used.sort_unstable();
    used.dedup();
    let distinct_stars = used.len();
    let matched_pts: Vec<(f64, f64)> = used.iter().map(|&i| image_pts[i]).collect();
    let all_spread = spread(image_pts);
    let spread_frac = if all_spread > 0.0 { spread(&matched_pts) / all_spread } else { 0.0 };

    // Gather star correspondences from every quad in the winning cluster.
    //
    // A single quad is weak evidence for WHICH stars correspond, not just for
    // the transform: this deterministic test field contains exact
    // translated-duplicate quads (same shape, different stars), so a
    // code-collision quad can legitimately land in the same (scale,
    // rotation, parity) bin as the true match while linking the wrong
    // stars. Taking "whichever vote is processed first" for a given image
    // star would let one such collision silently override the majority, so
    // instead tally every candidate sky star per image star across the whole
    // winning cluster and keep the one most quads agree on -- the same
    // "many weak votes beat one confident guess" idea the bin selection
    // above already relies on.
    let mut scale_sum = 0.0;
    let mut rot_x = 0.0;
    let mut rot_y = 0.0;
    let mut mirrored = false;
    let mut tally: Vec<(usize, Vec<(usize, u32)>)> = Vec::new();
    for &vi in &winners {
        let v = &votes[vi];
        scale_sum += v.scale;
        let r = v.rot.to_radians();
        rot_x += r.cos();
        rot_y += r.sin();
        mirrored = v.mirrored;
        let iq = &image_quads[v.iq];
        let cq = &cat_quads[v.cq];
        for k in 0..4 {
            let image_idx = iq.idx[k];
            let sky_idx = cq.idx[k];
            let entry = match tally.iter_mut().position(|(i, _)| *i == image_idx) {
                Some(pos) => &mut tally[pos],
                None => {
                    tally.push((image_idx, Vec::new()));
                    let last = tally.len() - 1;
                    &mut tally[last]
                }
            };
            match entry.1.iter_mut().find(|(s, _)| *s == sky_idx) {
                Some((_, c)) => *c += 1,
                None => entry.1.push((sky_idx, 1)),
            }
        }
    }

    // For each image star, keep the sky star the largest number of quads in
    // the winning cluster agreed on. Ties keep the earliest (lowest vote
    // index) candidate, for determinism.
    let mut pairs: Vec<Correspondence> = Vec::new();
    for (image_idx, candidates) in &tally {
        let mut best: Option<(usize, u32)> = None;
        for &(sky_idx, count) in candidates {
            if best.is_none_or(|(_, bc)| count > bc) {
                best = Some((sky_idx, count));
            }
        }
        if let Some((sky_idx, _)) = best {
            pairs.push(Correspondence {
                image: image_pts[*image_idx],
                sky: cat_sky[sky_idx],
            });
        }
    }

    let n = winners.len() as f64;
    Some(MatchResult {
        pairs,
        votes: winners.len(),
        scale: scale_sum / n,
        // Circular mean, so a cluster straddling 0/360 averages correctly.
        rotation_deg: rot_y.atan2(rot_x).to_degrees().rem_euclid(360.0),
        mirrored,
        quads_compared: compared,
        distinct_stars,
        spread_frac,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quad::build_quads;

    /// A deterministic star field in tangent-plane degrees.
    fn sky_field(n: usize) -> Vec<(f64, f64)> {
        let mut v = Vec::new();
        for i in 0..n {
            let t = i as f64;
            v.push((((t * 0.137) % 1.2) - 0.6, ((t * 0.291) % 0.7) - 0.35));
        }
        v
    }

    /// Map tangent-plane degrees to pixels with a known scale/rotation/offset.
    fn to_pixels(
        sky: &[(f64, f64)],
        scale_deg_per_px: f64,
        rot_deg: f64,
        mirrored: bool,
    ) -> Vec<(f64, f64)> {
        let r = rot_deg.to_radians();
        let (c, s) = (r.cos(), r.sin());
        sky.iter()
            .map(|&(xi, eta)| {
                let x0 = if mirrored { -xi } else { xi };
                let px = (x0 * c + eta * s) / scale_deg_per_px + 1920.0;
                let py = (-x0 * s + eta * c) / scale_deg_per_px + 1080.0;
                (px, py)
            })
            .collect()
    }

    const SCALE: f64 = 2.4614 / 3600.0;

    fn run(rot: f64, mirrored: bool, p: &MatchParams) -> Option<MatchResult> {
        let sky = sky_field(40);
        let img = to_pixels(&sky, SCALE, rot, mirrored);
        let iq = build_quads(&img, 6, 400);
        let cq = build_quads(&sky, 6, 400);
        // Catalogue "sky" positions stand in for RA/Dec here; the matcher only
        // carries them through, so the identity is fine for this test.
        match_quads(&img, &iq, &sky, &sky, &cq, p)
    }

    #[test]
    fn matches_an_unrotated_field_and_recovers_the_scale() {
        let r = run(0.0, false, &MatchParams::default()).expect("should match");
        assert!(r.votes >= 4, "only {} votes", r.votes);
        assert!(
            (r.scale - SCALE).abs() / SCALE < 0.02,
            "scale {} vs {SCALE}",
            r.scale
        );
        assert!(!r.mirrored);
        assert!(r.pairs.len() >= 8, "only {} correspondences", r.pairs.len());
    }

    #[test]
    fn matches_a_rotated_field_and_recovers_the_rotation() {
        for rot in [30.0f64, 122.6, 250.0] {
            let r = run(rot, false, &MatchParams::default())
                .unwrap_or_else(|| panic!("rotation {rot} should match"));
            let d = (r.rotation_deg - rot).abs() % 360.0;
            assert!(d < 6.0 || (360.0 - d) < 6.0, "rot {rot}: recovered {}", r.rotation_deg);
        }
    }

    #[test]
    fn a_mirrored_field_is_matched_and_flagged() {
        let r = run(40.0, true, &MatchParams::default()).expect("mirrored fields must match");
        assert!(r.mirrored, "parity must be reported, not silently absorbed");
    }

    #[test]
    fn a_small_but_unambiguous_field_still_matches() {
        // 12 stars carrying a zero-conflict, exactly-correct transform must
        // not be rejected for being small. A disjoint-packing gate rejected
        // this despite 142 concurring votes.
        let sky = sky_field(12);
        let img = to_pixels(&sky, SCALE, 17.0, false);
        let iq = build_quads(&img, 6, 400);
        let cq = build_quads(&sky, 6, 400);
        let r = match_quads(&img, &iq, &sky, &sky, &cq, &MatchParams::default())
            .expect("a small field with an exact transform must match");
        assert!((r.scale - SCALE).abs() / SCALE < 0.02);
        let d = (r.rotation_deg - 17.0).abs() % 360.0;
        assert!(d < 6.0 || (360.0 - d) < 6.0, "rotation {}", r.rotation_deg);
    }

    #[test]
    fn correspondences_actually_correspond() {
        // Every returned pair must be the same physical star -- this is what
        // the fit depends on, and a matcher that returns plausible-but-wrong
        // pairs produces a confident wrong WCS.
        let sky = sky_field(40);
        let img = to_pixels(&sky, SCALE, 0.0, false);
        let iq = build_quads(&img, 6, 400);
        let cq = build_quads(&sky, 6, 400);
        let r = match_quads(&img, &iq, &sky, &sky, &cq, &MatchParams::default()).unwrap();
        for c in &r.pairs {
            let i = img
                .iter()
                .position(|p| (p.0 - c.image.0).abs() < 1e-9 && (p.1 - c.image.1).abs() < 1e-9)
                .expect("image point must come from the input list");
            assert!(
                (sky[i].0 - c.sky.0).abs() < 1e-9 && (sky[i].1 - c.sky.1).abs() < 1e-9,
                "pair {c:?} links image star {i} to the wrong sky star"
            );
        }
    }

    #[test]
    fn the_scale_prior_rejects_a_field_at_the_wrong_scale() {
        // Knowing the pixel scale exactly is the sharpest pruning tool we have.
        let p = MatchParams {
            expected_scale: Some(SCALE * 4.0),
            scale_tol: 0.05,
            ..MatchParams::default()
        };
        assert!(run(0.0, false, &p).is_none(), "a 4x scale error must not match");
    }

    #[test]
    fn the_scale_prior_accepts_the_right_scale() {
        let p = MatchParams {
            expected_scale: Some(SCALE),
            scale_tol: 0.05,
            ..MatchParams::default()
        };
        assert!(run(0.0, false, &p).is_some());
    }

    #[test]
    fn unrelated_fields_produce_much_weaker_evidence_than_a_real_match() {
        // The matcher proposes; the confidence stage disposes. What this module
        // must guarantee is that a coincidental cluster is distinguishable --
        // far fewer agreeing votes, and localised rather than spanning the
        // frame -- not that it refuses to report one.
        let a = sky_field(40);
        let b: Vec<(f64, f64)> = (0..40)
            .map(|i| { let t = i as f64; (((t * 0.911) % 1.2) - 0.6, ((t * 0.577) % 0.7) - 0.35) })
            .collect();
        let img = to_pixels(&a, SCALE, 0.0, false);
        let iq = build_quads(&img, 6, 400);

        let real = match_quads(&img, &iq, &a, &a, &build_quads(&a, 6, 400), &MatchParams::default())
            .expect("the matching field must match");
        let coincidence = match_quads(&img, &iq, &b, &b, &build_quads(&b, 6, 400), &MatchParams::default());

        if let Some(c) = coincidence {
            assert!(c.votes * 3 < real.votes,
                "a coincidence should carry far less support: {} vs {}", c.votes, real.votes);
            assert!(c.spread_frac < 0.75 * real.spread_frac,
                "a coincidence should be localised: {} vs {}", c.spread_frac, real.spread_frac);
        }
    }

    #[test]
    fn empty_inputs_return_none_rather_than_panicking() {
        let p = MatchParams::default();
        assert!(match_quads(&[], &[], &[], &[], &[], &p).is_none());
    }

    #[test]
    fn quads_compared_is_reported_for_diagnostics() {
        let r = run(0.0, false, &MatchParams::default()).unwrap();
        assert!(r.quads_compared > 0, "the failure report needs this number");
    }
}
