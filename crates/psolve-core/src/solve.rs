//! The pipeline: FITS bytes plus catalogue stars in, a verified WCS out.
//!
//! The catalogue is PASSED IN rather than looked up, because this crate has no
//! filesystem access (Task 1's structural guarantee). The caller opens the
//! index, fetches the stars for the hinted region, and hands them over. That is
//! not a compromise -- it is what lets the whole pipeline be tested against a
//! synthetic catalogue with no index on disk.

use crate::background;
use crate::error::ReasonCode;
use crate::extract::{self, ExtractParams, Rejections};
use crate::fit::{self, FitResult, Wcs};
use crate::fits;
use crate::match_::{self, MatchParams};
use crate::project;
use crate::quad;
use crate::verify::{self, AcceptParams, Confidence};

/// A catalogue star as the solver needs it. Positions are at the catalogue's
/// epoch; proper motion is applied here using the frame's DATE-OBS.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CatalogStar {
    pub ra: f64,
    pub dec: f64,
    pub mag: f32,
    pub pmra: f32,
    pub pmdec: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SolveOptions {
    pub hint: Option<(f64, f64)>,
    pub scale_arcsec: Option<f64>,
    /// Catalogue epoch in decimal years; Gaia DR3 is 2016.0.
    pub catalog_epoch: f64,
    /// Saturation level. `None` derives it from the decoded data range, which
    /// is what you want: a 16-bit frame clips near 65535 while a Siril float
    /// frame clips near 1.0, and BSCALE/BZERO can move either.
    pub saturation: Option<f32>,
    pub extract: ExtractParams,
    pub match_: MatchParams,
    pub accept: AcceptParams,
    pub bg_tile: usize,
    pub max_quads: usize,
    /// Whether to fall back to pairwise-separation matching when no quad
    /// budget produced a transform.
    ///
    /// **Off by default, and that is deliberate.** Pair matching is the
    /// LAST resort in the retry ladder, after the scale/binning retry and
    /// the matched-filter re-extraction, both of which are cheaper and more
    /// often the cause. Defaulting this on made `solve_prepared` take the
    /// pair path on its FIRST attempt, which pre-empted those retries: 41
    /// corpus frames that had solved through the binning retry began
    /// solving through pair matching instead. They still solved and still
    /// agreed with ASTAP, but "a frame that solves today cannot change its
    /// route" had quietly stopped being true. The caller turns this on for
    /// the final attempt only -- see `psolve-cli`'s
    /// `solve_with_binning_retry`.
    pub pair_retry: bool,
    pub pairmatch: crate::pairmatch::PairMatchParams,
}

impl Default for SolveOptions {
    fn default() -> Self {
        SolveOptions {
            hint: None,
            scale_arcsec: None,
            catalog_epoch: 2016.0,
            saturation: None,
            extract: ExtractParams::default(),
            match_: MatchParams::default(),
            accept: AcceptParams::default(),
            bg_tile: 96,
            max_quads: 600,
            pair_retry: false,
            pairmatch: crate::pairmatch::PairMatchParams::default(),
        }
    }
}

/// The budget a `NoQuadMatch` is retried at, when the first attempt used less.
///
/// `max_quads` caps how many quads each side contributes. At the default 600
/// the image and catalogue quad sets are drawn from populations of very
/// different size in a dense field and do not overlap: seven ATR585M frames
/// with 274-500 usable stars, an exact hint and a plate scale correct to 0.36%
/// found **zero** matching codes out of 720,000 comparisons. Raising the
/// budget makes them overlap.
///
/// **1500 is measured, not derived.** Across the 19 frames ASTAP solves and
/// psolve does not: 600 recovers 0, **1500 recovers 5**, 3000 recovers 6 for
/// 2.4x the time. All 41 control frames were unaffected at every value.
/// Matching is `O(image_quads * catalogue_quads)`, so 600 -> 1500 is 6.25x the
/// comparisons -- which is why this is a retry rather than a raised default:
/// a frame that solves at its given budget never reaches it, and its answer
/// therefore cannot change.
///
/// ASTAP does not need an equivalent because it forms exactly one quad per
/// star, so its budget scales with the star count. See
/// `docs/superpowers/2026-08-24-astap-algorithm-comparison.md`.
pub const QUAD_RETRY_BUDGET: usize = 1500;

#[derive(Debug, Clone, PartialEq)]
pub struct Solution {
    pub wcs: Wcs,
    pub confidence: Confidence,
    pub fit: FitResult,
    /// (median FWHM px, median ellipticity, median position angle deg) -- the
    /// focus and tracking sensors that come free with solving.
    pub quality: Option<(f64, f64, f64)>,
    pub stars_detected: u32,
    pub stars_used: usize,
    pub stars_matched: usize,
    /// The quad budget that actually produced this solution -- `opts.max_quads`
    /// normally, [`QUAD_RETRY_BUDGET`] when the retry answered. Reported for
    /// the same reason `scale_source` is: a caller comparing two runs needs to
    /// know a parameter moved, and a silently-retried solve is
    /// indistinguishable from a first-attempt one without it.
    pub quad_budget: usize,
    /// Which matcher produced this solution: `"quad"` or `"pair"`. Reported
    /// for the same reason `quad_budget` is -- a caller comparing two runs
    /// needs to see that a different path answered, not infer it.
    pub matcher: &'static str,
    /// The pair-matching retry's own diagnostics, when it was the matcher
    /// that answered. `None` when quads answered.
    pub pair: Option<crate::pairmatch::PairMatchResult>,
    /// The image-side concentration `stratified_keep` gated on -- `None`
    /// when it could not have determined the outcome or was too small a
    /// sample to mean anything; see `extract::Extraction::concentration`'s
    /// doc for both conditions. Reported so a future selection regression
    /// is visible in the JSON rather than only inferable from a corpus run.
    pub concentration: Option<f64>,
    /// Whether image-side stratified selection actually ran for this frame.
    /// Well-defined even when `concentration` is `None`.
    pub stratified: bool,
    pub rejected: Rejections,
    pub mirrored: bool,
    pub epoch_years: Option<f64>,
    pub pm_years_applied: f64,
    /// 2 when a CFA mosaic was superpixel-binned during decode, else 1.
    /// The WCS below is expressed in FILE pixel coordinates regardless, but a
    /// consumer needs this to interpret the star pixel positions.
    pub binned: u32,
    /// Frame dimensions in FILE pixel coordinates -- i.e. NAXIS1/NAXIS2,
    /// already undoing any CFA superpixel binning, consistent with `wcs`
    /// which is likewise reported in FILE coordinates. A caller that wants
    /// the sky position of the image centre (as opposed to `wcs.crval`,
    /// which `fit_tan` pins to the caller's pointing hint, not the image
    /// centre) computes `wcs.pix_to_radec((nx as f64 - 1.0) / 2.0, (ny as
    /// f64 - 1.0) / 2.0)`. The `-1` matters: this crate's pixel frame is
    /// 0-based (extraction centroids over `0..nx`/`0..ny`), so the last
    /// valid coordinate is `nx - 1`, not `nx` -- unlike FITS's 1-based
    /// CRPIX convention. Dropping the `-1` shifts the computed centre by
    /// half a pixel.
    pub nx: usize,
    pub ny: usize,
    /// Milliseconds per stage, measured with `std::time::Instant`. That is
    /// direct `std` -- no external dependency is needed to read a clock, so
    /// (deviation from a stale draft comment: see the task report) there is
    /// no `SolveOptions.clock`; the crate reads the clock itself.
    pub timings_ms: Timings,
}

/// Per-stage wall-clock time inside `solve()`, in milliseconds. `total` is
/// measured from the start of `prepare()` (before the catalogue is even
/// touched) through the end of `solve_prepared()`, so unlike the eight
/// stage timings below it also spans the catalogue disc query that a
/// caller using the `prepare()`/`solve_prepared()` split runs in between
/// the two calls -- that query is otherwise uninstrumented. The residual
/// gap between `total` and the sum of the eight stages is that query;
/// measured on the CLI's own benchmark frame it is about 1.05 ms. See
/// `docs/astap-compat.md`'s "timings_ms" section for the full breakdown.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Timings {
    pub decode: f64,
    pub background: f64,
    pub extract: f64,
    /// Time spent in the CALLER between preparing the frame and starting
    /// this attempt: the catalogue index disc query, and -- when this is a
    /// retry -- every earlier attempt that failed.
    ///
    /// Without it the stages do not add up, and nothing says why.
    /// `PreparedFrame::t_start` is set once in [`prepare`] and never reset,
    /// so `total` spans EVERY attempt while the stage numbers describe only
    /// the last one. Measured over 25 corpus frames: the shortfall is 1.5 ms
    /// on a frame that solves first time and **124.8 ms** on one that solves
    /// through the binning retry. A reader profiling the second frame sees
    /// 78% of the solve unattributed and goes looking for a bottleneck that
    /// does not exist -- which is exactly what happened before this field
    /// was added.
    ///
    /// Measured, not derived as `total` minus the rest: a residual would
    /// silently absorb any future unaccounted time and keep claiming to be
    /// the catalogue fetch.
    pub caller: f64,
    pub quads: f64,
    pub catalogue: f64,
    pub match_: f64,
    pub fit: f64,
    pub verify: f64,
    /// Wall time since [`prepare`] began -- the whole invocation including
    /// any retries, NOT this attempt alone. Equals the sum of every other
    /// field above.
    pub total: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Solved(Box<Solution>),
    Failed {
        reason: ReasonCode,
        detail: String,
        stars_detected: u32,
        stars_used: usize,
        /// Same statistic as `Solution::concentration`, `None` under the
        /// same conditions (including failures that never reach extraction
        /// at all).
        concentration: Option<f64>,
        /// Same as `Solution::stratified`. `false` on any failure that
        /// never reached extraction.
        stratified: bool,
        rejected: Rejections,
    },
}

/// Derive a saturation level from the decoded data itself, when the caller
/// did not supply one.
///
/// Clipping has a signature: many pixels pinned at the same maximum. A frame
/// whose brightest value occurs once is not clipped, whatever its bit depth.
/// This works for 12-, 14- and 16-bit data and for normalised floats without
/// knowing which it is -- unlike a hardcoded ceiling picked by thresholding
/// on the data range, which is INERT on any bit depth the threshold does not
/// anticipate (12- and 14-bit ADUs written without bit-shifting, for
/// instance) while `rejected.saturated` still reports 0 as though the check
/// ran. `f32::INFINITY` when nothing is clipped is honest: the check ran and
/// found nothing, rather than being unreachable.
///
/// Exposed (not private to `solve`) so a caller that needs to know the
/// detected star count before it has decided on a catalogue depth -- sizing
/// `--cat-limit`, say -- can run the same extraction the solver will.
pub fn default_saturation(img: &fits::Image) -> f32 {
    let vmax = img.px.iter().copied().fold(f32::MIN, f32::max);
    let at_max = img.px.iter().filter(|v| **v >= vmax * (1.0 - 1e-6)).count();
    if at_max >= 8 { vmax * (1.0 - 1e-6) } else { f32::INFINITY }
}

pub fn solve(fits_bytes: &[u8], catalog: &[CatalogStar], opts: &SolveOptions) -> Outcome {
    match prepare(fits_bytes, opts) {
        Ok(prepared) => solve_prepared(&prepared, catalog, opts),
        Err(outcome) => outcome,
    }
}

/// Everything [`solve`] derives from the frame **alone**, before a catalogue
/// is involved at all: the parsed header, the decoded image, the background
/// estimate and the extracted stars. Produced by [`prepare`], consumed by
/// [`solve_prepared`].
///
/// It exists so a caller can size its catalogue query to the frame's own
/// star count without paying for the decode/background/extract twice. The
/// CLI's `--cat-limit` default is exactly that: "about 3x this frame's
/// detected stars". Getting that number used to mean a whole second
/// decode+background+extract pass ahead of [`solve`], which on a 3840x2160
/// frame cost ~67 ms -- around 40% of the entire solve, and unavoidable in
/// ASTAP-compatibility mode, which has no flag to pass an explicit limit.
///
/// The split changes no arithmetic: [`solve`] is now literally [`prepare`]
/// followed by [`solve_prepared`], so a caller that does not need the star
/// count up front sees exactly the behaviour it always did.
pub struct PreparedFrame {
    header: fits::FitsHeader,
    img: fits::Image,
    ex: extract::Extraction,
    ms_decode: f64,
    ms_background: f64,
    ms_extract: f64,
    t_start: std::time::Instant,
}

impl PreparedFrame {
    /// Detections that survived every rejection filter -- the stars the
    /// solve will actually try to match. This is the number to size a
    /// catalogue query against.
    pub fn usable_star_count(&self) -> usize {
        self.ex.stars.len()
    }

    /// This frame's measured IMAGE-side spatial concentration -- see
    /// `extract::Extraction::concentration`'s doc for when this is `None`.
    /// This is what gates `stratified_keep` internally; it is exposed here
    /// mainly for a caller that wants to report or log it ahead of the
    /// solve. It is NOT what a caller should use to gate a catalogue-side
    /// stratified-vs-brightest choice -- the catalogue's own spatial
    /// distribution can diverge from the image's (a dense catalogue clump
    /// with no image counterpart at all, say), so `psolve-cli`'s
    /// `cmd_solve::select_catalog` measures the catalogue side
    /// independently rather than reusing this value.
    pub fn concentration(&self) -> Option<f64> {
        self.ex.concentration
    }

    /// Whether image-side stratified selection actually ran for this frame.
    pub fn stratified(&self) -> bool {
        self.ex.stratified
    }

    /// Every source found above threshold, before rejection filtering.
    pub fn detected_star_count(&self) -> u32 {
        self.ex.detected
    }

    /// The pixel positions of every USABLE detection, in the same
    /// (possibly CFA-superpixel-binned) coordinate frame [`solve_prepared`]
    /// builds its own image quads from.
    ///
    /// Exposed for Task 7's blind-solve orchestration (`psolve-cli`). Blind
    /// solving needs to build image quads and query a `.psqidx` code-space
    /// index with them BEFORE any hint, catalogue, or `SolveOptions.hint`
    /// exists -- the code-space lookup lives in `psolve-index`, a
    /// filesystem-touching crate this one cannot depend on (`lib.rs`'s
    /// zero-dependency, no-filesystem guarantee), so the CLI is the only
    /// place index lookup and this crate's own quad/fit primitives
    /// (`quad::build_quads`, `blind::candidate_transform`) can be
    /// orchestrated together. A defensive copy, not a borrow: `ex.stars`
    /// stays private, and `Star` carries fields this doesn't need to expose.
    pub fn image_points(&self) -> Vec<(f64, f64)> {
        self.ex.stars.iter().map(|s| (s.x, s.y)).collect()
    }

    /// `(nx, ny)` of the coordinate frame [`PreparedFrame::image_points`]
    /// (and any quads built from it) lives in -- the possibly CFA
    /// superpixel-binned DECODE grid, **not necessarily** the FILE pixel
    /// dimensions ([`Solution::nx`]/[`Solution::ny`], which undo that
    /// binning for the final reported WCS). A caller projecting a
    /// candidate transform's `Wcs` to, say, the image centre (to seed a
    /// blind search's next catalogue disc) must evaluate it at THIS
    /// frame's centre, or a binned frame's seed pointing is systematically
    /// off.
    pub fn image_dims(&self) -> (usize, usize) {
        (self.img.nx, self.img.ny)
    }

    /// The plate scale [`solve_prepared`] would fall back to when the
    /// caller passes no explicit `SolveOptions.scale_arcsec` --
    /// `FOCALLEN`/`XPIXSZ`, already corrected for CFA superpixel binning the
    /// same way `solve_prepared`'s own fallback is. `None` when the header
    /// lacks those keywords.
    ///
    /// Exposed for Task 7's blind orchestration, which needs an estimate of
    /// an image quad's ON-SKY diagonal (to pick which `.psqidx` scale band
    /// to search) before any hint or catalogue exists to derive one from --
    /// the same header-only derivation [`solve_prepared`] already does
    /// internally, exposed here rather than duplicated.
    pub fn header_scale_arcsec(&self) -> Option<f64> {
        fits::pixel_scale_arcsec(&self.header).map(|s| s * self.img.binned as f64)
    }
}

/// The frame-only half of [`solve`]: parse, decode, estimate the background,
/// extract. `Err` carries the very same [`Outcome::Failed`] [`solve`] would
/// have returned for a frame that cannot get as far as needing a catalogue.
pub fn prepare(fits_bytes: &[u8], opts: &SolveOptions) -> Result<PreparedFrame, Outcome> {
    let fail = |reason: ReasonCode, detail: String| Outcome::Failed {
        reason,
        detail,
        stars_detected: 0,
        stars_used: 0,
        // No extraction ran yet -- there is nothing to report.
        concentration: None,
        stratified: false,
        rejected: Rejections::default(),
    };

    let t_start = std::time::Instant::now();
    let ms_since = |t: std::time::Instant| t.elapsed().as_secs_f64() * 1000.0;

    let header = match fits::FitsHeader::parse(fits_bytes) {
        Ok(h) => h,
        Err(e) => return Err(fail(e.reason(), e.to_string())),
    };
    let img = match fits::decode(fits_bytes, &header) {
        Ok(i) => i,
        Err(e) => return Err(fail(e.reason(), e.to_string())),
    };
    let ms_decode = ms_since(t_start);

    let t_stage = std::time::Instant::now();
    let bg = background::estimate(&img, opts.bg_tile);
    let ms_background = ms_since(t_stage);

    // Saturation is format-dependent, so it cannot be a constant inside the
    // extractor. A hardcoded 16-bit ceiling is INERT on a float frame, and an
    // inert check counts no rejection at all -- nothing in the output would say
    // it never ran.
    let t_stage = std::time::Instant::now();
    let mut ep = opts.extract;
    ep.saturation = opts.saturation.unwrap_or_else(|| default_saturation(&img));
    let ex = extract::extract(&img, &bg, &ep);
    let ms_extract = ms_since(t_stage);

    let fail_ex = |reason: ReasonCode, detail: String| Outcome::Failed {
        reason,
        detail,
        stars_detected: ex.detected,
        stars_used: ex.stars.len(),
        concentration: ex.concentration,
        stratified: ex.stratified,
        rejected: ex.rejected,
    };

    if ex.detected == 0 {
        return Err(fail_ex(ReasonCode::NoStars, "no sources above threshold".into()));
    }
    if ex.stars.is_empty() {
        // Everything found was rejected. If most of it was extended, say so --
        // that is the nebula case, and it is a different problem from cloud.
        let r = ex.rejected;
        let reason = if r.extended >= r.too_small && r.extended > 0 {
            ReasonCode::ExtendedOnly
        } else {
            ReasonCode::TooFewStars
        };
        return Err(fail_ex(reason, format!("all {} detections rejected", ex.detected)));
    }
    if ex.stars.len() < 4 {
        return Err(fail_ex(
            ReasonCode::TooFewStars,
            format!("{} usable stars, need at least 4", ex.stars.len()),
        ));
    }

    Ok(PreparedFrame { header, img, ex, ms_decode, ms_background, ms_extract, t_start })
}

/// The catalogue-dependent half of [`solve`], run against an already
/// [`prepare`]d frame: project, build quads, match, fit, verify.
///
/// The reported `timings_ms` carry [`prepare`]'s own decode/background/
/// extract measurements through unchanged, and `total` spans from the start
/// of [`prepare`] -- so it also covers whatever the caller did in between
/// (fetching the catalogue, typically).
pub fn solve_prepared(
    prepared: &PreparedFrame,
    catalog: &[CatalogStar],
    opts: &SolveOptions,
) -> Outcome {
    let PreparedFrame { header, img, ex, ms_decode, ms_background, ms_extract, t_start } = prepared;
    let (ms_decode, ms_background, ms_extract, t_start) =
        (*ms_decode, *ms_background, *ms_extract, *t_start);
    let ms_since = |t: std::time::Instant| t.elapsed().as_secs_f64() * 1000.0;
    // Everything the caller did between `prepare` returning and this attempt
    // beginning. `prepare`'s own three stages partition its duration, so
    // subtracting them from the elapsed time leaves exactly that interval --
    // the disc query on a first attempt, plus every failed attempt on a
    // retry. See `Timings::caller`.
    let ms_caller = (ms_since(t_start) - (ms_decode + ms_background + ms_extract)).max(0.0);

    let fail_ex = |reason: ReasonCode, detail: String| Outcome::Failed {
        reason,
        detail,
        stars_detected: ex.detected,
        stars_used: ex.stars.len(),
        concentration: ex.concentration,
        stratified: ex.stratified,
        rejected: ex.rejected,
    };

    // An empty catalogue cannot be matched against no matter where we think
    // the telescope was pointed -- that is the more fundamental problem, so
    // it is reported before a missing hint is. (Deviation from the brief: see
    // the task report -- the brief's code checked the hint first, which
    // reports FOV_MISMATCH for a frame that has no catalogue at all, and
    // separately reports INDEX_TOO_SHALLOW for `cat_pts.len() < 4` even when
    // the caller passed zero stars, neither of which is NO_QUAD_MATCH.)
    if catalog.is_empty() {
        return fail_ex(ReasonCode::NoQuadMatch, "no catalogue stars supplied".into());
    }

    // Pixel scale: the option wins, else the optics keywords.
    let scale_arcsec = opts
        .scale_arcsec
        .or_else(|| fits::pixel_scale_arcsec(header).map(|s| s * img.binned as f64));
    let hint = opts.hint.or_else(|| fits::hint_radec(header));
    let Some((ra0, dec0)) = hint else {
        // NOT FovMismatch: that code means a hint WAS available but disagreed
        // with what was found, which is a data problem a caller might
        // reasonably ignore or retry. This is a broken invocation (or a
        // frame format with no supported pointing keyword) -- reporting it
        // as FOV_MISMATCH told a caller branching on `reason` that the field
        // of view disagreed when in fact no hint was ever supplied.
        return fail_ex(
            ReasonCode::NoHint,
            "no pointing hint: pass --hint or supply OBJCTRA/OBJCTDEC or RA/DEC".into(),
        );
    };

    // Proper motion to the frame's epoch. Gaia is epoch 2016.0 and frames are
    // taken years later; over a decade this is a real correction.
    let epoch_years = fits::epoch_years(header);
    let pm_years = epoch_years.map(|y| y - opts.catalog_epoch).unwrap_or(0.0);

    // Project the catalogue into a tangent plane at the hint.
    let t_stage = std::time::Instant::now();
    let mut cat_sky: Vec<(f64, f64)> = Vec::with_capacity(catalog.len());
    let mut cat_pts: Vec<(f64, f64)> = Vec::with_capacity(catalog.len());
    for s in catalog {
        let (ra, dec) = project::apply_proper_motion(
            s.ra,
            s.dec,
            s.pmra as f64,
            s.pmdec as f64,
            pm_years,
        );
        if let Some(p) = project::radec_to_tangent(ra, dec, ra0, dec0) {
            cat_sky.push((ra, dec));
            cat_pts.push(p);
        }
    }
    let ms_catalogue = ms_since(t_stage);
    if cat_pts.len() < 4 {
        return fail_ex(
            ReasonCode::IndexTooShallow,
            format!("{} catalogue stars in the search region", cat_pts.len()),
        );
    }

    let image_pts: Vec<(f64, f64)> = ex.stars.iter().map(|s| (s.x, s.y)).collect();

    let mut mp = opts.match_;
    if mp.expected_scale.is_none() {
        mp.expected_scale = scale_arcsec.map(|s| s / 3600.0);
    }

    // Quad budgets to try, in order. The second exists because `max_quads`
    // caps each side independently: in a dense field the image and catalogue
    // quad sets are then drawn from populations of very different size and do
    // not overlap at all. See `QUAD_RETRY_BUDGET`.
    //
    // Ordering matters. The first budget is always the caller's, so a frame
    // that solves at it never evaluates the second and cannot take a
    // different path -- which is what makes the retry regression-free by
    // construction rather than by measurement. A caller already asking for
    // `QUAD_RETRY_BUDGET` or more gets no second attempt, because there would
    // be nothing to raise.
    let mut budgets = vec![opts.max_quads];
    if opts.max_quads < QUAD_RETRY_BUDGET {
        budgets.push(QUAD_RETRY_BUDGET);
    }

    let mut ms_quads = 0.0;
    let mut ms_match = 0.0;
    let mut last_detail: Option<String> = None;
    let mut matched: Option<(match_::MatchResult, usize)> = None;

    for budget in budgets {
        let t_stage = std::time::Instant::now();
        let iq = quad::build_quads(&image_pts, 6, budget);
        let cq = quad::build_quads(&cat_pts, 6, budget);
        ms_quads += ms_since(t_stage);
        if iq.is_empty() || cq.is_empty() {
            // No quads at all is not a budget problem -- a larger cap cannot
            // create points that are not there.
            last_detail = Some("no quads could be formed".into());
            break;
        }

        let t_stage = std::time::Instant::now();
        let found = match_::match_quads(&image_pts, &iq, &cat_pts, &cat_sky, &cq, &mp);
        ms_match += ms_since(t_stage);
        match found {
            Some(m) => {
                matched = Some((m, budget));
                break;
            }
            None => {
                last_detail = Some(format!(
                    "{} image quads vs {} catalogue quads, no consistent transform",
                    iq.len(),
                    cq.len()
                ));
            }
        }
    }

    // No quad budget produced a transform. If the caller has asked for it
    // -- which it does only on the last rung of its retry ladder -- try
    // matching on star PAIRS instead of four-star codes. See `pairmatch`,
    // and see `SolveOptions::pair_retry` for why this is not on by default.
    let mut pair_result: Option<crate::pairmatch::PairMatchResult> = None;
    if matched.is_none() && opts.pair_retry {
        if let Some(scale) = mp.expected_scale {
            let mut pp = opts.pairmatch;
            if pp.scale_deg_per_px <= 0.0 {
                pp.scale_deg_per_px = scale;
            }
            let t_stage = std::time::Instant::now();
            let ran = crate::pairmatch::match_pairs(&image_pts, &cat_pts, &cat_sky, &pp);
            ms_match += ms_since(t_stage);
            // The counts are the diagnosis of a failed solve, so they go in
            // the detail rather than being dropped on the floor.
            let note = match &ran {
                None => "pair matching could not run (plate scale or star count)".to_string(),
                Some(r) if !r.sufficient => format!(
                    "pair matching: best {} inliers of {} needed ({} runner-up), {} hypotheses over {} image and {} catalogue stars{}",
                    r.inliers,
                    pp.min_inliers,
                    r.runner_up,
                    r.hypotheses,
                    r.image_stars,
                    r.cat_stars,
                    if r.aborted {
                        ", abandoned early -- nothing reached the floor"
                    } else if r.truncated {
                        ", stopped at the hypothesis ceiling"
                    } else {
                        ", search exhausted"
                    }
                ),
                Some(_) => String::new(),
            };
            if note.is_empty() {
                pair_result = ran;
            } else {
                last_detail = Some(match last_detail {
                    Some(d) => format!("{d}; {note}"),
                    None => note,
                });
            }
        } else {
            // Saying so matters: the retry is silently unavailable without a
            // plate scale, and a flag accepted but not applied is the defect
            // this codebase keeps paying for.
            last_detail = Some(match last_detail {
                Some(d) => format!("{d}; pair matching needs a plate scale and had none"),
                None => "pair matching needs a plate scale and had none".into(),
            });
        }
    }

    // Which matcher answered. Reported downstream for the same reason
    // `quad_budget` and `scale_source` are: a caller comparing two runs
    // needs to see that a different path produced the answer.
    let matcher = if matched.is_some() { "quad" } else { "pair" };
    let (pairs, quad_budget): (Vec<fit::Correspondence>, usize) = match (&matched, &pair_result) {
        (Some((m, budget)), _) => (m.pairs.iter().map(|c| (c.image, c.sky)).collect(), *budget),
        (None, Some(r)) => (r.pairs.iter().map(|q| (q.image, q.sky)).collect(), 0),
        (None, None) => {
            return fail_ex(
                ReasonCode::NoQuadMatch,
                last_detail.unwrap_or_else(|| "no consistent transform".into()),
            )
        }
    };

    let t_stage = std::time::Instant::now();
    let Some(fitres) = fit::fit_tan(&pairs, ra0, dec0, 3.0) else {
        return fail_ex(
            ReasonCode::NoQuadMatch,
            format!("{} correspondences did not yield a fit", pairs.len()),
        );
    };
    let ms_fit = ms_since(t_stage);

    let t_stage = std::time::Instant::now();

    // Confidence, against the field this frame actually covers.
    //
    // `matched` is deliberately NOT `fitres.used`. A fit of ~12 points against
    // 6 parameters nearly interpolates, so a spatially-clustered coincidence
    // produces a low RMS and a high vote count while predicting nothing else in
    // the frame. The real test of a solution is REPROJECTION: push every
    // catalogue star through the fitted WCS and count how many land on a
    // detected star. A true solution predicts many stars it was never fitted
    // to; a coincidence predicts none.
    let scale_deg = fitres.wcs.scale_arcsec() / 3600.0;
    let field_area = (img.nx as f64 * scale_deg) * (img.ny as f64 * scale_deg);
    let tol_px = 2.0;
    let tol_deg = tol_px * scale_deg;

    let mut reprojected = 0usize;
    for &(cra, cdec) in &cat_sky {
        if let Some((px, py)) = fitres.wcs.radec_to_pix(cra, cdec) {
            if px < -tol_px || py < -tol_px
                || px > img.nx as f64 + tol_px
                || py > img.ny as f64 + tol_px
            {
                continue;
            }
            if ex.stars.iter().any(|s| {
                (s.x - px).abs() <= tol_px && (s.y - py).abs() <= tol_px
            }) {
                reprojected += 1;
            }
        }
    }

    let conf = verify::confidence(
        reprojected,
        ex.stars.len(),
        cat_pts.len(),
        tol_deg,
        field_area.max(1e-9),
    );
    let rms_px = if scale_deg > 0.0 { fitres.rms_deg / scale_deg } else { f64::INFINITY };

    if !verify::accept(&conf, rms_px, &opts.accept) {
        // Deliberately no WCS: a solver that guesses corrupts the caller's
        // measurement with no visible symptom.
        return fail_ex(
            ReasonCode::LowConfidence,
            format!(
                "{} matched, {:.1} decades (need {:.1}), rms {:.2} px (need <= {:.2})",
                conf.matched, conf.log_odds, opts.accept.min_log_odds, rms_px,
                opts.accept.max_rms_px
            ),
        );
    }
    let ms_verify = ms_since(t_stage);

    // The fit ran on the binned grid; a consumer applies the WCS to the FILE.
    // A binned coordinate b covers file pixels 2b and 2b+1, so its centre is
    // at file coordinate 2b + 0.5, and each binned pixel spans two file
    // pixels -- CFA frame + mono frame at the same optics on the same sky
    // reported field of view 2x apart before this conversion, silently: both
    // reported "solved":true, and the emitted crpix/cd described a pixel
    // grid that does not exist in the file.
    let wcs = if img.binned > 1 {
        let f = img.binned as f64;
        Wcs {
            crval: fitres.wcs.crval,
            crpix: [
                fitres.wcs.crpix[0] * f + (f - 1.0) / 2.0,
                fitres.wcs.crpix[1] * f + (f - 1.0) / 2.0,
            ],
            cd: [
                [fitres.wcs.cd[0][0] / f, fitres.wcs.cd[0][1] / f],
                [fitres.wcs.cd[1][0] / f, fitres.wcs.cd[1][1] / f],
            ],
        }
    } else {
        fitres.wcs
    };

    Outcome::Solved(Box::new(Solution {
        wcs,
        confidence: conf,
        fit: fitres,
        quality: extract::quality(&ex.stars),
        stars_detected: ex.detected,
        stars_used: ex.stars.len(),
        stars_matched: reprojected,
        quad_budget,
        matcher,
        pair: pair_result,
        concentration: ex.concentration,
        stratified: ex.stratified,
        rejected: ex.rejected,
        // Handedness comes from the FITTED WCS, not from the matcher.
        //
        // The CD determinant IS the parity -- it is ground truth about the
        // optical train. The matcher's own flag says which catalogue
        // orientation produced the winning cluster, expressed against the
        // tangent plane's axis convention, so it is offset by the projection's
        // handedness. Two sources for one fact is one too many, and this is
        // the authoritative one. Binning does not change handedness (it is a
        // uniform positive scale), so parity is read from either wcs.
        mirrored: fitres.wcs.parity() == crate::fit::Parity::Mirrored,
        epoch_years,
        pm_years_applied: pm_years,
        binned: img.binned,
        // FILE coordinates: img.nx/ny is the (possibly superpixel-binned)
        // decode grid, and `wcs` above has already been converted back to
        // FILE pixels when binned, so the dimensions must match it.
        nx: img.nx * img.binned as usize,
        ny: img.ny * img.binned as usize,
        timings_ms: Timings {
            decode: ms_decode,
            background: ms_background,
            extract: ms_extract,
            caller: ms_caller,
            quads: ms_quads,
            catalogue: ms_catalogue,
            match_: ms_match,
            fit: ms_fit,
            verify: ms_verify,
            total: ms_since(t_start),
        },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal valid FITS bytes with the given header cards and 16-bit data.
    fn fits_with(cards: &[String], nx: usize, ny: usize, px: &[u16]) -> Vec<u8> {
        let mut s = String::new();
        for c in cards {
            s.push_str(&format!("{c:<80}"));
        }
        s.push_str(&format!("{:<80}", "END"));
        while !s.len().is_multiple_of(2880) {
            s.push(' ');
        }
        let mut out = s.into_bytes();
        for i in 0..nx * ny {
            let v = *px.get(i).unwrap_or(&0);
            let stored = (v as i32 - 32768) as i16;
            out.extend_from_slice(&stored.to_be_bytes());
        }
        while !out.len().is_multiple_of(2880) {
            out.push(0);
        }
        out
    }

    fn blank_frame(nx: usize, ny: usize) -> Vec<u8> {
        let cards = vec![
            "SIMPLE  =                    T".to_string(),
            "BITPIX  =                   16".to_string(),
            "NAXIS   =                    2".to_string(),
            format!("NAXIS1  = {nx:>20}"),
            format!("NAXIS2  = {ny:>20}"),
            "BZERO   =                32768".to_string(),
        ];
        fits_with(&cards, nx, ny, &vec![1000u16; nx * ny])
    }

    #[test]
    fn a_frame_with_no_stars_fails_with_no_stars_not_a_panic() {
        let f = blank_frame(128, 128);
        let out = solve(&f, &[], &SolveOptions::default());
        match out {
            Outcome::Failed { reason, .. } => {
                assert!(
                    matches!(reason, ReasonCode::NoStars | ReasonCode::TooFewStars),
                    "got {reason:?}"
                );
            }
            Outcome::Solved(_) => panic!("a blank frame must not solve"),
        }
    }

    #[test]
    fn unreadable_bytes_fail_with_cannot_read() {
        let out = solve(b"this is not a FITS file", &[], &SolveOptions::default());
        match out {
            Outcome::Failed { reason, .. } => assert_eq!(reason, ReasonCode::CannotRead),
            Outcome::Solved(_) => panic!("garbage must not solve"),
        }
    }

    #[test]
    fn a_frame_with_stars_but_no_catalogue_fails_with_no_quad_match() {
        // Stars present, nothing to match them against.
        let (nx, ny) = (256usize, 256usize);
        let mut px = vec![1000u16; nx * ny];
        for i in 0..30 {
            let x = 20 + (i * 37) % 210;
            let y = 20 + (i * 61) % 210;
            for dy in 0..3 {
                for dx in 0..3 {
                    px[(y + dy) * nx + (x + dx)] = 30000;
                }
            }
        }
        let cards = vec![
            "SIMPLE  =                    T".to_string(),
            "BITPIX  =                   16".to_string(),
            "NAXIS   =                    2".to_string(),
            format!("NAXIS1  = {nx:>20}"),
            format!("NAXIS2  = {ny:>20}"),
            "BZERO   =                32768".to_string(),
        ];
        let f = fits_with(&cards, nx, ny, &px);
        let out = solve(&f, &[], &SolveOptions::default());
        match out {
            Outcome::Failed { reason, stars_detected, .. } => {
                assert!(stars_detected > 0, "the extractor should have found stars");
                assert!(
                    matches!(reason, ReasonCode::NoQuadMatch | ReasonCode::TooFewStars),
                    "got {reason:?}"
                );
            }
            Outcome::Solved(_) => panic!("no catalogue means no solution"),
        }
    }

    #[test]
    fn failure_reports_carry_the_rejection_counts() {
        // The diagnostic value of a failed solve lives entirely in these.
        let (nx, ny) = (128usize, 128usize);
        let mut px = vec![1000u16; nx * ny];
        px[64 * nx + 64] = 60000; // a lone hot pixel
        let cards = vec![
            "SIMPLE  =                    T".to_string(),
            "BITPIX  =                   16".to_string(),
            "NAXIS   =                    2".to_string(),
            format!("NAXIS1  = {nx:>20}"),
            format!("NAXIS2  = {ny:>20}"),
            "BZERO   =                32768".to_string(),
        ];
        let f = fits_with(&cards, nx, ny, &px);
        match solve(&f, &[], &SolveOptions::default()) {
            Outcome::Failed { rejected, .. } => {
                assert_eq!(rejected.too_small, 1, "the hot pixel must be counted");
            }
            Outcome::Solved(_) => panic!("one hot pixel is not a solve"),
        }
    }

    #[test]
    fn saturation_default_fires_on_a_frame_with_many_pixels_at_the_maximum() {
        // A 12-bit frame written without bit-shifting clips at 4095, far
        // below the old hardcoded 16-bit constant (65535*0.999) -- exactly
        // the inert-check bug this default replaces. Many pixels pinned at
        // the same maximum is the clipping signature the new default reads,
        // regardless of bit depth.
        let (nx, ny) = (128usize, 128usize);
        let mut px = vec![1000u16; nx * ny];
        for y in 60..68 {
            for x in 60..68 {
                px[y * nx + x] = 4095; // an 8x8 plateau at the clip level
            }
        }
        let cards = vec![
            "SIMPLE  =                    T".to_string(),
            "BITPIX  =                   16".to_string(),
            "NAXIS   =                    2".to_string(),
            format!("NAXIS1  = {nx:>20}"),
            format!("NAXIS2  = {ny:>20}"),
            "BZERO   =                32768".to_string(),
        ];
        let f = fits_with(&cards, nx, ny, &px);
        match solve(&f, &[], &SolveOptions::default()) {
            Outcome::Failed { rejected, .. } => {
                assert!(
                    rejected.saturated >= 1,
                    "a 12-bit-style clip must be caught without a caller-supplied threshold"
                );
            }
            Outcome::Solved(_) => panic!("a single clipped blob with no catalogue must not solve"),
        }
    }

    #[test]
    fn saturation_default_does_not_fire_on_an_ordinary_unclipped_frame() {
        // A frame whose brightest pixel occurs once (a real, non-flat-topped
        // peak) must not be treated as clipped, whatever its raw value.
        let (nx, ny) = (128usize, 128usize);
        let mut px = vec![1000u16; nx * ny];
        px[40 * nx + 40] = 60000; // a single bright pixel, not a plateau
        let cards = vec![
            "SIMPLE  =                    T".to_string(),
            "BITPIX  =                   16".to_string(),
            "NAXIS   =                    2".to_string(),
            format!("NAXIS1  = {nx:>20}"),
            format!("NAXIS2  = {ny:>20}"),
            "BZERO   =                32768".to_string(),
        ];
        let f = fits_with(&cards, nx, ny, &px);
        match solve(&f, &[], &SolveOptions::default()) {
            Outcome::Failed { rejected, .. } => {
                assert_eq!(rejected.saturated, 0, "a lone bright pixel is not clipping");
            }
            Outcome::Solved(_) => panic!("one hot pixel is not a solve"),
        }
    }

    #[test]
    fn an_explicit_hint_overrides_the_header() {
        // Only checks plumbing: with a hint supplied and no catalogue, the
        // failure must be about matching, never about a missing hint.
        let f = blank_frame(128, 128);
        let opts = SolveOptions {
            hint: Some((100.0, -20.0)),
            scale_arcsec: Some(2.4614),
            ..SolveOptions::default()
        };
        match solve(&f, &[], &opts) {
            Outcome::Failed { reason, .. } => {
                assert_ne!(reason, ReasonCode::NoHint, "the hint was supplied");
                assert_ne!(reason, ReasonCode::FovMismatch, "the hint was supplied");
            }
            Outcome::Solved(_) => panic!("blank frame"),
        }
    }

    /// Reproduces the exact defect verbatim: a solve with no hint anywhere
    /// (no `--hint`, no OBJCTRA/OBJCTDEC, no RA/DEC) used to come back
    /// `"reason":"FOV_MISMATCH"` with a detail string that talked about a
    /// missing hint -- the reason code contradicted its own detail, telling a
    /// caller branching on `reason` that the field of view disagreed when in
    /// fact no hint was ever supplied. This is the fixed contract: a
    /// distinct `NO_HINT` reason, and a detail that mentions every header
    /// keyword this crate now understands.
    #[test]
    fn a_missing_hint_reports_no_hint_not_fov_mismatch() {
        let (frame, catalog, _hint) = matching_frame_and_catalogue();
        let opts = SolveOptions {
            hint: None, // matching_frame_and_catalogue's frame has no OBJCTRA/OBJCTDEC or RA/DEC either
            scale_arcsec: Some(TEST_SCALE_DEG * 3600.0),
            saturation: Some(f32::INFINITY),
            ..SolveOptions::default()
        };
        match solve(&frame, &catalog, &opts) {
            Outcome::Failed { reason, detail, .. } => {
                assert_eq!(reason, ReasonCode::NoHint, "detail was: {detail}");
                assert!(
                    detail.contains("RA/DEC") && detail.contains("OBJCTRA"),
                    "detail must mention the newly-supported RA/DEC keys too: {detail}"
                );
            }
            Outcome::Solved(_) => panic!("no hint means no solve, whatever the catalogue says"),
        }
    }

    const TEST_SCALE_DEG: f64 = 2.4614 / 3600.0;
    const TEST_HINT: (f64, f64) = (100.0, -20.0);

    /// A synthetic frame with `n` painted 4x4 star blobs at deterministic
    /// pixel positions, plus the "truth" WCS relating those pixels to sky.
    /// Shared fixture behind both `an_unrelated_catalogue_does_not_produce_a_solve`
    /// and `reprojection_counts_stars_the_fit_never_saw`.
    fn painted_frame(nx: usize, ny: usize, n: usize) -> (Vec<u8>, Vec<(f64, f64)>, Wcs) {
        // `crpix` here is `[nx/2, ny/2]`, half a pixel off the true 0-based
        // image centre `[(nx-1)/2, (ny-1)/2]` -- the pre-fix convention (see
        // `sidecar.rs`'s "CRPIX convention" doc in psolve-cli). Harmless and
        // deliberate: `real_px` below is generated through this same WCS, so
        // the offset cancels exactly and the assertions are nowhere near
        // tight enough to notice. Not a statement about where the centre is.
        let truth = Wcs {
            crval: [TEST_HINT.0, TEST_HINT.1],
            crpix: [nx as f64 / 2.0, ny as f64 / 2.0],
            cd: [[-TEST_SCALE_DEG, 0.0], [0.0, TEST_SCALE_DEG]],
        };

        let mut real_px: Vec<(f64, f64)> = Vec::new();
        for i in 0..n {
            let x = 30 + (i * 37) % (nx - 68);
            let y = 30 + (i * 61) % (ny - 68);
            real_px.push((x as f64, y as f64));
        }

        let mut px = vec![1000u16; nx * ny];
        for &(x, y) in &real_px {
            let (xi, yi) = (x as usize, y as usize);
            for dy in 0..4 {
                for dx in 0..4 {
                    px[(yi + dy) * nx + (xi + dx)] = 30000;
                }
            }
        }
        let cards = vec![
            "SIMPLE  =                    T".to_string(),
            "BITPIX  =                   16".to_string(),
            "NAXIS   =                    2".to_string(),
            format!("NAXIS1  = {nx:>20}"),
            format!("NAXIS2  = {ny:>20}"),
            "BZERO   =                32768".to_string(),
        ];
        let f = fits_with(&cards, nx, ny, &px);
        (f, real_px, truth)
    }

    /// Project pixel positions through `truth` into sky positions, as a
    /// catalogue.
    fn catalog_from_pixels(truth: &Wcs, positions: &[(f64, f64)]) -> Vec<CatalogStar> {
        positions
            .iter()
            .map(|&(x, y)| {
                let (ra, dec) = truth.pix_to_radec(x, y);
                CatalogStar { ra, dec, mag: 12.0, pmra: 0.0, pmdec: 0.0 }
            })
            .collect()
    }

    fn opts_for(hint: (f64, f64)) -> SolveOptions {
        SolveOptions {
            hint: Some(hint),
            scale_arcsec: Some(TEST_SCALE_DEG * 3600.0),
            // painted_frame paints every star as a FLAT block of identical
            // pixels -- unlike a real PSF, dozens of pixels legitimately
            // share the exact peak value. default_saturation()'s
            // repeated-maximum heuristic (rightly) reads that as clipping on
            // real data; here it is just how this fixture paints a star, so
            // it is not what these matching/confidence tests are about.
            saturation: Some(f32::INFINITY),
            ..SolveOptions::default()
        }
    }

    /// A frame and a catalogue that genuinely correspond: the catalogue is
    /// built from the painted stars' own true centroids (a 4x4 uniform block
    /// centroids at +1.5,+1.5 from its corner).
    fn matching_frame_and_catalogue() -> (Vec<u8>, Vec<CatalogStar>, (f64, f64)) {
        let (f, real_px, truth) = painted_frame(640, 480, 40);
        let centroids: Vec<(f64, f64)> = real_px.iter().map(|&(x, y)| (x + 1.5, y + 1.5)).collect();
        let catalog = catalog_from_pixels(&truth, &centroids);
        (f, catalog, TEST_HINT)
    }

    /// Carried requirement from Task 9: the matcher deliberately does not
    /// refuse a coincidental correspondence -- it reports evidence and
    /// leaves judgement to `verify::accept`, which this pipeline calls. This
    /// is the end-to-end check that an unrelated catalogue is actually
    /// rejected, not merely "distinguishable" the way match_::tests checks.
    #[test]
    fn an_unrelated_catalogue_does_not_produce_a_solve() {
        let (f, real_catalog, hint) = matching_frame_and_catalogue();
        let (nx, ny) = (640usize, 480usize);
        let (_, real_px, truth) = painted_frame(nx, ny, 40);

        // An unrelated catalogue: same field of view, same star count, at
        // pixel positions that share no star with the painted field.
        let n = real_px.len();
        let unrelated_positions: Vec<(f64, f64)> = (0..n)
            .map(|i| {
                let x = 12 + (i * 53) % (nx - 24);
                let y = 12 + (i * 29) % (ny - 24);
                (x as f64, y as f64)
            })
            .collect();
        let unrelated_catalog = catalog_from_pixels(&truth, &unrelated_positions);

        let opts = opts_for(hint);

        // Sanity check on the fixture itself: the real catalogue must solve,
        // or a failure below would prove nothing about discrimination.
        match solve(&f, &real_catalog, &opts) {
            Outcome::Solved(_) => {}
            Outcome::Failed { reason, detail, .. } => {
                panic!("fixture is broken: the real catalogue should solve, got {reason:?}: {detail}");
            }
        }

        match solve(&f, &unrelated_catalog, &opts) {
            Outcome::Failed { reason, .. } => {
                // NOTE: this fixture is rejected by the QUAD MATCHER, not by the
                // reprojection gate -- verified by reverting `matched` to
                // `fitres.used` and observing no change. It pins the
                // end-to-end property, not the gate.
                // `reprojection_counts_stars_the_fit_never_saw` covers that.
                assert!(
                    matches!(reason, ReasonCode::NoQuadMatch | ReasonCode::LowConfidence),
                    "expected the matcher or the confidence gate to reject, got {reason:?}"
                );
            }
            Outcome::Solved(_) => panic!("an unrelated catalogue must not produce a solve"),
        }
    }

    /// This is what makes the reprojection gate discriminating, and it is the
    /// property that a revert to `fitres.used` would destroy: there,
    /// `stars_matched` could only ever equal the fitted-point count.
    ///
    /// Build a frame and a catalogue that agree, solve it, and require the
    /// reprojection count to exceed the number of correspondences the fit
    /// actually used. A coincidence cannot do this -- it predicts nothing
    /// outside the points it was fitted to.
    #[test]
    fn reprojection_counts_stars_the_fit_never_saw() {
        let (frame, catalog, hint) = matching_frame_and_catalogue();
        match solve(&frame, &catalog, &opts_for(hint)) {
            Outcome::Solved(s) => {
                assert!(
                    s.stars_matched > s.fit.used,
                    "reprojection found {} stars against {} fitted -- it is not \
                     doing independent work",
                    s.stars_matched,
                    s.fit.used
                );
                assert!(s.stars_matched >= 10);
            }
            Outcome::Failed { reason, detail, .. } => {
                panic!("the matching fixture must solve: {reason} -- {detail}")
            }
        }
    }

    /// `nx`/`ny` are the FILE pixel dimensions -- what a caller needs to
    /// compute the sky position of the image centre via
    /// `wcs.pix_to_radec(nx/2, ny/2)`. This is the fixture that
    /// `matching_frame_and_catalogue` paints at 640x480 with `binned == 1`,
    /// so the FILE dimensions equal the decode dimensions exactly.
    #[test]
    fn solution_reports_file_pixel_dimensions() {
        let (frame, catalog, hint) = matching_frame_and_catalogue();
        match solve(&frame, &catalog, &opts_for(hint)) {
            Outcome::Solved(s) => {
                assert_eq!(s.binned, 1, "fixture is unbinned mono data");
                assert_eq!((s.nx, s.ny), (640, 480));
            }
            Outcome::Failed { reason, detail, .. } => {
                panic!("the matching fixture must solve: {reason} -- {detail}")
            }
        }
    }

    /// Every stage must report a finite, non-negative duration, and they must
    /// sum to (approximately) the crate's own total -- otherwise a stage was
    /// left unmeasured (silently 0.0, indistinguishable from "fast") or
    /// double-counted.
    #[test]
    fn timings_cover_every_stage_and_sum_to_the_total() {
        let (frame, catalog, hint) = matching_frame_and_catalogue();
        match solve(&frame, &catalog, &opts_for(hint)) {
            Outcome::Solved(s) => {
                let t = s.timings_ms;
                for (name, v) in [
                    ("decode", t.decode),
                    ("background", t.background),
                    ("extract", t.extract),
                    ("quads", t.quads),
                    ("catalogue", t.catalogue),
                    ("match_", t.match_),
                    ("fit", t.fit),
                    ("verify", t.verify),
                    ("total", t.total),
                ] {
                    assert!(v.is_finite() && v >= 0.0, "{name} timing was {v}");
                }
                let sum = t.decode
                    + t.background
                    + t.extract
                    + t.quads
                    + t.catalogue
                    + t.match_
                    + t.fit
                    + t.verify;
                assert!(
                    sum <= t.total + 1.0,
                    "stages summed to {sum} ms but total was only {} ms",
                    t.total
                );
            }
            Outcome::Failed { reason, detail, .. } => {
                panic!("the matching fixture must solve: {reason} -- {detail}")
            }
        }
    }
}
