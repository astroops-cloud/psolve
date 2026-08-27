//! Matching on star PAIRS, for frames whose quads do not match.
//!
//! ## Why a second matcher exists at all
//!
//! A quad code matches only when all four of its stars survive on both sides
//! AND the nearest-neighbour graph that selected them is preserved. The
//! first condition goes as completeness to the fourth power; the second is
//! broken by a single interloper displacing a true neighbour out of the top
//! four. Neither binds a pair: two stars either both survive or they do not,
//! and their separation does not depend on what else was detected nearby.
//!
//! Measured on the corpus: across the 106 frames that reached
//! `NoQuadMatch` with the quad budget already raised, the median usable star
//! count is 26 and 77 of them are under 50. Quads are starved at those
//! counts. Pairs are not.
//!
//! ## Why this is a retry and not the default
//!
//! Not because quads are better -- pairs solve strictly more here -- but
//! because pairs are far more expensive in the tail. On 60 frames that BOTH
//! matchers solve, pair matching ran a better median (0.035 s against
//! 0.058 s) and a much worse p90 (4.82 s against 0.16 s), for 35.6 s of
//! total wall clock against 4.4 s. Extrapolated across the corpus that is
//! roughly 100 minutes against 13.
//!
//! Running it only where quads found nothing buys the extra frames for the
//! cost of the frames that need them. It also makes the change
//! regression-free by construction rather than by measurement: a frame whose
//! quads match never reaches this module, so its answer cannot move.
//!
//! ## Why hypotheses and not votes
//!
//! Two designs were tried first and both failed; they are recorded because
//! they are why this one is shaped as it is.
//!
//! 1. **Accumulate votes in a correspondence grid.** Each agreement between
//!    an image separation and a catalogue separation votes for the star
//!    correspondences it implies. On a real frame this cast **268 million
//!    votes across 157 thousand cells** -- a noise floor near 1700 per cell
//!    against a true signal of tens. A pair carries ONE number where a quad
//!    code carries four, and one number is not enough to accumulate at these
//!    star counts.
//! 2. **The same, restricted to a dominant rotation.** There is no rotation
//!    to find: the winning window of the rotation histogram held 0.38% of
//!    its weight where a flat histogram holds 0.35%. The peak was noise.
//!
//! What works is not accumulating at all. Two correspondences plus the
//! implied scale fix a similarity transform outright, so every agreement can
//! be treated as a HYPOTHESIS and tested against every other star in the
//! frame. A true hypothesis places many stars on catalogue counterparts; a
//! coincidental one places none. Verification is what makes a
//! low-information primitive usable -- the same reason a quad solver
//! reprojects rather than trusting its code match.
//!
//! Nothing here decides whether a solve is good enough. These
//! correspondences go to `fit` and then to `verify`, which judges them
//! statistically against chance. `min_inliers` below is a floor on what is
//! worth fitting, not a confidence test.

/// One image-to-sky correspondence produced by a verified hypothesis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pairing {
    pub image: (f64, f64),
    pub sky: (f64, f64),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PairMatchParams {
    /// Degrees per pixel. Required: an image separation cannot be compared
    /// with a catalogue separation without it, and the method rests on that
    /// comparison. This is why the retry is hinted-only -- see
    /// [`PairMatchParams::scale_deg_per_px`]'s use in `solve`.
    pub scale_deg_per_px: f64,
    /// Positional tolerance in pixels. Used both to decide two separations
    /// agree and to decide a transformed star landed on a catalogue star.
    pub tol_px: f64,
    /// Fractional tolerance added to the above, covering plate-scale error.
    /// Kept tight: a loose value widens every match window and multiplies
    /// the hypotheses tested, which is the whole cost of this module.
    pub scale_tol: f64,
    /// Brightest image stars considered; the list arrives brightest-first.
    pub max_image_stars: usize,
    /// Catalogue stars per image star. Magnitude matching: the counterparts
    /// of a frame's brightest N detections lie among the catalogue's
    /// brightest few N. The disc is fetched several times deeper than that,
    /// and every star past it can only produce a hypothesis that is wrong.
    pub cat_per_image: usize,
    pub min_cat_stars: usize,
    pub max_cat_stars: usize,
    /// Image stars a hypothesis must place on catalogue stars to be offered
    /// to the fit at all.
    pub min_inliers: usize,
    /// Stop at this many inliers. A count this high has no coincidental
    /// explanation, so continuing only costs time.
    pub early_exit_inliers: usize,
    /// Hard ceiling on hypotheses tested, so a dense frame degrades to a
    /// slow answer rather than an unbounded one.
    pub max_hypotheses: u64,
    /// Give up once this many hypotheses have been tested without any of
    /// them reaching [`PairMatchParams::min_inliers`].
    ///
    /// A frame that will never be offered to the fit should not be charged
    /// the whole sweep, and without this rule it was: a `NoQuadMatch` cost
    /// ~70 ms before this matcher existed and 3,286 ms median after it, with
    /// those frames accounting for 63% of a head-to-head run's entire wall
    /// time.
    ///
    /// **1,000,000 is measured.** Across the 84 corpus frames this matcher
    /// rescues, EVERY one reached `min_inliers` within **91,128** hypotheses
    /// -- median 264, p90 9,116. Frames that never solve run past four
    /// million and never reach it at all. The threshold sits 11x above the
    /// worst observed rescue, so the rule discriminates on a margin rather
    /// than on a boundary.
    ///
    /// Note what it does NOT gate on: the margin between the best hypothesis
    /// and the runner-up. 43 of those 84 rescues had `inliers == runner_up`
    /// and solved correctly anyway -- including one 0.067 deg from the
    /// commanded pointing where ASTAP was 10.5 deg out. Whether the evidence
    /// suffices is `verify`'s judgement, not this module's.
    ///
    /// `u64::MAX` disables the rule.
    pub abort_without_promise: u64,
}

impl Default for PairMatchParams {
    fn default() -> Self {
        PairMatchParams {
            scale_deg_per_px: 0.0,
            tol_px: 2.5,
            scale_tol: 0.005,
            max_image_stars: 80,
            cat_per_image: 4,
            min_cat_stars: 60,
            max_cat_stars: 300,
            min_inliers: 8,
            early_exit_inliers: 24,
            max_hypotheses: 40_000_000,
            abort_without_promise: 1_000_000,
        }
    }
}

/// What a run found, in enough detail to tell a real answer from a lucky one.
#[derive(Debug, Clone, PartialEq)]
pub struct PairMatchResult {
    pub pairs: Vec<Pairing>,
    /// Image stars the winning hypothesis placed on a catalogue star.
    pub inliers: usize,
    /// The best count any REJECTED hypothesis reached. The gap between this
    /// and `inliers` is the margin the answer won by; reported rather than
    /// thresholded, for the same reason `match_` reports `distinct_stars`
    /// -- whether the evidence suffices is `verify`'s judgement, not this
    /// module's.
    pub runner_up: usize,
    pub image_stars: usize,
    pub cat_stars: usize,
    /// Separation agreements found, and hypotheses actually tested (four
    /// readings per agreement). The ratio to `inliers` is what a slow frame
    /// looks like.
    pub agreements: u64,
    pub hypotheses: u64,
    /// Whether the winning transform was the reflected one.
    pub mirrored: bool,
    /// True when the hypothesis ceiling stopped the search. The answer may
    /// then be the best of a truncated search rather than of the whole one.
    pub truncated: bool,
    /// Hypotheses tested before any of them first reached `min_inliers`.
    /// `None` if none ever did. This is what separates a frame that is going
    /// to work from one that is not, early enough to act on.
    pub hypotheses_to_promise: Option<u64>,
    /// True when the search stopped because nothing showed promise.
    pub aborted: bool,
    /// Whether `inliers` cleared [`PairMatchParams::min_inliers`]. When
    /// false, `pairs` is EMPTY -- a caller that ignores this flag gets a
    /// refusal from the fit rather than a plausible wrong answer, which is
    /// the only safe way to return a result that must not be used.
    ///
    /// Reported rather than signalled by returning nothing, because the
    /// counts below are the diagnosis of a failed solve and they used to
    /// disappear exactly when a reader needed them.
    pub sufficient: bool,
}

/// A pair of points: separation and the two indices into the point list.
struct Sep {
    d: f64,
    a: u32,
    b: u32,
}

/// Every pair among `pts` separated by between `lo` and `hi`, sorted by
/// separation.
///
/// The sort is what turns matching into a linear sweep instead of a nested
/// loop -- the service a star tracker's k-vector performs, done directly
/// because both lists are built here rather than read from an index.
fn separations(pts: &[(f64, f64)], lo: f64, hi: f64) -> Vec<Sep> {
    let mut out = Vec::new();
    for i in 0..pts.len() {
        for j in (i + 1)..pts.len() {
            let dx = pts[j].0 - pts[i].0;
            let dy = pts[j].1 - pts[i].1;
            let d = (dx * dx + dy * dy).sqrt();
            if d >= lo && d <= hi {
                out.push(Sep { d, a: i as u32, b: j as u32 });
            }
        }
    }
    out.sort_by(|x, y| x.d.partial_cmp(&y.d).unwrap_or(std::cmp::Ordering::Equal));
    out
}

/// A uniform bucket grid over the catalogue's tangent-plane positions.
///
/// Every hypothesis tests every image star, so this lookup is the inner loop
/// of the module. A linear scan here made the search quadratic in the
/// catalogue size for no benefit.
struct Grid {
    cell: f64,
    x0: f64,
    y0: f64,
    nx: usize,
    ny: usize,
    buckets: Vec<Vec<u32>>,
}

impl Grid {
    fn build(pts: &[(f64, f64)], cell: f64) -> Grid {
        let (mut x0, mut y0) = (f64::INFINITY, f64::INFINITY);
        let (mut x1, mut y1) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
        for &(x, y) in pts {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
        let nx = (((x1 - x0) / cell).ceil() as usize + 1).max(1);
        let ny = (((y1 - y0) / cell).ceil() as usize + 1).max(1);
        let mut buckets = vec![Vec::new(); nx * ny];
        for (i, &(x, y)) in pts.iter().enumerate() {
            let cx = (((x - x0) / cell) as usize).min(nx - 1);
            let cy = (((y - y0) / cell) as usize).min(ny - 1);
            buckets[cy * nx + cx].push(i as u32);
        }
        Grid { cell, x0, y0, nx, ny, buckets }
    }

    /// Index of the nearest point within `tol`, if any.
    fn nearest(&self, pts: &[(f64, f64)], x: f64, y: f64, tol: f64) -> Option<u32> {
        let cx = ((x - self.x0) / self.cell).floor();
        let cy = ((y - self.y0) / self.cell).floor();
        if !cx.is_finite() || !cy.is_finite() {
            return None;
        }
        if cx < -1.0 || cy < -1.0 || cx > self.nx as f64 || cy > self.ny as f64 {
            return None;
        }
        let (cx, cy) = (cx as isize, cy as isize);
        let t2 = tol * tol;
        let mut best: Option<(u32, f64)> = None;
        for dy in -1isize..=1 {
            for dx in -1isize..=1 {
                let (gx, gy) = (cx + dx, cy + dy);
                if gx < 0 || gy < 0 || gx >= self.nx as isize || gy >= self.ny as isize {
                    continue;
                }
                for &i in &self.buckets[gy as usize * self.nx + gx as usize] {
                    let p = pts[i as usize];
                    let d2 = (p.0 - x).powi(2) + (p.1 - y).powi(2);
                    if d2 <= t2 && best.is_none_or(|(_, b)| d2 < b) {
                        best = Some((i, d2));
                    }
                }
            }
        }
        best.map(|(i, _)| i)
    }
}

/// Best-scoring hypothesis so far, and the best score anything rejected got.
struct Best {
    inliers: usize,
    runner_up: usize,
    pairs: Option<(Vec<Pairing>, bool)>,
}

/// Match image stars to catalogue stars by pairwise separation.
///
/// `image_pts` are pixel coordinates, brightest first. `cat_pts` are tangent
/// plane coordinates in degrees and `cat_sky` the matching sky positions, in
/// the same order, also brightest first. Returns `None` when no hypothesis
/// reached `min_inliers` -- which is not a statement that the frame is
/// unsolvable, only that this matcher found nothing worth fitting.
pub fn match_pairs(
    image_pts: &[(f64, f64)],
    cat_pts: &[(f64, f64)],
    cat_sky: &[(f64, f64)],
    p: &PairMatchParams,
) -> Option<PairMatchResult> {
    if p.scale_deg_per_px <= 0.0 || !p.scale_deg_per_px.is_finite() {
        return None;
    }
    let ni = image_pts.len().min(p.max_image_stars);
    if ni < 4 {
        return None;
    }
    let want = (ni * p.cat_per_image).max(p.min_cat_stars);
    let nc = cat_pts.len().min(cat_sky.len()).min(want).min(p.max_cat_stars);
    if nc < 4 {
        return None;
    }
    let img = &image_pts[..ni];
    let cat = &cat_pts[..nc];
    let s = p.scale_deg_per_px;

    let mut max_img: f64 = 0.0;
    for i in 0..ni {
        for j in (i + 1)..ni {
            let d = (img[i].0 - img[j].0).powi(2) + (img[i].1 - img[j].1).powi(2);
            max_img = max_img.max(d);
        }
    }
    max_img = max_img.sqrt();

    let tol_deg = p.tol_px * s;
    // Below this a separation carries too little information to be worth a
    // hypothesis: nearly every catalogue pair agrees with it.
    let lo_deg = 20.0 * tol_deg;
    let hi_deg = max_img * s * (1.0 + p.scale_tol) + tol_deg;
    if hi_deg <= lo_deg {
        return None;
    }

    let isep = separations(img, lo_deg / s, hi_deg / s);
    let csep = separations(cat, lo_deg, hi_deg);
    if isep.is_empty() || csep.is_empty() {
        return None;
    }

    let grid = Grid::build(cat, (tol_deg * 4.0).max(1e-9));

    let mut st = Best { inliers: 0, runner_up: 0, pairs: None };
    let mut hyp_to_promise: Option<u64> = None;
    let mut aborted = false;
    let mut hypotheses: u64 = 0;
    let mut agreements: u64 = 0;
    let mut truncated = false;

    // One hypothesis, tested: map every image star through the transform the
    // two correspondences imply, and count how many land on a catalogue
    // star.
    let test = |st: &mut Best, ia: usize, ib: usize, ca: usize, cb: usize, mirror: bool| {
        let (ux, uy) = (img[ib].0 - img[ia].0, img[ib].1 - img[ia].1);
        let uy = if mirror { -uy } else { uy };
        let den = ux * ux + uy * uy;
        if den <= 0.0 {
            return;
        }
        let (vx, vy) = (cat[cb].0 - cat[ca].0, cat[cb].1 - cat[ca].1);
        // Complex division: one factor carrying both the scale and the
        // rotation that takes the image pair onto the catalogue pair.
        let kr = (vx * ux + vy * uy) / den;
        let ki = (vy * ux - vx * uy) / den;
        let mut pairs: Vec<Pairing> = Vec::new();
        for &(px, py) in img.iter() {
            let (dx, dy) = (px - img[ia].0, py - img[ia].1);
            let dy = if mirror { -dy } else { dy };
            let x = cat[ca].0 + kr * dx - ki * dy;
            let y = cat[ca].1 + ki * dx + kr * dy;
            if let Some(j) = grid.nearest(cat, x, y, tol_deg) {
                pairs.push(Pairing { image: (px, py), sky: cat_sky[j as usize] });
            }
        }
        let inliers = pairs.len();
        if inliers > st.inliers {
            st.runner_up = st.inliers;
            st.inliers = inliers;
            st.pairs = Some((pairs, mirror));
        } else if inliers > st.runner_up {
            st.runner_up = inliers;
        }
    };

    let mut lo_idx = 0usize;
    'sweep: for is in &isep {
        let d = is.d * s;
        let tol = tol_deg + d * p.scale_tol;
        while lo_idx < csep.len() && csep[lo_idx].d < d - tol {
            lo_idx += 1;
        }
        let mut k = lo_idx;
        while k < csep.len() && csep[k].d <= d + tol {
            let cs = &csep[k];
            k += 1;
            agreements += 1;
            let (ia, ib) = (is.a as usize, is.b as usize);
            let (ca, cb) = (cs.a as usize, cs.b as usize);
            // A separation does not say which endpoint is which, and
            // distance is invariant under reflection, so all four readings
            // are distinct hypotheses. Testing settles them; nothing
            // upstream can. This is also why no parity has to be assumed:
            // a mirrored field is found, not configured.
            for &(x, y) in &[(ca, cb), (cb, ca)] {
                test(&mut st, ia, ib, x, y, false);
                test(&mut st, ia, ib, x, y, true);
                hypotheses += 2;
            }
            if hyp_to_promise.is_none() && st.inliers >= p.min_inliers {
                hyp_to_promise = Some(hypotheses);
            }
            if st.inliers >= p.early_exit_inliers {
                break 'sweep;
            }
            // Nothing has reached the floor the fit needs, and enough has
            // been tried that nothing is going to. Give up rather than pay
            // for the rest of a sweep whose answer would be refused anyway.
            if hyp_to_promise.is_none() && hypotheses >= p.abort_without_promise {
                aborted = true;
                break 'sweep;
            }
            if hypotheses >= p.max_hypotheses {
                truncated = true;
                break 'sweep;
            }
        }
    }

    let sufficient = st.inliers >= p.min_inliers;
    let (pairs, mirrored) = match st.pairs {
        Some((pairs, mirrored)) if sufficient => (pairs, mirrored),
        // Ran, found nothing worth fitting. The counts still travel.
        _ => (Vec::new(), false),
    };
    Some(PairMatchResult {
        pairs,
        inliers: st.inliers,
        runner_up: st.runner_up,
        image_stars: ni,
        cat_stars: nc,
        agreements,
        hypotheses,
        mirrored,
        truncated,
        hypotheses_to_promise: hyp_to_promise,
        aborted,
        sufficient,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A deterministic generator. This crate has no dependencies, and a test
    /// that changes its scene between runs cannot be debugged when it fails.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> f64 {
            // xorshift64*, adequate for scattering points and nothing else.
            let mut x = self.0;
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            self.0 = x;
            let v = x.wrapping_mul(0x2545_F491_4F6C_DD1D);
            (v >> 11) as f64 / (1u64 << 53) as f64
        }
        fn range(&mut self, lo: f64, hi: f64) -> f64 {
            lo + (hi - lo) * self.next()
        }
    }

    /// image points, catalogue points, catalogue sky, stars truly present.
    type Scene = (Vec<(f64, f64)>, Vec<(f64, f64)>, Vec<(f64, f64)>, usize);

    const SCALE: f64 = 1.0 / 3600.0; // 1 arcsec per pixel
    const FIELD_DEG: f64 = 0.5;

    /// A synthetic scene: catalogue stars in a tangent plane, and the image
    /// stars that a frame covering them would show.
    ///
    /// `keep` is the fraction of catalogue stars actually detected --
    /// completeness, the quantity this whole module exists to be robust to.
    /// `spurious` adds detections with no catalogue counterpart, which is
    /// what a hot pixel or a cosmic ray looks like to the matcher.
    fn scene(
        seed: u64,
        n_cat: usize,
        keep: f64,
        spurious: usize,
        rot_deg: f64,
        mirror: bool,
    ) -> Scene {
        let mut rng = Rng(seed);
        let mut cat_pts = Vec::new();
        let mut cat_sky = Vec::new();
        for _ in 0..n_cat {
            let x = rng.range(-FIELD_DEG / 2.0, FIELD_DEG / 2.0);
            let y = rng.range(-FIELD_DEG / 2.0, FIELD_DEG / 2.0);
            cat_pts.push((x, y));
            // A stand-in sky position: the matcher never interprets these,
            // it only carries them through, so any injective map serves.
            cat_sky.push((100.0 + x, 20.0 + y));
        }

        // The inverse of the transform the matcher has to discover.
        let th = rot_deg.to_radians();
        let (c, s) = (th.cos(), th.sin());
        let to_image = |(x, y): (f64, f64)| -> (f64, f64) {
            // Undo rotation and scale; mirror flips the image y axis.
            let (px, py) = ((x * c + y * s) / SCALE, (-x * s + y * c) / SCALE);
            let py = if mirror { -py } else { py };
            (px + 1000.0, py + 700.0)
        };

        let mut image_pts = Vec::new();
        let mut truly_present = 0usize;
        for (i, &p) in cat_pts.iter().enumerate() {
            // Deterministic thinning: every star decides once, by index.
            if (i as f64 / n_cat as f64) < keep {
                image_pts.push(to_image(p));
                truly_present += 1;
            }
        }
        for _ in 0..spurious {
            let x = rng.range(-FIELD_DEG / 2.0, FIELD_DEG / 2.0);
            let y = rng.range(-FIELD_DEG / 2.0, FIELD_DEG / 2.0);
            image_pts.push(to_image((x, y)));
        }
        (image_pts, cat_pts, cat_sky, truly_present)
    }

    fn params() -> PairMatchParams {
        PairMatchParams { scale_deg_per_px: SCALE, ..PairMatchParams::default() }
    }

    #[test]
    fn recovers_a_clean_field() {
        let (img, cat, sky, present) = scene(1, 60, 1.0, 0, 17.0, false);
        let r = match_pairs(&img, &cat, &sky, &params()).expect("a clean field must match");
        assert!(
            r.inliers >= present / 2,
            "only {} of {} present stars were placed",
            r.inliers,
            present
        );
        assert!(!r.mirrored, "field was not mirrored");
    }

    #[test]
    fn recovers_a_mirrored_field() {
        // Parity is discovered, not configured: nothing upstream tells this
        // module the optical train has an odd number of reflections.
        let (img, cat, sky, _) = scene(2, 60, 1.0, 0, -40.0, true);
        let r = match_pairs(&img, &cat, &sky, &params()).expect("a mirrored field must match");
        assert!(r.mirrored, "the mirrored field was not identified as mirrored");
    }

    #[test]
    fn survives_completeness_that_starves_a_quad() {
        // 30% completeness. A quad needs four specific mutually-neighbouring
        // stars to survive, so the matchable fraction goes as 0.3^4 = 0.8%;
        // a pair needs two, and does not care which two.
        let (img, cat, sky, present) = scene(3, 120, 0.30, 25, 55.0, false);
        let r = match_pairs(&img, &cat, &sky, &params())
            .expect("30% completeness must still match on pairs");
        assert!(
            r.inliers >= 10,
            "{} inliers from {} present stars is too few to fit",
            r.inliers,
            present
        );
        assert!(
            r.inliers > r.runner_up,
            "the answer must beat the best rejected hypothesis ({} vs {})",
            r.inliers,
            r.runner_up
        );
    }

    #[test]
    fn a_catalogue_from_the_wrong_field_yields_nothing() {
        // The null. The image is a real field; the catalogue describes a
        // different patch of sky entirely. A matcher that answers here is
        // the expensive kind of broken -- it returns something plausible.
        let (img, _, _, _) = scene(4, 80, 1.0, 0, 12.0, false);
        let (_, cat, sky, _) = scene(999, 80, 1.0, 0, 12.0, false);
        let r = match_pairs(&img, &cat, &sky, &params()).expect("the search runs");
        assert!(!r.sufficient, "matched an unrelated catalogue");
        assert!(r.pairs.is_empty(), "offered correspondences for an unrelated catalogue");
    }

    #[test]
    fn pure_noise_yields_nothing() {
        let mut rng = Rng(77);
        let img: Vec<(f64, f64)> =
            (0..80).map(|_| (rng.range(0.0, 2000.0), rng.range(0.0, 1400.0))).collect();
        let (_, cat, sky, _) = scene(5, 120, 1.0, 0, 0.0, false);
        let r = match_pairs(&img, &cat, &sky, &params()).expect("the search runs");
        assert!(!r.sufficient, "matched a field of random points");
        assert!(r.pairs.is_empty(), "offered correspondences for random points");
    }

    #[test]
    fn min_inliers_is_enforced_and_load_bearing() {
        // Mutation guard on the specific constant this test names: a scene
        // that clears the floor by a known margin must be refused when the
        // floor is raised past what it achieves. Without this, `min_inliers`
        // could be set to anything and every other test here would pass.
        let (img, cat, sky, _) = scene(6, 100, 0.45, 10, 25.0, false);
        let got = match_pairs(&img, &cat, &sky, &params()).expect("scene should match");
        let above = got.inliers;
        assert!(above >= 8, "scene must clear the default floor to be a valid probe");

        let strict = PairMatchParams { min_inliers: above + 1, ..params() };
        let refused = match_pairs(&img, &cat, &sky, &strict)
            .expect("the search still RAN, so its counts must survive");
        assert!(
            !refused.sufficient,
            "a floor above the achievable inlier count ({above}) must refuse"
        );
        assert!(
            refused.pairs.is_empty(),
            "an insufficient result must carry no correspondences -- a caller \
             ignoring `sufficient` has to get a refusal from the fit, not a guess"
        );
        assert_eq!(refused.inliers, above, "the counts must still be reported");
    }

    /// The abort must not fire on a frame that was going to work.
    ///
    /// Measured across the 84 corpus frames pair matching rescues, every one
    /// reached `min_inliers` within 91,128 hypotheses -- median 264. Frames
    /// that never solve run past four million and never reach it. The
    /// default sits 11x above the worst observed rescue; this pins the
    /// mechanism rather than that constant.
    #[test]
    fn the_abort_spares_a_frame_that_shows_promise() {
        let (img, cat, sky, _) = scene(11, 100, 0.6, 15, 41.0, false);
        // An abort threshold of 1 fires at the first opportunity -- yet a
        // frame that reaches the floor immediately must survive it.
        let eager = PairMatchParams { abort_without_promise: 1, ..params() };
        let r = match_pairs(&img, &cat, &sky, &eager).expect("should run");
        assert!(r.sufficient, "a solvable frame was abandoned by the abort");
        assert!(!r.aborted, "a frame that reached the floor must not be marked aborted");
        assert!(
            r.hypotheses_to_promise.is_some(),
            "a sufficient result must record when it first showed promise"
        );
    }

    /// And it must fire on one that was not, rather than paying for the
    /// whole sweep. Without this rule a NoQuadMatch cost 3,286 ms median
    /// where it had cost ~70 ms before this matcher existed.
    #[test]
    fn the_abort_fires_on_a_hopeless_frame() {
        // Image stars unrelated to the catalogue: nothing will ever reach
        // the floor, so the only question is how long that takes to admit.
        let (img, _, _, _) = scene(12, 90, 1.0, 0, 8.0, false);
        let (_, cat, sky, _) = scene(888, 200, 1.0, 0, 8.0, false);

        let patient = PairMatchParams { abort_without_promise: u64::MAX, ..params() };
        let full = match_pairs(&img, &cat, &sky, &patient).expect("should run");
        assert!(!full.sufficient, "fixture must be hopeless");
        assert!(!full.aborted);

        let eager = PairMatchParams { abort_without_promise: 500, ..params() };
        let cut = match_pairs(&img, &cat, &sky, &eager).expect("should run");
        assert!(cut.aborted, "the abort did not fire on a hopeless frame");
        assert!(!cut.sufficient);
        assert!(
            cut.hypotheses < full.hypotheses,
            "aborting must actually shorten the search ({} vs {})",
            cut.hypotheses,
            full.hypotheses
        );
    }

    #[test]
    fn no_plate_scale_is_a_refusal_not_a_guess() {
        // The retry is hinted-only for this reason: separations cannot be
        // compared across the two lists without a scale, and inventing one
        // would produce a confident answer from nothing.
        let (img, cat, sky, _) = scene(7, 60, 1.0, 0, 0.0, false);
        let p = PairMatchParams { scale_deg_per_px: 0.0, ..PairMatchParams::default() };
        assert!(match_pairs(&img, &cat, &sky, &p).is_none(), "matched with no plate scale");
        let p = PairMatchParams { scale_deg_per_px: f64::NAN, ..PairMatchParams::default() };
        assert!(match_pairs(&img, &cat, &sky, &p).is_none(), "matched with a NaN plate scale");
    }

    #[test]
    fn too_few_stars_on_either_side_is_a_refusal() {
        let (img, cat, sky, _) = scene(8, 60, 1.0, 0, 0.0, false);
        assert!(match_pairs(&img[..3], &cat, &sky, &params()).is_none(), "matched on 3 image stars");
        assert!(match_pairs(&img, &cat[..3], &sky[..3], &params()).is_none(), "matched on 3 catalogue stars");
        assert!(match_pairs(&[], &cat, &sky, &params()).is_none(), "matched on no image stars");
    }

    #[test]
    fn the_hypothesis_ceiling_is_reported_not_hidden() {
        // A truncated search may return the best of a partial sweep. That is
        // acceptable; concealing it is not, because a caller comparing two
        // runs would see a different answer with no visible cause.
        let (img, cat, sky, _) = scene(9, 120, 0.35, 40, 33.0, false);
        let capped = PairMatchParams { max_hypotheses: 8, early_exit_inliers: usize::MAX, ..params() };
        if let Some(r) = match_pairs(&img, &cat, &sky, &capped) {
            assert!(r.truncated, "a search stopped by the ceiling must say so");
        }
    }

    #[test]
    fn correspondences_are_consistent_with_one_transform() {
        // The pairs handed to the fit must all obey a single similarity
        // transform. If they did not, `fit_tan` would absorb the
        // disagreement into a large residual and `verify` would be asked to
        // judge a fit built on contradictions.
        let (img, cat, sky, _) = scene(10, 80, 0.8, 5, 63.0, false);
        let r = match_pairs(&img, &cat, &sky, &params()).expect("should match");
        assert!(r.pairs.len() >= 8);
        // Recover the transform from the first two pairs and check the rest.
        let (i0, s0) = (r.pairs[0].image, r.pairs[0].sky);
        let (i1, s1) = (r.pairs[1].image, r.pairs[1].sky);
        let (ux, uy) = (i1.0 - i0.0, i1.1 - i0.1);
        let (vx, vy) = (s1.0 - s0.0, s1.1 - s0.1);
        let den = ux * ux + uy * uy;
        assert!(den > 0.0);
        let kr = (vx * ux + vy * uy) / den;
        let ki = (vy * ux - vx * uy) / den;
        for q in &r.pairs {
            let (dx, dy) = (q.image.0 - i0.0, q.image.1 - i0.1);
            let x = s0.0 + kr * dx - ki * dy;
            let y = s0.1 + ki * dx + kr * dy;
            let err = ((x - q.sky.0).powi(2) + (y - q.sky.1).powi(2)).sqrt();
            assert!(
                err < 10.0 * SCALE,
                "correspondence off the common transform by {:.4} deg",
                err
            );
        }
    }
}
