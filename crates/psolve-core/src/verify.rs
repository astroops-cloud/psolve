//! How much we believe a solution.
//!
//! A caller of this library treats "solved" as proof the telescope saw sky --
//! the reasoning being that a star pattern matched a catalogue, which a
//! photograph of a house cannot do. That guarantee is worth exactly the
//! false-positive rate, so confidence is COMPUTED here, never assumed.
//!
//! Under the null hypothesis that matches are coincidence, the count of image
//! stars landing within `tol` of some catalogue star is Poisson with mean
//!     lambda = n_image * n_cat * pi*tol^2 / field_area
//! and the log-odds against chance is ln(k!) + lambda - k*ln(lambda), reported
//! in decades.
//!
//! That expression is the Poisson PMF's distance from its mode, which is
//! U-shaped in k around lambda: it grows large whether k lands far above OR
//! far below the chance expectation. Only the upper side is evidence against
//! coincidence -- a match count no bigger than what chance alone would
//! produce is not surprising, however far below the mean it happens to sit.
//! So the formula only applies once matched exceeds lambda; at or below it
//! there is no excess to explain and log-odds is zero.
//!
//! # The blind null: the same per-test lambda, vastly more tests
//!
//! Everything above was calibrated against a **disc of known position**. A
//! caller supplied a pointing hint, the matcher settled on ONE transform, and
//! that single transform was put to the reprojection test. `AcceptParams`'s
//! `min_log_odds: 12.0` therefore means: this one hypothesis would arise by
//! chance with probability about `1e-12`.
//!
//! A blind solve has no hint. It generates a candidate transform from every
//! (image quad, catalogue candidate) pair the code-space lookup offers, and
//! tests all of them. Applying the same 12.0 to that search is the mistake
//! that makes a solver lie confidently, and it is not hypothetical -- the
//! motivating incident for this milestone is a wide-search solve that came
//! back 87.77 degrees from the truth and was reported as a success by its
//! own confidence gate. The only reason anyone noticed was that a commanded
//! pointing existed to compare against. **Blind solving has no such
//! comparison**, so the gate has to stand on its own.
//!
//! ## What changes, and what does not
//!
//! **`lambda` does not change.** Once a candidate transform exists, the test
//! is identical to the hinted one: push the catalogue stars that fall in
//! THIS frame's footprint through the candidate WCS and count how many land
//! within `tol` of a detection. The relevant density is still "catalogue
//! stars over this frame's own area", so
//!
//! ```text
//!     lambda = n_image * n_cat * pi*tol^2 / field_area
//! ```
//!
//! holds verbatim, with the same numbers. The sky's size does **not** enter
//! as a larger area term -- writing `A = 41253 deg^2` would be wrong, because
//! no coincidence is ever scored against the whole sky at once. Each
//! hypothesis is scored against one frame-sized footprint.
//!
//! **The number of hypotheses changes, by four orders of magnitude.** The
//! hinted path performs exactly one test. A blind solve performs one per
//! candidate pair: up to `SolveOptions::max_quads` = 600 image quads, times
//! the ~21 candidates a `.psqidx` code-space lookup returns per image quad
//! (measured in Task 4), summed over every scale band swept. Call that count
//! `M`; for a single band it is about `1.26e4`.
//!
//! ## The correction
//!
//! Let `p_i` be the probability that hypothesis `i` clears the threshold
//! under the null, and let `F` be the number of hypotheses that do. Then
//!
//! ```text
//!     E[F] = sum_i p_i  <=  M * alpha_1
//!     P(F >= 1) <= E[F]                    (Markov; equivalently a union bound)
//! ```
//!
//! To give the WHOLE blind search the same family-wise false-alarm budget
//! `alpha = 1e-12` the hinted single test already had, require
//! `alpha_1 <= alpha / M`, which in decades is simply
//!
//! ```text
//!     min_log_odds_blind = min_log_odds_hinted + log10(M)
//! ```
//!
//! That is a Bonferroni correction, chosen for a specific reason: the union
//! bound needs **no independence assumption**, and these hypotheses are
//! heavily dependent -- they share image stars, share catalogue stars, and
//! neighbouring image quads are built from overlapping detections. Sidak's
//! `1 - (1-alpha)^(1/M)` would require independence we do not have, and at
//! `alpha = 1e-12` it differs from `alpha/M` by under one part in `1e11`
//! anyway. A false-discovery-rate procedure controls the wrong quantity
//! entirely: the requirement here is zero wrong answers, not a bounded
//! fraction of them.
//!
//! ## What counts toward `M`, and what does not
//!
//! `M` is the number of hypotheses **actually examined**, not the number of
//! quads in the index. A quad index holds millions of quads, but the lookup
//! only ever OFFERS about 21 per image quad; the rest were never tested and
//! contribute nothing to `E[F]`. Counting them would add roughly five more
//! decades of penalty for no statistical reason and would reject genuine
//! solves.
//!
//! Conversely `M` must include candidates killed by the cheap geometric
//! pre-checks (`blind.rs`'s shape-code and scale-consistency tests). Those
//! checks are part of the same selection procedure, so a coincidence that
//! reaches the confidence stage has already been selected out of the larger
//! family. The honest family is every pair OFFERED.
//!
//! Because the offered count varies enormously per image quad and per band
//! -- Task 4 measured a mean of 21.4 candidates on band 0 against 0.5 on
//! band 3, a 40x spread -- `M` cannot be reconstructed as
//! `quads * mean_candidates * bands`. Any single scalar undercounts every
//! above-mean quad, and an undercount BUYS A LOOSER GATE. So `M` is
//! accumulated by observation, one image quad at a time, by
//! [`HypothesisCount`]. Note especially that the length of
//! `blind::candidate_transforms`'s returned vector is the number of
//! SURVIVORS, not the number offered, and is exactly the wrong number to
//! record here.
//!
//! For a single-band blind solve, `log10(1.26e4) = 4.1`, so the gate sits at
//! about **16.1 decades against the hinted 12.0**. Doubling the candidates
//! examined adds `log10(2) = 0.30` -- stricter, never looser.
//!
//! ## Three limits of this model, recorded rather than hidden
//!
//! 1. `-log10 P(X = k)` is not the tail probability `-log10 P(X >= k)`; it
//!    overstates the evidence by at most `log10(1 / (1 - lambda/(k+1)))`,
//!    which for the regime here (`lambda < 1`, `k >= 10`) is under 0.05
//!    decades. Real, bounded, and dwarfed by the multiplicity term -- but it
//!    errs toward optimism, so it is stated.
//! 2. The Poisson null assumes catalogue stars are scattered uniformly over
//!    the frame. Real fields cluster, and a clustered field throws up more
//!    coincidences than a uniform one at the same mean density. That
//!    mis-specification is shared with the hinted gate, is not what this
//!    correction addresses, and is exactly why `min_matched` survives
//!    unchanged into the blind parameters and why the blind path keeps its
//!    own geometric checks rather than leaning on this gate alone.
//! 3. **The reprojection count is not a draw from `Poisson(lambda)`, and
//!    this one is larger than the other two put together.** `solve.rs`
//!    counts every catalogue star that reprojects within `tol_px` of a
//!    detection -- *including the correspondences the transform was fit to*,
//!    which land on their own detections by construction. The null
//!    distribution of the raw count is therefore `n_fit + Poisson(lambda)`,
//!    not `Poisson(lambda)`, and scoring it as though it were the latter
//!    credits a coincidence with `n_fit` free matches.
//!
//!    The arithmetic, at the reference rig (`lambda = 0.234`,
//!    `n_fit = 4`): a 16.10-decade gate scored on the RAW count is first
//!    cleared at `matched = 12`, which the formula reports as 16.34
//!    decades. But the excess over the free matches is only `12 - 4 = 8`,
//!    and the true null probability of that event is `P(X >= 8) = 1.8e-10`
//!    -- **9.74 decades, not 16.34**. Delivered per-test `alpha_1` is
//!    ~`1.8e-10` and family-wise ~`2.3e-6` per frame, not the `1e-12` the
//!    derivation claims to preserve. The multiplicity correction buys 4.1
//!    decades; this leak costs about 6, so uncorrected it more than undoes
//!    the whole point of this task. Bonferroni is valid only conditional on
//!    each per-test p-value being valid: multiplying `M` miscalibrated
//!    tests INHERITS the miscalibration rather than dividing it away.
//!
//!    The leak is pre-existing and shared with the hinted gate, so removing
//!    it from `confidence` would change the hinted path -- which is exactly
//!    what must not happen. [`blind_confidence`] therefore restores the null
//!    on the blind path only, by scoring `reprojected - n_fit`. A genuine
//!    118-match solve loses 4 of them and still reports 259 decades, so
//!    this costs real solves nothing.

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Confidence {
    /// Decades of evidence against the coincidence hypothesis.
    pub log_odds: f64,
    /// How many matches this configuration would produce by chance alone.
    pub chance_matches: f64,
    pub matched: usize,
}

/// ln(k!) by Stirling with the half-log correction. There is no lgamma in std
/// and none is needed at this precision.
fn ln_factorial(k: f64) -> f64 {
    if k < 2.0 {
        return 0.0;
    }
    k * k.ln() - k + 0.5 * (2.0 * std::f64::consts::PI * k).ln()
}

pub fn confidence(
    matched: usize,
    n_image: usize,
    n_cat: usize,
    tol_deg: f64,
    field_area_deg2: f64,
) -> Confidence {
    let k = matched as f64;
    if matched == 0 || n_image == 0 || n_cat == 0 || tol_deg <= 0.0 || field_area_deg2 <= 0.0 {
        return Confidence { log_odds: 0.0, chance_matches: 0.0, matched };
    }
    let lambda = (n_image as f64) * (n_cat as f64) * std::f64::consts::PI * tol_deg * tol_deg
        / field_area_deg2;
    if !lambda.is_finite() || lambda <= 0.0 {
        return Confidence { log_odds: 0.0, chance_matches: 0.0, matched };
    }
    // A match count at or below the chance expectation is not an excess to
    // explain, however improbable that exact count is under the Poisson
    // model -- the PMF's distance from its mode is symmetric-ish in both
    // directions, and only the upper tail counts as evidence against
    // coincidence.
    if k <= lambda {
        return Confidence { log_odds: 0.0, chance_matches: lambda, matched };
    }
    // -ln P(k | chance), in decades.
    let neg_ln_p = ln_factorial(k) + lambda - k * lambda.ln();
    let log_odds = (neg_ln_p / std::f64::consts::LN_10).max(0.0);
    Confidence {
        log_odds: if log_odds.is_finite() { log_odds } else { 0.0 },
        chance_matches: lambda,
        matched,
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AcceptParams {
    pub min_matched: usize,
    pub min_log_odds: f64,
    pub max_rms_px: f64,
}

impl Default for AcceptParams {
    fn default() -> Self {
        AcceptParams {
            // Ten correspondences over-determine six parameters by enough that
            // the residuals are a real check rather than a restatement.
            min_matched: 10,
            min_log_odds: 12.0,
            max_rms_px: 3.0,
        }
    }
}

impl AcceptParams {
    /// The same gate, corrected for a blind search that examined
    /// `hypotheses` candidate transforms.
    ///
    /// See this module's "The blind null" section for the derivation. The
    /// short form: the per-test `lambda` is unchanged, so the only thing a
    /// blind search alters is how many chances coincidence gets, and holding
    /// the family-wise false-alarm budget fixed costs exactly
    /// `log10(hypotheses)` decades.
    ///
    /// Derived from [`AcceptParams::default`] rather than restating its
    /// numbers, so a future change to the hinted defaults carries through
    /// here instead of silently diverging. `min_matched` and `max_rms_px`
    /// are deliberately untouched: the multiplicity correction is a
    /// statement about multiplicity, and landing it anywhere but
    /// `min_log_odds` would make the two gates hard to compare.
    ///
    /// `hypotheses == 1` yields the hinted threshold exactly: one test needs
    /// no correction. The result is never LOOSER than the hinted gate for
    /// any input, including `usize::MAX` (a finite 19.3 decades).
    ///
    /// **`hypotheses == 0` produces a gate nothing can clear**
    /// (`min_log_odds` of infinity), not the hinted threshold. A search that
    /// examined nothing has, by construction, found nothing, so there is no
    /// honest reading under which it accepts a solution. The earlier
    /// "0 and 1 both mean no correction" reading was sound arithmetic and an
    /// unsafe API: a Task 7 wiring bug -- a band count left at zero, or a
    /// count accidentally taken from survivors that happened to be zero --
    /// would have silently reverted the blind path to the hinted 12.0 with
    /// no visible symptom, which is precisely the failure this module
    /// exists to close. Failing closed makes that bug loud.
    ///
    /// **The caller owes an honest `hypotheses`.** It must count every
    /// candidate pair the search OFFERED across the entire run -- every
    /// image quad, every lookup candidate, every scale band -- because an
    /// undercount buys a looser gate. Use [`HypothesisCount`]; do not
    /// reconstruct it from a mean, and do not take it from the number of
    /// candidates that survived filtering.
    pub fn blind(hypotheses: usize) -> AcceptParams {
        let base = AcceptParams::default();
        AcceptParams {
            min_log_odds: base.min_log_odds + multiplicity_decades(hypotheses),
            ..base
        }
    }
}

/// The Bonferroni penalty, in decades: `log10(hypotheses)`.
///
/// Monotone non-decreasing and never negative, which is the property that
/// matters -- examining more candidates can only ever make the gate
/// stricter. `log10(usize::MAX)` is a finite 19.27.
///
/// Zero hypotheses returns infinity, not zero: see [`AcceptParams::blind`]
/// for why a search that examined nothing must fail closed rather than
/// silently reverting to the uncorrected threshold.
pub fn multiplicity_decades(hypotheses: usize) -> f64 {
    if hypotheses == 0 {
        return f64::INFINITY;
    }
    let d = (hypotheses as f64).log10();
    if d.is_finite() && d > 0.0 { d } else { 0.0 }
}

/// Accumulates `M` -- the number of candidate hypotheses a blind search
/// offered -- by observation rather than by formula.
///
/// A multiplicative estimate (`quads * candidates_per_quad * bands`) cannot
/// express the real count, because the offered count per image quad varies
/// by ~40x across bands (Task 4: mean 21.4 on band 0, 0.5 on band 3) and
/// varies again from quad to quad within a band. Any single scalar
/// undercounts every above-mean quad, and an undercount buys a looser gate.
/// So the search reports what it actually saw:
///
/// ```
/// use psolve_core::verify::{AcceptParams, HypothesisCount};
/// let mut m = HypothesisCount::new();
/// for _band in 0..6 {
///     for _image_quad in 0..600 {
///         // `candidates` is what the code-space lookup RETURNED, counted
///         // before any filtering -- never the survivor count.
///         let candidates: usize = 21;
///         m.offered(candidates);
///     }
/// }
/// let params = AcceptParams::blind(m.total());
/// assert!(params.min_log_odds > 16.0);
/// ```
///
/// **Ordering matters.** The threshold depends on the total, so the gate is
/// applied ONCE, after the search has finished enumerating -- score every
/// candidate, keep the best, then judge it against the final `M`. Gating
/// incrementally against a partial total would test early candidates at a
/// threshold too low, which is the same undercount by another route.
///
/// Saturating rather than wrapping, so an absurd input yields an absurdly
/// STRICT gate rather than a wrapped-to-small one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HypothesisCount {
    total: usize,
}

impl HypothesisCount {
    pub fn new() -> HypothesisCount {
        HypothesisCount { total: 0 }
    }

    /// Record the candidates one code-space lookup OFFERED for one image
    /// quad. Call once per (image quad, band) lookup.
    ///
    /// The argument is the length of the candidate slice handed to
    /// `blind::candidate_transforms`, **not** the length of the vector it
    /// returns -- that one counts only the candidates that survived the
    /// shape and scale checks, and those checks are part of the selection
    /// procedure this count exists to charge for.
    pub fn offered(&mut self, candidates: usize) {
        self.total = self.total.saturating_add(candidates);
    }

    /// The accumulated `M`, ready for [`AcceptParams::blind`].
    pub fn total(&self) -> usize {
        self.total
    }

    /// True when no candidate was ever offered. [`AcceptParams::blind`]
    /// turns this into a gate nothing can clear.
    pub fn is_empty(&self) -> bool {
        self.total == 0
    }
}

/// [`confidence`], with the free matches removed -- the blind path's
/// entry point, and the one that makes `min_log_odds` mean what the
/// derivation says it means.
///
/// `reprojected` is the raw count `solve.rs` computes: every catalogue star
/// landing within `tol` of a detection. `n_fit` is how many correspondences
/// the candidate transform was fit to (`FitResult::used` -- 4 straight out
/// of `blind::candidate_transform`). Those `n_fit` stars reproject onto
/// their own detections BY CONSTRUCTION and carry no evidence, so the raw
/// count is distributed as `n_fit + Poisson(lambda)` under the null. This
/// function scores the excess, `reprojected - n_fit`, which is the part
/// that is actually `Poisson(lambda)`.
///
/// See limit 3 in this module's doc for the arithmetic: at the reference
/// rig, ignoring this turns a nominal `1e-12` per-test false-alarm rate into
/// a delivered `1.8e-10`, which more than cancels the multiplicity
/// correction this module exists to apply.
///
/// The returned `Confidence.matched` is the EXCESS, not the raw count, so
/// `AcceptParams::min_matched` also means "independent matches" on the blind
/// path -- a stricter and more honest reading of the same number. A genuine
/// solve is unaffected: 118 raw matches becomes 114 and still reports 259
/// decades.
///
/// Saturating: `n_fit >= reprojected` yields zero excess and therefore zero
/// evidence, which is the correct answer -- a transform that predicts
/// nothing beyond the stars it was built from has demonstrated nothing.
///
/// [`confidence`] itself is deliberately NOT changed. The same leak exists
/// on the hinted path, where it is mitigated by a hint the caller supplied
/// and by testing exactly one hypothesis; closing it there would move the
/// hinted path's numbers, which this milestone forbids.
pub fn blind_confidence(
    reprojected: usize,
    n_fit: usize,
    n_image: usize,
    n_cat: usize,
    tol_deg: f64,
    field_area_deg2: f64,
) -> Confidence {
    confidence(
        reprojected.saturating_sub(n_fit),
        n_image,
        n_cat,
        tol_deg,
        field_area_deg2,
    )
}

/// All three must hold. Confidence without a tight fit means the
/// correspondences are wrong; a tight fit on too few stars is not evidence.
pub fn accept(c: &Confidence, rms_px: f64, p: &AcceptParams) -> bool {
    c.matched >= p.min_matched
        && c.log_odds >= p.min_log_odds
        && rms_px.is_finite()
        && rms_px <= p.max_rms_px
}

#[cfg(test)]
mod tests {
    use super::*;

    // The reference rig: 2.626 x 1.477 deg = 3.878 deg^2, ~500 image stars,
    // ~300 catalogue stars, matching within about 2 pixels (5 arcsec).
    const AREA: f64 = 3.878;
    const TOL: f64 = 5.0 / 3600.0;

    #[test]
    fn a_genuine_solve_is_overwhelmingly_unlikely_by_chance() {
        let c = confidence(118, 502, 300, TOL, AREA);
        assert!(c.chance_matches < 1.0, "expected chance matches {}", c.chance_matches);
        assert!(c.log_odds > 50.0, "log odds {} is too weak for 118 matches", c.log_odds);
    }

    #[test]
    fn a_handful_of_coincidences_is_not_convincing() {
        // With a generous tolerance, a few matches really can be chance.
        let c = confidence(3, 500, 300, 0.05, AREA);
        assert!(c.chance_matches > 1.0, "this configuration should expect coincidences");
        assert!(c.log_odds < 10.0, "log odds {} overstates 3 chance matches", c.log_odds);
    }

    #[test]
    fn log_odds_rises_with_the_number_of_matches() {
        let a = confidence(10, 500, 300, TOL, AREA);
        let b = confidence(40, 500, 300, TOL, AREA);
        let d = confidence(120, 500, 300, TOL, AREA);
        assert!(a.log_odds < b.log_odds, "{} !< {}", a.log_odds, b.log_odds);
        assert!(b.log_odds < d.log_odds, "{} !< {}", b.log_odds, d.log_odds);
    }

    #[test]
    fn a_looser_tolerance_weakens_the_same_match_count() {
        let tight = confidence(20, 500, 300, TOL, AREA);
        let loose = confidence(20, 500, 300, TOL * 20.0, AREA);
        assert!(
            loose.log_odds < tight.log_odds,
            "loosening the tolerance must not strengthen the claim: {} vs {}",
            loose.log_odds,
            tight.log_odds
        );
    }

    #[test]
    fn zero_matches_is_no_evidence() {
        let c = confidence(0, 500, 300, TOL, AREA);
        assert_eq!(c.matched, 0);
        assert!(c.log_odds <= 0.0, "zero matches claimed {} decades", c.log_odds);
    }

    #[test]
    fn degenerate_inputs_do_not_produce_nan() {
        for c in [
            confidence(10, 0, 300, TOL, AREA),
            confidence(10, 500, 0, TOL, AREA),
            confidence(10, 500, 300, 0.0, AREA),
            confidence(10, 500, 300, TOL, 0.0),
        ] {
            assert!(c.log_odds.is_finite(), "log_odds was {}", c.log_odds);
            assert!(c.chance_matches.is_finite());
        }
    }

    #[test]
    fn acceptance_requires_matches_confidence_and_a_tight_fit_together() {
        let p = AcceptParams::default();
        let good = confidence(118, 502, 300, TOL, AREA);
        assert!(accept(&good, 0.17, &p), "a clean solve must be accepted");

        // Confident but a poor fit -- the correspondences are wrong somehow.
        assert!(!accept(&good, 25.0, &p), "a large RMS must veto acceptance");

        // A tight fit on too few stars is not evidence.
        let thin = confidence(4, 502, 300, TOL, AREA);
        assert!(!accept(&thin, 0.1, &p), "four matches must not be accepted");
    }

    /// A realistic single-band blind search: 600 image quads (the
    /// `SolveOptions::max_quads` default) times the ~21 candidates a
    /// code-space lookup returns.
    const BLIND_M: usize = 600 * 21;

    /// A candidate transform out of `blind::candidate_transform` is fit to
    /// exactly four correspondences, and those four reproject onto their own
    /// detections for free.
    const N_FIT: usize = 4;

    #[test]
    fn a_genuine_solve_clears_the_blind_gate() {
        // The same clean solve the hinted tests use, scored the blind way
        // (its four free matches deducted). 259 decades against a corrected
        // threshold of ~16.1 is not a close call, which is the point:
        // correcting for multiplicity AND removing the free matches costs a
        // genuine match nothing.
        let c = blind_confidence(118, N_FIT, 502, 300, TOL, AREA);
        let blind = AcceptParams::blind(BLIND_M);
        assert_eq!(c.matched, 114, "the four fitted correspondences must be deducted");
        assert!(
            c.log_odds > 250.0,
            "a genuine solve should still be overwhelming: {} decades",
            c.log_odds
        );
        assert!(
            accept(&c, 0.17, &blind),
            "a genuine solve must survive the blind gate: {} decades vs {}",
            c.log_odds,
            blind.min_log_odds
        );
    }

    /// Limit 3, pinned as a test rather than left as prose. The raw
    /// reprojection count includes the `n_fit` correspondences the transform
    /// was fit to, so scoring it directly credits a coincidence with `n_fit`
    /// free matches and delivers a per-test false-alarm rate orders of
    /// magnitude worse than the derivation claims.
    ///
    /// At the reference rig the raw-count gate is first cleared at
    /// `matched = 12`, reported as 16.34 decades -- but only `12 - 4 = 8` of
    /// those matches are evidence, and 8 is worth 9.74 decades, not 16.34.
    /// `blind_confidence` scores the 8.
    #[test]
    fn the_fitted_correspondences_are_not_counted_as_evidence() {
        let raw = confidence(12, 500, 300, TOL, AREA);
        let corrected = blind_confidence(12, N_FIT, 500, 300, TOL, AREA);

        assert_eq!(corrected.matched, 8);
        assert!(
            raw.log_odds > 16.0,
            "fixture check: the raw count is supposed to look convincing ({} decades)",
            raw.log_odds
        );
        assert!(
            corrected.log_odds < 11.0,
            "the four free matches were still being counted: {} decades",
            corrected.log_odds
        );
        // And the whole point: the raw count clears the blind gate while the
        // honest excess does not.
        let blind = AcceptParams::blind(BLIND_M);
        assert!(
            raw.log_odds >= blind.min_log_odds,
            "fixture check: raw {} should clear {}",
            raw.log_odds,
            blind.min_log_odds
        );
        assert!(
            !accept(&corrected, 0.5, &blind),
            "{} decades of real evidence must not clear a {}-decade gate",
            corrected.log_odds,
            blind.min_log_odds
        );
    }

    #[test]
    fn a_transform_predicting_nothing_beyond_its_own_fit_has_no_evidence() {
        // Reprojection found exactly the stars the fit was built from, and
        // nothing else. That is zero independent evidence, not "four
        // matches" -- and saturating subtraction must not underflow when the
        // fit used more points than reprojected.
        for (reprojected, n_fit) in [(4usize, 4usize), (3, 4), (0, 4), (0, 0)] {
            let c = blind_confidence(reprojected, n_fit, 500, 300, TOL, AREA);
            assert_eq!(c.matched, reprojected.saturating_sub(n_fit));
            assert_eq!(c.log_odds, 0.0, "reprojected={reprojected} n_fit={n_fit}");
        }
    }

    #[test]
    fn blind_confidence_agrees_with_confidence_once_the_free_matches_are_removed() {
        // It is a re-scoring of the same statistic, not a different model --
        // so it must be exactly `confidence` of the excess, bit for bit.
        for (reprojected, n_fit) in [(118usize, 4usize), (40, 4), (14, 4), (12, 0), (500, 300)] {
            let a = blind_confidence(reprojected, n_fit, 500, 300, TOL, AREA);
            let b = confidence(reprojected.saturating_sub(n_fit), 500, 300, TOL, AREA);
            assert_eq!(a.log_odds.to_bits(), b.log_odds.to_bits());
            assert_eq!(a.chance_matches.to_bits(), b.chance_matches.to_bits());
            assert_eq!(a.matched, b.matched);
        }
    }

    /// **The test that encodes the whole point of this module's blind
    /// section.** A configuration whose evidence lands between the hinted
    /// and blind thresholds must be accepted by one and refused by the
    /// other -- otherwise the correction is decorative.
    ///
    /// The configuration is not contrived: it is the reference rig
    /// (`AREA`, `TOL`, ~500 detections, ~300 catalogue stars) with exactly
    /// `min_matched` reprojected coincidences. It clears every hinted
    /// criterion -- 10 matches, a tight 0.5 px residual, 12.96 decades --
    /// and is precisely what a blind search finds when it gets to try
    /// twelve thousand candidate transforms instead of one.
    #[test]
    fn a_coincidence_that_clears_the_hinted_gate_does_not_clear_the_blind_one() {
        let c = confidence(10, 500, 300, TOL, AREA);
        let hinted = AcceptParams::default();
        let blind = AcceptParams::blind(BLIND_M);

        assert!(
            c.log_odds > hinted.min_log_odds,
            "fixture is broken -- this must clear the HINTED gate to prove anything: {} vs {}",
            c.log_odds,
            hinted.min_log_odds
        );
        assert!(
            accept(&c, 0.5, &hinted),
            "fixture is broken -- the hinted gate must accept this"
        );
        assert!(
            !accept(&c, 0.5, &blind),
            "the blind gate accepted a {}-decade coincidence at a {}-decade threshold",
            c.log_odds,
            blind.min_log_odds
        );
    }

    /// The correction must scale the right way: examining more candidates
    /// can only make the gate harder to clear, and by the derived amount.
    #[test]
    fn doubling_the_candidates_examined_makes_the_gate_stricter() {
        let mut prev = AcceptParams::blind(1).min_log_odds;
        let mut m = 1usize;
        for _ in 0..20 {
            m *= 2;
            let next = AcceptParams::blind(m).min_log_odds;
            assert!(next > prev, "M={m}: {next} did not exceed {prev}");
            // Exactly log10(2) per doubling -- the derivation, not just a
            // direction.
            assert!(
                (next - prev - 2.0f64.log10()).abs() < 1e-12,
                "M={m}: doubling cost {} decades, expected {}",
                next - prev,
                2.0f64.log10()
            );
            prev = next;
        }
    }

    #[test]
    fn the_blind_gate_is_never_looser_than_the_hinted_one() {
        let hinted = AcceptParams::default();
        for m in [0usize, 1, 2, 21, 600, BLIND_M, 1_000_000, usize::MAX] {
            let b = AcceptParams::blind(m);
            assert!(!b.min_log_odds.is_nan(), "M={m} gave NaN");
            assert!(
                b.min_log_odds >= hinted.min_log_odds,
                "M={m} produced a LOOSER gate: {} < {}",
                b.min_log_odds,
                hinted.min_log_odds
            );
            // The correction lands on min_log_odds alone.
            assert_eq!(b.min_matched, hinted.min_matched);
            assert_eq!(b.max_rms_px, hinted.max_rms_px);
        }
        // Every count that actually searched something is finite.
        for m in [1usize, 2, 21, 600, BLIND_M, 1_000_000, usize::MAX] {
            assert!(AcceptParams::blind(m).min_log_odds.is_finite(), "M={m}");
        }
    }

    /// One hypothesis needs no correction -- that is the identity the
    /// derivation predicts, and the only input for which the blind and
    /// hinted gates coincide.
    #[test]
    fn a_search_of_exactly_one_hypothesis_reduces_to_the_hinted_gate() {
        assert_eq!(AcceptParams::blind(1), AcceptParams::default());
        assert_eq!(multiplicity_decades(1), 0.0);
    }

    /// **A blind search that examined nothing must refuse, not fall back to
    /// the hinted threshold.** The arithmetic for `M = 0` is vacuous either
    /// way, but the API is not: a Task 7 wiring bug (a band count left at
    /// zero, a count taken from survivors that happened to be zero) would
    /// silently revert the blind path to 12.0 with no visible symptom. That
    /// is exactly the failure this module exists to close, so it fails
    /// closed instead.
    #[test]
    fn a_search_that_examined_nothing_refuses_everything() {
        let p = AcceptParams::blind(0);
        assert_ne!(p, AcceptParams::default(), "M=0 must NOT be the hinted gate");
        assert_eq!(multiplicity_decades(0), f64::INFINITY);
        assert_eq!(p.min_log_odds, f64::INFINITY);

        // Not even an overwhelming genuine solve gets through a search that
        // never happened -- there is no honest reading under which it could.
        let overwhelming = blind_confidence(118, N_FIT, 502, 300, TOL, AREA);
        assert!(overwhelming.log_odds > 250.0, "fixture check");
        assert!(
            !accept(&overwhelming, 0.17, &p),
            "a search of zero hypotheses accepted a {}-decade claim",
            overwhelming.log_odds
        );
    }

    #[test]
    fn a_single_image_quad_whose_lookup_offered_nothing_is_not_a_free_pass() {
        // One image quad, one band, zero candidates offered: no hypothesis
        // was ever formed, so the gate must refuse rather than relax.
        let mut m = HypothesisCount::new();
        m.offered(0);
        assert!(m.is_empty());
        assert_eq!(m.total(), 0);
        let p = AcceptParams::blind(m.total());
        assert_eq!(p.min_log_odds, f64::INFINITY);
        let weak = blind_confidence(14, N_FIT, 500, 300, TOL * 4.0, AREA);
        assert!(!accept(&weak, 0.5, &p), "{} decades must not pass", weak.log_odds);
    }

    #[test]
    fn zero_matches_cannot_clear_the_blind_gate() {
        let c = blind_confidence(0, N_FIT, 500, 300, TOL, AREA);
        assert!(!accept(&c, 0.0, &AcceptParams::blind(BLIND_M)));
        assert!(!accept(&c, 0.0, &AcceptParams::blind(0)));
    }

    /// `M` is accumulated by observation because it cannot be reconstructed
    /// from a mean: Task 4 measured 21.4 candidates per quad on band 0
    /// against 0.5 on band 3, so any single scalar undercounts every
    /// above-mean quad -- and an undercount buys a looser gate.
    #[test]
    fn the_hypothesis_count_sums_what_was_offered_rather_than_assuming_a_mean() {
        // A realistic, deliberately uneven band sweep: the per-band means
        // differ by 40x, and quads within a band differ from each other.
        let bands: [&[usize]; 4] = [&[30, 21, 14, 21], &[9, 12, 7], &[3, 2], &[1, 0]];
        let mut m = HypothesisCount::new();
        for band in bands {
            for &offered in band {
                m.offered(offered);
            }
        }
        assert_eq!(m.total(), 30 + 21 + 14 + 21 + 9 + 12 + 7 + 3 + 2 + 1);
        assert!(!m.is_empty());

        // The multiplicative estimate this replaces -- quads x mean x bands,
        // with the mean taken as band 0's 21 -- would have said 21*10 = 210
        // hypotheses for the same search. Two of the ten quads offered MORE
        // than 21, so a per-quad mean silently undercounts; the accumulator
        // charges for exactly what happened.
        // Derived from `bands`, not restated as literals: an earlier version
        // of this block asserted `30 >= 21` over a hardcoded array, which is
        // a literal compared against a literal and could never fail.
        let flat_estimate = 21 * bands.iter().map(|b| b.len()).sum::<usize>();
        let observed = m.total();
        assert!(
            observed < flat_estimate,
            "the accumulator ({observed}) must come in under the flat \
quads x mean estimate ({flat_estimate}) on this sweep"
        );
        let over_mean = bands[0].iter().filter(|&&o| o > 21).count();
        assert!(
            over_mean >= 1,
            "band 0 must contain a quad offering strictly more than the 21 a \
per-quad mean would charge -- that is the undercount this accumulator exists \
to avoid, and without it this test proves nothing"
        );

        // Uniform sweeps still come out exactly right.
        let mut flat = HypothesisCount::new();
        for _ in 0..600 {
            flat.offered(21);
        }
        assert_eq!(flat.total(), 12_600);
        let mut six_bands = HypothesisCount::new();
        for _ in 0..6 {
            for _ in 0..600 {
                six_bands.offered(21);
            }
        }
        assert_eq!(six_bands.total(), 75_600);
        assert!(
            AcceptParams::blind(six_bands.total()).min_log_odds
                > AcceptParams::blind(flat.total()).min_log_odds,
            "more bands must mean a stricter gate"
        );
    }

    #[test]
    fn the_hypothesis_count_saturates_rather_than_wrapping() {
        // An absurd count must produce an absurdly STRICT gate, never a
        // wrapped-to-small one that quietly reopens the hole.
        let mut m = HypothesisCount::new();
        m.offered(usize::MAX);
        m.offered(usize::MAX);
        m.offered(1);
        assert_eq!(m.total(), usize::MAX);
        assert!(AcceptParams::blind(m.total()).min_log_odds > 30.0);
    }

    #[test]
    fn the_hypothesis_count_starts_empty_and_only_ever_grows() {
        let m = HypothesisCount::new();
        assert_eq!(m, HypothesisCount::default());
        assert!(m.is_empty());
        let mut m = m;
        let mut prev = 0usize;
        for offered in [5usize, 0, 21, 1] {
            m.offered(offered);
            assert!(m.total() >= prev, "the count must never shrink");
            prev = m.total();
        }
        assert_eq!(m.total(), 27);
    }

    /// **The hinted path must be numerically unchanged, not merely close.**
    /// These bit patterns were captured from the build immediately before
    /// the blind gate was added; any drift in `confidence`, `accept` or the
    /// hinted defaults changes them.
    #[test]
    fn the_hinted_gate_is_bit_identical_to_its_pre_blind_values() {
        let p = AcceptParams::default();
        assert_eq!(p.min_matched, 10);
        assert_eq!(p.min_log_odds.to_bits(), 0x4028_0000_0000_0000);
        assert_eq!(p.max_rms_px.to_bits(), 0x4008_0000_0000_0000);

        for (matched, n_image, n_cat, tol, area, log_odds_bits, chance_bits) in [
            (118usize, 502usize, 300usize, TOL, AREA, 0x4070_ce96_6dba_dc69u64, 0x3fce_1fbb_a291_d268u64),
            (10, 500, 300, TOL, AREA, 0x4029_eaa1_6fce_cb6a, 0x3fce_0102_482b_3fb8),
            (40, 500, 300, TOL, AREA, 0x4052_4daf_0817_f378, 0x3fce_0102_482b_3fb8),
            (120, 500, 300, TOL, AREA, 0x4071_287d_b841_7a9d, 0x3fce_0102_482b_3fb8),
            (3, 500, 300, 0.05, AREA, 0x0000_0000_0000_0000, 0x4072_fca3_71ab_5e52),
            (20, 500, 300, TOL * 20.0, AREA, 0x0000_0000_0000_0000, 0x4057_70c9_c861_c9c8),
            (6, 500, 300, 0.02, AREA, 0x0000_0000_0000_0000, 0x4048_4d9e_0223_0817),
            (0, 500, 300, TOL, AREA, 0x0000_0000_0000_0000, 0x0000_0000_0000_0000),
            (4, 502, 300, TOL, AREA, 0x400f_e485_2b55_dce2, 0x3fce_1fbb_a291_d268),
        ] {
            let c = confidence(matched, n_image, n_cat, tol, area);
            assert_eq!(
                c.log_odds.to_bits(),
                log_odds_bits,
                "confidence({matched}, {n_image}, {n_cat}, ..) log_odds moved to {}",
                c.log_odds
            );
            assert_eq!(
                c.chance_matches.to_bits(),
                chance_bits,
                "confidence({matched}, {n_image}, {n_cat}, ..) chance_matches moved to {}",
                c.chance_matches
            );
            assert_eq!(c.matched, matched);
        }
    }

    #[test]
    fn the_default_threshold_rejects_a_plausible_chance_configuration() {
        // The property that protects the horizon probe: a configuration that
        // could arise by chance must not be reported as solved.
        let c = confidence(6, 500, 300, 0.02, AREA);
        assert!(!accept(&c, 0.5, &AcceptParams::default()));
    }
}
