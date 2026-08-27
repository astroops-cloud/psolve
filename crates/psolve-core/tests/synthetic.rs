//! Closed loop: build a catalogue, choose a WCS, render the frame it implies,
//! solve it, and demand the WCS back.
//!
//! This is the test that catches sign, parity and RA-direction errors -- the
//! classic way a plate solver is subtly wrong while every unit test passes.

use psolve_core::error::ReasonCode;
use psolve_core::fit::{Parity, Wcs};
use psolve_core::project::angsep_deg;
use psolve_core::solve::{
    prepare, solve, solve_prepared, CatalogStar, Outcome, SolveOptions, QUAD_RETRY_BUDGET,
};
use psolve_core::verify::AcceptParams;

const NX: usize = 1024;
const NY: usize = 768;
/// The reference rig's plate scale.
const SCALE_ARCSEC: f64 = 2.4614;

/// Deterministic pseudo-random scatter -- deliberately NOT a low-discrepancy
/// sequence.
///
/// Halton and R2 both equidistribute, which makes every star's local
/// neighbourhood resemble every other's, so k-nearest-neighbour quads alias
/// across the field. Measured swap rates against ground truth were 18% with
/// Halton and 77% with R2. A real star field is Poisson-scattered, with clumps
/// and voids, and that irregularity is what makes its quads distinctive. A
/// hashed counter reproduces it and stays exactly reproducible, so a failure
/// is still the same failure every time.
fn scatter(i: usize) -> (f64, f64) {
    let mut z = (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    let mut next = || {
        z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut x = z;
        x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        x ^ (x >> 31)
    };
    let a = next();
    let b = next();
    (
        (a >> 11) as f64 / (1u64 << 53) as f64,
        (b >> 11) as f64 / (1u64 << 53) as f64,
    )
}

/// A truth WCS for an arbitrary pixel grid and pixel scale.
fn truth_wcs_for(
    nx: usize, ny: usize, scale_deg: f64, ra0: f64, dec0: f64, rot_deg: f64, mirrored: bool,
) -> Wcs {
    let r = rot_deg.to_radians();
    let (c, si) = (r.cos(), r.sin());
    let m = if mirrored { -1.0 } else { 1.0 };
    Wcs {
        crval: [ra0, dec0],
        // `crpix` here is `[nx/2, ny/2]`, half a pixel off the true 0-based
        // image centre `[(nx-1)/2, (ny-1)/2]` -- the pre-fix convention (see
        // `sidecar.rs`'s "CRPIX convention" doc in psolve-cli). Harmless and
        // deliberate: this WCS is the truth the frame and catalogue are both
        // painted from, so the offset cancels exactly and no assertion here
        // is tight enough to notice. Not a statement about where the centre is.
        crpix: [nx as f64 / 2.0, ny as f64 / 2.0],
        cd: [[-scale_deg * c * m, scale_deg * si], [scale_deg * si * m, scale_deg * c]],
    }
}

fn truth_wcs(ra0: f64, dec0: f64, rot_deg: f64, mirrored: bool) -> Wcs {
    truth_wcs_for(NX, NY, SCALE_ARCSEC / 3600.0, ra0, dec0, rot_deg, mirrored)
}

/// A catalogue covering an arbitrary `nx x ny` field, plus the pixel
/// positions those stars land on under the truth WCS.
fn catalogue_for_grid(w: &Wcs, nx: usize, ny: usize, n: usize) -> (Vec<CatalogStar>, Vec<(f64, f64)>) {
    let mut cat = Vec::new();
    let mut pix = Vec::new();
    let mut i = 0usize;
    // Sample slightly beyond the frame so edge stars exist and land off-frame
    // or in the edge margin, exercising the extractor's edge filter rather
    // than pretending it never fires.
    while cat.len() < n && i < n * 8 {
        let (u, v) = scatter(i);
        let fx = u * 1.25 - 0.125;
        let fy = v * 1.25 - 0.125;
        i += 1;
        let px = fx * nx as f64;
        let py = fy * ny as f64;
        let (ra, dec) = w.pix_to_radec(px, py);
        cat.push(CatalogStar {
            ra,
            dec,
            mag: 10.0 + (cat.len() % 40) as f32 * 0.1,
            pmra: 0.0,
            pmdec: 0.0,
        });
        pix.push((px, py));
    }
    (cat, pix)
}

/// A catalogue covering the field, plus the pixel positions those stars land
/// on under the truth WCS.
fn catalogue_for(w: &Wcs, n: usize) -> (Vec<CatalogStar>, Vec<(f64, f64)>) {
    catalogue_for_grid(w, NX, NY, n)
}

/// Render a 16-bit FITS frame with Gaussian stars at the given pixel
/// positions, on an arbitrary `nx x ny` grid. `optics` is `(xpixsz_um,
/// focal_mm)`; `bayer` writes a `BAYERPAT` card so `decode()` superpixel-bins
/// the frame 2x2.
fn render_grid(
    pix: &[(f64, f64)], hint: (f64, f64), nx: usize, ny: usize,
    optics: Option<(f64, f64)>, bayer: bool,
) -> Vec<u8> {
    let mut img = vec![1000f64; nx * ny];
    // Deterministic texture so the background sigma is not degenerate.
    for (i, v) in img.iter_mut().enumerate() {
        *v += ((i * 2654435761usize) % 97) as f64 * 0.4;
    }
    let sigma = 1.8f64;
    for (k, &(cx, cy)) in pix.iter().enumerate() {
        // Vary brightness so ordering is meaningful.
        let peak = 8000.0 - (k % 30) as f64 * 120.0;
        let r = 5i64;
        for dy in -r..=r {
            for dx in -r..=r {
                let x = cx.round() as i64 + dx;
                let y = cy.round() as i64 + dy;
                if x < 0 || y < 0 || x >= nx as i64 || y >= ny as i64 {
                    continue;
                }
                let ex = x as f64 - cx;
                let ey = y as f64 - cy;
                let v = peak * (-(ex * ex + ey * ey) / (2.0 * sigma * sigma)).exp();
                img[y as usize * nx + x as usize] += v;
            }
        }
    }

    let mut cards: Vec<String> = vec![
        "SIMPLE  =                    T".into(),
        "BITPIX  =                   16".into(),
        "NAXIS   =                    2".into(),
        format!("NAXIS1  = {nx:>20}"),
        format!("NAXIS2  = {ny:>20}"),
        "BZERO   =                32768".into(),
    ];
    if let Some((xpixsz_um, focal_mm)) = optics {
        cards.push(format!("FOCALLEN=            {focal_mm}"));
        cards.push(format!("XPIXSZ  =            {xpixsz_um}"));
        cards.push("XBINNING=                    1".into());
    }
    if bayer {
        cards.push("BAYERPAT= 'RGGB'".into());
    }
    let ra_h = hint.0 / 15.0;
    let rh = ra_h.floor();
    let rm = ((ra_h - rh) * 60.0).floor();
    let rs = ((ra_h - rh) * 60.0 - rm) * 60.0;
    let sign = if hint.1 < 0.0 { "-" } else { "+" };
    let ad = hint.1.abs();
    let dd = ad.floor();
    let dm = ((ad - dd) * 60.0).floor();
    let ds = ((ad - dd) * 60.0 - dm) * 60.0;
    cards.push(format!("OBJCTRA = '{rh:02.0} {rm:02.0} {rs:05.2}'"));
    cards.push(format!("OBJCTDEC= '{sign}{dd:02.0} {dm:02.0} {ds:04.1}'"));
    cards.push("DATE-OBS= '2026-07-29T10:47:02'".into());

    let mut s = String::new();
    for c in &cards {
        s.push_str(&format!("{c:<80}"));
    }
    s.push_str(&format!("{:<80}", "END"));
    while !s.len().is_multiple_of(2880) {
        s.push(' ');
    }
    let mut out = s.into_bytes();
    for v in &img {
        let clamped = v.clamp(0.0, 65535.0) as u16;
        let stored = (clamped as i32 - 32768) as i16;
        out.extend_from_slice(&stored.to_be_bytes());
    }
    while !out.len().is_multiple_of(2880) {
        out.push(0);
    }
    out
}

/// Render on the module's reference grid. `with_optics` writes
/// FOCALLEN/XPIXSZ/XBINNING so the pixel scale matches `SCALE_ARCSEC`.
fn render(pix: &[(f64, f64)], hint: (f64, f64), with_optics: bool) -> Vec<u8> {
    let optics = if with_optics { Some((2.9, 243.0)) } else { None };
    render_grid(pix, hint, NX, NY, optics, false)
}

fn opts(hint: (f64, f64)) -> SolveOptions {
    SolveOptions {
        hint: Some(hint),
        scale_arcsec: Some(SCALE_ARCSEC),
        ..SolveOptions::default()
    }
}

/// Solve a synthetic field and assert the truth WCS comes back.
fn closed_loop(ra0: f64, dec0: f64, rot: f64, mirrored: bool) {
    let w = truth_wcs(ra0, dec0, rot, mirrored);
    let (cat, pix) = catalogue_for(&w, 90);
    let frame = render(&pix, (ra0, dec0), true);

    let out = solve(&frame, &cat, &opts((ra0, dec0)));
    let sol = match out {
        Outcome::Solved(s) => s,
        Outcome::Failed { reason, detail, stars_detected, stars_used, .. } => panic!(
            "ra0={ra0} dec0={dec0} rot={rot} mirrored={mirrored} failed: {reason} -- {detail} \
             (detected {stars_detected}, used {stars_used})"
        ),
    };

    // The centre must come back to well under a pixel.
    let truth_centre = w.pix_to_radec(NX as f64 / 2.0, NY as f64 / 2.0);
    let got_centre = sol.wcs.pix_to_radec(NX as f64 / 2.0, NY as f64 / 2.0);
    let sep_arcsec = angsep_deg(truth_centre.0, truth_centre.1, got_centre.0, got_centre.1) * 3600.0;
    assert!(
        sep_arcsec < SCALE_ARCSEC,
        "centre off by {sep_arcsec:.3} arcsec (more than one pixel) at ra0={ra0} dec0={dec0} rot={rot}"
    );

    assert!(
        (sol.wcs.scale_arcsec() - SCALE_ARCSEC).abs() < 0.02,
        "scale {} should be {SCALE_ARCSEC}",
        sol.wcs.scale_arcsec()
    );

    let want_parity = if mirrored { Parity::Mirrored } else { Parity::Normal };
    assert_eq!(sol.wcs.parity(), want_parity, "parity must be recovered, not assumed");
    assert_eq!(sol.mirrored, mirrored, "the reported mirrored flag must agree");

    assert!(sol.confidence.log_odds > 12.0, "log odds {}", sol.confidence.log_odds);
    assert!(sol.stars_matched >= 10, "only {} matched", sol.stars_matched);
}

#[test]
fn recovers_a_wcs_at_the_reference_pointing() {
    closed_loop(274.689087, -13.810971, 122.6, false);
}

#[test]
fn recovers_a_wcs_at_many_rotations() {
    for rot in [0.0, 37.0, 90.0, 180.0, 271.0, 359.0] {
        closed_loop(100.0, 20.0, rot, false);
    }
}

#[test]
fn recovers_a_mirrored_wcs() {
    // An odd number of reflections in the optical train produces this, and a
    // solver that assumes one handedness fails half the equipment it meets.
    for rot in [0.0, 45.0, 200.0] {
        closed_loop(100.0, 20.0, rot, true);
    }
}

#[test]
fn recovers_a_wcs_at_high_declination() {
    closed_loop(60.0, 75.0, 20.0, false);
    closed_loop(200.0, -78.0, 300.0, false);
}

#[test]
fn recovers_a_wcs_across_the_ra_wrap() {
    closed_loop(0.3, 10.0, 15.0, false);
    closed_loop(359.7, -10.0, 195.0, false);
}

#[test]
fn recovers_a_wcs_over_a_sweep_of_pointings() {
    // Deterministic sweep: same coverage as random, same failure every time.
    for i in 0..12 {
        let (u, v) = scatter(i);
        let ra = u * 360.0;
        let dec = v * 140.0 - 70.0;
        // Decorrelated from the (ra, dec) pair by a large index offset, so the
        // rotation isn't just a function of the pointing.
        let rot = scatter(i + 7919).0 * 360.0;
        closed_loop(ra, dec, rot, i % 3 == 0);
    }
}

#[test]
fn the_pixel_scale_is_taken_from_the_header_when_not_supplied() {
    let w = truth_wcs(120.0, -30.0, 40.0, false);
    let (cat, pix) = catalogue_for(&w, 90);
    let frame = render(&pix, (120.0, -30.0), true);
    let o = SolveOptions { hint: Some((120.0, -30.0)), ..SolveOptions::default() };
    match solve(&frame, &cat, &o) {
        Outcome::Solved(s) => {
            assert!((s.wcs.scale_arcsec() - SCALE_ARCSEC).abs() < 0.02);
        }
        Outcome::Failed { reason, detail, .. } => {
            panic!("header optics should have supplied the scale: {reason} -- {detail}")
        }
    }
}

#[test]
fn a_wrong_catalogue_does_not_produce_a_confident_answer() {
    // The property that protects a caller treating "solved" as proof of sky:
    // a catalogue from elsewhere must not yield a confident WCS.
    let w = truth_wcs(100.0, 20.0, 30.0, false);
    let (_, pix) = catalogue_for(&w, 90);
    let frame = render(&pix, (100.0, 20.0), true);

    let elsewhere = truth_wcs(250.0, -40.0, 10.0, false);
    let (wrong_cat, _) = catalogue_for(&elsewhere, 90);

    match solve(&frame, &wrong_cat, &opts((100.0, 20.0))) {
        Outcome::Failed { .. } => {}
        Outcome::Solved(s) => panic!(
            "solved against the wrong catalogue with {} matches and {:.1} decades",
            s.stars_matched, s.confidence.log_odds
        ),
    }
}

#[test]
fn a_cfa_frame_reports_the_same_sky_as_its_mono_equivalent() {
    // A CFA frame and its mono equivalent are the SAME camera geometry --
    // same NAXIS1/NAXIS2, same FOCALLEN/XPIXSZ -- imaging the same sky
    // through the same optics; the only difference is the BAYERPAT card,
    // which makes decode() superpixel-bin the frame 2x2 before the solver
    // ever sees it. Before this fix the two disagreed by exactly 2x in
    // reported field of view and plate scale, and the CFA solve's crpix/cd
    // described a pixel grid (half the file's own resolution) that does not
    // exist in the file -- both while reporting "solved":true.
    //
    // This is a psolve-core test (not a CLI one) because building two FITS
    // frames byte-for-byte, at the reference rig's real optics, is what the
    // bug needs and `synthetic.rs` already has the machinery for it; a
    // CLI-level check would have to reimplement the same rendering to get a
    // frame worth solving.
    let (ra0, dec0, rot) = (140.0, -25.0, 55.0);

    let w = truth_wcs(ra0, dec0, rot, false);
    let (cat, pix) = catalogue_for(&w, 90);

    // No explicit scale_arcsec here: SolveOptions.scale_arcsec, when set, is
    // in terms of the BINNED grid (see solve.rs and cmd_solve.rs's --scale
    // handling), so a test fixture that hardcodes one constant scale for
    // both frames would reproduce the exact bug this test exists to catch.
    // Leaving it None lets solve() derive it from FOCALLEN/XPIXSZ and
    // img.binned itself, which both frames carry identically.
    let solve_opts = |hint: (f64, f64)| SolveOptions { hint: Some(hint), ..SolveOptions::default() };

    let mono_frame = render(&pix, (ra0, dec0), true);
    let mono_sol = match solve(&mono_frame, &cat, &solve_opts((ra0, dec0))) {
        Outcome::Solved(s) => s,
        Outcome::Failed { reason, detail, .. } => panic!("mono failed: {reason} -- {detail}"),
    };

    // Identical file dimensions, identical optics keywords, identical
    // catalogue and star positions -- only BAYERPAT differs.
    let cfa_frame = render_grid(&pix, (ra0, dec0), NX, NY, Some((2.9, 243.0)), true);
    let cfa_sol = match solve(&cfa_frame, &cat, &solve_opts((ra0, dec0))) {
        Outcome::Solved(s) => s,
        Outcome::Failed { reason, detail, .. } => panic!("cfa failed: {reason} -- {detail}"),
    };

    assert_eq!(mono_sol.binned, 1, "the mono solve must report no binning");
    assert_eq!(cfa_sol.binned, 2, "the CFA solve must report the binning it applied");

    assert!(
        (mono_sol.wcs.scale_arcsec() - cfa_sol.wcs.scale_arcsec()).abs() < 0.05,
        "mono scale {} arcsec/px vs cfa scale {} arcsec/px -- same file, same optics, must agree",
        mono_sol.wcs.scale_arcsec(), cfa_sol.wcs.scale_arcsec()
    );

    // The two solves must agree on WHERE on the sky the frame is, not just
    // its scale. crval alone doesn't establish that if crpix differs (it
    // does, slightly -- see the crpix check below), so compare the sky
    // position each WCS reports at the same file-pixel centre.
    let mono_centre = mono_sol.wcs.pix_to_radec(NX as f64 / 2.0, NY as f64 / 2.0);
    let cfa_centre = cfa_sol.wcs.pix_to_radec(NX as f64 / 2.0, NY as f64 / 2.0);
    let centre_sep_arcsec =
        angsep_deg(mono_centre.0, mono_centre.1, cfa_centre.0, cfa_centre.1) * 3600.0;
    assert!(
        centre_sep_arcsec < 1.0,
        "mono centre ({}, {}) vs cfa centre ({}, {}) -- {centre_sep_arcsec:.4} arcsec apart, \
         should be well under one pixel ({SCALE_ARCSEC} arcsec/px) for the same sky, same optics",
        mono_centre.0, mono_centre.1, cfa_centre.0, cfa_centre.1
    );

    // The WCS must describe FILE pixel coordinates: crpix near the mono
    // solve's (both describe the SAME file grid), not the internal binned
    // grid's -- a consumer applies it to the file it has.
    assert!(
        (cfa_sol.wcs.crpix[0] - mono_sol.wcs.crpix[0]).abs() < 5.0,
        "crpix[0] {} should be near the mono solve's {}, not the binned grid's",
        cfa_sol.wcs.crpix[0], mono_sol.wcs.crpix[0]
    );
}

/// The solver's tolerance for catalogue/image set mismatch, measured.
///
/// Every other fixture hands the solver a catalogue that IS the image's star
/// set, which no real solve ever has. This pins the actual boundary.
///
/// **Envelope widened 2026-08-24 by the quad-budget retry, and this test is
/// how that became visible.** It previously asserted the opposite -- that a
/// 50%-both-ways mismatch does NOT solve at the shipped default, needing
/// roughly triple the budget (600 and 1200 failed, 1800 and above succeeded).
/// Its own panic message said "if this now SOLVES at the default budget the
/// envelope has widened -- good news, but update this test and the ledger
/// rather than deleting it", and that is what happened: it now solves with
/// **33 matches at 66.0 decades**.
///
/// That is independent corroboration of the retry, from a fixture built for a
/// different purpose. 50% mismatch both ways is precisely the real-frame
/// condition measured on the ATR585M -- completeness 42.4% on a frame that
/// fails against 63.1% on one that solves -- and a quad matches only if all
/// four of its stars survive on both sides, so the matchable fraction goes as
/// completeness^4. See `docs/superpowers/2026-08-24-atr585m-diagnostic.md`.
///
/// Kept rather than replaced, because the property worth pinning is the
/// boundary itself: a future change that narrows it again should fail here.
#[test]
fn a_fifty_percent_catalogue_mismatch_now_solves_at_the_default_budget() {
    // Every other fixture in this file hands the solver a catalogue that is
    // EXACTLY the image's star set. No real solve looks like that: the
    // catalogue always holds stars the frame cannot see (too faint, cropped
    // elsewhere), and the frame always holds stars the catalogue lacks
    // (below the survey's depth, blended, whatever).
    //
    // Quads are seeded from each star's LOCAL nearest neighbours (see
    // quad::build_quads), so with half of every star's neighbourhood replaced
    // by a DIFFERENT random half on each side, a quad built purely from the
    // common stars is rare. The retry is what now finds one.
    let (ra0, dec0) = (140.0, 15.0);
    let w = truth_wcs(ra0, dec0, 55.0, false);
    let (all_cat, all_pix) = catalogue_for(&w, 180);

    // Three groups of 60: [0,60) common to both, [60,120) image-only,
    // [120,180) catalogue-only.
    let image_pix = &all_pix[0..120]; // common + image-only: half unmatched.
    let mut catalog: Vec<CatalogStar> = all_cat[0..60].to_vec(); // common
    catalog.extend_from_slice(&all_cat[120..180]); // + catalogue-only: half unmatched.

    let frame = render(image_pix, (ra0, dec0), true);
    let at_default = opts((ra0, dec0)); // max_quads: 600, plus the retry.

    match solve(&frame, &catalog, &at_default) {
        Outcome::Solved(s) => {
            // The retry must be what answered. If this ever reports the base
            // budget, the fixture stopped exercising the mismatch case and the
            // assertions below prove nothing about it.
            assert_eq!(
                s.quad_budget, QUAD_RETRY_BUDGET,
                "this fixture must be solved BY the retry -- at the base budget it cannot match"
            );
            let truth_centre = w.pix_to_radec(NX as f64 / 2.0, NY as f64 / 2.0);
            let got_centre = s.wcs.pix_to_radec(NX as f64 / 2.0, NY as f64 / 2.0);
            let sep_arcsec =
                angsep_deg(truth_centre.0, truth_centre.1, got_centre.0, got_centre.1) * 3600.0;
            assert!(
                sep_arcsec < 1.0,
                "centre off by {sep_arcsec:.4} arcsec despite a 50% mismatch both ways"
            );
            assert!(s.stars_matched >= 10, "only {} matched", s.stars_matched);
            assert!(s.confidence.log_odds > 12.0, "only {:.1} decades", s.confidence.log_odds);
        }
        Outcome::Failed { reason, detail, .. } => panic!(
            "a 50% mismatch both ways solved at the default budget from 2026-08-24;              if it no longer does, the retry has been narrowed or removed: {reason} -- {detail}"
        ),
    }
}

#[test]
fn quality_metrics_come_back_with_the_solution() {
    // Rendered stars have sigma 1.8, so FWHM = 2.3548*1.8 = 4.24 px.
    let w = truth_wcs(274.689087, -13.810971, 0.0, false);
    let (cat, pix) = catalogue_for(&w, 90);
    let frame = render(&pix, (274.689087, -13.810971), true);
    match solve(&frame, &cat, &opts((274.689087, -13.810971))) {
        Outcome::Solved(s) => {
            let (fwhm, ecc, _) = s.quality.expect("quality should be reported");
            assert!((fwhm - 4.24).abs() < 1.5, "fwhm {fwhm} should be near 4.24");
            assert!(ecc < 0.25, "rendered stars are round, got ellipticity {ecc}");
        }
        Outcome::Failed { reason, detail, .. } => panic!("{reason} -- {detail}"),
    }
}

// ---------------------------------------------------------------------------
// The blind gate, driven end to end.
//
// `verify::AcceptParams::blind(M)` is the multiplicity-corrected threshold
// that makes blind solving safe in principle, and until these tests it had
// never been observed refusing anything outside `verify.rs`'s own unit tests.
// The blind-solve milestone's acceptance measurement ran 109 real frames
// against a deliberately wrong-sky quad index and recorded zero false
// positives -- but every one of those refusals came from star extraction or
// the quad matcher, upstream of the gate. `LOW_CONFIDENCE`, the gate's own
// refusal code, appeared zero times, so the measurement validated the
// pipeline and said nothing about the gate.
//
// These two tests close that hole with a fixture, not a rig: a frame whose
// only match against a wrong catalogue is a compact group, carried all the
// way through extraction, quad matching and the fit, so the ONLY stage left
// to refuse it is `verify::accept`.
//
// ## Fidelity, stated rather than implied
//
// This drives the hinted pipeline (`solve_prepared`) with the blind gate
// substituted into `SolveOptions::accept`. That exercises the real
// `AcceptParams::blind` arithmetic against a real candidate, but two things
// about the production blind path are NOT reproduced here, and neither can
// be from inside psolve-core:
//
//   * the candidate arrives from `match_::match_quads`, not from a `.psqidx`
//     code-space lookup driven through `blind::candidate_transform` -- that
//     lookup lives in psolve-index and is orchestrated by psolve-cli;
//   * the score is `verify::confidence`, not `verify::blind_confidence`, so
//     the correspondences the fit was built from are still counted as
//     evidence (verify.rs's limit 3). The real blind path deducts them,
//     which makes it STRICTER than what is measured here -- so a candidate
//     these tests show being refused would also be refused there.
//
// What is genuinely pinned is the thing that was never observed: a real
// candidate reaching `verify::accept` and being turned away by the blind
// threshold, and the threshold moving with `M` exactly as the Bonferroni
// derivation says it does.
// ---------------------------------------------------------------------------

/// A realistic single-band blind search: 600 image quads
/// (`SolveOptions::max_quads`) times the ~21 candidates a `.psqidx`
/// code-space lookup returns per image quad (Task 4's measurement). Same
/// constant `verify.rs`'s own tests use, for the same reason -- it is the
/// `M` a real blind solve of one band actually accumulates.
const BLIND_M: usize = 600 * 21;

/// Field stars, group stars and decoy catalogue entries.
///
/// These three numbers are not arbitrary: together they place the
/// coincidence's evidence deliberately BETWEEN the hinted gate (12.0
/// decades) and the blind one (16.10 at `BLIND_M`), which is the only window
/// in which the two gates can be told apart end to end.
///
/// `FIELD_STARS` sets the detection count (measured: 213), `DECOY_STARS`
/// sets the catalogue size (127 with the group), and those two fix
/// `lambda = n_image * n_cat * pi*tol^2 / area` at about 0.43. `GROUP_STARS`
/// sets how many coincidences the candidate gets to claim (measured: 12 of
/// the 14 survive extraction and reprojection). The result is a measured
/// 13.68 decades. Move any of them and the fixture stops testing the gap.
const FIELD_STARS: usize = 310;
const GROUP_STARS: usize = 14;
const DECOY_STARS: usize = 113;

/// Catalogue entries at the given pixel positions of `w`. Used to place
/// stars on the sky by where they would land on a frame, which is the only
/// convenient way to build a catalogue that covers a specific footprint.
fn catalogue_at(w: &Wcs, pix: &[(f64, f64)], mag: f32) -> Vec<CatalogStar> {
    pix.iter()
        .map(|&(x, y)| {
            let (ra, dec) = w.pix_to_radec(x, y);
            CatalogStar { ra, dec, mag, pmra: 0.0, pmdec: 0.0 }
        })
        .collect()
}

/// `n` catalogue stars scattered over the frame's footprint, from a scatter
/// offset far from the one the field itself uses, so not one of them
/// coincides with a real star of the frame.
fn decoy_catalogue(w: &Wcs, n: usize, seed: usize) -> Vec<CatalogStar> {
    let pix: Vec<(f64, f64)> = (seed..seed + n)
        .map(|i| {
            let (u, v) = scatter(i);
            (u * NX as f64, v * NY as f64)
        })
        .collect();
    catalogue_at(w, &pix, 12.0)
}

/// A compact group of `n` stars about `(cx, cy)`, no two closer than 9 px
/// (comfortably resolvable at the rendered FWHM of 4.24 px, so the extractor
/// returns them as separate sources rather than one blend).
///
/// Compactness is the whole mechanism. `quad::build_quads` seeds quads from
/// each star's six NEAREST neighbours, so a group only produces quads that
/// survive into both star sets if its members are each other's nearest
/// neighbours on both sides. A group spread out to the field's own spacing
/// (~62 px here) has its neighbourhoods invaded by field stars the wrong
/// catalogue does not contain, no quad is common to both sides, and the
/// solve dies at NO_QUAD_MATCH without ever reaching the gate -- measured,
/// not assumed: an earlier version of this fixture scattered the shared
/// stars over the whole frame and returned NO_QUAD_MATCH for every overlap
/// from 6 to 20 stars and every decoy count tried.
fn compact_group(n: usize, cx: f64, cy: f64, radius: f64, seed: usize) -> Vec<(f64, f64)> {
    let mut pix: Vec<(f64, f64)> = Vec::new();
    let mut i = seed;
    while pix.len() < n {
        let (u, v) = scatter(i);
        i += 1;
        let px = cx + (u * 2.0 - 1.0) * radius;
        let py = cy + (v * 2.0 - 1.0) * radius;
        if pix.iter().any(|&(x, y)| (x - px).hypot(y - py) < 9.0) {
            continue;
        }
        pix.push((px, py));
    }
    pix
}

/// The same point set, rotated about `from` and moved to `to`.
///
/// A congruence, not a similarity: distances are preserved exactly, so the
/// copy's quad codes equal the original's (`quad::quad_code` is invariant
/// under translation, rotation and scale) AND its implied plate scale is
/// unchanged, which matters because `MatchParams::expected_scale` is set
/// from the frame's optics with a 5% tolerance -- a rescaled copy would be
/// thrown out by the scale prior before the matcher ever voted, and the
/// candidate would never reach the gate.
fn congruent_copy(
    pix: &[(f64, f64)], from: (f64, f64), to: (f64, f64), rot_deg: f64,
) -> Vec<(f64, f64)> {
    let r = rot_deg.to_radians();
    let (c, s) = (r.cos(), r.sin());
    pix.iter()
        .map(|&(x, y)| {
            let (dx, dy) = (x - from.0, y - from.1);
            (to.0 + dx * c - dy * s, to.1 + dx * s + dy * c)
        })
        .collect()
}

/// The fixture both blind-gate tests run on.
///
/// A frame of `FIELD_STARS` scattered stars plus a compact group of
/// `GROUP_STARS`, and two catalogues for it:
///
///   * `truth` -- every star of the frame at its true position. The solve
///     that must be ACCEPTED, without which a test that only ever sees
///     refusals proves nothing (a gate that refuses everything would pass).
///   * `wrong` -- not one star of the frame. It holds `DECOY_STARS` stars
///     that are nowhere near any real one, plus a CONGRUENT COPY of the
///     compact group, rotated 140 degrees and moved to the other side of the
///     field. The copy's quad codes match the group's exactly, so the
///     matcher finds a genuine, consistent transform and the fit converges
///     tightly on it -- but the transform points the frame at the wrong
///     patch of sky, and nothing else in the catalogue corroborates it.
///
/// That is the motivating incident of this milestone in miniature: the
/// wide-search solve that came back 87.77 degrees from the truth and was
/// reported as a success by its own confidence gate. Here the same shape of
/// error is 6.89 arcmin, small only because both patches have to fit inside
/// one synthetic frame's footprint.
struct Fixture {
    frame: Vec<u8>,
    truth: Vec<CatalogStar>,
    wrong: Vec<CatalogStar>,
    hint: (f64, f64),
    truth_wcs: Wcs,
}

fn coincidence_fixture() -> Fixture {
    let (ra0, dec0, rot) = (140.0, 15.0, 55.0);
    let w = truth_wcs(ra0, dec0, rot, false);

    let (field_cat, field_pix) = catalogue_for(&w, FIELD_STARS);
    let group_pix = compact_group(GROUP_STARS, 700.0, 260.0, 42.0, 900_000);

    let mut frame_pix = field_pix.clone();
    frame_pix.extend_from_slice(&group_pix);
    let frame = render(&frame_pix, (ra0, dec0), true);

    let mut truth = field_cat;
    truth.extend(catalogue_at(&w, &group_pix, 11.0));

    let replica = congruent_copy(&group_pix, (700.0, 260.0), (300.0, 520.0), 140.0);
    let mut wrong = catalogue_at(&w, &replica, 11.0);
    wrong.extend(decoy_catalogue(&w, DECOY_STARS, 7_000_000));

    Fixture { frame, truth, wrong, hint: (ra0, dec0), truth_wcs: w }
}

/// How far, in arcmin, a solution's field centre sits from the truth.
fn centre_error_arcmin(truth: &Wcs, got: &Wcs) -> f64 {
    let (tx, ty) = truth.pix_to_radec(NX as f64 / 2.0, NY as f64 / 2.0);
    let (gx, gy) = got.pix_to_radec(NX as f64 / 2.0, NY as f64 / 2.0);
    angsep_deg(tx, ty, gx, gy) * 60.0
}

/// **The blind gate refusing a real candidate, end to end.**
///
/// Asserting merely "did not solve" would be worthless here -- the whole
/// question is WHICH stage refused, and every wrong-sky refusal Task 8
/// measured came from a stage upstream of the gate. So this test pins three
/// things together:
///
///   1. the wrong catalogue is refused with `LOW_CONFIDENCE` -- the gate's
///      own code -- and specifically NOT `NO_QUAD_MATCH` or `TOO_FEW_STARS`,
///      which would mean the candidate never got there;
///   2. the SAME frame and the SAME wrong catalogue are ACCEPTED when the
///      only thing changed is the threshold, from `blind(BLIND_M)` back to
///      the hinted default. Nothing upstream of `verify::accept` can explain
///      an outcome that flips on `min_log_odds` alone, so this is what
///      establishes the candidate reached the gate rather than dying before
///      it;
///   3. that accepted answer is WRONG by 6.89 arcmin, which is what the
///      blind gate is refusing and why refusing it matters.
///
/// And the complementary case: the same frame against its own true
/// catalogue is accepted AT THE BLIND GATE. Without that pair this test
/// could pass on a gate that refuses everything.
#[test]
fn the_blind_gate_refuses_a_candidate_the_hinted_gate_accepts() {
    let f = coincidence_fixture();
    let base = opts(f.hint);
    let blind = SolveOptions { accept: AcceptParams::blind(BLIND_M), ..base };
    let hinted = SolveOptions { accept: AcceptParams::default(), ..base };

    // (2) first: the candidate reaches the gate and the HINTED threshold
    // lets it through -- a confidently wrong answer, which is the failure
    // this milestone exists to prevent.
    let wrong_at_hinted = match solve(&f.frame, &f.wrong, &hinted) {
        Outcome::Solved(s) => s,
        Outcome::Failed { reason, detail, stars_detected, stars_used, .. } => panic!(
            "fixture is broken -- the candidate must REACH the gate for this test to \
             prove anything, and it died at {reason} instead: {detail} \
             (detected {stars_detected}, used {stars_used})"
        ),
    };
    // Measured: 12 matched, 13.68 decades, 6.89 arcmin off.
    assert!(
        wrong_at_hinted.confidence.log_odds > AcceptParams::default().min_log_odds,
        "fixture check: the coincidence must clear the hinted gate, got {:.2} decades",
        wrong_at_hinted.confidence.log_odds
    );
    assert!(
        wrong_at_hinted.confidence.log_odds < AcceptParams::blind(BLIND_M).min_log_odds,
        "fixture check: {:.2} decades is no longer BELOW the blind threshold {:.2}, so this \
         fixture has stopped testing the gap between the two gates -- retune FIELD_STARS / \
         GROUP_STARS / DECOY_STARS rather than deleting the test",
        wrong_at_hinted.confidence.log_odds,
        AcceptParams::blind(BLIND_M).min_log_odds
    );

    // (3) what the hinted gate just accepted is not the sky the frame saw.
    let err = centre_error_arcmin(&f.truth_wcs, &wrong_at_hinted.wcs);
    assert!(
        err > 1.0,
        "fixture check: the accepted answer is supposed to be WRONG, but its centre is \
         {err:.3} arcmin from the truth"
    );

    // (1) the gate, and only the gate, refuses it.
    match solve(&f.frame, &f.wrong, &blind) {
        Outcome::Failed { reason, detail, .. } => assert_eq!(
            reason,
            ReasonCode::LowConfidence,
            "the blind gate must be what refuses this candidate, not a stage upstream of it \
             -- got {reason}: {detail}"
        ),
        Outcome::Solved(s) => panic!(
            "the blind gate accepted a {:.2}-decade coincidence at a {:.2}-decade threshold, \
             {:.2} arcmin from the truth",
            s.confidence.log_odds,
            AcceptParams::blind(BLIND_M).min_log_odds,
            centre_error_arcmin(&f.truth_wcs, &s.wcs)
        ),
    }

    // The complementary case. A gate that refuses everything would satisfy
    // everything above.
    match solve(&f.frame, &f.truth, &blind) {
        Outcome::Solved(s) => {
            let err = centre_error_arcmin(&f.truth_wcs, &s.wcs);
            assert!(err * 60.0 < SCALE_ARCSEC, "centre off by {err:.5} arcmin");
            assert!(
                s.confidence.log_odds > AcceptParams::blind(BLIND_M).min_log_odds,
                "a genuine solve should clear the blind gate with room to spare, got {:.2}",
                s.confidence.log_odds
            );
        }
        Outcome::Failed { reason, detail, .. } => panic!(
            "the blind gate refused the frame's OWN catalogue -- a gate that refuses \
             everything proves nothing: {reason} -- {detail}"
        ),
    }
}

/// **The multiplicity correction itself, end to end.**
///
/// `AcceptParams::blind(M)` is `12.0 + log10(M)`, and until this test that
/// relationship was pinned only at the arithmetic level -- `verify.rs`'s
/// unit tests compare thresholds to each other and never put a candidate
/// through the pipeline. Here ONE candidate, extracted once, is judged at a
/// sweep of hypothesis counts, and the only thing that changes between runs
/// is `M`.
///
/// Two properties are asserted:
///
///   * monotonicity -- more hypotheses examined can only ever make the gate
///     harder to clear, so once the sweep refuses, it must never accept
///     again at a larger `M`;
///   * the crossover lands exactly where `12.0 + log10(M)` says it does.
///     The candidate scores `L` decades, so it must survive
///     `M = floor(10^(L-12))` and die at one more than that. Predicting the
///     crossover from the measured `L` rather than hardcoding it keeps this
///     a test of the RELATIONSHIP: it stays sharp even if the fixture's
///     evidence drifts, and it cannot be satisfied by a gate that happens to
///     sit near the right value.
///
/// `prepare` is called once and `solve_prepared` reused, so every run in the
/// sweep sees byte-identical detections -- a difference in outcome cannot be
/// blamed on the extractor.
#[test]
fn a_candidate_accepted_at_a_small_hypothesis_count_is_refused_at_a_large_one() {
    let f = coincidence_fixture();
    let base = opts(f.hint);
    let prepared = match prepare(&f.frame, &base) {
        Ok(p) => p,
        Err(o) => panic!("fixture frame must prepare: {o:?}"),
    };
    let at = |m: usize| {
        solve_prepared(&prepared, &f.wrong, &SolveOptions { accept: AcceptParams::blind(m), ..base })
    };

    // M = 1 is the hinted gate exactly (log10(1) = 0), so this is the same
    // acceptance the previous test establishes -- read here for its
    // log-odds, which is what predicts the crossover.
    let log_odds = match at(1) {
        Outcome::Solved(s) => s.confidence.log_odds,
        Outcome::Failed { reason, detail, .. } => panic!(
            "fixture is broken -- a search of ONE hypothesis applies no correction at all, \
             so this candidate must be accepted there: {reason} -- {detail}"
        ),
    };
    assert!(
        log_odds > 12.0 && log_odds < 19.0,
        "fixture check: {log_odds:.2} decades must sit inside the range a usize M can \
         actually cross (log10(usize::MAX) = 19.27) for a crossover to exist at all"
    );

    // The last M that still accepts, and the first that does not.
    let last_accepting = 10f64.powf(log_odds - 12.0).floor() as usize;
    let first_refusing = last_accepting + 1;
    assert!(
        AcceptParams::blind(last_accepting).min_log_odds <= log_odds
            && AcceptParams::blind(first_refusing).min_log_odds > log_odds,
        "fixture check: {log_odds:.6} decades lands on the knife edge between M={last_accepting} \
         ({:.6}) and M={first_refusing} ({:.6}); nudge the fixture off it",
        AcceptParams::blind(last_accepting).min_log_odds,
        AcceptParams::blind(first_refusing).min_log_odds
    );

    match at(last_accepting) {
        Outcome::Solved(_) => {}
        Outcome::Failed { reason, detail, .. } => panic!(
            "M={last_accepting} sets the gate to {:.4} decades, which {log_odds:.4} clears -- \
             but the pipeline refused it anyway: {reason} -- {detail}",
            AcceptParams::blind(last_accepting).min_log_odds
        ),
    }
    match at(first_refusing) {
        Outcome::Failed { reason, .. } => assert_eq!(
            reason,
            ReasonCode::LowConfidence,
            "one more hypothesis must be refused BY THE GATE, not by a stage upstream"
        ),
        Outcome::Solved(s) => panic!(
            "M={first_refusing} sets the gate to {:.4} decades and the candidate carries only \
             {:.4} -- accepting it means the multiplicity correction is not being applied",
            AcceptParams::blind(first_refusing).min_log_odds,
            s.confidence.log_odds
        ),
    }

    // Monotone across the whole representable range, including the real
    // single-band count and the saturating extreme.
    let mut refused_at: Option<usize> = None;
    for m in [1usize, 10, 100, 1_000, BLIND_M, 1_000_000, 1_000_000_000, usize::MAX] {
        let accepted = matches!(at(m), Outcome::Solved(_));
        match (accepted, refused_at) {
            (true, Some(earlier)) => panic!(
                "M={m} accepted a candidate that M={earlier} had already refused -- examining \
                 MORE hypotheses must never loosen the gate"
            ),
            (false, None) => refused_at = Some(m),
            _ => {}
        }
    }
    assert_eq!(
        refused_at,
        Some(100),
        "with {log_odds:.2} decades the sweep should first refuse at M=100 (gate {:.2})",
        AcceptParams::blind(100).min_log_odds
    );
}

/// A quad budget too small to match must be retried at a larger one.
///
/// **Why this exists.** On the primary rig ASTAP solves 19 frames psolve does
/// not (`docs/superpowers/2026-08-23-astap-head-to-head.md`). Seven of those
/// have 274-500 usable stars, an exact pointing hint, a plate scale correct to
/// 0.36%, and still find **zero** matching quad codes out of 720,000
/// comparisons. The binding constraint is `max_quads`: at 600 the image and
/// catalogue quad sets are drawn from populations of very different size and
/// do not overlap. Raising the budget makes them overlap -- frame 15181 solves
/// at 0.161" from ASTAP with log-odds 625 once it does.
///
/// ASTAP forms exactly one quad per star, so its budget scales with the star
/// count and never binds this way; psolve's is a fixed cap. See
/// `docs/superpowers/2026-08-24-astap-algorithm-comparison.md`.
///
/// The retry fires only on `NoQuadMatch`. `TooFewStars` cannot be helped by
/// more quads, and retrying past a `LowConfidence` refusal would be retrying
/// until a confidence gate is beaten -- the shape that once produced a
/// confident solve 87.77 degrees from the truth.
///
/// Driving this from a deliberately tiny starting budget rather than a
/// pathological star field keeps it deterministic: 8 quads cannot match, 1500
/// can, and the field is the same one every other test here uses.
#[test]
fn a_quad_budget_too_small_to_match_is_retried_at_a_larger_one() {
    let (ra0, dec0) = (100.0, 20.0);
    let w = truth_wcs(ra0, dec0, 0.0, false);
    let (cat, pix) = catalogue_for(&w, 90);
    let frame = render(&pix, (ra0, dec0), true);

    let starved = SolveOptions { max_quads: 8, ..opts((ra0, dec0)) };
    match solve(&frame, &cat, &starved) {
        Outcome::Solved(s) => {
            assert_eq!(
                s.quad_budget, QUAD_RETRY_BUDGET,
                "the solve must report the budget that actually answered, not the one asked for"
            );
        }
        Outcome::Failed { reason, detail, .. } => {
            panic!("8 quads should have failed and been retried at {QUAD_RETRY_BUDGET}: {reason} -- {detail}")
        }
    }
}

/// The retry must be unreachable for a frame that solves at the budget it was
/// given. That is what makes this change regression-free by construction
/// rather than by measurement: 10,141 frames solve today at 600 and none of
/// them can take a different path.
#[test]
fn a_frame_that_solves_at_its_given_budget_never_retries() {
    let (ra0, dec0) = (100.0, 20.0);
    let w = truth_wcs(ra0, dec0, 0.0, false);
    let (cat, pix) = catalogue_for(&w, 90);
    let frame = render(&pix, (ra0, dec0), true);

    match solve(&frame, &cat, &opts((ra0, dec0))) {
        Outcome::Solved(s) => assert_eq!(
            s.quad_budget,
            SolveOptions::default().max_quads,
            "a frame that solves first time must report the default budget, proving no retry ran"
        ),
        Outcome::Failed { reason, .. } => panic!("the control field must solve: {reason}"),
    }
}

/// Every reported stage must add up to `total`.
///
/// This is a real defect guard, not bookkeeping. `PreparedFrame::t_start` is
/// set once in `prepare` and never reset, so `total` spans every attempt
/// while the per-stage numbers describe only the last one. Before
/// `Timings::caller` existed, a frame that solved through the binning retry
/// reported stages summing to 37.5 ms against a total of 171.5 ms -- 78%
/// unattributed, with nothing saying why -- and that shortfall was read as a
/// hidden bottleneck in the catalogue fetch and chased for an hour. It was
/// the earlier attempt, correctly spent and simply unreported.
///
/// The tolerance is the measurement overhead between stage boundaries, which
/// is tens of microseconds on a solve of tens of milliseconds. It is
/// deliberately tight: a loose bound here would pass with `caller` wired to
/// a constant, which is precisely the thing this test exists to prevent.
#[test]
fn the_reported_stages_account_for_the_whole_solve() {
    let (ra0, dec0) = (83.0, -5.0);
    let w = truth_wcs(ra0, dec0, 12.0, false);
    let (cat, pix) = catalogue_for(&w, 90);
    let frame = render(&pix, (ra0, dec0), true);

    let sol = match solve(&frame, &cat, &opts((ra0, dec0))) {
        Outcome::Solved(s) => s,
        Outcome::Failed { reason, detail, .. } => panic!("fixture must solve: {reason} -- {detail}"),
    };
    let t = &sol.timings_ms;
    let parts = t.decode
        + t.background
        + t.extract
        + t.caller
        + t.quads
        + t.catalogue
        + t.match_
        + t.fit
        + t.verify;
    let gap = (t.total - parts).abs();
    assert!(
        gap <= 0.25 + t.total * 0.01,
        "stages do not account for the solve: parts {parts:.3} ms vs total {t:.3?} ms \
         (gap {gap:.3} ms)"
    );
    assert!(t.caller >= 0.0, "caller must never be negative, got {}", t.caller);
}

/// `caller` must actually MEASURE the interval, not report zero.
///
/// The test above cannot tell the difference: `solve` runs `prepare` and
/// `solve_prepared` back to back, so the true value there is ~0 and a
/// hard-coded zero passes it. Verified by mutation -- wiring `ms_caller` to
/// `0.0` leaves that test green, which is exactly the hollow-test shape this
/// project keeps paying for.
///
/// So this one splits the two calls and puts a known delay between them,
/// which is what the real CLI does with the catalogue disc query and, on a
/// retry, with an entire failed attempt.
#[test]
fn caller_measures_the_gap_between_preparing_and_solving() {
    let (ra0, dec0) = (83.0, -5.0);
    let w = truth_wcs(ra0, dec0, 12.0, false);
    let (cat, pix) = catalogue_for(&w, 90);
    let frame = render(&pix, (ra0, dec0), true);
    let o = opts((ra0, dec0));

    let prepared = psolve_core::solve::prepare(&frame, &o).expect("fixture must prepare");
    let delay = std::time::Duration::from_millis(60);
    std::thread::sleep(delay);
    let sol = match psolve_core::solve::solve_prepared(&prepared, &cat, &o) {
        Outcome::Solved(s) => s,
        Outcome::Failed { reason, detail, .. } => panic!("fixture must solve: {reason} -- {detail}"),
    };

    let t = &sol.timings_ms;
    assert!(
        t.caller >= 50.0,
        "caller {:.2} ms did not capture a {} ms gap -- it is not measuring the interval",
        t.caller,
        delay.as_millis()
    );
    // And the accounting still closes with a large caller value.
    let parts = t.decode
        + t.background
        + t.extract
        + t.caller
        + t.quads
        + t.catalogue
        + t.match_
        + t.fit
        + t.verify;
    let gap = (t.total - parts).abs();
    assert!(
        gap <= 0.25 + t.total * 0.01,
        "stages do not account for the solve: parts {parts:.3} ms vs total {:.3} ms",
        t.total
    );
}
