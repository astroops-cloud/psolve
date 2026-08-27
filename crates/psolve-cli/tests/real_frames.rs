//! Real-frame regression, run through the compiled `psolve` binary.
//!
//! `psolve-cli` is a bin-only crate (no `[lib]` target -- see the comment in
//! `src/cmd_solve.rs`), so this cannot call `solve_cmd` or its helpers
//! directly; it shells out, the same way `cli_solve_success.rs` does.

use std::path::Path;
use std::process::Command;

fn bin() -> std::path::PathBuf {
    let mut p = std::env::current_exe().unwrap();
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("psolve")
}

/// Pull `"field":{"center":{"ra":..,"dec":..}` out of the JSON without a JSON
/// dependency. `field.center` is the image-centre sky position -- not
/// `crval`, which `fit_tan` pins to the caller's pointing hint and therefore
/// trivially echoes it back regardless of where the frame actually points
/// (see the `field_center_reports_the_image_centre_not_the_hint` regression
/// in `cli_solve_success.rs`). ASTAP's own `CRVAL1/CRVAL2` for a single-frame
/// solve is the image centre, so this is the correct field to compare
/// against ASTAP's numbers.
fn extract_field_center(json: &str) -> Option<(f64, f64)> {
    let key = "\"center\":{\"ra\":";
    let start = json.find(key)? + key.len();
    let rest = &json[start..];
    let ra_end = rest.find(',')?;
    let ra: f64 = rest[..ra_end].trim().parse().ok()?;
    let after_comma = &rest[ra_end + 1..];
    let dec_key = "\"dec\":";
    let dec_start = after_comma.find(dec_key)? + dec_key.len();
    let dec_rest = &after_comma[dec_start..];
    let dec_end = dec_rest.find('}')?;
    let dec: f64 = dec_rest[..dec_end].trim().parse().ok()?;
    Some((ra, dec))
}

struct TestSolution {
    center_ra: f64,
    center_dec: f64,
}

/// Solve `path` against `idx` at CLI defaults (no `--hint`, no extraction
/// overrides): the header's own `OBJCTRA`/`OBJCTDEC` supply the pointing
/// hint, exactly as an operator running `psolve solve <file> --index <idx>`
/// would get. Returns `None` on any non-solve outcome or unparseable output.
fn solve_for_test(path: &str, idx: &Path) -> Option<TestSolution> {
    let o = Command::new(bin())
        .args(["solve", path, "--index"])
        .arg(idx)
        .output()
        .ok()?;
    let stdout = String::from_utf8_lossy(&o.stdout).to_string();
    if !stdout.contains("\"solved\":true") {
        return None;
    }
    let (center_ra, center_dec) = extract_field_center(&stdout)?;
    Some(TestSolution { center_ra, center_dec })
}

/// Great-circle separation in arcseconds -- the spherical law of cosines is
/// numerically fine at the sub-degree separations this test checks (a
/// mis-solve would be many arcminutes off, far outside the regime where that
/// formula's cancellation error would matter).
fn angular_sep_arcsec(ra1: f64, dec1: f64, ra2: f64, dec2: f64) -> f64 {
    let (r1, d1, r2, d2) =
        (ra1.to_radians(), dec1.to_radians(), ra2.to_radians(), dec2.to_radians());
    let cos_c = d1.sin() * d2.sin() + d1.cos() * d2.cos() * (r1 - r2).cos();
    cos_c.clamp(-1.0, 1.0).acos().to_degrees() * 3600.0
}

/// Real-frame regression. Synthetic fixtures encode the assumptions of
/// whoever wrote them -- M1's Gaia parser passed 93 unit tests while
/// retaining 0.14% of real rows, because every fixture shared the author's
/// wrong belief about the format. These frames are the antidote.
///
/// Skips rather than fails when the rig index is absent, so the suite still
/// runs on a machine without the 0.22 GB index.
#[test]
fn real_frames_solve_at_defaults() {
    let idx = std::path::Path::new(
        concat!(env!("HOME"), "/astroops/data/gaia-dr3-g14-dec45-nside64.psidx"),
    );
    if !idx.exists() {
        eprintln!("skipping: rig index not present");
        return;
    }
    // (frame, ASTAP CRVAL1, ASTAP CRVAL2)
    //
    // The second frame is the Task 4 sparse frame: 1491 detected / 95 used at
    // default extraction params (1373 rejected as too_small). It does NOT
    // need a min-pix/sigma change -- it solves at completely unmodified CLI
    // defaults. The sweep pinning `--cat-limit` to a fixed value (1500, the
    // value that works on the *other* frame) to make the min-pix x sigma
    // grid apples-to-apples masked this: `default_cat_limit` sizes itself
    // from THIS frame's own 95-detection count (95*3=285, clamped to the
    // 300 floor), and 300 is inside the narrow window (<=400) where the
    // matcher succeeds -- 450 and above return NO_QUAD_MATCH. See
    // docs/superpowers/2026-08-14-m3-first-real-frame.md, "Sparse frame",
    // for the full table.
    let frames = [
        (
            concat!(env!("HOME"), "/astroops/library/eagle/lights/H/\
                     2026-07-29_22-47-02_H_120.00s_100g_1x1_0001_-10.00.fits"),
            274.6890869201_f64,
            -13.81097073266_f64,
        ),
        (
            concat!(env!("HOME"), "/astroops/library/eagle/lights/H/\
                     2026-08-11_22-26-00_H_120.00s_100g_1x1_0001_-9.90.fits"),
            274.7273080441_f64,
            -13.84718397251_f64,
        ),
    ];
    for (path, ra, dec) in frames {
        if !std::path::Path::new(path).exists() {
            eprintln!("skipping {path}: not present");
            continue;
        }
        let sol = solve_for_test(path, idx).expect("must solve at defaults");
        let sep = angular_sep_arcsec(sol.center_ra, sol.center_dec, ra, dec);
        assert!(sep < 10.0, "{path}: {sep:.1}\" from ASTAP");
    }
}

/// Pull `"wcs":{"crpix":[x,y]` out of native mode's JSON. This is
/// psolve-core's raw, 0-based `Wcs.crpix` -- unlike `field.center`, nothing
/// converts it, and the ASTAP-mode sidecar writers take exactly this same
/// value and add `+ 1.0` (see `sidecar.rs`'s "CRPIX convention" module doc).
fn extract_native_crpix(json: &str) -> Option<(f64, f64)> {
    let key = "\"crpix\":[";
    let start = json.find(key)? + key.len();
    let end = json[start..].find(']')? + start;
    let mut parts = json[start..end].split(',');
    let x: f64 = parts.next()?.trim().parse().ok()?;
    let y: f64 = parts.next()?.trim().parse().ok()?;
    Some((x, y))
}

/// Pull `CRPIX1=<value>`/`CRPIX2=<value>` out of a `.ini` sidecar
/// (`sidecar::format_ini_success`'s output -- see that function's own doc).
fn parse_ini_crpix(ini: &str) -> Option<(f64, f64)> {
    let get = |key: &str| -> Option<f64> {
        let line = ini.lines().find(|l| l.starts_with(key))?;
        line[key.len()..].trim().parse().ok()
    };
    Some((get("CRPIX1=")?, get("CRPIX2=")?))
}

fn scratch_dir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("psolve-real-frames-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// The bug this pins (Task 11's fix round): psolve-core's internal pixel
/// coordinates are 0-based (`extract.rs` centroids over array indices
/// `0..nx`/`0..ny`), but FITS's own `CRPIX1`/`CRPIX2` convention is 1-based.
/// `sidecar.rs`'s `.ini`/`.wcs`/`-update` writers must add `+ 1.0` when
/// crossing from one to the other; they did not, so every sidecar and every
/// `-update`'d header this crate ever wrote had a CRPIX exactly one pixel
/// off both axes -- invisible to the pre-existing byte-exact tests because
/// those hand-transcribed a `Wcs` literal already holding the FITS-side
/// (1-based) value and asserted pass-through, never a real solve's
/// (0-based) output through the real conversion.
///
/// This test drives a REAL solve on a REAL frame through BOTH entry points
/// -- native mode (`psolve solve`, which reports `wcs.crpix` raw/0-based in
/// its JSON) and ASTAP mode (`psolve -f ...`, which writes the `.ini`
/// sidecar) -- with no `--hint`/`-ra`/`-spd` override on either, so both
/// resolve the identical pointing hint from the frame's own
/// `OBJCTRA`/`OBJCTDEC` and therefore run the identical underlying solve
/// (same bytes, same catalogue, same extraction defaults -- `solve()` is
/// deterministic). The two entry points' CRPIX must then differ by exactly
/// `1.0` on both axes: that is the whole conversion, pinned against a real
/// fit rather than a hand-built number.
///
/// A second, independent check corroborates against ASTAP's own real
/// solution for this exact frame, parsed from its own header (not
/// hand-transcribed): the frame is `library/prawn/...0023...fits`, the same
/// real solve `sidecar_ini.rs`/`sidecar_wcs.rs`'s fixtures are transcribed
/// from, with `CRPIX1`/`CRPIX2` at the frame's exact geometric centre
/// (1920.5, 1080.5 for this 3840x2160 frame -- ASTAP always centres CRPIX
/// there, letting CRVAL absorb the solved pointing). The frame's own
/// `OBJCTRA`/`OBJCTDEC` hint is off from ASTAP's true solved centre by
/// roughly 70-80" (real mount pointing error, not a bug), which at this
/// frame's ~2.45"/px scale moves psolve's OWN fitted CRPIX away from centre
/// by up to a few dozen pixels through the CD matrix -- so this check uses
/// a tolerance wide enough to absorb that genuine drift (150 px, comfortably
/// under half this frame's own width) while still being tight enough that a
/// CRPIX convention regression far larger than one pixel (an axis swap, a
/// units error, a missing conversion entirely reflected through a very
/// different hint) would still be caught. The exact, zero-tolerance check
/// above is what actually pins the one-pixel bug; this one is corroboration
/// against real ASTAP ground truth, per this fix round's own instruction.
#[test]
fn sidecar_crpix_is_one_based_and_agrees_with_astap_on_a_real_solve() {
    let idx = std::path::Path::new(
        concat!(env!("HOME"), "/astroops/data/gaia-dr3-g14-dec45-nside64.psidx"),
    );
    if !idx.exists() {
        eprintln!("skipping: rig index not present");
        return;
    }
    let source = std::path::Path::new(concat!(
        env!("HOME"),
        "/astroops/library/prawn/lights/S/2026-07-28_23-12-39_S_300.00s_100g_1x1_0023_-9.90.fits"
    ));
    if !source.exists() {
        eprintln!("skipping: reference frame not present");
        return;
    }
    // ~/astroops is read-only: read the bytes once, copy into scratch, and
    // do every solve/write against the copy. Never write into ~/astroops.
    let bytes = std::fs::read(source).expect("reading the real reference frame");
    let hdr = psolve_core::fits::FitsHeader::parse(&bytes).expect("real frame must have a header");
    let astap_crpix = (
        hdr.num("CRPIX1").expect("real frame must carry ASTAP's own CRPIX1"),
        hdr.num("CRPIX2").expect("real frame must carry ASTAP's own CRPIX2"),
    );
    let nx = hdr.num("NAXIS1").unwrap();
    let ny = hdr.num("NAXIS2").unwrap();
    assert!(
        (astap_crpix.0 - (nx / 2.0 + 0.5)).abs() < 1e-6
            && (astap_crpix.1 - (ny / 2.0 + 0.5)).abs() < 1e-6,
        "test's own assumption (ASTAP centres CRPIX on this frame) does not hold: \
         real header CRPIX {astap_crpix:?}, frame centre ({}, {})",
        nx / 2.0 + 0.5,
        ny / 2.0 + 0.5
    );

    let dir = scratch_dir("crpix");
    let native_copy = dir.join("native.fits");
    let astap_copy = dir.join("astap.fits");
    std::fs::write(&native_copy, &bytes).unwrap();
    std::fs::write(&astap_copy, &bytes).unwrap();

    // Native mode: no --hint and no --radius, so it resolves OBJCTRA/OBJCTDEC
    // from the header itself and sizes the search disc from the SAME header
    // (`cmd_solve::default_radius_deg`) that ASTAP mode below now also
    // prefers (the fix this test was updated for: ASTAP mode used to size
    // its disc from `-r` alone, which happened to equal a hand-picked 1.8
    // here and made the two paths' catalogues match by coincidence; now both
    // paths genuinely derive the SAME radius from the SAME header formula,
    // which is the whole point of the fix and a strictly better pin of "same
    // catalogue" than the old coincidence was).
    let o = Command::new(bin())
        .args(["solve"])
        .arg(&native_copy)
        .args(["--index"])
        .arg(idx)
        .output()
        .expect("running native mode");
    let native_json = String::from_utf8_lossy(&o.stdout).to_string();
    assert!(native_json.contains("\"solved\":true"), "native mode did not solve: {native_json}");
    let native_crpix = extract_native_crpix(&native_json).expect("crpix must parse from native JSON");

    // ASTAP mode: no -ra/-spd, same header fallback (main.rs's own doc
    // comment on this hint resolution); no -fov either, and -r 180 (ASTAP's
    // own "all-sky"/no-constraint convention) so it never clips the
    // header-derived radius -- the header is left free to win exactly as
    // `astap_args::search_radius_deg`'s doc comment says it should. Never
    // -update -- only the sidecars are written, and only into the scratch
    // copy's own directory.
    let db_dir = std::path::Path::new(concat!(env!("HOME"), "/astroops/data"));
    let o = Command::new(bin())
        .arg("-f")
        .arg(&astap_copy)
        .args(["-r", "180", "-d"])
        .arg(db_dir)
        .output()
        .expect("running ASTAP mode");
    assert_eq!(o.status.code(), Some(0), "ASTAP mode did not solve: {}", String::from_utf8_lossy(&o.stderr));
    let ini_path = astap_copy.with_extension("ini");
    let ini = std::fs::read_to_string(&ini_path).expect("reading the written .ini sidecar");
    let sidecar_crpix = parse_ini_crpix(&ini).expect("CRPIX1/CRPIX2 must parse from the .ini sidecar");

    // The exact pin: same solve, same crpix, offset by exactly the FITS
    // 1-based correction on both axes.
    assert!(
        (sidecar_crpix.0 - (native_crpix.0 + 1.0)).abs() < 1e-6,
        "sidecar CRPIX1 {} should be native wcs.crpix[0] {} + 1.0 = {}",
        sidecar_crpix.0, native_crpix.0, native_crpix.0 + 1.0
    );
    assert!(
        (sidecar_crpix.1 - (native_crpix.1 + 1.0)).abs() < 1e-6,
        "sidecar CRPIX2 {} should be native wcs.crpix[1] {} + 1.0 = {}",
        sidecar_crpix.1, native_crpix.1, native_crpix.1 + 1.0
    );

    // The corroborating, loose-tolerance check against real ASTAP ground
    // truth (this frame's own header, parsed above, not hand-transcribed).
    let dx = (sidecar_crpix.0 - astap_crpix.0).abs();
    let dy = (sidecar_crpix.1 - astap_crpix.1).abs();
    assert!(
        dx < 150.0 && dy < 150.0,
        "sidecar CRPIX {sidecar_crpix:?} is too far from ASTAP's real CRPIX {astap_crpix:?} \
         for this frame (dx={dx:.1}px, dy={dy:.1}px) -- a genuine mount-pointing-driven drift \
         of a few dozen pixels is expected here, not hundreds"
    );
}

/// The bug this pins (Task 11's fix round): `cmd_solve.rs`'s `field.center`
/// evaluated `pix_to_radec(nx/2, ny/2)` -- FITS's 1-based image centre --
/// against psolve-core's 0-based `Wcs`, half a pixel off-centre on both
/// axes. At this rig's ~2.45"/px scale that is roughly a **1.6" systematic
/// bias**, which was almost exactly Task 11's original 1.68" median
/// separation across 9219 real solves. Reverting the fix
/// (`(nx as f64 - 1.0) / 2.0` back to `nx as f64 / 2.0`) leaves every other
/// `psolve-cli` test green -- nothing else in the suite pins this specific
/// term, which is exactly how it shipped once already.
///
/// This test drives a REAL solve on the same real reference frame the
/// CRPIX test above uses (`library/prawn/...0023...fits`), with `--hint`
/// set to that frame's own real ASTAP `CRVAL1`/`CRVAL2` (parsed from the
/// header, not hand-transcribed) rather than the header's
/// `OBJCTRA`/`OBJCTDEC`. That is not circular for what this test checks:
/// ASTAP always centres `CRPIX` at the frame's exact geometric centre
/// (confirmed by an in-test assertion, matching the sibling test above), so
/// its `CRVAL` **is**, by construction, the true sky position of the image
/// centre -- independent of any pointing hint. Feeding psolve a very
/// accurate hint makes its own fit converge close to that same true centre
/// (tight fit RMS on this rig, ~0.4"), which is what makes a decisive,
/// non-flaky tolerance possible: a fixed half-pixel bug reliably produces
/// ~1.6-1.7" of error via the CD matrix on this frame regardless of hint
/// accuracy, while a correct implementation with an accurate hint lands
/// within a small fraction of an arcsecond. The tolerance below (1.0") sits
/// decisively between the two.
#[test]
fn field_center_matches_astap_header_crval_on_a_real_solve() {
    let idx = std::path::Path::new(
        concat!(env!("HOME"), "/astroops/data/gaia-dr3-g14-dec45-nside64.psidx"),
    );
    if !idx.exists() {
        eprintln!("skipping: rig index not present");
        return;
    }
    let source = std::path::Path::new(concat!(
        env!("HOME"),
        "/astroops/library/prawn/lights/S/2026-07-28_23-12-39_S_300.00s_100g_1x1_0023_-9.90.fits"
    ));
    if !source.exists() {
        eprintln!("skipping: reference frame not present");
        return;
    }
    let bytes = std::fs::read(source).expect("reading the real reference frame");
    let hdr = psolve_core::fits::FitsHeader::parse(&bytes).expect("real frame must have a header");
    let astap_crval = (
        hdr.num("CRVAL1").expect("real frame must carry ASTAP's own CRVAL1"),
        hdr.num("CRVAL2").expect("real frame must carry ASTAP's own CRVAL2"),
    );
    let astap_crpix = (
        hdr.num("CRPIX1").expect("real frame must carry ASTAP's own CRPIX1"),
        hdr.num("CRPIX2").expect("real frame must carry ASTAP's own CRPIX2"),
    );
    let nx = hdr.num("NAXIS1").unwrap();
    let ny = hdr.num("NAXIS2").unwrap();
    assert!(
        (astap_crpix.0 - (nx / 2.0 + 0.5)).abs() < 1e-6
            && (astap_crpix.1 - (ny / 2.0 + 0.5)).abs() < 1e-6,
        "test's own assumption (ASTAP centres CRPIX, so CRVAL is the true image \
         centre) does not hold for this frame: header CRPIX {astap_crpix:?}, \
         frame centre ({}, {})",
        nx / 2.0 + 0.5,
        ny / 2.0 + 0.5
    );

    let dir = scratch_dir("field-center");
    let copy = dir.join("frame.fits");
    std::fs::write(&copy, &bytes).unwrap();

    let o = Command::new(bin())
        .args(["solve"])
        .arg(&copy)
        .args(["--index"])
        .arg(idx)
        .args(["--hint", &format!("{},{}", astap_crval.0, astap_crval.1)])
        .args(["--radius", "1.8"])
        .output()
        .expect("running native mode");
    let json = String::from_utf8_lossy(&o.stdout).to_string();
    assert!(json.contains("\"solved\":true"), "did not solve: {json}");
    let (center_ra, center_dec) = extract_field_center(&json).expect("field.center must parse");

    let sep = angular_sep_arcsec(center_ra, center_dec, astap_crval.0, astap_crval.1);
    assert!(
        sep < 1.0,
        "field.center ({center_ra},{center_dec}) is {sep:.3}\" from ASTAP's real CRVAL \
         {astap_crval:?} (== the true image centre for this frame) -- a correct \
         0-based-centre evaluation should land within a small fraction of an \
         arcsecond given this accurate a hint; ~1.6-1.7\" here would mean the \
         half-pixel field.center bug regressed"
    );
}
