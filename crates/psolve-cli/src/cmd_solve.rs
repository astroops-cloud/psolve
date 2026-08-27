//! `psolve solve` -- read a frame, fetch its catalogue neighbourhood, solve.
//!
//! Exit codes matter here: 0 solved, 1 NOT SOLVED (a normal outcome -- clouds
//! are not a bug), 2 usage/config, 3 index problem. A script must be able to
//! tell a cloudy frame from a broken invocation.

use crate::flag;
use psolve_core::solve::{CatalogStar, Outcome, PreparedFrame, SolveOptions};
use psolve_core::verify::{AcceptParams, HypothesisCount};
use psolve_index::quad_format::QuadRecord;
use psolve_index::quad_reader::QuadIndex;
use psolve_index::reader::Index;
use std::path::Path;
use std::process::ExitCode;
use std::time::Instant;

/// The `build` field of the solve JSON: a git-derived identifier set by
/// `build.rs`, distinct from `psolve` (the crate version). See `build.rs`'s
/// module doc for why this exists and `docs/superpowers/specs/
/// 2026-08-13-psolve-design.md` §7.2 for the contract. `env!`, not
/// `option_env!`: `build.rs` unconditionally emits `PSOLVE_BUILD_ID` (falling
/// back to the literal `"unknown"` itself when `git` is unavailable), so the
/// variable is always present at compile time -- `env!`'s compile-time
/// failure mode is therefore unreachable, not a risk being papered over.
const BUILD_ID: &str = env!("PSOLVE_BUILD_ID");

/// Non-finite values are not valid JSON tokens. M1 shipped this bug; do not
/// ship it again.
fn json_number(v: f64) -> String {
    if v.is_finite() { format!("{v:.10}") } else { "null".to_string() }
}

/// `None` means "not meaningful to report" (see
/// `psolve_core::extract::Extraction::concentration` and this crate's
/// `catalog_concentration`) -- emitted as JSON `null`, the same as a
/// non-finite `f64` already is, rather than a number that would silently
/// mislead a consumer into thinking a gate decision was based on it.
fn json_option_number(v: Option<f64>) -> String {
    match v {
        Some(v) => json_number(v),
        None => "null".to_string(),
    }
}

/// Escape a string that came from a file before it goes near JSON.
fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// Flags that consume the following token. Their values must never be
/// mistaken for the positional FILE argument.
const VALUED_FLAGS: &[&str] = &[
    "--index", "--hint", "--scale", "--radius", "--cat-limit", "--saturation",
    "--sigma", "--min-pix", "--keep", "--max-ellipticity", "--quad-index",
    "--max-mag",
];

/// The first token that is neither a flag nor a flag's value.
///
/// `find(|a| !a.starts_with("--"))` is not enough: a valued flag's value does
/// not start with `--` either, so `--index t.psidx frame.fits` would bind
/// `t.psidx` as the frame, never read the real file, and report a clean
/// "not solved" -- a broken invocation disguised as bad weather, which is the
/// one thing the exit-code contract exists to prevent.
fn positional<'a>(args: &[&'a str]) -> Option<&'a str> {
    let mut i = 0;
    while i < args.len() {
        let a = args[i];
        if VALUED_FLAGS.contains(&a) {
            i += 2;
        } else if a.starts_with("--") {
            i += 1;
        } else {
            return Some(a);
        }
    }
    None
}

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
    half_diagonal * RADIUS_MARGIN
}

/// The tight search radius the last rung retries at, as a fraction of the
/// frame's half-diagonal.
///
/// [`default_radius_for`] uses **1.10x** the half-diagonal -- half the frame's
/// diagonal plus a pointing-error margin. That is the right default, but a
/// disc sized to be safe against pointing error is larger than the frame, and
/// every catalogue star outside the frame has no possible image counterpart.
/// Each one lowers completeness, and a quad matches only when all four of its
/// stars survive on both sides.
///
/// **0.5 is measured.** Against the 38 corpus frames still failing after every
/// other rung, sweeping the radius as a fraction of the half-diagonal:
///
/// ```text
///   0.35 -> 20/28    0.50 -> 24/28    0.60 -> 22/28    0.70 -> 22/28
///   0.80 -> 23/28    0.90 -> 16/28    1.00 -> 14/28
/// ```
///
/// Broad and flat from 0.35 to 0.8, so 0.5 sits on a plateau rather than a
/// spike. A disc this size does not reach the frame's corners, which is the
/// point rather than a compromise.
///
/// The rescued solves are correct, not merely accepted: against ASTAP's own
/// recorded answers for the same frames, median **0.61"**, p90 1.81", max
/// 2.60", all 24 inside 5 arcsec and none beyond 60.
///
/// A cross-frame prior was measured first and rescues **zero** -- hinting a
/// frame from a solved neighbour in the same session gave 0 of 28, because
/// the commanded pointing and the prior differ by a median of 0.0161 deg and
/// the disc is degrees across. The centre was never the problem, so this rung
/// needs no sequence state and no neighbouring frame; the frame's own header
/// supplies everything.
pub(crate) const RADIUS_RETRY_HALF_DIAG_FRAC: f64 = 0.5;

/// What [`default_radius_for`] multiplies the half-diagonal by, so the rung
/// above can recover the half-diagonal from a header-derived radius instead
/// of re-deriving it from the optics keywords a second time.
const RADIUS_MARGIN: f64 = 1.10;

/// Radius derived purely from the frame header's own optics keywords, via
/// [`default_radius_for`] -- `None` when the header is absent or lacks the
/// `FOCALLEN`/`XPIXSZ` keywords `field_width_deg`/`field_height_deg` need.
///
/// `pub(crate)`, not private: this is the ONE place the header-derived
/// formula is ever applied to a `FitsHeader`. Native mode's own
/// [`default_radius_deg`] below falls back further, to a fixed constant, when
/// this returns `None`; ASTAP-compatible mode's `astap_args::search_radius_deg`
/// falls back to `-fov`/`-r` instead. Both must derive from the SAME
/// header-usable formula, not two drifting copies of it -- that drift (native
/// asking the frame, compat asking only `-fov`/`-r`) is the defect this
/// function exists to close.
pub(crate) fn header_radius_deg(hdr: Option<&psolve_core::fits::FitsHeader>) -> Option<f64> {
    let h = hdr?;
    match (psolve_core::fits::field_width_deg(h), psolve_core::fits::field_height_deg(h)) {
        (Some(w), Some(ht)) => Some(default_radius_for(w, ht)),
        // Height alone: treat the frame as square rather than inventing an
        // aspect ratio. Errs slightly wide, which is the safe direction.
        (None, Some(ht)) => Some(default_radius_for(ht, ht)),
        _ => None,
    }
}

/// Default search radius: half the frame's diagonal plus pointing-error
/// margin, when the optics keywords allow it; the old fixed constant
/// otherwise, rather than guessing narrow and failing mysteriously.
fn default_radius_deg(hdr: Option<&psolve_core::fits::FitsHeader>) -> f64 {
    header_radius_deg(hdr).unwrap_or(2.5)
}

/// Default catalogue depth: 3x the frame's own usable star count. Too many
/// catalogue stars make matching HARDER, not easier -- true quads drown in
/// false ones -- so this is sized rather than maximised (the Task 4
/// sparse-frame fix).
///
/// `usable` comes from [`psolve_core::solve::PreparedFrame`], i.e. from the
/// extraction the solve itself will use. It used to come from a *second*,
/// defaults-only decode+background+extract run ahead of `solve()`, whose own
/// doc comment admitted the duplication and priced it as "a plate solve
/// happens once per frame, not in a hot loop". That price turned out to be
/// ~67 ms on a 3840x2160 frame -- about 40% of the whole solve, and
/// unavoidable in ASTAP-compatibility mode, which has no `--cat-limit` flag
/// to opt out with. `prepare()`/`solve_prepared()` now decode and extract
/// once and hand the same star set to both.
///
/// `pub(crate)`, not private: `main.rs`'s ASTAP-mode dispatch reuses this
/// exact sizing rather than a second, drifting copy of the same heuristic --
/// the fix it embodies must protect both entry points equally.
pub(crate) fn cat_limit_for(usable: usize) -> usize {
    (usable * 3).clamp(300, 5000)
}

/// The catalogue-side gate: below this, `select_catalog` takes the
/// untouched legacy path (`brightest_in_disc`'s own result, reused as-is).
///
/// **A separate constant from the image-side threshold**
/// (`psolve_core::extract::CONCENTRATION_THRESHOLD`), not the same number
/// reused -- see that constant's doc for the measurement that showed a
/// single shared value does not work even after both statistics are
/// normalised to the same ~1.0 uniform baseline: the two REPORTED
/// distributions differ in scale throughout, not only at that baseline.
///
/// Calibrated against the renormalised statistic (see
/// `catalog_concentration`'s doc for the normalisation) measured on real
/// frames from `~/astroops`: the 300-frame agreement-corpus sample reads
/// median 1.40, p90 1.81, max 2.71; the 276 baseline-failing frames read
/// median 1.64, and split by target the same way the image-side threshold
/// did -- HD 93308/C 76/Eta Carina (targets stratification demonstrably
/// helped) read 2.25-2.71, while Corona Australis/War and Peace/Centaurus
/// A/Caldwell 101 (targets it did not help, and which regressed under the
/// unconditional design) read 1.10-1.34. 2.0 sits in the gap between those
/// two groups: it excludes every not-helped target and the two
/// regression-causing corpus targets (Centaurus A 1.10, Cats Paw Nebula
/// 1.31), while still catching most of the helped ones (M 8, at median
/// 1.64, is the one rescued target this threshold does NOT catch -- see
/// `docs/superpowers/2026-08-15-conditional-stratification-results.md` for
/// the full data and that tradeoff stated plainly).
///
/// Full-corpus re-validation at this value (read-only, 9,495 like-for-like
/// frames against the committed baseline): this gate fires on 496 of them
/// (the image-side gate fires on 3 -- see
/// `psolve_core::extract::CONCENTRATION_THRESHOLD`'s doc); solve rate
/// 9268/9495 (97.61%, baseline 9219/9495, +49 net: 55 gains, 6
/// regressions); separation median 0.530" (baseline 0.531"), p90 0.945"
/// (baseline 0.946"), p99 3.098" (baseline 3.110"); of the 9,213 frames
/// solved in both runs, 386 (4.2%) have a nonzero fitted-centre shift at
/// all, median shift over the whole both-solved population 0.000". Compare
/// the original unconditional design's regression: 99.2% of both-solved
/// frames shifted, median 0.30".
pub(crate) const CATALOG_CONCENTRATION_THRESHOLD: f64 = 2.0;

/// The catalogue-side stratified-vs-legacy decision. Deliberately NOT
/// `psolve_core::extract::should_stratify` -- see
/// `CATALOG_CONCENTRATION_THRESHOLD`'s doc for why the two sides need
/// separate thresholds even though both statistics share the same ~1.0
/// uniform baseline.
fn catalog_should_stratify(concentration: f64) -> bool {
    concentration >= CATALOG_CONCENTRATION_THRESHOLD
}

/// Concentration of what legacy `Index::brightest_in_disc` would ITSELF
/// return, across the disc's own AREA -- the catalogue-side analogue of
/// `psolve_core::extract`'s image-side statistic.
///
/// This is a SEPARATE measurement from the image-side concentration, not a
/// reuse of it: they describe different populations (detected stars in the
/// frame vs. catalogue stars in the search disc) that are usually but not
/// always correlated. `cross_path_catalogue_selection.rs`'s fixture is the
/// case where they are not -- a dense catalogue clump ~1 deg away with no
/// image counterpart at all, sitting next to an otherwise unremarkable,
/// spatially uniform image. Gating the catalogue fetch on the IMAGE
/// statistic would leave that fixture on the legacy path and starve out
/// every real match, which is exactly the defect Task 1-3 fixed; gating it
/// on this measurement instead catches it correctly while still leaving an
/// ordinary frame's catalogue fetch on the legacy path.
///
/// The denominator is `disc solid angle / average HEALPix cell solid
/// angle`, NOT `cells_in_disc(...).len()` (the padded CANDIDATE cell set,
/// padded by `max_pixrad(nside)` -- ~1.03 deg at nside 64 -- so a cell that
/// merely touches the disc is never missed by the actual fetch).
/// Fix-round-1 finding, measured against real frames: the padded count made
/// a genuinely uniform catalogue read concentration ~2.4-3.3, not ~1.0, and
/// worse, it is FOV-dependent in a way this project's corpus cannot
/// exercise (every frame in it is 2.4-4.5 deg wide) -- for a narrow-field
/// rig at radius <~0.5 deg, the true disc sits inside 1-2 HEALPix cells
/// while the padded count is 7-9, so the padded statistic reads 5-9 and
/// fires UNCONDITIONALLY regardless of any real clumping. A pure-geometry
/// cell count scales with the disc's own area instead, so it stays
/// comparable across every FOV, including one narrower than a single
/// HEALPix cell (`ncells_effective` is clamped to a floor of 1 rather than
/// allowed below it -- there is nowhere else for the stars to be, so a
/// sub-cell disc correctly reads as uniform and never fires).
///
/// Returns `(concentration, meaningful)`. `meaningful` is `false` when
/// there were fewer candidate stars than the disc's own effective cell
/// count -- the busiest-cell reading is shot noise below that, the same
/// reasoning as `psolve_core::extract`'s `CONCENTRATION_MIN_N` on the image
/// side, just against a continuous rather than fixed cell count.
///
/// `select_catalog` uses `meaningful` to gate the DECISION, not only the
/// reported number -- caught empirically while testing this function: at a
/// wide disc radius against a modest star count (`ncells_effective` in the
/// thousands, `recs.len()` in the hundreds), a genuinely uniform catalogue
/// spreads to at most 1-2 stars per occupied cell purely from there being
/// far more cells than stars, and `max_count / (recs.len() / ncells_effective)`
/// reads as a large number from that alone -- not from any real clumping.
/// Letting the gate fire on that is the same defect as the image side's
/// small-`keep` case, just reached from the opposite direction (too few
/// candidates for a large cell count, rather than too few for a fixed one).
pub(crate) fn catalog_concentration(
    recs: &[psolve_index::record::StarRecord],
    nside: u32,
    radius_deg: f64,
) -> (f64, bool) {
    if recs.is_empty() {
        return (1.0, false);
    }
    let r = radius_deg.to_radians();
    let disc_sr = 2.0 * std::f64::consts::PI * (1.0 - r.cos());
    let cell_sr = 4.0 * std::f64::consts::PI / (12.0 * (nside as f64).powi(2));
    let ncells_effective = (disc_sr / cell_sr).max(1.0);

    let mut counts: std::collections::HashMap<u64, u32> = std::collections::HashMap::new();
    for rec in recs {
        let cell = psolve_index::healpix::ang2pix_nest(nside, rec.ra_deg(), rec.dec_deg());
        *counts.entry(cell).or_insert(0) += 1;
    }
    let max_count = counts.values().copied().max().unwrap_or(0) as f64;
    let uniform_share = recs.len() as f64 / ncells_effective;
    let concentration = max_count / uniform_share;
    let meaningful = recs.len() as f64 >= ncells_effective;
    (concentration, meaningful)
}

/// The result of one catalogue disc query, plus the diagnostics behind the
/// choice it made.
pub(crate) struct CatalogSelection {
    pub(crate) recs: Vec<psolve_index::record::StarRecord>,
    /// `None` when there were too few candidate stars for the statistic to
    /// mean anything -- see `catalog_concentration`'s doc.
    pub(crate) concentration: Option<f64>,
    /// Whether `stratified_in_disc` actually ran, as opposed to `recs`
    /// being `brightest_in_disc`'s own result reused unmodified.
    pub(crate) stratified: bool,
}

/// Drop catalogue stars fainter than `max_mag`.
///
/// `--cat-limit` is a COUNT, and a count means a different magnitude on every
/// index. Measured across the three indexes on this machine: a 0.25 deg disc
/// at limit 1500 returns 165 stars reaching G=14.99 from the g14 index and
/// 1,172 stars reaching **G=18.00** from g18. Same flag, same frame, a
/// catalogue three magnitudes deeper.
///
/// That matters because catalogue stars the frame could never have detected
/// do not merely fail to help -- they lower completeness, and quad matching
/// needs all four of a quad's stars present on both sides, so the matchable
/// fraction falls as completeness to the fourth power. A magnitude ceiling
/// is the only way to say "no fainter than this" independently of which
/// index the flag is pointed at.
///
/// Applied after selection rather than inside the index query: the fetch is
/// 1-3 ms (measured, all three depths) and the records arrive
/// magnitude-sorted from `brightest_in_disc`, so filtering here costs one
/// pass and needs no new index API.
fn cap_by_mag(
    mut recs: Vec<psolve_index::record::StarRecord>,
    max_mag: Option<f32>,
) -> Vec<psolve_index::record::StarRecord> {
    if let Some(m) = max_mag {
        // The record stores milli-magnitudes as i16; comparing in that unit
        // avoids a float conversion per star and matches the stored value
        // exactly rather than through a rounding of it.
        let milli = (m * 1000.0).clamp(i16::MIN as f32, i16::MAX as f32) as i16;
        recs.retain(|r| r.mag_milli <= milli);
    }
    recs
}

/// Choose between legacy and stratified catalogue selection for one disc
/// query. `pub(crate)`, not private: both entry points (`solve_cmd` below
/// and `main.rs`'s `astap_cmd`) call this SAME function rather than each
/// computing its own catalogue-side decision -- the 2026-08-14 scale/binning
/// retry's own defect (a fix landing in `cmd_solve.rs` alone, leaving
/// `main.rs`'s ASTAP dispatch on stale behaviour) is exactly the shape a
/// second, drifting copy of this decision would reintroduce.
///
/// Below the gate this returns EXACTLY `brightest_in_disc`'s own result --
/// it is computed first and reused as-is, never recomputed -- so the
/// ungated path is bit-identical to pre-stratification behaviour on the
/// catalogue side too, the same guarantee `stratified_keep` makes on the
/// image side.
pub(crate) fn select_catalog(
    index: &Index,
    ra_deg: f64,
    dec_deg: f64,
    radius_deg: f64,
    limit: usize,
    max_mag: Option<f32>,
) -> CatalogSelection {
    let legacy = cap_by_mag(index.brightest_in_disc(ra_deg, dec_deg, radius_deg, limit), max_mag);
    let (conc, meaningful) = catalog_concentration(&legacy, index.header().nside, radius_deg);
    // `meaningful` gates the decision itself, not only the reported number
    // -- see `catalog_concentration`'s doc for why an unmet floor must
    // force legacy rather than trust a noisy raw value.
    let stratified = meaningful && catalog_should_stratify(conc);
    let recs = if stratified {
        // Capped on this branch too. Stratified selection spreads its picks
        // over the disc rather than taking them in magnitude order, so it
        // will reach fainter than `brightest_in_disc` does for the same
        // limit -- which is exactly the case the cap exists to bound.
        cap_by_mag(index.stratified_in_disc(ra_deg, dec_deg, radius_deg, limit), max_mag)
    } else {
        legacy
    };
    CatalogSelection { recs, concentration: meaningful.then_some(conc), stratified }
}

/// Build `ExtractParams` from the command line, falling back to the crate
/// defaults field by field. A malformed value is a usage error: silently
/// keeping the default sends the user off debugging the wrong thing (the same
/// reasoning `--radius` and `--cat-limit` already follow below).
///
/// Returns `String` rather than a dedicated error enum: `psolve-cli` has no
/// usage-error type today (the established pattern is an inline `eprintln!` +
/// `ExitCode::from(2)`, see `--saturation` below), and four flags do not
/// justify inventing one. The single caller turns `Err` into exactly that
/// existing `eprintln!` + exit-2 behaviour, so observable CLI semantics are
/// unchanged.
fn extract_params_from(args: &[&str]) -> Result<psolve_core::extract::ExtractParams, String> {
    let mut p = psolve_core::extract::ExtractParams::default();
    if let Some(v) = flag(args, "--sigma") {
        p.k_sigma = match v.parse::<f32>() {
            Ok(x) if x.is_finite() && x > 0.0 => x,
            _ => return Err(format!("--sigma must be a positive finite number, got {v:?}")),
        };
    }
    if let Some(v) = flag(args, "--min-pix") {
        p.min_pix = match v.parse::<u32>() {
            Ok(x) if x > 0 => x,
            _ => return Err(format!("--min-pix must be a positive integer, got {v:?}")),
        };
    }
    if let Some(v) = flag(args, "--keep") {
        p.keep = match v.parse::<usize>() {
            Ok(x) if x > 0 => x,
            _ => return Err(format!("--keep must be a positive integer, got {v:?}")),
        };
    }
    if let Some(v) = flag(args, "--max-ellipticity") {
        p.max_ellipticity = match v.parse::<f64>() {
            Ok(x) if x.is_finite() && (0.0..=1.0).contains(&x) => x,
            _ => {
                return Err(format!(
                    "--max-ellipticity must be a finite number in 0.0..=1.0, got {v:?}"
                ))
            }
        };
    }
    Ok(p)
}

/// Everything the binning retry needs to redo the catalogue disc query at a
/// corrected radius. Passed by the two HINTED entry points only.
///
/// `radius_header_deg` is deliberately the UNCAPPED header-derived radius,
/// not the radius the first fetch actually used. Dividing an already-capped
/// radius by `XBINNING` and dividing the header value then re-capping are
/// different numbers whenever the cap bound the first fetch, and only the
/// latter is correct. The distinction is invisible in native mode (no cap)
/// and load-bearing in ASTAP mode, whose `-r` is a caller ceiling.
pub(crate) struct CatalogRefetch<'a> {
    pub(crate) index: &'a Index,
    pub(crate) hint_ra: f64,
    pub(crate) hint_dec: f64,
    pub(crate) radius_header_deg: f64,
    /// ASTAP mode's `-r` ceiling. `None` in native mode.
    pub(crate) radius_cap: Option<f64>,
    pub(crate) limit: usize,
    /// Native mode's `--radius`. A caller-supplied radius is an assertion,
    /// exactly as `--scale` is, and is never overridden: the retry falls
    /// back to today's scale-only behaviour.
    pub(crate) explicit_radius: bool,
    /// The magnitude ceiling in force, carried so the refetched disc obeys
    /// the same one. A refetch that silently dropped it would hand the
    /// solver a deeper catalogue than the first fetch used, on the exact
    /// path taken when the first fetch already failed.
    pub(crate) max_mag: Option<f32>,
}

/// One solve attempt's result, including which scale solved it and -- when
/// the retry refetched -- the catalogue that actually produced it.
pub(crate) struct SolveAttempt {
    pub(crate) outcome: Outcome,
    pub(crate) scale_source: &'static str,
    /// The disc the binning retry refetched, when it refetched one --
    /// `Some` whenever the retry issued a second catalogue query, WHATEVER
    /// the retry then returned. `None` means [`outcome`](Self::outcome) came
    /// from the caller's own first fetch: either no retry ran, or it ran
    /// against that same disc.
    ///
    /// Deliberately not conditioned on success. `outcome` is the retry's
    /// outcome once a retry runs -- solved or failed -- so whenever this is
    /// `Some` it is THIS selection, not the caller's first fetch, that
    /// produced what `outcome` says. A caller that reports the first disc's
    /// concentration alongside either kind of result emits a
    /// plausible-looking wrong number, which is the failure shape this
    /// codebase pays for most; on a failure it also misleads precisely the
    /// person debugging why the frame did not solve.
    pub(crate) refetched: Option<CatalogSelection>,
}

/// Solve a prepared frame, retrying once at `scale / XBINNING` if the first
/// attempt failed and the scale it used was header-derived -- the CLI-side
/// policy fix for XPIXSZ's binning ambiguity (see this module's own doc on
/// the call site below for the full reasoning). `pub(crate)`, not private:
/// both production entry points -- native `psolve solve` (`solve_cmd` below)
/// and ASTAP-compatible `psolve -f ...` (`main.rs`'s `astap_cmd`) call
/// `prepare`/`solve_prepared` themselves and both need this same retry, or
/// the 810 real bin-2 sv405 frames that motivated it stay unsolvable through
/// whichever entry point does not call it -- exactly the gap fix round 1
/// found: the retry lived only in `solve_cmd`, so the ASTAP-compatible path
/// (the one `ingest.identify.astap_solve` and therefore AstroOps' drop-in
/// integration actually uses) was still broken.
///
/// `explicit_scale_given` must be `true` only when the caller resolved
/// `opts.scale_arcsec` from an explicit, caller-supplied assertion (native
/// mode's `--scale`) rather than letting it fall back to the header --  an
/// explicit scale is never second-guessed, so the retry is skipped
/// unconditionally when this is `true`. ASTAP's own flag grammar has no
/// scale-override flag at all (`-fov` only bounds the search radius, see
/// `astap_args.rs`), so `astap_cmd` always passes `false` here -- the scale
/// there is unconditionally header-derived, and that must not widen this
/// gate beyond its own `XBINNING > 1` check, which still applies regardless.
///
/// Returns a [`SolveAttempt`]: the final outcome, which scale produced it --
/// `"explicit"` | `"header"` | `"header/binning-retry"` -- so a caller that
/// surfaces this (native mode's JSON `field.scale_source`) can report it, and
/// the refetched [`CatalogSelection`] when the retry redid the disc query,
/// whether or not that retry then solved.
/// ASTAP mode does not surface `scale_source` today (its `.ini` format has no
/// field for it) but shares the same retry.
///
/// `refetch` is `Some` only for the HINTED entry points, whose disc is centred
/// on the hint and sized by the same header-derived scale this corrects. See
/// [`CatalogRefetch`].
/// Kernel sigma, in pixels, for the matched-filter extraction retry.
///
/// **1.5 is measured.** Scored on completeness -- the fraction of in-frame
/// catalogue stars psolve detects, against ASTAP's solved WCS -- over the
/// ATR585M frames ASTAP solves and psolve does not: baseline 11.0%, sigma 1.0
/// gives 27.9%, **sigma 1.5 gives 28.0%**. Sigma 1.0 recovered one more frame
/// of six but cost two of eleven controls; 1.5 cost none. See
/// `docs/superpowers/2026-08-24-detection-experiments.md`.
///
/// Do NOT pair this with a lowered `--sigma`: measured together they give
/// 13.8%, worse than either alone, because a lower threshold re-admits
/// exactly the noise the filter suppressed.
pub(crate) const MATCHED_FILTER_SIGMA: f64 = 1.5;

/// Whether a failure is worth re-extracting for.
///
/// `NoQuadMatch` and `TooFewStars` are both consistent with "the detector did
/// not find enough of the right stars", which is what the matched filter
/// addresses. `LowConfidence` is not: a transform was found and the evidence
/// gate refused it, and re-extracting to get past a confidence gate is the
/// shape that once produced a confident solve 87.77 degrees from the truth.
fn worth_re_extracting(outcome: &Outcome) -> bool {
    matches!(
        outcome,
        Outcome::Failed {
            reason: psolve_core::error::ReasonCode::NoQuadMatch
                | psolve_core::error::ReasonCode::TooFewStars,
            ..
        }
    )
}

// Eight arguments, one over clippy's default. Bundling them into a struct was
// considered and rejected: every one is an independent axis this function
// switches on (the frame, the catalogue, the options, the header, whether the
// scale was explicit, the refetch, the bytes), a struct would name them twice,
// and this is deliberately the ONE function all three solve paths call so that
// a new retry cannot reach one entry point and miss another -- which is the
// defect that shipped on 2026-08-14. Growing here is the point.
#[allow(clippy::too_many_arguments)]
pub(crate) fn solve_with_binning_retry(
    path: &str,
    prepared: &psolve_core::solve::PreparedFrame,
    catalog: &[CatalogStar],
    opts: &SolveOptions,
    hdr: Option<&psolve_core::fits::FitsHeader>,
    explicit_scale_given: bool,
    refetch: Option<CatalogRefetch<'_>>,
    // The frame's bytes, for the matched-filter retry, which must re-run
    // extraction and therefore cannot reuse `prepared`. `None` disables that
    // retry -- the blind path passes `None`, since it has already spent a
    // large search and a second full extraction is not the cheap step there.
    fits_bytes: Option<&[u8]>,
) -> SolveAttempt {
    let mut result = psolve_core::solve::solve_prepared(prepared, catalog, opts);
    let mut scale_source = if explicit_scale_given { "explicit" } else { "header" };
    // The best inputs any rung has produced, for the pair-matching rung at
    // the foot of this ladder. A retry that improved the catalogue disc or
    // the star list improved it whether or not it went on to solve, and the
    // last rung should not be handed the inputs an earlier rung already
    // superseded. Measured: running pair matching on the original inputs
    // instead of these recovered 73 of the corpus's failing frames where
    // the improved ones recover 87.
    let mut best_catalog: Option<Vec<CatalogStar>> = None;
    let mut best_prepared: Option<psolve_core::solve::PreparedFrame> = None;
    // The corrected plate scale, once the binning retry has established one.
    // Pair matching converts pixel separations to angles, so it is the rung
    // most sensitive to the scale being wrong.
    let mut best_scale: Option<f64> = None;
    let mut refetched = None;

    // XPIXSZ is ambiguous when XBINNING > 1: `pixel_scale_arcsec` assumes
    // XPIXSZ is the PHYSICAL pixel and multiplies it by the binning factor
    // itself, which is correct for most rigs -- but this project's sv405
    // rig's driver already writes the ALREADY-BINNED pixel into XPIXSZ, so
    // multiplying by binning again overstates the plate scale by another
    // factor of `binning` (measured: 9.26 written where the physical pixel
    // is 4.63, deriving 15.6"/px where the truth is 7.8"/px -- 0 of 400
    // affected frames solved at the wrong scale, ~67% solve at the right
    // one). There is no reliable way to tell which convention a single
    // header used, so this does not guess: it solves at the header-derived
    // scale first and, only on failure, retries ONCE at scale/binning -- the
    // value implied by "XPIXSZ was already binned". Skipped whenever the
    // caller passed an explicit scale (that is the caller's own assertion)
    // and whenever XBINNING <= 1 (the sole case this ambiguity can even
    // exist). A failed solve now costs roughly 2x as long on binned frames
    // -- acceptable, since psolve solves in well under 100 ms even once.
    if matches!(result, Outcome::Failed { .. }) && !explicit_scale_given {
        let xbinning = hdr.and_then(|h| h.num("XBINNING")).unwrap_or(1.0);
        let header_scale = hdr.and_then(psolve_core::fits::pixel_scale_arcsec).map(|s| {
            let cfa_binning = hdr.map(psolve_core::fits::binning_factor).unwrap_or(1);
            s * cfa_binning as f64
        });
        if let (true, Some(header_scale)) = (xbinning > 1.0, header_scale) {
            let alt_scale = header_scale / xbinning;
            eprintln!(
                "solving {path}: header scale {header_scale:.4}\"/px did not solve; \
retrying once at {alt_scale:.4}\"/px (scale / XBINNING {xbinning:.0}, in case XPIXSZ was \
already binned)"
            );
            let mut retry_opts = *opts;
            retry_opts.scale_arcsec = Some(alt_scale);
            best_scale = Some(alt_scale);

            // The retry must also redo the CATALOGUE, not just the scale.
            // The disc radius is derived from the same inflated scale, so it
            // comes out `xbinning` times too wide -- 6.02 deg where the frame
            // needs 3.01 -- and the star budget is spent across `xbinning^2`
            // times too much sky. Measured 2026-08-22: 0 of 791 real bin-2
            // frames solved with the scale corrected but the catalogue left
            // alone; 790 of 791 solve once the disc is right. Correcting the
            // scale against a swamped catalogue reports NO_QUAD_MATCH, which
            // reads as "unsolvable frame" rather than "the disc was twice too
            // wide".
            //
            // Refetch only when the caller supplied the means and did not
            // assert a radius of its own. Skipped when the corrected radius
            // equals what was already fetched (a cap binding both times) --
            // no query is issued to arrive at the same disc.
            let alt_catalog: Option<(Vec<CatalogStar>, CatalogSelection)> = refetch
                .as_ref()
                .filter(|r| !r.explicit_radius)
                .and_then(|r| {
                    let first =
                        r.radius_cap.map_or(r.radius_header_deg, |c| r.radius_header_deg.min(c));
                    let corrected = r.radius_header_deg / xbinning;
                    let corrected = r.radius_cap.map_or(corrected, |c| corrected.min(c));
                    if (corrected - first).abs() < 1e-9 {
                        return None;
                    }
                    let sel =
                        select_catalog(r.index, r.hint_ra, r.hint_dec, corrected, r.limit, r.max_mag);
                    // Both counts, not just the radii: "300 -> 60" is the
                    // whole point of this fix and the number a corpus run
                    // needs to show it fired, without re-instrumenting the
                    // binary to recover it afterwards.
                    eprintln!(
                        "solving {path}: refetched the catalogue -- {} stars within \
{corrected:.4} deg (was {} within {first:.4}) -- the first disc was derived from the \
uncorrected scale",
                        sel.recs.len(),
                        catalog.len()
                    );
                    let stars: Vec<CatalogStar> = sel
                        .recs
                        .iter()
                        .map(|s| CatalogStar {
                            ra: s.ra_deg(),
                            dec: s.dec_deg(),
                            mag: s.mag(),
                            pmra: s.pmra_mas_yr(),
                            pmdec: s.pmdec_mas_yr(),
                        })
                        .collect();
                    Some((stars, sel))
                });

            let retry_result = match &alt_catalog {
                Some((stars, _)) => {
                    psolve_core::solve::solve_prepared(prepared, stars, &retry_opts)
                }
                None => psolve_core::solve::solve_prepared(prepared, catalog, &retry_opts),
            };
            if matches!(retry_result, Outcome::Solved(_)) {
                scale_source = "header/binning-retry";
            }
            // NOT conditioned on the retry solving. `result` becomes the
            // retry's outcome either way, so the refetched disc is the one
            // that produced whatever is reported -- and a failure reported
            // against the discarded first disc's concentration misleads the
            // exact person reading it, the one debugging a frame that did
            // not solve.
            if let Some((stars, sel)) = alt_catalog {
                // Kept, not dropped: the last rung below matches against
                // the disc that actually produced this outcome, not the
                // one the corrected radius replaced.
                best_catalog = Some(stars);
                refetched = Some(sel);
            }
            result = retry_result;
        }
    }

    // Matched-filter extraction retry, LAST -- after the scale retry has had
    // its turn, because a wrong plate scale is cheaper to fix and more often
    // the cause. Re-extraction means a full second decode+background+extract
    // (25.3 ms -> 211.3 ms of extraction alone on a real frame), so it is
    // paid only by frames that have already failed everything else.
    //
    // A frame that solves never reaches this, so no solved answer can change.
    if let Some(bytes) = fits_bytes {
        if worth_re_extracting(&result) {
            let mut mf_opts = *opts;
            mf_opts.extract.matched_filter_sigma = MATCHED_FILTER_SIGMA;
            if let Ok(re) = psolve_core::solve::prepare(bytes, &mf_opts) {
                let before = prepared.usable_star_count();
                let after = re.usable_star_count();
                let cat_now: &[CatalogStar] =
                    best_catalog.as_deref().unwrap_or(catalog);
                let retry = psolve_core::solve::solve_prepared(&re, cat_now, &mf_opts);
                if matches!(retry, Outcome::Solved(_)) {
                    eprintln!(
                        "solving {path}: re-extracted with a matched filter (sigma {MATCHED_FILTER_SIGMA}) \
-- {after} usable stars (was {before})"
                    );
                    result = retry;
                    scale_source = "header/matched-filter";
                } else if after > before {
                    // Did not solve, but found more stars than the first
                    // extraction did. The last rung wants those.
                    best_prepared = Some(re);
                }
            }
        }
    }

    // Pair matching, the last rung. After the scale retry and the
    // matched-filter re-extraction, because both are cheaper and more often
    // the cause, and because pair matching is the expensive one: on 60
    // frames both matchers solve it ran a 4.82 s p90 against the quad
    // path's 0.16 s.
    //
    // Placing it here rather than defaulting it on inside `solve_prepared`
    // is not a style choice. With it on by default the FIRST attempt took
    // the pair path, pre-empting the two retries above: 41 corpus frames
    // that had solved through the binning retry solved through pair
    // matching instead. Same answers, but a route that moved underneath a
    // frame that was already working.
    //
    // A frame that solves never reaches this, so no solved answer changes.
    if matches!(result, Outcome::Failed { .. }) {
        let mut pair_opts = *opts;
        pair_opts.pair_retry = true;
        if let Some(sc) = best_scale {
            pair_opts.scale_arcsec = Some(sc);
        }
        if best_prepared.is_some() {
            pair_opts.extract.matched_filter_sigma = MATCHED_FILTER_SIGMA;
        }
        let frame = best_prepared.as_ref().unwrap_or(prepared);
        let cat: &[CatalogStar] = best_catalog.as_deref().unwrap_or(catalog);
        let retry = psolve_core::solve::solve_prepared(frame, cat, &pair_opts);
        if matches!(retry, Outcome::Solved(_)) {
            result = retry;
            scale_source = "header/pair-match";
        } else {
            // Adopt the failure too, not only the success. This rung's
            // detail starts from the quad rung's own message and appends the
            // pair search's counts to it, so it is a strict superset -- and
            // discarding it left every `NO_QUAD_MATCH` reporting
            // "1500 image quads vs 1500 catalogue quads" with no trace that
            // a second matcher had run at all.
            //
            // Guarded by `keep_most_informative`'s rule rather than assigned
            // outright: a `LowConfidence` already in hand is the only
            // evidence the acceptance gate was ever reached, and must not be
            // overwritten by a failure from earlier in the pipeline.
            let mut held = Some((result, scale_source));
            keep_most_informative(&mut held, (retry, scale_source));
            let (kept, src) = held.expect("keep_most_informative always leaves a value");
            result = kept;
            scale_source = src;
        }
    }

    // Tight-radius rung, the last one. A disc sized against pointing error is
    // larger than the frame, and catalogue stars outside the frame can only
    // lower completeness -- see `RADIUS_RETRY_HALF_DIAG_FRAC` for the sweep
    // that measured it and for the cross-frame prior this replaced.
    //
    // Last, and refetching rather than reusing, so a frame that solves at the
    // default disc never reaches it and cannot change its answer.
    if matches!(result, Outcome::Failed { .. }) {
        let tight = refetch
            .as_ref()
            .filter(|r| !r.explicit_radius)
            .and_then(|r| {
                let half_diag = r.radius_header_deg / RADIUS_MARGIN;
                let tight = half_diag * RADIUS_RETRY_HALF_DIAG_FRAC;
                let already =
                    r.radius_cap.map_or(r.radius_header_deg, |c| r.radius_header_deg.min(c));
                // No query to arrive at a disc already used, and never wider
                // than what has been tried: this rung only ever narrows.
                if tight >= already || tight <= 0.0 {
                    return None;
                }
                let sel = select_catalog(r.index, r.hint_ra, r.hint_dec, tight, r.limit, r.max_mag);
                let stars: Vec<CatalogStar> = sel
                    .recs
                    .iter()
                    .map(|s| CatalogStar {
                        ra: s.ra_deg(),
                        dec: s.dec_deg(),
                        mag: s.mag(),
                        pmra: s.pmra_mas_yr(),
                        pmdec: s.pmdec_mas_yr(),
                    })
                    .collect();
                Some((stars, sel, tight))
            });
        if let Some((stars, sel, tight)) = tight {
            let mut tight_opts = *opts;
            tight_opts.pair_retry = true;
            if let Some(sc) = best_scale {
                tight_opts.scale_arcsec = Some(sc);
            }
            if best_prepared.is_some() {
                tight_opts.extract.matched_filter_sigma = MATCHED_FILTER_SIGMA;
            }
            let frame = best_prepared.as_ref().unwrap_or(prepared);
            let retry = psolve_core::solve::solve_prepared(frame, &stars, &tight_opts);
            if matches!(retry, Outcome::Solved(_)) {
                eprintln!(
                    "solving {path}: retried at a tight search radius {tight:.4} deg \
({RADIUS_RETRY_HALF_DIAG_FRAC}x the frame half-diagonal) -- {} catalogue stars",
                    stars.len()
                );
                result = retry;
                scale_source = "header/tight-radius";
                refetched = Some(sel);
            } else {
                let mut held = Some((result, scale_source));
                keep_most_informative(&mut held, (retry, scale_source));
                let (kept, src) = held.expect("keep_most_informative always leaves a value");
                result = kept;
                scale_source = src;
            }
        }
    }

    SolveAttempt { outcome: result, scale_source, refetched }
}

// ---------------------------------------------------------------------------
// Blind solving (Task 7): wiring the pieces Tasks 4-6 left unreachable.
//
// A hinted solve fetches a catalogue disc around a known pointing and
// matches image quads against it. Blind solving has no pointing: instead it
// takes every image quad's own scale-invariant code and looks it up
// directly in a `.psqidx` code-space index (`psolve-index`), which offers a
// handful of candidate catalogue quads per lookup regardless of where in
// the sky they are. `psolve-core::blind::candidate_transform` turns a
// surviving (image quad, candidate) pair into a local TAN transform; this
// module clusters those by their implied sky position, refines the
// best-agreed-on cluster through the SAME hinted pipeline every ordinary
// solve already uses, and judges the result against the multiplicity-
// corrected gate `psolve-core::verify` derived for exactly this situation
// (see `solve_blind`'s own doc for why that gate -- not `solve_prepared`'s
// own internal one -- must be the final word).

/// Euclidean tolerance, in quad-code units, for a `.psqidx` code-space
/// lookup. The same order of magnitude `match_::MatchParams::default().
/// code_tol` uses for the hinted path's own code-space comparison (Task 4's
/// own code-space search was measured against it) -- not read from
/// `SolveOptions.match_` directly, since that is a hinted-path knob a
/// caller might override for reasons unrelated to this lookup, and this
/// tolerance should not silently move with it.
const BLIND_CODE_TOL: f64 = 0.02;

/// How many distinct candidate sky positions a blind search will attempt a
/// full refinement solve against, in descending order of how many
/// independent image quads agreed on them, before giving up. Each attempt
/// costs one real catalogue disc fetch plus one full hinted-style
/// match+fit -- cheap (tens of ms) but not free, so this still bounds the
/// worst case (many false-positive clusters, no true one) well inside the
/// milestone's 5s target rather than working through every cluster found.
///
/// **Re-measured for fix round 1: 5 was binding, and cost real solves.**
/// The search now keeps iterating until a candidate clears the real blind
/// gate (`solve_blind`'s own doc explains why stopping at the first
/// `solve_prepared`-internal "Solved" was wrong: that gate scores exactly
/// one hypothesis and a coincidental early consensus can pass it while
/// still being false). Measured on a real-frame sample: every hinted-
/// solvable frame that still failed blind at the original cap of 5 had
/// exhausted all 5 attempts without ever reaching the true cluster; two
/// needed the 9th and 10th candidate cluster tried, and both then produced
/// correct answers (within ~0.01" of their own hinted centres). At 40, the
/// sample reached parity with the hinted path (no blind-solvable frame left
/// on the table, zero disagreements over 5"), with a measured worst-case
/// wall clock of 1.50s -- still 3.3x inside the 5s target this constant's
/// own budget reasoning is about.
const MAX_BLIND_CLUSTER_ATTEMPTS: usize = 40;

/// Above this many surviving candidate transforms, skip the O(n^2)
/// neighbour-count clustering below and fall back to ordering by fit
/// quality (RMS) alone. Ordinary frames -- even an unmatched one -- produce
/// at most a few hundred survivors in practice; this is a defensive bound
/// against a pathological input, not a value expected to bind.
const BLIND_CLUSTER_N2_CAP: usize = 4000;

/// The outcome of one blind-solve orchestration run, plus the search
/// diagnostics both entry points log to stderr. Never folded into the
/// solved/failed JSON: that format is shared with the hinted path, which
/// must stay byte-for-byte unchanged (Task 6/7's own binding constraint).
pub(crate) struct BlindSearch {
    pub(crate) outcome: Outcome,
    /// `M`: every candidate this search OFFERED to
    /// `blind::candidate_transforms`, summed across every (image quad,
    /// band) lookup -- **not** the number of candidate transforms that
    /// survived. Exactly the number handed to `verify::AcceptParams::blind`
    /// to reach `outcome`'s accept/reject decision.
    pub(crate) hypotheses: usize,
    pub(crate) image_quads: usize,
    pub(crate) survivors: usize,
    pub(crate) clusters_tried: usize,
    pub(crate) scale_source: &'static str,
}

/// Resolve a `.psqidx` quad record's four star references back to sky
/// positions via the paired `.psidx`, in the SAME position order
/// `blind::candidate_transform` requires (`star_idx[k]` <-> position `k`).
/// `None` if any of the four indices fails to resolve -- `Index::star_at`
/// already refuses to read past its own record count, and a pairing that
/// passed `QuadIndex::open`'s fingerprint check should never produce that,
/// but this is untrusted on-disk input and must not panic if it somehow
/// does.
fn resolve_quad_sky(star_index: &Index, star_idx: &[u32; 4]) -> Option<[(f64, f64); 4]> {
    let mut out = [(0.0, 0.0); 4];
    for (slot, &gi) in out.iter_mut().zip(star_idx.iter()) {
        *slot = star_index.star_at(gi).map(|r| (r.ra_deg(), r.dec_deg()))?;
    }
    Some(out)
}

/// Keep the most informative blind-cluster failure seen so far.
///
/// `LowConfidence` is the blind gate's own refusal code, and the only
/// evidence in the emitted JSON that a candidate ever reached
/// `AcceptParams::blind` at all. Both call sites used to assign
/// unconditionally, so a later cluster failing earlier in the pipeline
/// silently overwrote it -- and a frame whose gate actually ran reported a
/// bare `NO_QUAD_MATCH`. That makes the reason code useless for the one
/// question a null-sky measurement asks: did the gate refuse this, or did
/// nothing ever reach it? (Task 8's acceptance run could not answer that
/// from its own JSON; it took an instrumented build.)
///
/// Gate refusals therefore win over non-gate failures. Among two failures
/// of the same kind the later one wins, preserving the previous behaviour.
fn keep_most_informative(
    current: &mut Option<(Outcome, &'static str)>,
    incoming: (Outcome, &'static str),
) {
    use psolve_core::error::ReasonCode;
    let incoming_is_gate =
        matches!(incoming.0, Outcome::Failed { reason: ReasonCode::LowConfidence, .. });
    let held_is_gate = matches!(
        current,
        Some((Outcome::Failed { reason: ReasonCode::LowConfidence, .. }, _))
    );
    if incoming_is_gate || !held_is_gate {
        *current = Some(incoming);
    }
}

/// Which `.psqidx` bands are worth searching for an image quad whose pixel
/// diagonal is `diag_px`, given an (optional) plate scale estimate.
///
/// A quad code is scale-invariant (`quad.rs`'s own module doc), so bands
/// exist purely to recover the physical footprint a code-space match alone
/// throws away: this quad's ACTUAL on-sky diagonal has to match the
/// physical footprint the candidate catalogue quad was drawn from, or the
/// "match" is comparing two shapes at wildly different real scales that
/// merely happen to look similar. With a scale estimate, only bands within
/// a factor of 2 either way of the quad's own implied diagonal are
/// searched -- generous against ordinary header-scale error (the
/// XPIXSZ/binning ambiguity `solve_with_binning_retry` exists for is at
/// most a factor of the binning value, almost always 2), narrow enough to
/// keep `M` close to the single-band case Task 6's derivation and tests are
/// built around (~12,600, not ~75,600). Landing in the wrong band from a
/// bad scale estimate only makes the search less SENSITIVE, never less
/// honest -- `M` is counted from what was actually offered, not assumed.
///
/// Without a scale estimate, every band is searched -- `M` grows to match
/// (up to ~6x, the "six bands" case `verify.rs`'s own doc names), and the
/// multiplicity gate grows with it, exactly as it should for a search that
/// genuinely tried more.
///
/// **This is the one stage the binning retry does not reach** (noted in the
/// `blind-solve` whole-branch review, 2026-08-23). `scale_arcsec` comes from
/// `header_scale_arcsec()`, which is `fits::pixel_scale_arcsec(header) *
/// img.binned` -- and `pixel_scale_arcsec` ALREADY multiplies by `XBINNING`
/// (`fits.rs:186`). On a hardware-binned SV405CC frame the driver still
/// writes `BAYERPAT`, so `img.binned` is 2 as well and the factor lands
/// twice: 2x too coarse, the entire reason `solve_with_binning_retry`
/// exists. The window admitted here is
/// `diag_deg / band_scale` in `[0.5, 2.0]`, while `band_for_diag`'s geometric
/// midpoints put a quad's true band at `[0.707, 1.414)`; a 2x overestimate
/// shifts that to `[1.414, 2.828)`, so roughly the upper half falls outside
/// the window. The `bands.is_empty()` fallback does NOT fire, because other
/// (wrong) bands still qualify.
///
/// The effect is reduced sensitivity, never a wrong answer: `M` is counted
/// from what was actually offered, so the gate stays honest, and the Task 8
/// acceptance run's 26 SV405CC frames all solved blind wherever they solved
/// hinted. Widening the window when `XBINNING > 1` is the obvious fix and is
/// deliberately not taken here -- it would change blind-path sensitivity on
/// one rig with no measurement to justify the new number.
fn select_bands(n_bands: usize, band_scales_deg: &[f32], diag_px: f64, scale_arcsec: Option<f64>) -> Vec<usize> {
    let Some(scale) = scale_arcsec else {
        return (0..n_bands).collect();
    };
    let diag_deg = diag_px * scale / 3600.0;
    if !diag_deg.is_finite() || diag_deg <= 0.0 {
        return (0..n_bands).collect();
    }
    let mut bands: Vec<usize> = (0..n_bands)
        .filter(|&b| {
            let bs = *band_scales_deg.get(b).unwrap_or(&0.0) as f64;
            bs > 0.0 && (0.5..=2.0).contains(&(diag_deg / bs))
        })
        .collect();
    if bands.is_empty() {
        // The estimate landed between bands (or outside every band's 2x
        // window): fall back to the single nearest band by log-ratio rather
        // than searching none at all.
        if let Some((nearest, _)) = (0..n_bands)
            .filter_map(|b| {
                let bs = *band_scales_deg.get(b).unwrap_or(&0.0) as f64;
                (bs > 0.0).then(|| (b, (diag_deg / bs).ln().abs()))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
        {
            bands.push(nearest);
        }
    }
    bands
}

/// Blind-solve orchestration, shared by both entry points
/// (`solve_cmd` below and `main.rs`'s `astap_cmd`) -- the 2026-08-14
/// scale/binning retry's own defect (a fix landing in `cmd_solve.rs` alone,
/// leaving ASTAP-compatible dispatch on stale behaviour) is exactly the
/// shape a second, drifting copy of this orchestration would reintroduce.
///
/// `prepared` must already exist (decode/background/extract, hint-
/// independent), `base_opts.hint` is ignored (this function decides its
/// own), and `base_opts.accept` is likewise ignored -- both entry points
/// pass whatever they already built for the hint-independent fields
/// (extract params, scale, saturation, catalogue epoch, `max_quads`).
///
/// ## Why this reuses `solve_prepared` for refinement rather than trusting
/// a single quad's own local fit
///
/// `blind::candidate_transform`'s own fit is accurate at the four points it
/// was built from but is not expected to extrapolate to sub-arcsecond
/// accuracy across a whole multi-degree frame (bounded, but real -- see
/// that module's own report). Reprojecting a surviving candidate's fit at
/// this frame's own centre gives a SEED position good to roughly
/// arcminutes, not sub-arcsecond -- plenty to seed a real catalogue disc
/// fetch at the usual search radius. Handing that seed to the exact same
/// `solve_prepared` a hinted solve already uses gets the milestone's own
/// 30" accuracy criterion for free: `solve_prepared` matches MANY quads
/// against a REAL catalogue disc and refits on every correspondence it
/// finds, exactly as it always does.
///
/// ## Why `solve_prepared`'s own accept gate cannot be the final word
///
/// `solve_prepared` is called below with a deliberate PASS-THROUGH accept
/// (`min_matched: 0`, `min_log_odds: -inf`, `max_rms_px: inf`), not
/// `AcceptParams::blind(hypotheses)`. `solve_prepared` computes its OWN
/// confidence via bare `verify::confidence`, which -- per `verify.rs`'s
/// "blind null" doc and Task 6's fix round 1 -- credits a candidate with
/// `n_fit` free matches (the correspondences it was fit to, which reproject
/// onto themselves by construction) and is calibrated to score exactly ONE
/// hypothesis, not the `hypotheses` this search actually tried. Using it
/// here would silently reopen both defects Task 6 exists to close. The
/// pass-through lets `solve_prepared`'s well-tested matcher/fitter run to
/// completion whenever it CAN converge; the real accept/reject decision
/// happens below, against `blind_confidence` (`n_fit` deducted) and
/// `AcceptParams::blind(hypotheses)` -- the REAL multiplicity this search
/// examined.
///
/// ## `M`
///
/// Accumulated by `verify::HypothesisCount`, one `(image quad, band)`
/// lookup at a time, charged with the length of the candidate slice handed
/// to `blind::candidate_transforms` -- never the length of the `Vec` it
/// returns (survivors only). The gate is applied exactly once, after every
/// image quad and band has been searched -- never incrementally against a
/// partial total.
#[allow(clippy::too_many_arguments)] // orchestration: see this fn's own doc
pub(crate) fn solve_blind(
    path: &str,
    prepared: &PreparedFrame,
    hdr: Option<&psolve_core::fits::FitsHeader>,
    star_index: &Index,
    quad_index: &QuadIndex,
    radius_deg: f64,
    cat_limit: usize,
    // Magnitude ceiling, same meaning as in the hinted path. Blind wants it
    // at least as much: it resolves candidate positions all over the sky,
    // so an uncapped deep index costs it a deeper catalogue at every one.
    max_mag: Option<f32>,
    base_opts: &SolveOptions,
    explicit_scale_given: bool,
) -> BlindSearch {
    let fail_no_match = |detail: String, hypotheses: usize, image_quads: usize| BlindSearch {
        outcome: Outcome::Failed {
            reason: psolve_core::error::ReasonCode::NoQuadMatch,
            detail,
            stars_detected: prepared.detected_star_count(),
            stars_used: prepared.usable_star_count(),
            concentration: prepared.concentration(),
            stratified: prepared.stratified(),
            rejected: psolve_core::extract::Rejections::default(),
        },
        hypotheses,
        image_quads,
        survivors: 0,
        clusters_tried: 0,
        scale_source: "n/a",
    };

    let image_pts = prepared.image_points();
    let (nx, ny) = prepared.image_dims();
    let iq = psolve_core::quad::build_quads(&image_pts, 6, base_opts.max_quads);
    if iq.is_empty() {
        return fail_no_match("no image quads could be formed".into(), 0, 0);
    }

    let scale_estimate = base_opts.scale_arcsec.or_else(|| prepared.header_scale_arcsec());
    let n_bands = quad_index.header().n_bands as usize;
    let band_scales = quad_index.header().band_scales_deg();

    let mut hyp = HypothesisCount::new();
    let mut survivors: Vec<psolve_core::fit::FitResult> = Vec::new();
    for q in &iq {
        for band in select_bands(n_bands, &band_scales, q.diag, scale_estimate) {
            let cands: Vec<QuadRecord> = quad_index.candidates(q.code, BLIND_CODE_TOL, band).collect();
            let resolved: Vec<[(f64, f64); 4]> =
                cands.iter().filter_map(|c| resolve_quad_sky(star_index, &c.star_idx)).collect();
            // Charged with what was actually handed to `candidate_transforms`
            // below -- see this function's own "M" doc; never `cands.len()`
            // alone if a resolution somehow failed, and never the survivor
            // count `candidate_transforms` returns.
            hyp.offered(resolved.len());
            if resolved.is_empty() {
                continue;
            }
            survivors.extend(psolve_core::blind::candidate_transforms(q, &image_pts, &resolved));
        }
    }
    let hypotheses = hyp.total();

    if survivors.is_empty() {
        return fail_no_match(
            format!("{} image quads, {hypotheses} candidate hypotheses offered, none survived", iq.len()),
            hypotheses,
            iq.len(),
        );
    }

    // Cluster survivors by their IMPLIED field-centre pointing (each
    // candidate's own local fit, evaluated at this frame's own centre --
    // see this function's own doc for why that seed is trusted only to
    // roughly arcminute precision, not sub-arcsecond).
    let centers: Vec<(f64, f64)> = survivors
        .iter()
        .map(|f| f.wcs.pix_to_radec((nx as f64 - 1.0) / 2.0, (ny as f64 - 1.0) / 2.0))
        .collect();

    let mut order: Vec<usize> = (0..centers.len()).collect();
    if centers.len() <= BLIND_CLUSTER_N2_CAP {
        let counts: Vec<usize> = (0..centers.len())
            .map(|i| {
                (0..centers.len())
                    .filter(|&j| {
                        psolve_core::project::angsep_deg(
                            centers[i].0, centers[i].1, centers[j].0, centers[j].1,
                        ) <= radius_deg
                    })
                    .count()
            })
            .collect();
        order.sort_by(|&a, &b| {
            counts[b].cmp(&counts[a]).then_with(|| survivors[a].rms_deg.total_cmp(&survivors[b].rms_deg))
        });
    } else {
        // Defensive fallback for a pathologically large survivor set -- see
        // `BLIND_CLUSTER_N2_CAP`'s own doc.
        order.sort_by(|&a, &b| survivors[a].rms_deg.total_cmp(&survivors[b].rms_deg));
    }

    // The REAL accept gate, fixed once here -- `hypotheses` (M) is already
    // final (every image quad and band was searched above, before this
    // point), so evaluating every attempt below against this SAME
    // `accept_params` does not violate "gate once with the final M": M does
    // not change per attempt, only which candidate is being judged against
    // it does.
    let accept_params = AcceptParams::blind(hypotheses);

    let mut tried_centers: Vec<(f64, f64)> = Vec::new();
    let mut last_failure: Option<(Outcome, &'static str)> = None;
    let mut clusters_tried = 0usize;

    for &i in &order {
        if clusters_tried >= MAX_BLIND_CLUSTER_ATTEMPTS {
            break;
        }
        let c = centers[i];
        // NOTE: `radius_deg` is doing double duty here. Its primary meaning
        // is "catalogue disc radius"; this use is "how far apart two
        // candidate pointings must be to count as independent clusters".
        // At the normal ~1.5 deg the two coincide sensibly, but `radius_deg`
        // is user-facing: `--radius 15`, or ASTAP `-r 180` on a frame with no
        // FOCALLEN/XPIXSZ and no `-fov` (where `search_radius_deg` passes
        // `-r` through verbatim), collapses every survivor into the first
        // `tried_centers` entry. Exactly one cluster is then ever attempted,
        // defeating `MAX_BLIND_CLUSTER_ATTEMPTS` -- which fix round 1
        // measured as load-bearing, two frames needing the 9th and 10th
        // cluster. Not a safety hole (a smaller search only refuses more),
        // but the tolerance would be better derived from the frame's own
        // field half-diagonal than borrowed from the disc radius. Raised in
        // the whole-branch review, 2026-08-23; left as-is because changing
        // it alters which frames solve and that needs its own measurement.
        if tried_centers
            .iter()
            .any(|&t| psolve_core::project::angsep_deg(t.0, t.1, c.0, c.1) <= radius_deg)
        {
            // Same neighbourhood as an already-attempted seed -- not an
            // independent try.
            continue;
        }
        tried_centers.push(c);
        clusters_tried += 1;

        let selection =
            select_catalog(star_index, c.0, c.1, radius_deg, cat_limit, max_mag);
        let catalog: Vec<CatalogStar> = selection
            .recs
            .iter()
            .map(|r| CatalogStar {
                ra: r.ra_deg(),
                dec: r.dec_deg(),
                mag: r.mag(),
                pmra: r.pmra_mas_yr(),
                pmdec: r.pmdec_mas_yr(),
            })
            .collect();
        let n_cat = catalog.len();

        let mut attempt_opts = *base_opts;
        attempt_opts.hint = Some(c);
        // Pass-through -- see this function's own doc: `solve_prepared`'s
        // OWN gate scores exactly one hypothesis and must not be trusted;
        // it exists here only to let the matcher/fitter run to completion.
        attempt_opts.accept =
            AcceptParams { min_matched: 0, min_log_odds: f64::NEG_INFINITY, max_rms_px: f64::INFINITY };

        // `None`: the blind path's disc is centred on a CANDIDATE CLUSTER,
        // not on a hint, and its radius is not the header-derived one this
        // correction divides. Refetching here is unmeasured, and the blind
        // path was proved bit-identical to the hinted one's behaviour in the
        // blind-solve milestone's Task 6 -- it stays that way until someone
        // measures it. Deliberate omission, not an oversight.
        let attempt = solve_with_binning_retry(
            path,
            prepared,
            &catalog,
            &attempt_opts,
            hdr,
            explicit_scale_given,
            None,
            // No matched-filter retry on the blind path. A blind solve has
            // already paid for a large code-space search across many
            // hypotheses; a second full decode+background+extract per cluster
            // is not the cheap step here, and the blind path's cost model is
            // measured (median 1.243 s) against a 5 s bar.
            None,
        );
        let (result, scale_source) = (attempt.outcome, attempt.scale_source);

        let mut s = match result {
            Outcome::Solved(s) => s,
            failed @ Outcome::Failed { .. } => {
                keep_most_informative(&mut last_failure, (failed, scale_source));
                continue;
            }
        };

        // `solve_prepared` said "Solved" only via the pass-through above --
        // that means the matcher found A consensus and the fit converged,
        // nothing more (`match_.rs`'s own tests document that an UNRELATED
        // field can return `Some` there: "the matcher proposes; the
        // confidence stage disposes"). One coincidental consensus in an
        // early cluster must NOT end the search: judge THIS candidate
        // against the real gate now, and if it fails, keep trying the
        // remaining clusters rather than stopping here.
        //
        // `s.fit` is `fitres` copied UNCONVERTED from `solve_prepared` --
        // i.e. in the (possibly CFA-binned) grid the match/fit actually ran
        // in, not the FILE-pixel grid `s.wcs`/`s.nx`/`s.ny` report.
        // `tol_px = 2.0` mirrors `solve.rs`'s own hardcoded reprojection
        // tolerance (private to that crate, so duplicated here rather than
        // imported -- a drift risk noted for a future reader) applied in
        // THAT SAME grid, which is also the grid `s.stars_matched` was
        // counted in; recomputing tol_deg/field_area from `s.fit.wcs`'s own
        // (unconverted) scale, rather than `s.wcs`'s FILE-pixel one, keeps
        // them in the statistic's own units.
        let scale_deg_binned = s.fit.wcs.scale_arcsec() / 3600.0;
        let tol_px = 2.0;
        let tol_deg = (tol_px * scale_deg_binned).max(1e-12);
        let img_nx_binned = s.nx as f64 / s.binned as f64;
        let img_ny_binned = s.ny as f64 / s.binned as f64;
        let field_area_deg2 =
            ((img_nx_binned * scale_deg_binned) * (img_ny_binned * scale_deg_binned)).max(1e-9);
        let rms_px = if scale_deg_binned > 0.0 { s.fit.rms_deg / scale_deg_binned } else { f64::INFINITY };

        let conf = psolve_core::verify::blind_confidence(
            s.stars_matched,
            s.fit.used,
            s.stars_used,
            n_cat,
            tol_deg,
            field_area_deg2,
        );

        if psolve_core::verify::accept(&conf, rms_px, &accept_params) {
            s.confidence = conf;
            return BlindSearch {
                outcome: Outcome::Solved(s),
                hypotheses,
                image_quads: iq.len(),
                survivors: survivors.len(),
                clusters_tried,
                scale_source,
            };
        }

        // A real consensus that is not good enough -- keep this as the
        // most informative failure seen so far (a `LowConfidence` detail
        // beats a bare `NoQuadMatch` if every remaining cluster fails to
        // even reach a fit), but do NOT stop: a later cluster may still be
        // the true one.
        keep_most_informative(
            &mut last_failure,
            (
            Outcome::Failed {
                reason: psolve_core::error::ReasonCode::LowConfidence,
                detail: format!(
                    "blind: {} matched (excess over {} fitted correspondences), {:.2} decades \
(need {:.2} across {hypotheses} hypotheses), rms {:.2} px (need <= {:.2})",
                    conf.matched, s.fit.used, conf.log_odds, accept_params.min_log_odds, rms_px,
                    accept_params.max_rms_px,
                ),
                stars_detected: s.stars_detected,
                stars_used: s.stars_used,
                concentration: s.concentration,
                stratified: s.stratified,
                rejected: s.rejected,
            },
            scale_source,
            ),
        );
    }

    // Every cluster tried (up to the cap) either never reached a fit or
    // failed the real gate -- no candidate accepted.
    let (outcome, scale_source) = last_failure.unwrap_or_else(|| {
        (
            fail_no_match(
                format!(
                    "{} image quads, {hypotheses} hypotheses, {} candidate transforms \
survived across {clusters_tried} cluster(s), none reached a fit",
                    iq.len(),
                    survivors.len()
                ),
                hypotheses,
                iq.len(),
            )
            .outcome,
            "n/a",
        )
    });
    BlindSearch {
        outcome,
        hypotheses,
        image_quads: iq.len(),
        survivors: survivors.len(),
        clusters_tried,
        scale_source,
    }
}

pub fn solve_cmd(args: &[&str]) -> ExitCode {
    let Some(path) = positional(args) else {
        eprintln!("psolve solve: <FILE> is required");
        return ExitCode::from(2);
    };
    let Some(index_path) = flag(args, "--index") else {
        eprintln!("psolve solve: --index <FILE> is required");
        return ExitCode::from(2);
    };

    let hint = match flag(args, "--hint") {
        None => None,
        Some(spec) => {
            let parts: Vec<&str> = spec.split(',').collect();
            if parts.len() != 2 {
                eprintln!("psolve solve: --hint must be RA,DEC in degrees");
                return ExitCode::from(2);
            }
            match (parts[0].trim().parse::<f64>(), parts[1].trim().parse::<f64>()) {
                (Ok(a), Ok(b)) if a.is_finite() && b.is_finite()
                    && (0.0..=360.0).contains(&a) && (-90.0..=90.0).contains(&b) => Some((a, b)),
                _ => {
                    eprintln!("psolve solve: --hint must be finite RA,DEC within 0..360,-90..90");
                    return ExitCode::from(2);
                }
            }
        }
    };

    // --scale is the FILE-pixel plate scale (matching what FOCALLEN/XPIXSZ
    // would report). The binning multiplication that a CFA frame needs
    // happens below, once the header is available -- validated here so a
    // garbage value is still rejected before the file is even opened.
    let scale_arcsec_raw = match flag(args, "--scale") {
        None => None,
        Some(v) => match v.parse::<f64>() {
            Ok(s) if s.is_finite() && s > 0.0 => Some(s),
            _ => {
                eprintln!("psolve solve: --scale must be a positive number of arcsec/px");
                return ExitCode::from(2);
            }
        },
    };

    let saturation = match flag(args, "--saturation") {
        None => None,
        Some(v) => match v.parse::<f32>() {
            Ok(s) if s.is_finite() && s > 0.0 => Some(s),
            _ => {
                eprintln!("psolve solve: --saturation must be a positive pixel value");
                return ExitCode::from(2);
            }
        },
    };

    let extract_params = match extract_params_from(args) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("psolve solve: {e}");
            return ExitCode::from(2);
        }
    };

    // Catalogue depth matched to the image, not maximised: too many catalogue
    // stars make matching HARDER, because true quads drown in false ones. As
    // with --radius, an unparseable or non-positive value must be a usage
    // error, not a silent fallback to the default. Validated here, with the
    // other flags and before the file is even opened, so a typo still exits
    // `2` rather than being outranked by whatever the frame itself turns out
    // to be.
    let cat_limit_override = match flag(args, "--cat-limit") {
        None => None,
        Some(v) => match v.parse::<usize>() {
            Ok(n) if n > 0 => Some(n),
            _ => {
                eprintln!("psolve solve: --cat-limit must be a positive integer");
                return ExitCode::from(2);
            }
        },
    };

    // Magnitude ceiling on the catalogue. A malformed value is a usage
    // error rather than a silently ignored flag, matching every other
    // numeric flag here.
    let max_mag = match flag(args, "--max-mag") {
        None => None,
        Some(v) => match v.parse::<f32>() {
            Ok(m) if m.is_finite() => Some(m),
            _ => {
                eprintln!("psolve solve: --max-mag must be a finite number");
                return ExitCode::from(2);
            }
        },
    };

    // `.psqidx` blind-solve quad index. Optional: absent, this command
    // behaves exactly as it always has (a hintless frame still returns
    // NO_HINT). Given explicitly, a failure to open it (missing file,
    // corrupt, paired against the wrong `.psidx`) is a usage/config
    // problem, the same class `--index` itself already reports -- unlike
    // ASTAP-compatible mode's auto-discovery (`main.rs`'s
    // `resolve_quad_index_path`), a caller who names one explicitly gets an
    // explicit error, not a silent fall-back to NO_HINT.
    let quad_index_path = flag(args, "--quad-index");

    let bytes = match std::fs::read(Path::new(path)) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("psolve solve: cannot read {path}: {e}");
            return ExitCode::from(2);
        }
    };

    // Parse the header once here: the CLI needs the dimensions for the reported
    // field of view and, when --hint is absent, the mount's commanded pointing
    // to know where to fetch catalogue stars from. Core parses it again for its
    // own use, which costs microseconds and keeps the two independent.
    let hdr = psolve_core::fits::FitsHeader::parse(&bytes).ok();
    let nx_hint = hdr.as_ref().and_then(|h| h.int("NAXIS1")).unwrap_or(0).max(0);
    let ny_hint = hdr.as_ref().and_then(|h| h.int("NAXIS2")).unwrap_or(0).max(0);

    // SolveOptions.scale_arcsec is compared against the BINNED grid inside
    // solve() -- the header-derived path already multiplies by img.binned
    // (see solve.rs). An explicit --scale must match that grid too, or a
    // correct arcsec/px value for a CFA frame is silently 2x off and
    // MatchParams.scale_tol rejects every true quad.
    let scale_arcsec = scale_arcsec_raw.map(|s| {
        let binning = hdr.as_ref().map(psolve_core::fits::binning_factor).unwrap_or(1);
        s * binning as f64
    });

    let index = match Index::open(Path::new(index_path)) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("psolve solve: {e}");
            return ExitCode::from(3);
        }
    };

    let t0 = Instant::now();

    // The search region: the frame's half-diagonal plus margin for hint error.
    // Without optics keywords we cannot size it, so fall back to something
    // generous rather than guessing narrow and failing mysteriously. An
    // unparseable or non-positive value is a broken invocation, not "narrow
    // search region" -- it must not silently fall back to the default and
    // report a clean NOT_SOLVED for what is actually a typo.
    let radius_deg = match flag(args, "--radius") {
        None => default_radius_deg(hdr.as_ref()),
        Some(v) => match v.parse::<f64>() {
            Ok(r) if r.is_finite() && r > 0.0 => r,
            _ => {
                eprintln!("psolve solve: --radius must be a positive number of degrees");
                return ExitCode::from(2);
            }
        },
    };

    // Whether the radius is a CALLER ASSERTION rather than a derived default:
    // the binning retry never overrides an explicit `--radius`, exactly as it
    // never overrides an explicit `--scale`.
    let explicit_radius = flag(args, "--radius").is_some();

    // Resolved the same way as always: --hint, else OBJCTRA/OBJCTDEC or
    // RA/DEC from the header. `None` here used to be an immediate NO_HINT
    // return; it now falls through to the blind path below when
    // `--quad-index` was given, and to the SAME NO_HINT return otherwise --
    // see that branch for why the JSON there is untouched.
    let resolved_hint = hint.or_else(|| hdr.as_ref().and_then(psolve_core::fits::hint_radec));

    // Hint-independent options, shared by both the hinted and blind paths
    // below (each sets its own `hint`/`accept` afterward -- the blind path
    // via `solve_blind`, which ignores both fields on this base value).
    let opts_base = SolveOptions {
        hint: None,
        scale_arcsec,
        saturation,
        catalog_epoch: index.header().epoch,
        extract: extract_params,
        ..SolveOptions::default()
    };

    // Which plate scale actually produced the solve, for the JSON's
    // `field.scale_source`. Set alongside `outcome` below by
    // `solve_with_binning_retry`/`solve_blind`, which every path here calls
    // -- see `solve_with_binning_retry`'s own doc for why this is shared
    // rather than living inline here.
    let mut scale_source = "header";

    // Catalogue-side selection diagnostics, populated only when a query
    // actually ran -- a failure that never gets that far (bad header, no
    // stars at all) never queried a catalogue, so there is nothing to
    // report. The blind path leaves these `None`/`false`: its own catalogue
    // selection happens once per candidate cluster inside `solve_blind`,
    // not once against a single caller-supplied disc, so a single
    // concentration/stratified pair would not mean the same thing here.
    let mut cat_concentration: Option<f64> = None;
    let mut cat_stratified = false;

    // `resolved_hint` decides the branch FIRST, before `prepare()` ever
    // runs -- matching the pre-blind code exactly for the no-hint,
    // no-quad-index case: that NO_HINT JSON must fire regardless of
    // whether this frame would ALSO fail extraction, the same guarantee it
    // always made. Calling `prepare()` unconditionally up front and
    // checking the hint afterward would silently reorder that -- a
    // hintless, starless frame would report NO_STARS instead of NO_HINT --
    // which is exactly the kind of one-entry-point-only-looking-correct
    // defect this task exists to avoid reintroducing (see this module's
    // "Blind solving" doc).
    let outcome = match resolved_hint {
        Some((hra, hdec)) => {
            // UNCHANGED hinted path (`prepare()` call and all).
            match psolve_core::solve::prepare(&bytes, &opts_base) {
                Err(failed) => failed,
                Ok(prepared) => {
                    let limit =
                        cat_limit_override.unwrap_or_else(|| cat_limit_for(prepared.usable_star_count()));
                    let selection =
                        select_catalog(&index, hra, hdec, radius_deg, limit, max_mag);
                    cat_concentration = selection.concentration;
                    cat_stratified = selection.stratified;
                    let catalog: Vec<CatalogStar> = selection
                        .recs
                        .iter()
                        .map(|r| CatalogStar {
                            ra: r.ra_deg(),
                            dec: r.dec_deg(),
                            mag: r.mag(),
                            pmra: r.pmra_mas_yr(),
                            pmdec: r.pmdec_mas_yr(),
                        })
                        .collect();

                    eprintln!(
                        "solving {path}: {} catalogue stars within {radius_deg} deg of {hra:.4},{hdec:.4}",
                        catalog.len()
                    );

                    let mut opts = opts_base;
                    opts.hint = Some((hra, hdec));
                    let attempt = solve_with_binning_retry(
                        path,
                        &prepared,
                        &catalog,
                        &opts,
                        hdr.as_ref(),
                        scale_arcsec_raw.is_some(),
                        // `.map`, not `unwrap_or(radius_deg)`: with no
                        // header-derived radius there is nothing to divide
                        // by XBINNING, so the refetch is suppressed rather
                        // than issued against a fallback constant. ASTAP
                        // mode expresses the same rule the same way; the
                        // `None` case is unreachable in practice (a frame
                        // with no FOCALLEN/XPIXSZ has no header scale to
                        // retry either), and divergent policy for one
                        // condition is how the two surfaces drift apart.
                        header_radius_deg(hdr.as_ref()).map(|radius_header_deg| CatalogRefetch {
                            max_mag,
                            index: &index,
                            hint_ra: hra,
                            hint_dec: hdec,
                            // Native mode has no cap, so the header-derived
                            // radius IS what was fetched unless --radius was
                            // given, in which case `explicit_radius`
                            // suppresses the refetch entirely.
                            radius_header_deg,
                            radius_cap: None,
                            limit,
                            explicit_radius,
                        }),
                        Some(&bytes),
                    );
                    scale_source = attempt.scale_source;
                    // Report the catalogue that actually produced the
                    // reported outcome -- solved OR failed -- not the
                    // discarded first fetch. Placed before the `match
                    // outcome` below so BOTH printers read the corrected
                    // values; `SolveAttempt::refetched` is `Some` whenever
                    // the refetch happened, independent of the result.
                    if let Some(sel) = &attempt.refetched {
                        cat_concentration = sel.concentration;
                        cat_stratified = sel.stratified;
                    }
                    attempt.outcome
                }
            }
        }
        None => match quad_index_path {
            None => {
                // UNCHANGED: no --hint, no header hint, no --quad-index to
                // fall back to -- exactly today's NO_HINT JSON, returned
                // before `prepare()` is ever called, exactly as before.
                // `index` is already open at this point (opened above,
                // before `--radius`/`--hint` are even resolved), so this
                // failure has a resolved index and must carry it -- see
                // `BUILD_ID`'s doc and spec §7.2 for why every failure path
                // that has one must emit it.
                println!(
                    "{{\"psolve\":\"0.1.0\",\"build\":\"{}\",\"solved\":false,\"reason\":\"NO_HINT\",\
\"detail\":\"no pointing hint: pass --hint or supply OBJCTRA/OBJCTDEC or RA/DEC\",\
\"index\":{{\"name\":\"{}\"}}}}",
                    json_escape(BUILD_ID),
                    json_escape(index.header().name_str()),
                );
                return ExitCode::from(1);
            }
            Some(qpath) => {
                let quad_index = match QuadIndex::open(Path::new(qpath), &index) {
                    Ok(q) => q,
                    Err(e) => {
                        eprintln!("psolve solve: {e}");
                        return ExitCode::from(3);
                    }
                };
                match psolve_core::solve::prepare(&bytes, &opts_base) {
                    Err(failed) => failed,
                    Ok(prepared) => {
                        let limit = cat_limit_override
                            .unwrap_or_else(|| cat_limit_for(prepared.usable_star_count()));
                        let search = solve_blind(
                            path,
                            &prepared,
                            hdr.as_ref(),
                            &index,
                            &quad_index,
                            radius_deg,
                            limit,
                            max_mag,
                            &opts_base,
                            scale_arcsec_raw.is_some(),
                        );
                        eprintln!(
                            "solving {path}: blind search -- {} image quads, {} hypotheses \
offered, {} candidate transform(s) survived, {} cluster(s) attempted",
                            search.image_quads, search.hypotheses, search.survivors, search.clusters_tried,
                        );
                        scale_source = search.scale_source;
                        search.outcome
                    }
                }
            }
        },
    };
    let ms = t0.elapsed().as_secs_f64() * 1000.0;

    match outcome {
        Outcome::Solved(s) => {
            let (fwhm, ecc, pa) = s.quality.unwrap_or((f64::NAN, f64::NAN, f64::NAN));
            // The sky position of the IMAGE CENTRE, not of CRVAL: `fit_tan`
            // pins CRVAL to the caller's pointing hint and lets CRPIX absorb
            // the offset, so `pix_to_radec(crpix)` trivially echoes the hint
            // back on every solve, regardless of where the frame actually
            // points. `s.nx`/`s.ny` are already in FILE pixel coordinates,
            // matching the WCS emitted below.
            //
            // psolve's internal pixel coordinates are 0-BASED: `extract.rs`
            // centroids blobs over array indices `x = i % nx`, `y = i / nx`
            // (`0..nx`, `0..ny`), and `fit_tan` fits `Wcs.crpix` in that same
            // frame. The 0-based centre of an nx-wide axis is `(nx-1)/2`, not
            // `nx/2` -- `nx/2` is the FITS 1-based centre (where pixel 1 is
            // the first column). Using `nx/2` here evaluated `pix_to_radec`
            // half a pixel off-centre on both axes, which at a typical
            // ~2.5"/px plate scale is roughly a 1.6" systematic bias -- this
            // was the single largest term in Task 11's measured 1.68" median
            // separation against ASTAP (see task-11-report.md's fix-round
            // addendum; corrected, re-derived over the full 9495-frame run,
            // the median drops to 0.531" against the DB centre and 0.117"
            // against real ASTAP header CRVAL on the subset that carries one).
            //
            // FITS-convention artifacts (the `.ini`/`.wcs` sidecars, `-update`)
            // are a separate concern: they take `s.wcs.crpix` directly and
            // must add +1 to cross from this 0-based internal convention to
            // FITS's 1-based one -- see `sidecar.rs`'s module doc.
            let c = s.wcs.pix_to_radec((s.nx as f64 - 1.0) / 2.0, (s.ny as f64 - 1.0) / 2.0);

            // Spec section 7.2: emit CD *and* CDELT+PC, always.
            //
            // core/astrometry._with_pc in the sibling astroops repo exists only
            // because ASTAP writes CD, Siril writes PC/CDELT, and every consumer
            // assumed PC -- so a perfectly good solved frame raised
            // KeyError('PC1_1'). Emitting both kills that bug class at the source
            // instead of downstream. CD = CDELT * PC by definition.
            let cdelt = [
                -(s.wcs.cd[0][0].hypot(s.wcs.cd[1][0])),
                s.wcs.cd[0][1].hypot(s.wcs.cd[1][1]),
            ];
            let pc = if cdelt[0] != 0.0 && cdelt[1] != 0.0 {
                [
                    [s.wcs.cd[0][0] / cdelt[0], s.wcs.cd[0][1] / cdelt[0]],
                    [s.wcs.cd[1][0] / cdelt[1], s.wcs.cd[1][1] / cdelt[1]],
                ]
            } else {
                [[f64::NAN, f64::NAN], [f64::NAN, f64::NAN]]
            };
            let scale_deg = s.wcs.scale_arcsec() / 3600.0;
            let (fov_w, fov_h) = (nx_hint as f64 * scale_deg, ny_hint as f64 * scale_deg);
            println!(
                "{{\"psolve\":\"0.1.0\",\"build\":\"{}\",\"solved\":true,\"reason\":null,\
\"confidence\":{{\"log_odds\":{},\"chance_matches\":{}}},\
\"wcs\":{{\"crval\":[{},{}],\"crpix\":[{},{}],\
\"cd\":[[{},{}],[{},{}]],\"cdelt\":[{},{}],\"pc\":[[{},{}],[{},{}]],\
\"parity\":\"{}\"}},\
\"field\":{{\"center\":{{\"ra\":{},\"dec\":{}}},\"fov_deg\":[{},{}],\
\"scale_arcsec\":{},\"orientation_pa\":{},\"scale_source\":\"{}\"}},\
\"stars\":{{\"detected\":{},\"used\":{},\"matched\":{},\"matcher\":\"{}\",\"quad_budget\":{},\"pair\":{},\"concentration\":{},\"stratified\":{},\
\"rejected\":{{\"too_small\":{},\"extended\":{},\"saturated\":{},\"elongated\":{},\"edge\":{}}}}},\
\"catalog\":{{\"concentration\":{},\"stratified\":{}}},\
\"fit\":{{\"rms_arcsec\":{},\"max_residual_arcsec\":{},\"radial_trend\":{}}},\
\"quality\":{{\"fwhm_px\":{},\"ellipticity\":{},\"ellipticity_pa\":{}}},\
\"epoch\":{{\"pm_years_applied\":{}}},\
\"index\":{{\"name\":\"{}\"}},\
\"timings_ms\":{{\"decode\":{},\"background\":{},\"extract\":{},\"caller\":{},\"quads\":{},\
\"catalogue\":{},\"match\":{},\"fit\":{},\"verify\":{},\"total\":{}}}}}",
                json_escape(BUILD_ID),
                json_number(s.confidence.log_odds),
                json_number(s.confidence.chance_matches),
                json_number(s.wcs.crval[0]), json_number(s.wcs.crval[1]),
                json_number(s.wcs.crpix[0]), json_number(s.wcs.crpix[1]),
                json_number(s.wcs.cd[0][0]), json_number(s.wcs.cd[0][1]),
                json_number(s.wcs.cd[1][0]), json_number(s.wcs.cd[1][1]),
                json_number(cdelt[0]), json_number(cdelt[1]),
                json_number(pc[0][0]), json_number(pc[0][1]),
                json_number(pc[1][0]), json_number(pc[1][1]),
                if s.mirrored { "mirrored" } else { "normal" },
                json_number(c.0), json_number(c.1),
                json_number(fov_w), json_number(fov_h),
                json_number(s.wcs.scale_arcsec()), json_number(s.wcs.orientation_deg()), scale_source,
                s.stars_detected, s.stars_used, s.stars_matched, s.matcher, s.quad_budget,
                pair_json(s.pair.as_ref()),
                json_option_number(s.concentration), s.stratified,
                s.rejected.too_small, s.rejected.extended, s.rejected.saturated,
                s.rejected.elongated, s.rejected.edge,
                json_option_number(cat_concentration), cat_stratified,
                json_number(s.fit.rms_deg * 3600.0),
                json_number(s.fit.max_residual_deg * 3600.0),
                json_number(s.fit.radial_trend),
                json_number(fwhm), json_number(ecc), json_number(pa),
                json_number(s.pm_years_applied),
                json_escape(index.header().name_str()),
                json_number(s.timings_ms.decode),
                json_number(s.timings_ms.background),
                json_number(s.timings_ms.extract),
                json_number(s.timings_ms.caller),
                json_number(s.timings_ms.quads),
                json_number(s.timings_ms.catalogue),
                json_number(s.timings_ms.match_),
                json_number(s.timings_ms.fit),
                json_number(s.timings_ms.verify),
                // The CLI's own wall clock, not `s.timings_ms.total`: it
                // spans everything from opening the index onward, where
                // `total` starts at `prepare()`. The difference is
                // `Index::open` plus argument handling.
                //
                // The gap between this and the sum of the stages above used
                // to be described here as the catalogue disc query, ~1 ms.
                // That was right only for a frame that solves on its first
                // attempt. `PreparedFrame::t_start` is never reset, so on a
                // RETRIED frame the same gap also contains every earlier
                // attempt: measured across 25 corpus frames, 1.5 ms when
                // `scale_source` is `header` and **124.8 ms** when it is
                // `header/binning-retry`. `Timings::caller` now reports that
                // interval directly, so the stages account for themselves
                // instead of leaving a reader to guess.
                //
                // It was ~66 ms until the final-review fix, and that 66 ms
                // was NOT the disc query, which is what this comment (and
                // `docs/astap-compat.md`) used to claim. It was
                // `default_cat_limit` running a second, complete
                // decode+background+extract ahead of `solve()` purely to
                // count stars. The isolating measurement, release, same
                // frame, pre-fix binary: with the auto limit, 147.7 ms wall
                // and a 66.1 ms gap; with `--cat-limit 537` passed
                // explicitly -- the exact value the auto path computes, so
                // the catalogue, the solve and the disc query are all
                // identical and only the probe is skipped -- 83.8 ms wall
                // and a 0.96 ms gap. The gap was the probe. Sizing the
                // limit from `PreparedFrame` removed it.
                json_number(ms),
            );
            ExitCode::SUCCESS
        }
        Outcome::Failed { reason, detail, stars_detected, stars_used, concentration, stratified, rejected } => {
            // Every failure that reaches this branch has come through
            // `prepare()`, which runs strictly after `Index::open` above --
            // so `index` is always resolved here too. Emitting it is spec
            // §7.2's fix for the other half of the same incident `BUILD_ID`
            // exists for: a consumer keyed a provenance record on `index`
            // and misclassified samples that landed on this branch, because
            // `index` used to appear only on `Outcome::Solved`.
            //
            // `cat_concentration`/`cat_stratified` describe the disc this
            // failure actually came from: the binning retry's refetched one
            // when it refetched, otherwise the first `select_catalog` above
            // -- `None`/`false` if `prepare()` itself failed before a
            // catalogue query was ever made. A failure carrying the
            // discarded first disc's concentration would mislead exactly
            // the reader who most needs it.
            println!(
                "{{\"psolve\":\"0.1.0\",\"build\":\"{}\",\"solved\":false,\"reason\":\"{}\",\"detail\":\"{}\",\
\"stars\":{{\"detected\":{},\"used\":{},\"concentration\":{},\"stratified\":{},\
\"rejected\":{{\"too_small\":{},\"extended\":{},\"saturated\":{},\"elongated\":{},\"edge\":{}}}}},\
\"catalog\":{{\"concentration\":{},\"stratified\":{}}},\
\"index\":{{\"name\":\"{}\"}},\
\"timings_ms\":{{\"total\":{}}}}}",
                json_escape(BUILD_ID),
                reason.as_str(),
                json_escape(&detail),
                stars_detected, stars_used, json_option_number(concentration), stratified,
                rejected.too_small, rejected.extended, rejected.saturated,
                rejected.elongated, rejected.edge,
                json_option_number(cat_concentration), cat_stratified,
                json_escape(index.header().name_str()),
                json_number(ms),
            );
            // Not solved is a NORMAL outcome, not an error.
            ExitCode::from(1)
        }
    }
}

// `psolve-cli` is a bin-only crate (no `[lib]` target), so an external
// `tests/` integration test cannot reach a private/`pub(crate)` function --
// there is nothing for it to link against. Co-locating the unit test here
// matches the convention used throughout `psolve-core` and `psolve-index`
// (see e.g. `psolve-core/src/fits.rs`'s own `mod tests`).
/// The pair-matching retry's diagnostics as a JSON object, or `null` when
/// quads answered.
///
/// `runner_up` is the point of it. It is the best score any hypothesis this
/// module REJECTED reached, so the gap to `inliers` is the margin the answer
/// won by. A solve accepted on a narrow margin is one to look at, and
/// without this the JSON could not distinguish it from a decisive one.
fn pair_json(p: Option<&psolve_core::pairmatch::PairMatchResult>) -> String {
    match p {
        None => "null".to_string(),
        Some(r) => format!(
            "{{\"inliers\":{},\"runner_up\":{},\"image_stars\":{},\"cat_stars\":{},\
\"agreements\":{},\"hypotheses\":{},\"mirrored\":{},\"truncated\":{},\
\"hypotheses_to_promise\":{},\"aborted\":{}}}",
            r.inliers, r.runner_up, r.image_stars, r.cat_stars,
            r.agreements, r.hypotheses, r.mirrored, r.truncated,
            r.hypotheses_to_promise.map(|v| v.to_string()).unwrap_or_else(|| "null".into()),
            r.aborted
        ),
    }
}

#[cfg(test)]
mod tests {

    /// The blind gate's refusal must survive a later cluster that fails
    /// earlier in the pipeline. This is the diagnostic that tells a
    /// null-sky measurement whether the gate ran at all -- Task 8 could not
    /// answer that from the JSON because both call sites overwrote
    /// unconditionally.
    mod keep_most_informative {
        use super::super::keep_most_informative;
        use psolve_core::error::ReasonCode;
        use psolve_core::solve::Outcome;
        use psolve_core::extract::Rejections;

        fn failed(reason: ReasonCode, detail: &str) -> Outcome {
            Outcome::Failed {
                reason,
                detail: detail.to_string(),
                stars_detected: 0,
                stars_used: 0,
                concentration: None,
                stratified: false,
                rejected: Rejections::default(),
            }
        }

        fn reason_of(o: &Option<(Outcome, &'static str)>) -> ReasonCode {
            match o {
                Some((Outcome::Failed { reason, .. }, _)) => *reason,
                _ => panic!("expected a held failure"),
            }
        }

        fn detail_of(o: &Option<(Outcome, &'static str)>) -> String {
            match o {
                Some((Outcome::Failed { detail, .. }, _)) => detail.clone(),
                _ => panic!("expected a held failure"),
            }
        }

        #[test]
        fn gate_refusal_is_not_masked_by_a_later_bare_failure() {
            let mut held = None;
            keep_most_informative(&mut held, (failed(ReasonCode::LowConfidence, "gate"), "hdr"));
            keep_most_informative(&mut held, (failed(ReasonCode::NoQuadMatch, "later"), "hdr"));
            assert_eq!(
                reason_of(&held),
                ReasonCode::LowConfidence,
                "a later NoQuadMatch overwrote the gate's own refusal -- the JSON would then \
claim nothing reached the gate when something did"
            );
            assert_eq!(detail_of(&held), "gate");
        }

        #[test]
        fn a_gate_refusal_replaces_an_earlier_bare_failure() {
            let mut held = None;
            keep_most_informative(&mut held, (failed(ReasonCode::NoQuadMatch, "early"), "hdr"));
            keep_most_informative(&mut held, (failed(ReasonCode::LowConfidence, "gate"), "hdr"));
            assert_eq!(reason_of(&held), ReasonCode::LowConfidence);
            assert_eq!(detail_of(&held), "gate");
        }

        #[test]
        fn among_bare_failures_the_later_one_wins_as_before() {
            let mut held = None;
            keep_most_informative(&mut held, (failed(ReasonCode::NoQuadMatch, "first"), "hdr"));
            keep_most_informative(&mut held, (failed(ReasonCode::NoQuadMatch, "second"), "hdr"));
            assert_eq!(detail_of(&held), "second");
        }

        #[test]
        fn among_gate_refusals_the_later_one_wins_as_before() {
            let mut held = None;
            keep_most_informative(&mut held, (failed(ReasonCode::LowConfidence, "first"), "hdr"));
            keep_most_informative(&mut held, (failed(ReasonCode::LowConfidence, "second"), "hdr"));
            assert_eq!(detail_of(&held), "second");
        }

        #[test]
        fn the_first_failure_is_always_taken() {
            let mut held = None;
            keep_most_informative(&mut held, (failed(ReasonCode::TooFewStars, "only"), "hdr"));
            assert_eq!(reason_of(&held), ReasonCode::TooFewStars);
        }
    }
    use super::{
        cap_by_mag, default_radius_for, extract_params_from, resolve_quad_sky, select_bands,
        select_catalog, solve_blind, BLIND_CODE_TOL, RADIUS_MARGIN,
        RADIUS_RETRY_HALF_DIAG_FRAC, VALUED_FLAGS,
    };
    use super::{Index, QuadIndex, SolveOptions};

    const TEST_NSIDE: u32 = 64;

    /// A real on-disk index built from `(ra_deg, dec_deg, mag)` triples, in a
    /// process- and fixture-unique scratch directory (same pattern
    /// `tests/cross_path_catalogue_selection.rs` uses for its own on-disk
    /// index -- `select_catalog`/`catalog_concentration` are `pub(crate)`,
    /// so an external integration test cannot reach them at all; this crate
    /// is bin-only, no `[lib]` target).
    fn build_test_index(tag: &str, stars: &[(f64, f64, f32)]) -> Index {
        let dir = std::env::temp_dir()
            .join(format!("psolve-cmd-solve-test-index-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)
            .unwrap_or_else(|e| panic!("creating scratch dir {}: {e}", dir.display()));
        let path = dir.join("t.psidx");
        let mut b = psolve_index::builder::Builder::new(TEST_NSIDE, 20.0, 2016.0, "cmd-solve-test")
            .expect("nside 64 is valid");
        for &(ra, dec, mag) in stars {
            b.push(ra, dec, mag, 0.0, 0.0);
        }
        let mut f = std::fs::File::create(&path)
            .unwrap_or_else(|e| panic!("creating {}: {e}", path.display()));
        b.finish(&mut f).expect("writing a well-formed index must not fail");
        Index::open(&path).unwrap_or_else(|e| panic!("opening {}: {e}", path.display()))
    }

    /// Great-circle separation in degrees -- a self-contained haversine, so
    /// these tests can filter candidate HEALPix cells to ones whose centre
    /// genuinely lies within a query radius rather than merely touching the
    /// padded candidate set `cells_in_disc` returns.
    fn angsep_deg(ra1: f64, dec1: f64, ra2: f64, dec2: f64) -> f64 {
        let (ra1, dec1, ra2, dec2) =
            (ra1.to_radians(), dec1.to_radians(), ra2.to_radians(), dec2.to_radians());
        let (dra, ddec) = (ra2 - ra1, dec2 - dec1);
        let a = (ddec / 2.0).sin().powi(2) + dec1.cos() * dec2.cos() * (dra / 2.0).sin().powi(2);
        2.0 * a.min(1.0).sqrt().asin().to_degrees()
    }

    /// `cap_by_mag` is the whole `--max-mag` feature; these pin its edges.
    ///
    /// The cap is INCLUSIVE, and that matters: `--max-mag 14` against a g14
    /// index must not silently drop the mag-14.000 stars the index was built
    /// to contain.
    #[test]
    fn cap_by_mag_is_inclusive_and_drops_only_fainter_stars() {
        let stars: Vec<(f64, f64, f32)> = (0..7)
            .map(|k| (180.0, 0.0, 12.0 + k as f32))
            .collect();
        let index = build_test_index("magcap", &stars);
        let all = index.brightest_in_disc(180.0, 0.0, 2.0, 100);
        assert_eq!(all.len(), 7, "fixture must load");

        let capped = cap_by_mag(all.clone(), Some(14.0));
        assert_eq!(capped.len(), 3, "12, 13 and 14 are all <= 14");
        assert!(
            capped.iter().all(|r| r.mag() <= 14.0 + 1e-6),
            "a star fainter than the cap survived"
        );
        assert!(
            capped.iter().any(|r| (r.mag() - 14.0).abs() < 1e-3),
            "the cap must be inclusive -- the mag-14 star was dropped"
        );

        assert_eq!(
            cap_by_mag(all.clone(), None).len(),
            all.len(),
            "no cap must be a no-op"
        );
        assert!(
            cap_by_mag(all.clone(), Some(0.0)).is_empty(),
            "a cap brighter than everything must drop everything"
        );
    }

    /// The cap has to bind on BOTH selection branches. Stratified selection
    /// spreads its picks over the disc instead of taking them in magnitude
    /// order, so it reaches fainter than `brightest_in_disc` for the same
    /// limit -- exactly the case the cap exists to bound. A cap applied to
    /// the legacy branch only would be silently absent on dense fields.
    #[test]
    fn the_cap_binds_on_the_stratified_branch_too() {
        let (ra0, dec0, radius, limit) = (180.0, 0.0, 1.5, 60);
        // A clump dense enough to trip the stratification gate, plus faint
        // outliers spread across the disc for stratified selection to reach.
        let mut stars = Vec::new();
        for k in 0..400 {
            stars.push((180.0 + (k % 20) as f64 * 0.002, 0.02 * (k / 20) as f64, 9.0));
        }
        for cell in psolve_index::healpix::cells_in_disc(TEST_NSIDE, ra0, dec0, radius) {
            let (cra, cdec) = psolve_index::healpix::pix2ang_nest(TEST_NSIDE, cell);
            if angsep_deg(ra0, dec0, cra, cdec) <= radius {
                stars.push((cra, cdec, 17.0));
            }
        }
        let index = build_test_index("magcap-strat", &stars);

        let uncapped = select_catalog(&index, ra0, dec0, radius, limit, None);
        let capped = select_catalog(&index, ra0, dec0, radius, limit, Some(12.0));
        assert!(
            uncapped.recs.iter().any(|r| r.mag() > 12.0),
            "fixture must return something the cap can remove"
        );
        assert!(
            capped.recs.iter().all(|r| r.mag() <= 12.0 + 1e-6),
            "a star fainter than the cap survived selection ({} of {} recs)",
            capped.recs.iter().filter(|r| r.mag() > 12.0).count(),
            capped.recs.len()
        );
        assert!(!capped.recs.is_empty(), "the cap must not empty the catalogue here");
    }

    /// The tight rung recovers the half-diagonal by dividing the
    /// header-derived radius by `RADIUS_MARGIN`, rather than re-deriving it
    /// from the optics keywords a second time. If those two ever disagree the
    /// rung silently retries at the wrong radius, so this pins the
    /// round-trip.
    #[test]
    fn the_tight_radius_is_the_documented_fraction_of_the_half_diagonal() {
        for (w, h) in [(1.5_f64, 0.84_f64), (2.6, 1.48), (0.42, 0.42), (3.0, 2.0)] {
            let half_diag = (w * w + h * h).sqrt() / 2.0;
            let header = default_radius_for(w, h);
            let recovered = header / RADIUS_MARGIN;
            assert!(
                (recovered - half_diag).abs() < 1e-12,
                "half-diagonal round-trip broken for {w}x{h}: {recovered} vs {half_diag}"
            );
            let tight = recovered * RADIUS_RETRY_HALF_DIAG_FRAC;
            assert!(
                tight < header,
                "the tight rung must NARROW: {tight} is not below the default {header}"
            );
            assert!(
                (tight - half_diag * RADIUS_RETRY_HALF_DIAG_FRAC).abs() < 1e-12,
                "the tight radius is not the documented fraction of the half-diagonal"
            );
        }
    }

    /// The rung's whole reason for existing: a smaller disc holds fewer
    /// catalogue stars, and the ones it drops are the ones outside the frame
    /// that no image star could ever match.
    #[test]
    fn a_tighter_radius_returns_a_smaller_catalogue() {
        let (ra0, dec0, limit) = (180.0_f64, 0.0_f64, 5000_usize);
        let mut stars = Vec::new();
        for cell in psolve_index::healpix::cells_in_disc(TEST_NSIDE, ra0, dec0, 5.0) {
            let (cra, cdec) = psolve_index::healpix::pix2ang_nest(TEST_NSIDE, cell);
            for k in 0..6 {
                stars.push((cra, cdec, 10.0 + k as f32 * 0.01));
            }
        }
        let index = build_test_index("tight-radius", &stars);
        let wide = select_catalog(&index, ra0, dec0, 3.0, limit, None).recs.len();
        let tight = select_catalog(&index, ra0, dec0, 3.0 * RADIUS_RETRY_HALF_DIAG_FRAC, limit, None)
            .recs
            .len();
        assert!(
            tight < wide,
            "a tighter disc returned no fewer stars ({tight} vs {wide}) -- the rung \
             cannot help if the catalogue does not shrink"
        );
        assert!(tight > 0, "the tight disc must not be empty");
    }

    /// The catalogue-side analogue of `extract.rs`'s own bit-identity test:
    /// below the gate, `select_catalog` must return `brightest_in_disc`'s
    /// own result -- not a close approximation of it.
    ///
    /// The fixture places several stars at the centre of EVERY HEALPix cell
    /// genuinely inside the query radius, uniformly -- not a naive
    /// linear-in-RA/linear-in-Dec scatter, which over-samples near the
    /// disc's Dec extremes and is not actually uniform on the sphere; and
    /// not a huge, sparse radius either, which was caught while writing this
    /// test: at a radius wide enough that the disc's effective cell count
    /// vastly exceeds the candidate star count, ANY set of stars reads as
    /// falsely concentrated from sheer sparsity (`catalog_concentration`'s
    /// own `meaningful` flag exists for exactly this), which is a different
    /// thing from being genuinely spread and low-concentration.
    #[test]
    fn select_catalog_below_the_gate_returns_exactly_brightest_in_discs_result() {
        let (ra0, dec0, radius, limit) = (180.0, 0.0, 5.0, 5000);
        let mut stars = Vec::new();
        for cell in psolve_index::healpix::cells_in_disc(TEST_NSIDE, ra0, dec0, radius) {
            let (cra, cdec) = psolve_index::healpix::pix2ang_nest(TEST_NSIDE, cell);
            if angsep_deg(ra0, dec0, cra, cdec) > radius {
                continue; // a candidate cell that only touches the padded disc
            }
            for k in 0..6 {
                stars.push((cra, cdec, 10.0 + k as f32 * 0.01));
            }
        }
        let index = build_test_index("ungated", &stars);

        let direct = index.brightest_in_disc(ra0, dec0, radius, limit);
        let selection = select_catalog(&index, ra0, dec0, radius, limit, None);

        assert!(
            !selection.stratified,
            "fixture must be below the gate for this test to mean anything"
        );
        assert_eq!(
            selection.recs, direct,
            "below the gate, select_catalog must reproduce brightest_in_disc's own result exactly"
        );
    }

    /// The near-threshold case on the catalogue side -- this is the
    /// statistic that actually decides real corpus frames (see
    /// `psolve_core::extract::CONCENTRATION_THRESHOLD`'s doc), so its own
    /// boundary needs the same direct check the image side gets. Finds the
    /// boundary by search, exactly as `extract.rs`'s
    /// `the_gate_is_correct_right_at_its_own_boundary` does, rather than
    /// hardcoding a concentration value that would go stale the moment the
    /// threshold is recalibrated. Searches on `select_catalog`'s own
    /// `stratified` flag directly, not a hand-rolled reimplementation of its
    /// decision, so the search cannot silently drift from what the function
    /// under test actually does (this caught a real bug while writing this
    /// test: an earlier version compared `catalog_concentration` against
    /// `should_stratify` alone, missing the `meaningful` gate `select_catalog`
    /// also applies).
    #[test]
    fn select_catalog_gate_is_correct_right_at_its_own_boundary() {
        // radius=5deg keeps the effective cell count small (~93) so a
        // modest background comfortably clears `catalog_concentration`'s
        // `meaningful` floor at every pile size tried below.
        let (ra0, dec0, radius, limit) = (180.0, 0.0, 5.0, 5000);
        let pile_cell = psolve_index::healpix::cells_in_disc(TEST_NSIDE, ra0, dec0, radius)
            .into_iter()
            .find(|&c| {
                let (cra, cdec) = psolve_index::healpix::pix2ang_nest(TEST_NSIDE, c);
                angsep_deg(ra0, dec0, cra, cdec) <= radius
            })
            .expect("radius 5deg at nside 64 must contain at least one full cell");
        let (pile_ra, pile_dec) = psolve_index::healpix::pix2ang_nest(TEST_NSIDE, pile_cell);

        let build = |pile: usize| -> Index {
            let mut stars = Vec::new();
            // A spread-out background, one star per candidate cell centre
            // inside the true radius -- uniform by construction, same
            // reasoning as the bit-identity test above.
            for cell in psolve_index::healpix::cells_in_disc(TEST_NSIDE, ra0, dec0, radius) {
                let (cra, cdec) = psolve_index::healpix::pix2ang_nest(TEST_NSIDE, cell);
                if angsep_deg(ra0, dec0, cra, cdec) > radius {
                    continue;
                }
                stars.push((cra, cdec, 15.0));
            }
            // The pile: brighter than the background, all in ONE known
            // cell, so `brightest_in_disc` favours it and its count in the
            // returned candidate set grows directly with `pile`.
            for i in 0..pile {
                stars.push((pile_ra, pile_dec, 5.0 + i as f32 * 0.0001));
            }
            build_test_index(&format!("boundary-{pile}"), &stars)
        };
        let stratified_at = |pile: usize| -> bool {
            select_catalog(&build(pile), ra0, dec0, radius, limit, None).stratified
        };

        let mut pile = 0usize;
        while !stratified_at(pile) {
            pile += 1;
            assert!(pile < 5000, "search did not converge -- fixture is broken");
        }
        let below = pile.saturating_sub(1);
        let above = pile;
        assert!(!stratified_at(below), "search invariant violated");
        assert!(stratified_at(above), "search invariant violated");

        let index_below = build(below);
        let legacy_below = index_below.brightest_in_disc(ra0, dec0, radius, limit);
        let selection_below = select_catalog(&index_below, ra0, dec0, radius, limit, None);
        assert!(!selection_below.stratified, "just below the gate must not stratify");
        assert_eq!(
            selection_below.recs, legacy_below,
            "just below the gate must be bit-identical to legacy"
        );

        let index_above = build(above);
        let selection_above = select_catalog(&index_above, ra0, dec0, radius, limit, None);
        assert!(selection_above.stratified, "just above the gate must stratify");
    }

    /// The shipped default must cover the frame's corners and little else. A
    /// default that oversizes the disc does not fail loudly -- it returns
    /// NO_QUAD_MATCH, which reads as "unsolvable frame" rather than "wrong
    /// default", and that is exactly how this shipped.
    #[test]
    fn default_radius_is_half_the_field_diagonal_with_margin() {
        // 3840x2160 at 2.4533 "/px -- the real eagle-rig frame.
        let w_deg: f64 = 3840.0 * 2.4533 / 3600.0; // 2.617
        let h_deg: f64 = 2160.0 * 2.4533 / 3600.0; // 1.472
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

    /// Every flag that takes a value must be in VALUED_FLAGS, or the
    /// positional scan binds the flag's value as the input FILE. That defect
    /// shipped once already in M2 (T13, --index) and produced a clean exit-1
    /// "not solved" for what was really a malformed invocation.
    #[test]
    fn every_valued_flag_is_registered_for_the_positional_scan() {
        for f in [
            "--index", "--hint", "--scale", "--radius", "--cat-limit",
            "--saturation", "--sigma", "--min-pix", "--keep", "--max-ellipticity",
            "--quad-index", "--max-mag",
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
        let args =
            ["--sigma", "3.5", "--min-pix", "3", "--keep", "900", "--max-ellipticity", "0.8"];
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
        // Silently falling back to the default on a typo is how a user ends
        // up debugging the wrong thing.
        assert!(extract_params_from(&["--min-pix", "abc"]).is_err());
        assert!(extract_params_from(&["--sigma", ""]).is_err());
    }

    /// Values that parse cleanly but are physically nonsensical must also be
    /// rejected, not accepted as a default fallback. Rust's float grammar
    /// accepts "nan" and "inf" as valid input, and without a finiteness
    /// guard `--sigma nan` propagates into `k_sigma = NaN`; every
    /// `pixel > NaN` comparison in extraction is then false, zero blobs are
    /// detected, and the CLI reports a clean "no sources above threshold"
    /// astronomical failure for what is actually a malformed invocation --
    /// exactly the disguise `positional()`'s doc comment warns about.
    #[test]
    fn a_nonsensical_extraction_flag_value_is_a_usage_error_not_a_default() {
        for args in [
            ["--sigma", "nan"].as_slice(),
            ["--sigma", "inf"].as_slice(),
            ["--sigma", "0"].as_slice(),
            ["--sigma", "-1"].as_slice(),
            ["--max-ellipticity", "nan"].as_slice(),
            ["--max-ellipticity", "5"].as_slice(),
            ["--keep", "0"].as_slice(),
            ["--min-pix", "0"].as_slice(),
        ] {
            assert!(
                extract_params_from(args).is_err(),
                "{args:?} must be a usage error, not silently accepted"
            );
        }
    }

    // -----------------------------------------------------------------
    // Blind solving (Task 7)
    // -----------------------------------------------------------------

    /// A synthetic FITS frame with `n` painted 4x4 star blobs at
    /// deterministic pixel positions -- the same construction
    /// `psolve_core::solve`'s own test fixtures use (`painted_frame`,
    /// private to that crate's test module, so reimplemented here rather
    /// than shared). This test needs a REAL `PreparedFrame` -- `solve_blind`
    /// takes one, not raw bytes -- to exercise the actual enumeration loop
    /// `hypothesis_count_matches_an_independent_recount_of_what_was_offered`
    /// cross-checks below.
    fn painted_frame_bytes(nx: usize, ny: usize, n: usize) -> Vec<u8> {
        let mut px = vec![1000u16; nx * ny];
        for i in 0..n {
            let x = 30 + (i * 37) % (nx - 68);
            let y = 30 + (i * 61) % (ny - 68);
            for dy in 0..4 {
                for dx in 0..4 {
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
        let mut s = String::new();
        for c in &cards {
            s.push_str(&format!("{c:<80}"));
        }
        s.push_str(&format!("{:<80}", "END"));
        while !s.len().is_multiple_of(2880) {
            s.push(' ');
        }
        let mut out = s.into_bytes();
        for &v in &px {
            let stored = (v as i32 - 32768) as i16;
            out.extend_from_slice(&stored.to_be_bytes());
        }
        while !out.len().is_multiple_of(2880) {
            out.push(0);
        }
        out
    }

    /// Writes a synthetic `.psqidx`, paired against `star_index`, with two
    /// bands: `planted` real quad codes copied verbatim from the caller
    /// (so at least SOME lookups are guaranteed to find something -- an
    /// index of pure noise codes would make the honesty check below
    /// vacuous, `expected == 0 == search.hypotheses` trivially), plus a
    /// spread of unrelated "noise" codes so this is not a single-record
    /// index. `star_idx` on every record references valid indices into
    /// `star_index` (`0..star_index.header().n_records`), so
    /// `resolve_quad_sky` never fails to resolve one.
    fn build_test_quad_index(
        tag: &str,
        star_index: &Index,
        n_stars: u32,
        planted: &[(usize, [f64; 4])],
    ) -> QuadIndex {
        let bands: [f32; 2] = [0.25, 8.0];
        let mut fingerprint = [0u8; 8];
        fingerprint.copy_from_slice(&star_index.header().records_sha256[..8]);
        let mut b = psolve_index::quad_builder::QuadIndexBuilder::new(
            TEST_NSIDE, 2016.0, 20.0, tag, fingerprint, &bands,
        )
        .expect("valid quad-index builder params");
        for &(band, code) in planted {
            let idx = [0u32, 1, 2, 3];
            b.push(band, code, idx).expect("planted record must push");
        }
        // Noise: deterministic, spread across [0,1) in every component,
        // referencing valid star indices so resolution never fails.
        for i in 0..40u32 {
            let band = (i % 2) as usize;
            let f = i as f64;
            let code = [(f * 0.083) % 1.0, (f * 0.157) % 1.0, (f * 0.211) % 1.0, (f * 0.269) % 1.0];
            let idx = [i % n_stars, (i + 1) % n_stars, (i + 2) % n_stars, (i + 3) % n_stars];
            b.push(band, code, idx).expect("noise record must push");
        }
        let dir = std::env::temp_dir()
            .join(format!("psolve-cmd-solve-test-quadindex-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("creating scratch dir: {e}"));
        let path = dir.join("q.psqidx");
        let mut buf = std::io::Cursor::new(Vec::new());
        b.finish(&mut buf).expect("writing a well-formed quad index must not fail");
        std::fs::write(&path, buf.into_inner()).unwrap_or_else(|e| panic!("writing {}: {e}", path.display()));
        QuadIndex::open(&path, star_index).unwrap_or_else(|e| panic!("opening {}: {e}", path.display()))
    }

    /// The Task 7 wiring requirement stated as a test, not just implemented:
    /// `search.hypotheses` (`M`, what `verify::AcceptParams::blind` is
    /// actually handed) must equal the TOTAL of what every `(image quad,
    /// band)` lookup in this search offered -- independently recomputed
    /// here by calling `QuadIndex::candidates` directly over the exact same
    /// image quads, not merely asserted to be "some positive number".
    ///
    /// Deliberately does not require the search to SOLVE: this fixture's
    /// planted quad codes are real matches in CODE SPACE only (by
    /// construction, copied from a real image quad's own code), not in sky
    /// geometry -- the star index they reference has no relationship to
    /// this frame's actual pixel layout, so `solve_blind`'s later
    /// refinement stage is expected to fail. `hypotheses` is populated by
    /// the ENUMERATION stage, before that refinement ever runs, which is
    /// exactly the ordering `verify.rs`/Task 6 require ("gate once, after
    /// enumeration") -- this test exercises that stage in isolation.
    #[test]
    fn hypothesis_count_matches_an_independent_recount_of_what_was_offered() {
        let stars: Vec<(f64, f64, f32)> =
            (0..10).map(|i| (10.0 + i as f64 * 0.01, 20.0 + i as f64 * 0.01, 10.0 + i as f32)).collect();
        let star_index = build_test_index("blind-hypcount", &stars);

        let opts = SolveOptions {
            hint: None,
            saturation: Some(f32::INFINITY), // painted blocks are flat -- see painted_frame_bytes
            ..SolveOptions::default()
        };
        let bytes = painted_frame_bytes(640, 480, 20);
        let prepared = psolve_core::solve::prepare(&bytes, &opts)
            .unwrap_or_else(|_| panic!("fixture must extract at least 4 usable stars"));
        assert!(prepared.usable_star_count() >= 4, "fixture must produce enough stars to form quads");

        let image_pts = prepared.image_points();
        let iq = psolve_core::quad::build_quads(&image_pts, 6, opts.max_quads);
        assert!(!iq.is_empty(), "fixture must form at least one image quad");
        // Plant the FIRST image quad's own code into the quad index (band
        // 0) -- guarantees at least one real hit, so `expected == 0` cannot
        // pass this test vacuously.
        let quad_index =
            build_test_quad_index("hypcount", &star_index, stars.len() as u32, &[(0, iq[0].code)]);

        let search = solve_blind(
            "test-frame",
            &prepared,
            None,
            &star_index,
            &quad_index,
            2.0,  // radius_deg
            50,   // cat_limit
            None, // max_mag
            &opts,
            false,
        );

        // Independent recount: the SAME image quads, the SAME band
        // selection (scale is unknown here -- no FOCALLEN/XPIXSZ in
        // `painted_frame_bytes`'s header, no `--scale` -- so `select_bands`
        // searches every band, exactly as `solve_blind` does), summing
        // `QuadIndex::candidates` directly rather than trusting anything
        // `solve_blind` itself computed.
        let n_bands = quad_index.header().n_bands as usize;
        let band_scales = quad_index.header().band_scales_deg();
        let scale_estimate = opts.scale_arcsec.or_else(|| prepared.header_scale_arcsec());
        assert!(scale_estimate.is_none(), "fixture sanity: this test relies on an unknown scale");

        let mut expected = 0usize;
        for q in &iq {
            for band in select_bands(n_bands, &band_scales, q.diag, scale_estimate) {
                let resolved = quad_index
                    .candidates(q.code, BLIND_CODE_TOL, band)
                    .filter_map(|c| resolve_quad_sky(&star_index, &c.star_idx))
                    .count();
                expected += resolved;
            }
        }

        assert!(expected > 0, "fixture must offer at least the planted candidate");
        assert_eq!(
            search.hypotheses, expected,
            "M must equal exactly what was offered to candidate_transforms, recomputed independently"
        );
        assert_eq!(search.image_quads, iq.len());
    }

    /// `M` must be sourced from what was OFFERED, never from the survivor
    /// count `candidate_transforms` returns (Task 6/7's own binding
    /// constraint -- `verify.rs`'s "What counts toward `M`" doc). This
    /// fixture's noise records (40 of them, spread deterministically,
    /// unrelated to the frame's real geometry) almost certainly fail
    /// `blind::candidate_transform`'s shape/scale checks and so do not
    /// survive -- if `hypotheses` were wired to the survivor count instead
    /// of the offered one, it would undercount here, which is exactly the
    /// loosening direction Task 6's review forbids.
    #[test]
    fn hypotheses_offered_can_exceed_survivors() {
        // Ten near-collinear catalogue stars (deliberately: `ra = 10 +
        // i*0.01`, `dec = 20 + i*0.01`) -- `quad::quad_code` refuses a
        // degenerate/near-collinear configuration, so every candidate this
        // fixture offers is expected to fail `blind::candidate_transform`'s
        // shape check, guaranteeing the offered/survivor gap this test
        // exists to demonstrate rather than leaving it to chance.
        let stars: Vec<(f64, f64, f32)> =
            (0..10).map(|i| (10.0 + i as f64 * 0.01, 20.0 + i as f64 * 0.01, 10.0 + i as f32)).collect();
        let star_index = build_test_index("blind-offered-vs-survivors", &stars);
        let opts = SolveOptions { hint: None, saturation: Some(f32::INFINITY), ..SolveOptions::default() };
        let bytes = painted_frame_bytes(640, 480, 20);
        let prepared = psolve_core::solve::prepare(&bytes, &opts).expect("fixture must extract stars");
        let image_pts = prepared.image_points();
        let iq = psolve_core::quad::build_quads(&image_pts, 6, opts.max_quads);

        // Five records, ALL at image quad 0's own code (so a single
        // `candidates()` lookup offers all five at once, well inside
        // `BLIND_CODE_TOL`), each referencing a different quadruple of the
        // near-collinear stars above.
        let planted: Vec<(usize, [f64; 4])> = (0..5).map(|_| (0, iq[0].code)).collect();
        let quad_index = build_test_quad_index("offered-vs-survivors", &star_index, stars.len() as u32, &planted);
        // `build_test_quad_index` always uses star_idx [0,1,2,3] for every
        // planted record; that's fine here since the point is that this
        // ONE candidate is offered five times over (once per planted
        // duplicate) and none of the five are expected to survive.

        let candidates: Vec<super::QuadRecord> =
            quad_index.candidates(iq[0].code, BLIND_CODE_TOL, 0).collect();
        assert!(candidates.len() >= 5, "expected at least the five planted duplicates, got {}", candidates.len());
        let resolved: Vec<[(f64, f64); 4]> =
            candidates.iter().filter_map(|c| resolve_quad_sky(&star_index, &c.star_idx)).collect();
        assert_eq!(resolved.len(), candidates.len(), "every planted record must resolve");

        let survivors = psolve_core::blind::candidate_transforms(&iq[0], &image_pts, &resolved);
        assert!(
            survivors.len() < resolved.len(),
            "a near-collinear catalogue configuration must not survive the shape check: \
             {} survivors out of {} offered",
            survivors.len(),
            resolved.len()
        );

        // And the same gap must be visible through the full orchestration:
        // `search.hypotheses` (offered) must be at least as large as what
        // this single lookup alone offered, which is already known to
        // exceed what would survive it.
        let search =
            solve_blind("test-frame", &prepared, None, &star_index, &quad_index, 2.0, 50, None, &opts, false);
        assert!(
            search.hypotheses >= resolved.len(),
            "the full search's M ({}) must account for at least this one lookup's {} offered",
            search.hypotheses,
            resolved.len()
        );
    }
}

