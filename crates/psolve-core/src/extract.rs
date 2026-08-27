//! Star extraction: threshold, connected components, rejection, centroid.
//!
//! Every rejection is COUNTED BY REASON. Those counts are the whole diagnostic
//! value of a failed solve -- "clouds" and "the nebula swamped it" look
//! identical unless they are separate numbers.

use crate::background::Background;
use crate::fits::Image;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Star {
    pub x: f64,
    pub y: f64,
    pub flux: f64,
    pub peak: f32,
    pub npix: u32,
    pub fwhm_px: f64,
    pub ellipticity: f64,
    /// Position angle of the major axis, degrees CCW from +x. Meaningful only
    /// when ellipticity is non-trivial; a systematic value across a frame is
    /// guiding drift, which is why it is reported at all.
    pub theta_deg: f64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Rejections {
    pub too_small: u32,
    pub extended: u32,
    pub saturated: u32,
    pub elongated: u32,
    pub edge: u32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ExtractParams {
    pub k_sigma: f32,
    pub min_pix: u32,
    /// Gaussian sigma, in pixels, of a **matched filter** applied before
    /// thresholding. `0.0` disables it, which is the default.
    ///
    /// Convolving with a kernel matched to the PSF is the textbook optimal
    /// detector for point sources -- SExtractor does exactly this, and so does
    /// the blind-solver this project's quad approach derives from (its domain
    /// name is deliberately not written here: `no_filesystem.rs` tokenises
    /// comments and the suffix is on its forbidden list). Detection runs
    /// on the filtered image and every measurement still runs on the original.
    ///
    /// **Off by default because it is not free in either time or quality.**
    /// Measured on a real ATR585M frame: extraction 25.3 ms -> 211.3 ms, total
    /// solve 167 ms -> 351 ms. And on frames that already solve it *lowers*
    /// completeness -- 68.6% -> 61.8% across 12 controls -- because a smoothed
    /// image merges close neighbours. It earns its cost only where detection
    /// is the binding constraint: across the frames ASTAP solves and psolve
    /// does not, completeness rose 11.0% -> 28.0%. See
    /// `docs/superpowers/2026-08-24-detection-experiments.md`.
    ///
    /// So callers should enable this as a RETRY after a first attempt has
    /// failed, never as a default.
    pub matched_filter_sigma: f64,
    /// Extended-source cap as a MULTIPLE of the median detection area. A fixed
    /// pixel count is wrong at a different focal length or in different seeing;
    /// a relative cap scales with the frame it is given.
    pub max_pix_factor: f64,
    pub max_ellipticity: f64,
    pub edge_margin: usize,
    pub keep: usize,
    /// Pixel value at or above which a detection is considered clipped.
    ///
    /// This MUST be supplied by the caller, because it is format-dependent: a
    /// 16-bit frame saturates near 65535 while a Siril float frame saturates
    /// near 1.0. A constant here is inert on one of them, and an inert check
    /// counts no rejection at all -- which is worse than a wrong one, because
    /// nothing in the output says it did not run.
    pub saturation: f32,
}

impl Default for ExtractParams {
    fn default() -> Self {
        ExtractParams {
            k_sigma: 5.0,
            min_pix: 4,
            matched_filter_sigma: 0.0,
            max_pix_factor: 25.0,
            max_ellipticity: 0.6,
            edge_margin: 8,
            // The number ASTAP was measured selecting on this rig.
            keep: 500,
            // Right for the 16-bit unsigned frames this project's cameras
            // produce (BZERO 32768, see fits::decode). A caller reading
            // 32-bit float data (e.g. Siril output, which saturates near 1.0)
            // must override this -- see `saturation`'s doc comment.
            saturation: 65535.0 * 0.999,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Extraction {
    pub stars: Vec<Star>,
    pub rejected: Rejections,
    /// Connected components found before any rejection or capping.
    pub detected: u32,
    /// The image-side concentration statistic `stratified_keep` gated on --
    /// see [`concentration_stat`] -- reported ONLY when it was capable of
    /// changing the outcome: `stars.len() > keep` (below that,
    /// `stratified_keep`'s own early return forces the legacy path no
    /// matter what this number says, so reporting it would describe a
    /// decision that was never made) AND `keep >= CONCENTRATION_MIN_N` (the
    /// fixed 8x8 grid needs at least that many candidates for "busiest
    /// cell" to mean anything rather than shot noise -- see
    /// [`CONCENTRATION_MIN_N`]'s doc). `None` otherwise. Fixing a real
    /// defect: an earlier version of this field reported a number
    /// unconditionally, and 780 real corpus frames read >= 5.0 purely from
    /// having as few as 12-30 usable stars spread over 64 cells, 777 of
    /// which never came near the gate.
    pub concentration: Option<f64>,
    /// Whether [`stratified_keep`] actually took the stratified path for
    /// this frame (as opposed to legacy sort+truncate, taken either because
    /// the gate did not fire or because it was never reachable). This is
    /// the fact a consumer actually wants -- "did stratification happen" --
    /// and it is well-defined even when `concentration` is `None`.
    pub stratified: bool,
}

/// One connected component, accumulated during the flood fill.
struct Blob {
    npix: u32,
    peak: f32,
    sum: f64,
    sx: f64,
    sy: f64,
    sxx: f64,
    syy: f64,
    sxy: f64,
    touches_edge: bool,
}

/// Background-subtracted image convolved with a separable Gaussian, plus the
/// factor the per-pixel noise is reduced by.
///
/// The matched filter for a point source is the source's own profile, so a
/// Gaussian of roughly the PSF width maximises detection signal-to-noise.
/// Convolution with a kernel normalised to sum 1 leaves the source peak
/// broadly intact while reducing white noise by the kernel's L2 norm, so the
/// detection threshold must be scaled by the same factor or the filter would
/// merely lower the effective sigma cut.
///
/// Separable: a 2D Gaussian is the outer product of two 1D Gaussians, so this
/// is two passes of `2r+1` taps rather than one of `(2r+1)^2`. Edges clamp to
/// the border pixel, which biases the outermost `r` columns slightly; frames
/// are thousands of pixels wide and `extract` already rejects detections
/// within `edge_margin` of the border, so nothing downstream sees it.
fn matched_filter(img: &Image, bg: &Background, sigma: f64) -> (Vec<f32>, f32) {
    let r = (3.0 * sigma).ceil().max(1.0) as usize;
    let raw: Vec<f64> = (0..=2 * r)
        .map(|i| {
            let d = i as f64 - r as f64;
            (-(d * d) / (2.0 * sigma * sigma)).exp()
        })
        .collect();
    let sum: f64 = raw.iter().sum();
    let k: Vec<f64> = raw.iter().map(|v| v / sum).collect();
    // Noise after a normalised convolution scales by the kernel's L2 norm;
    // separable means the 2D factor is the 1D factor squared.
    let l2 = k.iter().map(|v| v * v).sum::<f64>().sqrt();
    let noise_scale = (l2 * l2) as f32;

    let (nx, ny) = (img.nx, img.ny);
    let mut tmp = vec![0.0f32; nx * ny];
    for y in 0..ny {
        for x in 0..nx {
            let mut acc = 0.0f64;
            for (j, kv) in k.iter().enumerate() {
                let xx = (x as isize + j as isize - r as isize).clamp(0, nx as isize - 1) as usize;
                acc += kv * f64::from(img.px[y * nx + xx] - bg.level_at(xx, y));
            }
            tmp[y * nx + x] = acc as f32;
        }
    }
    let mut out = vec![0.0f32; nx * ny];
    for y in 0..ny {
        for x in 0..nx {
            let mut acc = 0.0f64;
            for (j, kv) in k.iter().enumerate() {
                let yy = (y as isize + j as isize - r as isize).clamp(0, ny as isize - 1) as usize;
                acc += kv * f64::from(tmp[yy * nx + x]);
            }
            out[y * nx + x] = acc as f32;
        }
    }
    (out, noise_scale)
}

pub fn extract(img: &Image, bg: &Background, p: &ExtractParams) -> Extraction {
    let n = img.nx * img.ny;
    let mut mask = vec![false; n];
    // Detect on the filtered image, measure on the original -- the separation
    // SExtractor makes for the same reason: smoothing improves the decision
    // about WHERE a source is and corrupts every measurement OF it.
    let filtered = (p.matched_filter_sigma > 0.0)
        .then(|| matched_filter(img, bg, p.matched_filter_sigma));
    for y in 0..img.ny {
        for x in 0..img.nx {
            let i = y * img.nx + x;
            mask[i] = match &filtered {
                Some((sm, noise_scale)) => sm[i] > p.k_sigma * bg.noise_at(x, y) * *noise_scale,
                None => img.px[i] > bg.level_at(x, y) + p.k_sigma * bg.noise_at(x, y),
            };
        }
    }

    let mut blobs: Vec<Blob> = Vec::new();
    // Iterative fill with an explicit stack: the reference frame contains a
    // 63,104-pixel nebula, and a recursive fill would blow the stack on it.
    let mut stack: Vec<usize> = Vec::with_capacity(1024);

    for seed in 0..n {
        if !mask[seed] {
            continue;
        }
        mask[seed] = false;
        stack.push(seed);
        let mut b = Blob {
            npix: 0, peak: 0.0, sum: 0.0, sx: 0.0, sy: 0.0,
            sxx: 0.0, syy: 0.0, sxy: 0.0, touches_edge: false,
        };
        while let Some(i) = stack.pop() {
            let x = i % img.nx;
            let y = i / img.nx;
            let v = img.px[i];
            let w = (v - bg.level_at(x, y)).max(0.0) as f64;
            b.npix += 1;
            b.sum += w;
            b.sx += x as f64 * w;
            b.sy += y as f64 * w;
            b.sxx += x as f64 * x as f64 * w;
            b.syy += y as f64 * y as f64 * w;
            b.sxy += x as f64 * y as f64 * w;
            if v > b.peak {
                b.peak = v;
            }
            if x < p.edge_margin || y < p.edge_margin
                || x + p.edge_margin >= img.nx || y + p.edge_margin >= img.ny
            {
                b.touches_edge = true;
            }
            let x0 = x.saturating_sub(1);
            let x1 = (x + 1).min(img.nx - 1);
            let y0 = y.saturating_sub(1);
            let y1 = (y + 1).min(img.ny - 1);
            for yy in y0..=y1 {
                for xx in x0..=x1 {
                    let j = yy * img.nx + xx;
                    if mask[j] {
                        mask[j] = false;
                        stack.push(j);
                    }
                }
            }
        }
        blobs.push(b);
    }

    let detected = blobs.len() as u32;

    // The extended-source cap is relative to this frame's own median blob.
    // Median over blobs that are PLAUSIBLY STARS, not over every blob: hot
    // pixels are far more numerous than stars on a noisy frame, and including
    // them collapses the median toward 1, which then rejects the real stars as
    // "extended" -- the exact inversion of this filter's purpose.
    let mut areas: Vec<u32> = blobs.iter().map(|b| b.npix).filter(|n| *n >= p.min_pix).collect();
    areas.sort_unstable();
    let median_area = if areas.is_empty() { 1 } else { areas[areas.len() / 2].max(1) };
    let max_pix = (median_area as f64 * p.max_pix_factor).max(p.min_pix as f64 * 4.0);

    let mut rejected = Rejections::default();
    let mut stars = Vec::with_capacity(blobs.len().min(p.keep));

    for b in &blobs {
        if b.npix < p.min_pix {
            rejected.too_small += 1;
            continue;
        }
        if (b.npix as f64) > max_pix {
            rejected.extended += 1;
            continue;
        }
        if b.peak >= p.saturation {
            rejected.saturated += 1;
            continue;
        }
        if b.touches_edge {
            rejected.edge += 1;
            continue;
        }
        if b.sum <= 0.0 {
            rejected.too_small += 1;
            continue;
        }

        let cx = b.sx / b.sum;
        let cy = b.sy / b.sum;
        // Central second moments.
        let mxx = (b.sxx / b.sum - cx * cx).max(0.0);
        let myy = (b.syy / b.sum - cy * cy).max(0.0);
        let mxy = b.sxy / b.sum - cx * cy;

        // Principal axes of the moment ellipse.
        let common = (((mxx - myy) * (mxx - myy)) / 4.0 + mxy * mxy).max(0.0).sqrt();
        let a2 = ((mxx + myy) / 2.0 + common).max(1e-9);
        let b2 = ((mxx + myy) / 2.0 - common).max(0.0);
        let a = a2.sqrt();
        let bb = b2.sqrt();
        let ellipticity = if a > 0.0 { 1.0 - bb / a } else { 0.0 };

        if ellipticity > p.max_ellipticity {
            rejected.elongated += 1;
            continue;
        }

        // FWHM from the geometric-mean sigma of the two axes.
        let sigma = (a * bb).sqrt().max(1e-6);
        let fwhm_px = 2.354_820_045 * sigma;
        let theta_deg = 0.5 * (2.0 * mxy).atan2(mxx - myy).to_degrees();

        stars.push(Star {
            x: cx,
            y: cy,
            flux: b.sum,
            peak: b.peak,
            npix: b.npix,
            fwhm_px,
            ellipticity,
            theta_deg,
        });
    }

    let pre_selection_len = stars.len();
    let (stars, stratified, concentration_raw) = stratified_keep(stars, img.nx, img.ny, p.keep);
    // See `Extraction::concentration`'s doc for both conditions. Note
    // `pre_selection_len > p.keep` implies `stratified_keep`'s own
    // candidate slice was exactly `p.keep` long (it is
    // `pre_selection_len.min(p.keep)`), so the second condition reduces to
    // checking `p.keep` itself here.
    let concentration = if pre_selection_len > p.keep && p.keep >= CONCENTRATION_MIN_N {
        Some(concentration_raw)
    } else {
        None
    };

    Extraction { stars, rejected, detected, concentration, stratified }
}

/// Fixed grid size for [`concentration_stat`]. Deliberately NOT `g`,
/// `stratified_keep`'s own crowding-adaptive cell count: `g` is a function of
/// `stars.len() / keep`, so two frames with the same spatial distribution but
/// different star counts would get different `g` and therefore incomparable
/// statistics. A fixed grid measures the same thing -- concentration relative
/// to a constant reference -- on every frame.
const CONCENTRATION_GRID: usize = 8;

/// The minimum candidate count for [`concentration_stat`]'s "busiest of 64
/// cells" reading to mean anything rather than shot noise. Below this, most
/// cells are empty by construction and a handful of stars landing in the
/// same cell by chance reads as a large number: a real corpus frame with 21
/// usable stars read `concentration: 12.19` from nothing but which of 64
/// cells 4 of those 21 happened to land in. `CONCENTRATION_GRID^2` -- an
/// average occupancy of at least 1 per cell -- is the natural floor.
///
/// [`stratified_keep`] enforces this against its own decision, not only
/// against what [`Extraction::concentration`] reports -- an unusually small
/// explicit `--keep` (the default, 500, always clears this) would otherwise
/// let a noisy small-`n` reading decide the outcome, which is the same
/// defect as trusting the number at all below this floor.
const CONCENTRATION_MIN_N: usize = CONCENTRATION_GRID * CONCENTRATION_GRID;

/// The IMAGE-side gate: below this, [`stratified_keep`] takes the untouched
/// legacy path (sort by flux, truncate) -- no perturbation, not even a
/// reordering.
///
/// **This is a SEPARATE constant from the catalogue-side threshold**
/// (`psolve-cli`'s `cmd_solve::CATALOG_CONCENTRATION_THRESHOLD`), not the
/// same number reused. An earlier version of this fix shipped a single
/// shared constant on the assumption that normalising both statistics to a
/// ~1.0 uniform baseline made one number meaningful for both; that turned
/// out to be wrong, caught by measuring the two REPORTED distributions
/// directly across the full corpus: they are not just offset from 1.0
/// differently but scaled differently throughout (image-side reads median
/// 2.69, p99 4.22 on this corpus -- a naturally noisier statistic, since a
/// fixed 8x8 pixel grid over the frame is a coarser partition than the
/// catalogue's actual HEALPix cells -- while catalogue-side reads median
/// 1.43, p99 3.97; see that constant's doc for the full distribution).
/// Concretely: applying the catalogue-calibrated value (2.0) to THIS
/// statistic instead made the image gate fire on 6,931 of 9,495 real
/// frames -- almost every ordinary frame -- instead of the 3 it fires on at
/// its own, separately calibrated value. Each side is calibrated against
/// ITS OWN measured distribution instead, and the two are compared only in
/// the sense that both express "multiples of a spatially uniform field".
///
/// Calibrated the same way the catalogue side is (see that constant's doc
/// for the full method): against the 300-frame agreement-corpus sample
/// (median 2.59, p90 4.88 on the brightest-`keep` slice) and the dense
/// targets stratification was built for (rescued targets' medians 6.7-12.0;
/// not-helped targets' max 4.99). On the real corpus this constant almost
/// never decides anything -- see [`stratified_keep`]'s own doc for why --
/// but it is exercised whenever `--keep` is small enough, or a frame dense
/// enough, that more than `keep` detections survive rejection. See
/// `docs/superpowers/2026-08-15-conditional-stratification-results.md` for
/// the full measurement.
pub(crate) const CONCENTRATION_THRESHOLD: f64 = 5.0;

/// The image-side stratified-vs-legacy decision, given a frame's measured
/// image-side concentration. `pub`, not `pub(crate)`, only so
/// `PreparedFrame::concentration()`'s callers can interpret the number they
/// read back -- it is NOT meant to be reused for the catalogue-side
/// decision, which has its own threshold and its own function
/// (`psolve-cli`'s `cmd_solve::catalog_should_stratify`) against a
/// genuinely different population (detected stars in the frame vs.
/// catalogue stars in the search disc; see that function's doc for a real
/// fixture where the two diverge and why gating one from the other's number
/// is wrong regardless of what either threshold is set to).
pub fn should_stratify(concentration: f64) -> bool {
    concentration >= CONCENTRATION_THRESHOLD
}

/// How spatially concentrated a set of detections is: the busiest cell's
/// occupancy on a fixed 8x8 grid, divided by that grid's uniform per-cell
/// share (`n / 64`) -- "the busiest cell holds N times its uniform share".
/// ~1.0 for a spatially uniform field; large for a clump, since a globular's
/// core can put most of a frame's detections in one or two cells. Scale-free
/// (depends only on relative distribution, not absolute star count) and
/// independent of `g` -- see [`CONCENTRATION_GRID`]'s doc for why that
/// matters.
///
/// `nx == 0 || ny == 0 || stars.is_empty()` returns 1.0 (uniform/undefined
/// rather than a divide-by-zero) -- there is no meaningful concentration to
/// report and the caller's degenerate-size handling already takes the legacy
/// path regardless of what this returns. Small non-empty `stars` are NOT
/// specially handled here -- this function always returns a number -- see
/// [`CONCENTRATION_MIN_N`] and `Extraction::concentration`'s doc for where
/// the "is this even meaningful" judgement is applied instead, to the
/// REPORTED value rather than to the gate's own decision.
pub(crate) fn concentration_stat(stars: &[Star], nx: usize, ny: usize) -> f64 {
    if stars.is_empty() || nx == 0 || ny == 0 {
        return 1.0;
    }
    let g = CONCENTRATION_GRID;
    let mut counts = vec![0u32; g * g];
    for s in stars {
        let cx = ((s.x / nx as f64) * g as f64) as usize;
        let cy = ((s.y / ny as f64) * g as f64) as usize;
        counts[cy.min(g - 1) * g + cx.min(g - 1)] += 1;
    }
    let max_count = counts.into_iter().max().unwrap_or(0) as f64;
    let uniform_share = stars.len() as f64 / (g * g) as f64;
    // uniform_share > 0 always holds here: stars is non-empty, so len() >= 1.
    max_count / uniform_share
}

/// Keep `keep` stars, gated on measured spatial concentration. Returns the
/// kept stars, whether the stratified path was actually taken, and the
/// concentration value the decision (or non-decision) was based on.
///
/// Brightness is spatially correlated: in a cluster the brightest 500
/// detections are all core members, packed into a few arcminutes, while the
/// catalogue's brightest spread across the whole field. The two sets then
/// barely overlap geometrically and no consistent transform exists to find.
/// Selecting per cell fixes that, and incidentally improves quad geometry,
/// since quads drawn from one corner constrain a fit weakly.
///
/// **On the real corpus this almost never fires**: only frames with more
/// than `keep` usable detections can reach the gate at all (see the early
/// return just below) -- 7,385 of 9,495 real frames clear that bar (most of
/// this project's rigs produce far more than 500 usable detections on a
/// typical field), but of those only **3** actually cross
/// [`CONCENTRATION_THRESHOLD`] and stratify. Omega Centauri is a case in
/// point: ~4,250 detections survive rejection there, comfortably over
/// `keep`, so the gate DOES run -- but [`concentration_stat`] on the
/// brightest-`keep` slice reads ~2.0, BELOW this module's threshold, so it
/// takes the legacy path anyway and is not rescued by this mechanism. (It
/// is not rescued by anything in this milestone: the established
/// diagnosis, confirmed at every commit since, is that globular failures
/// are upstream of selection entirely -- extreme detection counts defeat
/// quad-matching regardless of which `keep` stars were chosen. See
/// `docs/superpowers/2026-08-15-conditional-stratification-results.md`
/// section on globulars.) On this corpus the decision that actually
/// matters is the catalogue-side one
/// (`psolve-cli::cmd_solve::select_catalog`, gate reachable on effectively
/// every frame and firing on 496 of 9,495) -- see
/// [`CONCENTRATION_THRESHOLD`]'s doc.
///
/// **But on a field that is already close to spatially uniform, stratifying
/// still changes the answer** -- a round-robin across cells reorders
/// selection versus global brightest even when both end up choosing "the
/// same" stars, because `g` clamping to a small value does not make cell
/// traversal order match flux order. Measured against the full agreement
/// corpus, that "near-identity" cost a systematic 0.30" median centroid
/// shift on 9,173 frames that solved fine either way, and 46 outright
/// regressions concentrated in dense-but-not-extreme fields (Cats Paw,
/// Centaurus A). So below [`CONCENTRATION_THRESHOLD`] this function takes
/// the **exact legacy path**: sort by flux descending (already done, next
/// line), truncate, return -- byte-for-byte what pre-stratification
/// `extract()` produced, not an approximation of it.
///
/// Above the threshold, the grid adapts to crowding so that a sparse frame
/// is essentially unaffected, and unfilled cells donate their budget --
/// without that a frame whose signal occupies a minority of cells would
/// silently return far fewer than `keep` stars.
pub(crate) fn stratified_keep(
    mut stars: Vec<Star>,
    nx: usize,
    ny: usize,
    keep: usize,
) -> (Vec<Star>, bool, f64) {
    stars.sort_unstable_by(|p, q| q.flux.partial_cmp(&p.flux).unwrap_or(std::cmp::Ordering::Equal));

    // Concentration of the slice legacy selection would ITSELF return.
    // Computed unconditionally, even in the degenerate/early-return cases
    // below, so there is exactly one place this number comes from -- the
    // caller (`extract()`) decides whether it is meaningful to REPORT, but
    // the value itself always means the same thing.
    let candidate_end = stars.len().min(keep);
    let concentration = concentration_stat(&stars[..candidate_end], nx, ny);

    if keep == 0 || stars.len() <= keep || nx == 0 || ny == 0 {
        stars.truncate(keep);
        return (stars, false, concentration);
    }

    // `candidate_end < CONCENTRATION_MIN_N` gates the DECISION, not only
    // whether `Extraction::concentration` reports a number: with an
    // unusually small explicit `--keep` (the default, 500, always clears
    // this), the busiest-of-64-cells reading is shot noise from having too
    // few candidates for the grid, the same way a wide catalogue-side disc
    // against a modest star count is (see `psolve-cli`'s
    // `catalog_concentration` doc) -- letting a noisy number decide is the
    // same defect whichever side it is measured on.
    if candidate_end < CONCENTRATION_MIN_N || !should_stratify(concentration) {
        // The exact legacy path. `stars` is already flux-sorted above --
        // this must stay a plain truncate of that same sort, not a
        // re-derivation, or "bit-identical below threshold" stops being true.
        stars.truncate(keep);
        return (stars, false, concentration);
    }

    // g^2 cells. sqrt(detected/keep) grows with crowding; the factor of 2 and
    // the cap of 16 keep the cell count sane at both extremes. At 500
    // detections against keep 500 this is g = 2 (near-identity); at 20,575
    // detections it is g = 13, i.e. 169 cells of ~3 stars each.
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
    (out, true, concentration)
}

/// Median FWHM and ellipticity across the kept stars -- the frame-quality
/// sensors that come free with solving. `theta_deg` is the median position
/// angle, whose systematic value across a frame indicates guiding drift.
pub fn quality(stars: &[Star]) -> Option<(f64, f64, f64)> {
    if stars.is_empty() {
        return None;
    }
    let med = |mut v: Vec<f64>| -> f64 {
        v.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        v[v.len() / 2]
    };
    Some((
        med(stars.iter().map(|s| s.fwhm_px).collect()),
        med(stars.iter().map(|s| s.ellipticity).collect()),
        med(stars.iter().map(|s| s.theta_deg).collect()),
    ))
}

#[cfg(test)]
mod tests {

    /// The matched filter must be OFF unless a caller asks for it.
    ///
    /// It costs 25.3 ms -> 211.3 ms of extraction time on a real frame and
    /// *lowers* completeness on frames that already solve (68.6% -> 61.8%
    /// across 12 controls), so a default-on filter would be a regression for
    /// the overwhelming majority of frames. It is a retry, not a default.
    #[test]
    fn the_matched_filter_is_off_by_default() {
        assert_eq!(ExtractParams::default().matched_filter_sigma, 0.0);
    }

    /// With the filter disabled, extraction must be bit-identical to what it
    /// was before the filter existed. This is what makes enabling it a
    /// caller's choice rather than a change to every solve.
    #[test]
    fn a_zero_sigma_changes_nothing() {
        let (img, bg) = faint_field();
        let plain = extract(&img, &bg, &ExtractParams::default());
        let zero = extract(&img, &bg, &ExtractParams { matched_filter_sigma: 0.0, ..ExtractParams::default() });
        assert_eq!(plain.stars.len(), zero.stars.len());
        for (a, b) in plain.stars.iter().zip(zero.stars.iter()) {
            assert_eq!(a, b, "a disabled filter must not perturb extraction at all");
        }
    }

    /// The point of the filter: recover faint point sources that a raw-pixel
    /// threshold misses.
    ///
    /// Measured on real frames, completeness across the frames ASTAP solves
    /// and psolve does not rose 11.0% -> 28.0% with a kernel sigma of 1.5.
    /// This pins the mechanism synthetically: stars too faint to clear
    /// `k_sigma` on any single pixel do clear it once their own flux is
    /// gathered by a PSF-matched kernel.
    #[test]
    fn the_matched_filter_recovers_faint_stars_a_raw_threshold_misses() {
        let (img, bg) = faint_field();
        let plain = extract(&img, &bg, &ExtractParams::default());
        let filtered = extract(
            &img,
            &bg,
            &ExtractParams { matched_filter_sigma: 1.5, ..ExtractParams::default() },
        );
        assert!(
            filtered.stars.len() > plain.stars.len(),
            "matched filtering must find MORE faint stars than a raw-pixel threshold: \
got {} filtered vs {} plain",
            filtered.stars.len(),
            plain.stars.len()
        );
    }

    /// A field of faint stars near the detection limit, plus noise.
    ///
    /// Deterministic: the PRNG is a seeded splitmix64 written inline, because
    /// `psolve-core` has no dependencies -- not even dev-dependencies.
    fn faint_field() -> (Image, Background) {
        const NX: usize = 256;
        const NY: usize = 256;
        let mut px = vec![100.0f32; NX * NY];
        let mut s: u64 = 0x5EED;
        let mut next = || {
            s = s.wrapping_add(0x9E3779B97F4A7C15);
            let mut z = s;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
            ((z ^ (z >> 31)) >> 11) as f64 / (1u64 << 53) as f64
        };
        for v in px.iter_mut() {
            *v += ((next() - 0.5) * 6.0) as f32;
        }
        // Faint stars: peak just under the 5-sigma single-pixel threshold, so
        // no individual pixel clears it, but the integrated flux does.
        let mut img = Image { nx: NX, ny: NY, px, binned: 1 };
        for i in 0..40 {
            let cx = 20.0 + (i % 8) as f64 * 28.0;
            let cy = 20.0 + (i / 8) as f64 * 45.0;
            add_star(&mut img, cx, cy, 7.0, 1.4);
        }
        let bg = crate::background::estimate(&img, 64);
        (img, bg)
    }
    use super::*;
    use crate::background;
    use crate::fits::Image;

    /// Paint a round Gaussian star of the given peak and sigma.
    fn add_star(img: &mut Image, cx: f64, cy: f64, peak: f32, sigma: f64) {
        let r = (sigma * 4.0).ceil() as i64;
        for dy in -r..=r {
            for dx in -r..=r {
                let x = cx.round() as i64 + dx;
                let y = cy.round() as i64 + dy;
                if x < 0 || y < 0 || x >= img.nx as i64 || y >= img.ny as i64 {
                    continue;
                }
                let ex = x as f64 - cx;
                let ey = y as f64 - cy;
                let v = peak as f64 * (-(ex * ex + ey * ey) / (2.0 * sigma * sigma)).exp();
                img.px[y as usize * img.nx + x as usize] += v as f32;
            }
        }
    }

    /// Paint an elliptical Gaussian whose major axis points along `angle_deg`
    /// (degrees CCW from +x) -- the same convention `theta_deg`'s doc comment
    /// claims. `sig_a` is the sigma along that axis, `sig_b` across it.
    fn add_elliptical_star(
        img: &mut Image, cx: f64, cy: f64, peak: f32, sig_a: f64, sig_b: f64, angle_deg: f64,
    ) {
        let theta = angle_deg.to_radians();
        let (ct, st) = (theta.cos(), theta.sin());
        let r = (sig_a.max(sig_b) * 5.0).ceil() as i64;
        for dy in -r..=r {
            for dx in -r..=r {
                let x = cx.round() as i64 + dx;
                let y = cy.round() as i64 + dy;
                if x < 0 || y < 0 || x >= img.nx as i64 || y >= img.ny as i64 {
                    continue;
                }
                let ex = x as f64 - cx;
                let ey = y as f64 - cy;
                // Rotate into the ellipse's own frame: u along the major axis.
                let u = ex * ct + ey * st;
                let v = -ex * st + ey * ct;
                let val = peak as f64
                    * (-(u * u / (2.0 * sig_a * sig_a) + v * v / (2.0 * sig_b * sig_b))).exp();
                img.px[y as usize * img.nx + x as usize] += val as f32;
            }
        }
    }

    fn blank(nx: usize, ny: usize, sky: f32) -> Image {
        // A little deterministic texture so sigma is not exactly zero.
        // Scaled to `sky` rather than a fixed amplitude: a fixed +/-2.0
        // texture is negligible noise against a sky of 50-100 (the levels
        // every other test in this file uses, which is why this scaling
        // reproduces the same +/-2.0 at sky=100 and does not change their
        // behaviour) but it SWAMPS a near-zero sky, e.g. a Siril-style
        // 0..1 float frame -- a signal of 1.0 sits below a noise floor of
        // +/-2.0, so it can never cross any k-sigma detection threshold at
        // all, no matter what `saturation` is set to.
        let mut px = vec![sky; nx * ny];
        let amp = (sky.abs() * 0.02).max(1e-4);
        for (i, p) in px.iter_mut().enumerate() {
            let t = ((i * 2654435761usize) % 100) as f32 / 100.0;
            *p += (t - 0.5) * 2.0 * amp;
        }
        Image { nx, ny, px, binned: 1 }
    }

    fn run(img: &Image, p: &ExtractParams) -> Extraction {
        let bg = background::estimate(img, 32);
        extract(img, &bg, p)
    }

    /// A `Star` with the given position and flux; the rest are plausible
    /// fixed values, irrelevant to selection.
    fn mk_star(x: f64, y: f64, flux: f64) -> Star {
        Star {
            x,
            y,
            flux,
            peak: flux as f32,
            npix: 9,
            fwhm_px: 2.0,
            ellipticity: 0.1,
            theta_deg: 0.0,
        }
    }

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
        let (kept, stratified, _conc) = stratified_keep(stars, 1000, 1000, 100);
        assert!(stratified, "this fixture is exactly the case the gate exists to catch");
        assert_eq!(kept.len(), 100, "the budget must be filled");
        let in_clump = kept.iter().filter(|s| s.x < 40.0 && s.y < 40.0).count();
        assert!(
            in_clump < 30,
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
        let (kept, _stratified, _conc) = stratified_keep(stars, 1000, 1000, 200);
        assert_eq!(kept.len(), 200, "empty cells must donate budget to occupied ones");
    }

    #[test]
    fn fewer_stars_than_the_budget_keeps_all_of_them() {
        let stars: Vec<Star> = (0..30).map(|i| mk_star(i as f64 * 3.0, i as f64 * 7.0, 500.0)).collect();
        let (kept, stratified, _conc) = stratified_keep(stars, 1000, 1000, 500);
        assert!(!stratified, "fewer stars than the budget can never reach the gate");
        assert_eq!(kept.len(), 30);
    }

    #[test]
    fn the_result_is_sorted_brightest_first() {
        let stars: Vec<Star> = (0..200).map(|i| mk_star((i * 5 % 900) as f64, (i * 11 % 900) as f64, i as f64)).collect();
        let (kept, _stratified, _conc) = stratified_keep(stars, 1000, 1000, 50);
        for w in kept.windows(2) {
            assert!(w[0].flux >= w[1].flux, "downstream code assumes brightest-first order");
        }
    }

    #[test]
    fn a_degenerate_frame_size_does_not_panic() {
        let stars: Vec<Star> = (0..10).map(|i| mk_star(0.0, i as f64, 1.0)).collect();
        assert!(stratified_keep(stars.clone(), 0, 0, 5).0.len() <= 5);
        assert!(stratified_keep(stars, 1, 1, 5).0.len() <= 5);
    }

    /// Pins the two endpoints `concentration_stat` must produce: a field with
    /// exactly one detection per grid cell must read as exactly 1.0.
    #[test]
    fn concentration_stat_is_near_one_for_a_spatially_uniform_field() {
        let mut stars = Vec::new();
        for gy in 0..8 {
            for gx in 0..8 {
                stars.push(mk_star(gx as f64 * 100.0 + 50.0, gy as f64 * 100.0 + 50.0, 100.0));
            }
        }
        let c = concentration_stat(&stars, 800, 800);
        assert!((c - 1.0).abs() < 1e-9, "uniform field should give concentration ~1.0, got {c}");
    }

    #[test]
    fn concentration_stat_is_large_for_a_clump() {
        // 100 stars all inside one cell of an 800x800 frame on the fixed
        // 8x8 grid (cell size 100px) -- the busiest cell holds everything.
        let stars: Vec<Star> = (0..100)
            .map(|i| mk_star((i % 10) as f64, (i / 10) as f64, 1000.0 - i as f64))
            .collect();
        let c = concentration_stat(&stars, 800, 800);
        assert!(c > 20.0, "a single-cell clump should give a large concentration, got {c}");
    }

    #[test]
    fn concentration_stat_does_not_panic_on_degenerate_inputs() {
        assert_eq!(concentration_stat(&[], 800, 800), 1.0, "no stars: nothing to be concentrated");
        let one = [mk_star(5.0, 5.0, 100.0)];
        assert_eq!(concentration_stat(&one, 0, 0), 1.0, "zero-sized frame is undefined, not a panic");
        assert_eq!(concentration_stat(&one, 0, 800), 1.0);
        assert_eq!(concentration_stat(&one, 800, 0), 1.0);
        // A 1x1 frame: every star lands in cell (0,0), still well-defined.
        let c = concentration_stat(&one, 1, 1);
        assert!(c.is_finite() && c > 0.0, "1x1 frame produced {c}");
    }

    /// A population where every consecutive 64-star "wave" has exactly one
    /// star per grid cell, brightest wave first. Any prefix that is a
    /// multiple of 64 -- in particular the brightest-`keep` slice
    /// `stratified_keep` gates on -- is therefore perfectly spread across
    /// the grid, concentration exactly 1.0, regardless of how large `keep`
    /// is relative to the grid. (A
    /// naive "one star per cell, keep=20" fixture does NOT have this
    /// property: the brightest 20 of 64 such stars come from only the first
    /// 20 cells, which is itself a mild -- and misleading -- concentration
    /// artifact of small `keep` relative to 64 cells, not a signal about
    /// clumping.)
    fn uniform_waves(nx: usize, ny: usize, waves: usize) -> Vec<Star> {
        let mut stars = Vec::with_capacity(waves * 64);
        for wave in 0..waves {
            for cell in 0..64 {
                let (gx, gy) = (cell % 8, cell / 8);
                let flux = 100_000.0 - (wave * 64 + cell) as f64 * 0.5;
                let x = gx as f64 * (nx as f64 / 8.0) + 13.0;
                let y = gy as f64 * (ny as f64 / 8.0) + 27.0;
                stars.push(mk_star(x, y, flux));
            }
        }
        stars
    }

    /// The test that would have caught the regression this milestone exists
    /// to fix: below the gate, `stratified_keep` must not merely resemble
    /// legacy brightest-N selection -- it must reproduce it exactly, same
    /// stars in the same order. Comparing lengths or set membership would not
    /// have caught the 0.30" systematic centroid drift the unconditional
    /// stratifier produced on 9,173 frames that solved fine either way.
    #[test]
    fn below_threshold_stratified_keep_is_bit_identical_to_legacy_sort_and_truncate() {
        let stars = uniform_waves(800, 800, 8); // 512 stars, keep < len so the gate actually runs
        let keep = 256; // 4 waves' worth -- brightest-keep slice is exactly uniform
        assert!(
            concentration_stat(&stars[..keep], 800, 800) < CONCENTRATION_THRESHOLD,
            "fixture's brightest-keep slice must be below the gate for this test to mean anything"
        );

        let mut legacy = stars.clone();
        legacy.sort_unstable_by(|p, q| q.flux.partial_cmp(&p.flux).unwrap_or(std::cmp::Ordering::Equal));
        legacy.truncate(keep);

        let (got, stratified, _conc) = stratified_keep(stars, 800, 800, keep);
        assert!(!stratified, "fixture is below the gate, by the assertion above");
        assert_eq!(got, legacy, "below the gate, stratified_keep must reproduce the legacy path exactly");
    }

    /// The gate itself: fires on a clumped fixture, does not on a uniform
    /// one, same frame size and keep budget for both.
    #[test]
    fn the_gate_fires_on_a_clumped_fixture_and_not_on_a_uniform_one() {
        let keep = 256;

        let uniform = uniform_waves(800, 800, 8);
        let mut legacy_uniform = uniform.clone();
        legacy_uniform
            .sort_unstable_by(|p, q| q.flux.partial_cmp(&p.flux).unwrap_or(std::cmp::Ordering::Equal));
        legacy_uniform.truncate(keep);
        let (got_uniform, stratified_uniform, _conc) = stratified_keep(uniform, 800, 800, keep);
        assert!(!stratified_uniform, "gate must not fire on a uniform field");
        assert_eq!(got_uniform, legacy_uniform, "gate must not fire on a uniform field");

        // 300 bright stars packed into one corner cell -- more than `keep`,
        // so legacy selection would fill the entire budget from the clump
        // alone -- plus the same uniform waves as filler at much lower flux.
        let mut clumped = Vec::new();
        for i in 0..300 {
            clumped.push(mk_star((i % 20) as f64, (i / 20) as f64, 1_000_000.0 - i as f64));
        }
        clumped.extend(uniform_waves(800, 800, 8));
        let mut legacy_clumped = clumped.clone();
        legacy_clumped
            .sort_unstable_by(|p, q| q.flux.partial_cmp(&p.flux).unwrap_or(std::cmp::Ordering::Equal));
        legacy_clumped.truncate(keep);
        let (got_clumped, stratified_clumped, _conc) = stratified_keep(clumped, 800, 800, keep);
        assert!(stratified_clumped, "gate must fire on a clumped field");
        assert_ne!(got_clumped, legacy_clumped, "gate must fire on a clumped field");
    }

    /// A `uniform_waves` fixture with `extra` copies of an extremely bright
    /// star piled into cell (0,0) -- a controlled knob for pushing the
    /// brightest-`keep` slice's concentration up from the uniform baseline
    /// (1.0 at `extra = 0`) by a known amount, monotonically in `extra`.
    fn nudged_towards_a_clump(nx: usize, ny: usize, waves: usize, extra: usize) -> Vec<Star> {
        let mut stars = uniform_waves(nx, ny, waves);
        for i in 0..extra {
            stars.push(mk_star(13.0, 27.0, 1_000_000.0 + i as f64));
        }
        stars
    }

    /// The near-threshold case: the bit-identity test above sits at
    /// concentration 1.0, nowhere near [`CONCENTRATION_THRESHOLD`]. This
    /// finds the ACTUAL boundary by search -- rather than hardcoding a
    /// concentration value that would silently stop testing the boundary
    /// the moment the threshold is recalibrated -- and checks both sides of
    /// it: the largest fixture that stays under the gate must still be
    /// bit-identical to legacy, and the smallest fixture that crosses it
    /// must stratify.
    #[test]
    fn the_gate_is_correct_right_at_its_own_boundary() {
        let keep = 256;
        let conc_of = |extra: usize| -> f64 {
            let mut stars = nudged_towards_a_clump(800, 800, 8, extra);
            stars.sort_unstable_by(|p, q| q.flux.partial_cmp(&p.flux).unwrap_or(std::cmp::Ordering::Equal));
            stars.truncate(keep);
            concentration_stat(&stars, 800, 800)
        };

        let mut extra = 0usize;
        while conc_of(extra) < CONCENTRATION_THRESHOLD {
            extra += 1;
            assert!(extra < keep, "search did not converge -- fixture is broken");
        }
        let below = extra.saturating_sub(1);
        let above = extra;
        assert!(conc_of(below) < CONCENTRATION_THRESHOLD, "search invariant violated");
        assert!(conc_of(above) >= CONCENTRATION_THRESHOLD, "search invariant violated");

        let stars_below = nudged_towards_a_clump(800, 800, 8, below);
        let mut legacy_below = stars_below.clone();
        legacy_below
            .sort_unstable_by(|p, q| q.flux.partial_cmp(&p.flux).unwrap_or(std::cmp::Ordering::Equal));
        legacy_below.truncate(keep);
        let (got_below, strat_below, _conc) = stratified_keep(stars_below, 800, 800, keep);
        assert!(!strat_below, "just below the gate must not stratify");
        assert_eq!(got_below, legacy_below, "just below the gate must be bit-identical to legacy");

        let stars_above = nudged_towards_a_clump(800, 800, 8, above);
        let (_got_above, strat_above, _conc) = stratified_keep(stars_above, 800, 800, keep);
        assert!(strat_above, "just above the gate must stratify");
    }

    #[test]
    fn finds_stars_at_the_positions_they_were_painted() {
        let mut img = blank(256, 256, 100.0);
        let truth = [(60.0, 70.0), (128.5, 96.25), (200.0, 180.0)];
        for (x, y) in truth {
            add_star(&mut img, x, y, 5000.0, 1.6);
        }
        let ex = run(&img, &ExtractParams::default());
        assert_eq!(ex.stars.len(), 3, "expected 3 stars, got {}", ex.stars.len());
        for (tx, ty) in truth {
            let hit = ex
                .stars
                .iter()
                .any(|s| (s.x - tx).abs() < 0.3 && (s.y - ty).abs() < 0.3);
            assert!(hit, "no star recovered near ({tx},{ty}): {:?}",
                ex.stars.iter().map(|s| (s.x, s.y)).collect::<Vec<_>>());
        }
    }

    #[test]
    fn centroids_are_sub_pixel_accurate() {
        let mut img = blank(128, 128, 50.0);
        add_star(&mut img, 64.37, 64.62, 8000.0, 1.8);
        let ex = run(&img, &ExtractParams::default());
        assert_eq!(ex.stars.len(), 1);
        let s = &ex.stars[0];
        assert!((s.x - 64.37).abs() < 0.15, "x was {}", s.x);
        assert!((s.y - 64.62).abs() < 0.15, "y was {}", s.y);
    }

    #[test]
    fn a_single_hot_pixel_is_rejected_as_too_small_and_counted() {
        let mut img = blank(128, 128, 50.0);
        img.px[64 * 128 + 64] = 60000.0;
        let ex = run(&img, &ExtractParams::default());
        assert!(ex.stars.is_empty(), "a hot pixel is not a star");
        assert_eq!(ex.rejected.too_small, 1);
    }

    #[test]
    fn a_large_smooth_blob_is_rejected_as_extended() {
        // The Eagle Nebula case: the brightest thing in the reference frame was
        // a 63,104-pixel blob. It must never reach the quad builder.
        let mut img = blank(256, 256, 100.0);
        for (x, y) in [(40.0, 40.0), (80.0, 40.0), (120.0, 40.0), (160.0, 40.0),
                       (40.0, 80.0), (80.0, 80.0), (120.0, 80.0), (160.0, 80.0)] {
            add_star(&mut img, x, y, 5000.0, 1.6);
        }
        for y in 150..230 {
            for x in 60..200 {
                img.px[y * 256 + x] = 9000.0;
            }
        }
        let ex = run(&img, &ExtractParams::default());
        assert!(ex.rejected.extended >= 1, "the blob should be rejected as extended");
        assert!(
            ex.stars.iter().all(|s| s.y < 140.0),
            "no detection may come from the blob region"
        );
        assert!(ex.stars.len() >= 6, "the real stars must survive");
    }

    #[test]
    fn a_saturated_star_is_rejected_and_counted() {
        let mut img = blank(128, 128, 50.0);
        for y in 60..68 {
            for x in 60..68 {
                img.px[y * 128 + x] = 65535.0;
            }
        }
        let ex = run(&img, &ExtractParams::default());
        assert_eq!(ex.rejected.saturated, 1);
        assert!(ex.stars.is_empty());
    }

    #[test]
    fn a_trailed_star_is_rejected_and_counted() {
        let mut img = blank(128, 128, 50.0);
        // A streak: long in x, thin in y.
        for x in 50..80 {
            for y in 63..66 {
                img.px[y * 128 + x] = 8000.0;
            }
        }
        let ex = run(&img, &ExtractParams::default());
        assert_eq!(ex.rejected.elongated, 1, "a streak is not a usable star");
    }

    #[test]
    fn a_star_touching_the_edge_is_rejected_and_counted() {
        let mut img = blank(128, 128, 50.0);
        add_star(&mut img, 2.0, 64.0, 8000.0, 1.8);
        let ex = run(&img, &ExtractParams::default());
        assert_eq!(ex.rejected.edge, 1, "a truncated PSF has a biased centroid");
        assert!(ex.stars.is_empty());
    }

    #[test]
    fn fwhm_tracks_the_painted_width() {
        // FWHM = 2*sqrt(2*ln2)*sigma = 2.3548*sigma.
        for sigma in [1.5f64, 2.5, 3.5] {
            let mut img = blank(128, 128, 50.0);
            add_star(&mut img, 64.0, 64.0, 9000.0, sigma);
            let ex = run(&img, &ExtractParams::default());
            assert_eq!(ex.stars.len(), 1, "sigma {sigma}");
            let want = 2.3548 * sigma;
            let got = ex.stars[0].fwhm_px;
            assert!(
                (got - want).abs() < 0.6 * want.max(1.0) * 0.5,
                "sigma {sigma}: fwhm {got} should be near {want}"
            );
        }
    }

    #[test]
    fn theta_deg_reports_position_angle_against_an_absolute_reference() {
        // theta_deg's doc comment claims "degrees CCW from +x". Pin that
        // against a blob elongated along a KNOWN axis rather than against
        // another call to itself, which is exactly the shape of blind spot
        // that hid orientation_deg()'s 180-degree error (see fit.rs).
        let angle = 35.0;
        let mut img = blank(160, 160, 100.0);
        // a=3.0, b=1.6 gives ellipticity ~0.47, under max_ellipticity (0.6)
        // so the blob survives to be reported, not rejected as elongated.
        add_elliptical_star(&mut img, 80.0, 80.0, 9000.0, 3.0, 1.6, angle);
        let ex = run(&img, &ExtractParams::default());
        assert_eq!(ex.stars.len(), 1, "the ellipse must survive extraction");
        let s = &ex.stars[0];
        assert!(s.ellipticity > 0.1, "fixture should be measurably elongated, got {}", s.ellipticity);
        // A line has no direction, only an axis -- theta and theta+180 are
        // the same axis, so compare modulo 180.
        let d = (s.theta_deg - angle).rem_euclid(180.0);
        let d = d.min(180.0 - d);
        assert!(d < 5.0, "theta_deg {} should be near {angle} (mod 180)", s.theta_deg);
    }

    #[test]
    fn ellipticity_is_near_zero_for_a_round_star() {
        let mut img = blank(128, 128, 50.0);
        add_star(&mut img, 64.0, 64.0, 9000.0, 2.0);
        let ex = run(&img, &ExtractParams::default());
        assert!(ex.stars[0].ellipticity < 0.15, "round star had e={}", ex.stars[0].ellipticity);
    }

    #[test]
    fn results_are_sorted_brightest_first_and_capped_at_keep() {
        let mut img = blank(256, 256, 100.0);
        let mut n = 0;
        for gy in 0..10 {
            for gx in 0..10 {
                add_star(&mut img, 20.0 + gx as f64 * 22.0, 20.0 + gy as f64 * 22.0,
                         1000.0 + (n as f32) * 60.0, 1.7);
                n += 1;
            }
        }
        let p = ExtractParams { keep: 25, ..ExtractParams::default() };
        let ex = run(&img, &p);
        assert_eq!(ex.stars.len(), 25, "keep must cap the result");
        for w in ex.stars.windows(2) {
            assert!(w[0].flux >= w[1].flux, "must be brightest first");
        }
        assert!(ex.detected >= 90, "detected count reports pre-cap reality");
        // The regression this fixture also happens to cover: the gate IS
        // reachable here (~100 usable stars against keep=25), but keep=25 is
        // below CONCENTRATION_MIN_N (64) -- the fixed grid cannot mean
        // anything with that few candidates. A real corpus frame with 21
        // usable stars once reported `concentration: 12.19` from nothing but
        // which of 64 cells 4 of those 21 happened to land in.
        assert!(
            ex.concentration.is_none(),
            "keep=25 < CONCENTRATION_MIN_N must suppress the reported value, got {:?}",
            ex.concentration
        );
    }

    /// Below `keep`, `stratified_keep`'s own early return makes the legacy
    /// path unconditional -- concentration cannot have been the reason,
    /// however it happens to compute, so `Extraction` must not report one.
    #[test]
    fn concentration_is_none_when_the_gate_was_never_reachable() {
        let mut img = blank(256, 256, 100.0);
        for i in 0..5 {
            add_star(&mut img, 30.0 + i as f64 * 40.0, 30.0 + i as f64 * 40.0, 6000.0, 1.7);
        }
        let ex = run(&img, &ExtractParams::default()); // keep defaults to 500, far above 5 detections
        assert!(ex.concentration.is_none(), "5 usable stars can never exceed keep=500");
        assert!(!ex.stratified, "the legacy path is unconditional here");
    }

    /// When the gate genuinely ran with enough candidates to mean something,
    /// the reported concentration and the reported `stratified` flag must
    /// agree with each other -- a consumer reading both fields together must
    /// never see a contradiction between them.
    #[test]
    fn concentration_and_stratified_agree_when_both_are_reported() {
        let mut img = blank(256, 256, 100.0);
        let mut n = 0;
        for gy in 0..10 {
            for gx in 0..10 {
                add_star(&mut img, 20.0 + gx as f64 * 22.0, 20.0 + gy as f64 * 22.0,
                         1000.0 + (n as f32) * 60.0, 1.7);
                n += 1;
            }
        }
        // keep=64 clears CONCENTRATION_MIN_N exactly, and ~100 usable stars
        // clears keep, so both conditions for a reported value hold.
        let p = ExtractParams { keep: 64, ..ExtractParams::default() };
        let ex = run(&img, &p);
        let conc = ex.concentration.expect("gate was reachable and meaningful here");
        assert_eq!(
            ex.stratified,
            should_stratify(conc),
            "the reported flag must agree with the reported number"
        );
    }

    #[test]
    fn an_empty_field_yields_nothing_and_does_not_panic() {
        let img = blank(128, 128, 100.0);
        let ex = run(&img, &ExtractParams::default());
        assert!(ex.stars.is_empty());
        assert_eq!(ex.detected, 0);
    }

    #[test]
    fn the_caller_can_set_the_saturation_level_for_its_own_data_range() {
        // A Siril float frame saturates near 1.0, not near 65535.
        let mut img = blank(128, 128, 0.001);
        for y in 60..68 { for x in 60..68 { img.px[y * 128 + x] = 1.0; } }
        let bg = background::estimate(&img, 32);

        let inert = ExtractParams { saturation: 65535.0 * 0.999, ..ExtractParams::default() };
        assert_eq!(extract(&img, &bg, &inert).rejected.saturated, 0,
            "a 16-bit ceiling cannot fire on float data -- this is the bug");

        let right = ExtractParams { saturation: 0.999, ..ExtractParams::default() };
        assert_eq!(extract(&img, &bg, &right).rejected.saturated, 1,
            "with the correct level for this data range it must fire");
    }

    #[test]
    fn a_hot_pixel_swarm_does_not_make_real_stars_look_extended() {
        // The cap is relative to the median STAR area. If hot pixels count
        // toward that median they swamp it, the cap collapses, and every real
        // star is rejected as extended.
        let mut img = blank(256, 256, 100.0);
        let mut n = 0;
        for y in (4..252).step_by(4) {
            for x in (4..252).step_by(4) {
                if n < 400 { img.px[y * 256 + x] = 40000.0; n += 1; }
            }
        }
        for (x, y) in [(30.0, 200.0), (70.0, 200.0), (110.0, 200.0), (150.0, 200.0),
                       (190.0, 200.0), (30.0, 230.0), (70.0, 230.0), (110.0, 230.0)] {
            add_star(&mut img, x, y, 6000.0, 2.2);
        }
        let bg = background::estimate(&img, 32);
        let ex = extract(&img, &bg, &ExtractParams::default());
        assert!(ex.rejected.too_small >= 300, "the hot pixels should be rejected as too small");
        assert!(ex.stars.len() >= 6,
            "real stars must survive a hot-pixel swarm, got {} (extended={})",
            ex.stars.len(), ex.rejected.extended);
    }
}
