//! Task 7: blind solving wired end to end, through BOTH entry points.
//!
//! A fix of this exact shape (the XPIXSZ/binning retry) reached only
//! `cmd_solve.rs` on 2026-08-14 and left ASTAP-compatible dispatch on stale
//! behaviour -- caught only by running the same frame through both. This
//! file does that for blind solving specifically: one real frame, its
//! pointing keywords stripped in a scratch copy (never `~/astroops`, which
//! is strictly read-only project-wide), solved through native `psolve
//! solve ... --quad-index ...` AND ASTAP-compatible `psolve -f ... -r 180`
//! with no `-ra`/`-spd`, asserting both solve and agree.
//!
//! `psolve-cli` has no `[lib]` target, so every check here shells out to the
//! compiled binary, the same pattern every other black-box test file in
//! this crate uses.
//!
//! Skips (with an `eprintln!`, not a failure) when the real
//! `~/astroops/data/gaia-dr3-g16-dec45-nside64.{psidx,psqidx}` pair or the
//! reference frame is absent, matching `real_frames.rs`'s and
//! `blind_candidates_real_index.rs`'s own convention.

use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> PathBuf {
    let mut p = std::env::current_exe().unwrap();
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("psolve")
}

struct ScratchDir(PathBuf);

impl ScratchDir {
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn scratch_dir(tag: &str) -> ScratchDir {
    let d = std::env::temp_dir().join(format!("psolve-blind-solve-test-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap_or_else(|e| panic!("creating scratch dir {}: {e}", d.display()));
    ScratchDir(d)
}

struct RunResult {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

fn run(args: &[&str]) -> RunResult {
    let o = Command::new(bin()).args(args).output().unwrap_or_else(|e| panic!("spawning psolve: {e}"));
    RunResult {
        code: o.status.code(),
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
    }
}

fn g16_star_index() -> PathBuf {
    PathBuf::from(concat!(env!("HOME"), "/astroops/data/gaia-dr3-g16-dec45-nside64.psidx"))
}

fn g16_quad_index() -> PathBuf {
    PathBuf::from(concat!(env!("HOME"), "/astroops/data/gaia-dr3-g16-dec45-nside64.psqidx"))
}

/// Real frame that already carries a genuine ASTAP solution in its own
/// header (`CRVAL1`/`CRVAL2`/the CD matrix, `PLTSOLVD=T`) alongside its
/// mount's commanded `OBJCTRA`/`OBJCTDEC`/`RA`/`DEC` -- the ground truth
/// this test's blind solve is checked against is read straight out of the
/// header, not hand-transcribed.
fn reference_frame() -> PathBuf {
    PathBuf::from(concat!(
        env!("HOME"),
        "/astroops/library/eagle/lights/H/2026-07-29_22-47-02_H_120.00s_100g_1x1_0001_-10.00.fits"
    ))
}

/// This frame's own real ASTAP-solved centre, transcribed from its header's
/// `CRVAL1`/`CRVAL2` (see `reference_frame`'s doc) -- printed in full by the
/// python header dump used to build this test, not re-derived.
const REFERENCE_CRVAL: (f64, f64) = (274.6890869201, -13.81097073266);

/// Requires every real, read-only input this file needs; skips (returning
/// `None`) rather than failing when any is absent, so this suite still runs
/// on a machine without the multi-GB blind-solve artefacts.
fn require_fixtures() -> Option<(PathBuf, PathBuf, PathBuf)> {
    let (psidx, psqidx, frame) = (g16_star_index(), g16_quad_index(), reference_frame());
    if !psidx.exists() || !psqidx.exists() {
        eprintln!("skipping: real gaia-dr3-g16-dec45-nside64 psidx/psqidx pair not present");
        return None;
    }
    if !frame.exists() {
        eprintln!("skipping: reference frame not present");
        return None;
    }
    Some((psidx, psqidx, frame))
}

/// A byte-exact 80-column FITS header card, keyed by its first 8 bytes
/// (trimmed) -- mirrors `psolve_core::fits::FitsHeader::parse`'s own card
/// grammar closely enough to find and blank the pointing keywords, without
/// depending on any `psolve-cli`-internal (private, `fits_update.rs`) card
/// scanner this bin-only crate's integration tests cannot link against.
fn card_key(card: &[u8]) -> String {
    String::from_utf8_lossy(&card[0..8.min(card.len())]).trim().to_string()
}

/// A scratch copy of `bytes` with every `OBJCTRA`/`OBJCTDEC`/`RA`/`DEC`
/// header card blanked to a `COMMENT` card -- forcing
/// `psolve_core::fits::hint_radec` to return `None` while leaving every
/// other keyword (including the frame's own real, already-solved
/// `CRVAL1`/`CRVAL2`, which `hint_radec` never reads) untouched. This is
/// the acceptance criterion 5 scenario made concrete: a frame whose
/// pointing is unusable, the same shape a sentinel `DEC = -90.` frame is.
fn strip_pointing_hint(bytes: &[u8]) -> Vec<u8> {
    let hdr = psolve_core::fits::FitsHeader::parse(bytes).expect("real frame header must parse");
    let mut out = bytes.to_vec();
    let mut off = 0usize;
    while off + 80 <= hdr.data_offset {
        let card = &bytes[off..off + 80];
        if matches!(card_key(card).as_str(), "OBJCTRA" | "OBJCTDEC" | "RA" | "DEC") {
            for b in &mut out[off..off + 80] {
                *b = b' ';
            }
            out[off..off + 8].copy_from_slice(b"COMMENT ");
        }
        off += 80;
    }
    out
}

/// Pull `"field":{"center":{"ra":..,"dec":..}` out of native mode's JSON --
/// the image-centre sky position, not `crval` (`fit_tan` pins `crval` to
/// whatever hint/seed was used, hinted or blind, so it is not comparable
/// across the two paths on its own). Reimplemented here rather than shared
/// with `real_frames.rs`: `psolve-cli` is bin-only, so each black-box test
/// file that needs this parses the JSON itself.
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

/// Pull `M=`, the "hypotheses offered" count, out of psolve's own blind
/// search diagnostic line on stderr (`solve_blind`'s doc: never surfaced in
/// the JSON, since that format is shared with the hinted path, which must
/// not move). Both entry points print the identical wording -- see
/// `cmd_solve.rs`'s `solve_cmd` and `main.rs`'s `astap_cmd`.
fn extract_hypotheses_offered(stderr: &str) -> Option<usize> {
    let key = "hypotheses offered";
    let idx = stderr.find(key)?;
    // `stderr[..idx]` ends in the space that separates the number from
    // "hypotheses" itself -- trim it first, or `rfind` finds THAT space
    // (not the one before the number) and returns an empty slice.
    let before = stderr[..idx].trim_end();
    let num_start = before.rfind(char::is_whitespace).map(|i| i + 1).unwrap_or(0);
    before[num_start..].parse().ok()
}

fn parse_ini_f64(ini: &str, key: &str) -> Option<f64> {
    let line = ini.lines().find(|l| l.starts_with(key))?;
    line[key.len()..].trim().parse().ok()
}

/// The ASTAP-mode `.ini`'s own image-centre sky position -- NOT its bare
/// `CRVAL1`/`CRVAL2`, which (like native mode's own internal `Wcs.crval`)
/// `fit_tan` pins to whatever hint/seed point was used to solve, hinted or
/// blind. Comparing raw `CRVAL` values across the two entry points only
/// works when both happen to have converged through the exact same seed --
/// true for this specific frame today, but not a real invariant, and not
/// what native mode's OWN `field.center` means (`extract_field_center`'s
/// doc) -- so this instead reconstructs the `.ini`'s full WCS and evaluates
/// it at the image centre, the same computation `field.center` already is.
///
/// `CRPIX1`/`CRPIX2` in the `.ini` are FITS's 1-based convention
/// (`sidecar.rs`'s own "CRPIX convention" doc: `format_ini_success` writes
/// `w.crpix + 1.0`) -- subtracted back out here to reconstruct the SAME
/// 0-based `Wcs` native mode holds internally, evaluated at the identical
/// 0-based centre pixel `(nx-1)/2, (ny-1)/2` `cmd_solve.rs` uses for
/// `field.center`.
fn ini_image_center(ini: &str, nx: f64, ny: f64) -> Option<(f64, f64)> {
    let wcs = psolve_core::fit::Wcs {
        crval: [parse_ini_f64(ini, "CRVAL1=")?, parse_ini_f64(ini, "CRVAL2=")?],
        crpix: [parse_ini_f64(ini, "CRPIX1=")? - 1.0, parse_ini_f64(ini, "CRPIX2=")? - 1.0],
        cd: [
            [parse_ini_f64(ini, "CD1_1=")?, parse_ini_f64(ini, "CD1_2=")?],
            [parse_ini_f64(ini, "CD2_1=")?, parse_ini_f64(ini, "CD2_2=")?],
        ],
    };
    Some(wcs.pix_to_radec((nx - 1.0) / 2.0, (ny - 1.0) / 2.0))
}

/// `NAXIS1`/`NAXIS2` straight out of the frame's own header.
fn frame_dims(bytes: &[u8]) -> (f64, f64) {
    let hdr = psolve_core::fits::FitsHeader::parse(bytes).expect("real frame header must parse");
    (
        hdr.int("NAXIS1").expect("NAXIS1 must be present") as f64,
        hdr.int("NAXIS2").expect("NAXIS2 must be present") as f64,
    )
}

/// Great-circle separation in arcseconds.
fn angsep_arcsec(ra1: f64, dec1: f64, ra2: f64, dec2: f64) -> f64 {
    let (r1, d1, r2, d2) = (ra1.to_radians(), dec1.to_radians(), ra2.to_radians(), dec2.to_radians());
    let cos_c = d1.sin() * d2.sin() + d1.cos() * d2.cos() * (r1 - r2).cos();
    cos_c.clamp(-1.0, 1.0).acos().to_degrees() * 3600.0
}

/// A blind-solved frame's own local quad-fit refinement (`solve_blind`'s
/// own doc: reused via `solve_prepared`, the SAME multi-quad match/fit the
/// hinted path always runs) is expected to land close to sub-arcsecond
/// accuracy, same as any other solve through this pipeline -- but the
/// acceptance criterion this milestone actually promises is 30". Generous
/// against real mount pointing error and the seed's own extrapolation
/// slack (`solve_blind`'s doc: up to ~30" from a single local quad, before
/// refinement even runs).
const ACCEPTANCE_TOLERANCE_ARCSEC: f64 = 30.0;

/// Native `psolve solve`, no `--hint`, with `--quad-index`: a frame whose
/// pointing is unusable solves anyway, and lands close to the frame's own
/// real (ASTAP-solved) centre.
#[test]
fn a_hintless_frame_with_a_quad_index_solves_blind() {
    let Some((psidx, psqidx, frame)) = require_fixtures() else { return };
    let dir = scratch_dir("native");
    let bytes = std::fs::read(&frame).expect("reading the real reference frame");
    let blind_bytes = strip_pointing_hint(&bytes);
    let blind_path = dir.path().join("blind.fits");
    std::fs::write(&blind_path, &blind_bytes).unwrap();

    let r = run(&[
        "solve",
        blind_path.to_str().unwrap(),
        "--index",
        psidx.to_str().unwrap(),
        "--quad-index",
        psqidx.to_str().unwrap(),
    ]);
    assert_eq!(r.code, Some(0), "blind solve must succeed: stdout={} stderr={}", r.stdout, r.stderr);
    assert!(r.stdout.contains("\"solved\":true"), "stdout={}", r.stdout);
    let (ra, dec) = extract_field_center(&r.stdout).expect("field.center must parse");
    let sep = angsep_arcsec(ra, dec, REFERENCE_CRVAL.0, REFERENCE_CRVAL.1);
    assert!(
        sep < ACCEPTANCE_TOLERANCE_ARCSEC,
        "blind solve landed {sep:.2}\" from the frame's own real ASTAP-solved centre"
    );

    let hypotheses = extract_hypotheses_offered(&r.stderr)
        .unwrap_or_else(|| panic!("no 'hypotheses offered' diagnostic in stderr:\n{}", r.stderr));
    assert!(hypotheses > 0, "a solved blind search must have offered at least one hypothesis");
}

/// Without `--quad-index`, a hintless frame is UNCHANGED: `NO_HINT`, not a
/// crash and not a silent blind attempt.
#[test]
fn a_hintless_frame_without_a_quad_index_still_returns_no_hint() {
    let Some((psidx, _psqidx, frame)) = require_fixtures() else { return };
    let dir = scratch_dir("no-quad-index");
    let bytes = std::fs::read(&frame).expect("reading the real reference frame");
    let blind_bytes = strip_pointing_hint(&bytes);
    let blind_path = dir.path().join("blind.fits");
    std::fs::write(&blind_path, &blind_bytes).unwrap();

    let r = run(&["solve", blind_path.to_str().unwrap(), "--index", psidx.to_str().unwrap()]);
    assert_eq!(r.code, Some(1), "NO_HINT is a normal negative outcome: stdout={}", r.stdout);
    assert!(r.stdout.contains("\"reason\":\"NO_HINT\""), "stdout={}", r.stdout);
    assert!(r.stdout.contains("\"solved\":false"), "stdout={}", r.stdout);
}

/// A hint plus a quad index still takes the HINTED path -- it must not
/// silently go blind. Proven the strong way: `--quad-index` names a file
/// that does not exist. If the hinted path silently ignored the hint and
/// went blind anyway, this would fail to open the quad index and exit 3;
/// if it correctly stays hinted, the quad index is never even touched and
/// the (real, ordinary) hinted solve succeeds exactly as it would with no
/// `--quad-index` at all.
#[test]
fn a_hint_plus_a_quad_index_still_takes_the_hinted_path() {
    let Some((psidx, _psqidx, frame)) = require_fixtures() else { return };
    let dir = scratch_dir("hinted-with-quad-index");
    let bytes = std::fs::read(&frame).expect("reading the real reference frame");
    let path = dir.path().join("hinted.fits");
    std::fs::write(&path, &bytes).unwrap();
    let nonexistent_quad_index = dir.path().join("does-not-exist.psqidx");

    let r = run(&[
        "solve",
        path.to_str().unwrap(),
        "--index",
        psidx.to_str().unwrap(),
        "--hint",
        &format!("{},{}", REFERENCE_CRVAL.0, REFERENCE_CRVAL.1),
        "--quad-index",
        nonexistent_quad_index.to_str().unwrap(),
    ]);
    assert_eq!(
        r.code,
        Some(0),
        "a hinted solve must not be derailed by an unopenable --quad-index it never needed: \
         stdout={} stderr={}",
        r.stdout,
        r.stderr
    );
    assert!(r.stdout.contains("\"solved\":true"), "stdout={}", r.stdout);
    assert!(
        !r.stderr.contains("blind search"),
        "the hinted path must never print the blind search diagnostic: stderr={}",
        r.stderr
    );
}

/// ASTAP-compatible `-f ... -r 180`, no `-ra`/`-spd`: the SAME frame,
/// through the OTHER entry point, must also solve blind and agree with
/// native mode -- the specific gap a 2026-08-14 fix of this shape left
/// open (landed in `cmd_solve.rs` alone, `main.rs`'s ASTAP dispatch left
/// stale). `-d` points at a scratch directory holding SYMLINKS to the real
/// `.psidx`/`.psqidx` pair (never a copy of ~1.5 GB of read-only data, and
/// never touching `~/astroops` itself), auto-discovered by
/// `resolve_index_path`/`resolve_quad_index_path` exactly as a real `-d`
/// directory would be.
#[test]
fn the_same_frame_solves_blind_through_astap_mode_and_agrees_with_native() {
    let Some((psidx, psqidx, frame)) = require_fixtures() else { return };
    let dir = scratch_dir("astap");
    let db_dir = dir.path().join("db");
    std::fs::create_dir_all(&db_dir).unwrap();
    std::os::unix::fs::symlink(&psidx, db_dir.join("gaia16.psidx")).expect("symlinking psidx");
    std::os::unix::fs::symlink(&psqidx, db_dir.join("gaia16.psqidx")).expect("symlinking psqidx");

    let bytes = std::fs::read(&frame).expect("reading the real reference frame");
    let blind_bytes = strip_pointing_hint(&bytes);
    let blind_path = dir.path().join("blind.fits");
    std::fs::write(&blind_path, &blind_bytes).unwrap();

    // Native mode first, as the reference this test's whole point is to
    // agree with.
    let native = run(&[
        "solve",
        blind_path.to_str().unwrap(),
        "--index",
        psidx.to_str().unwrap(),
        "--quad-index",
        psqidx.to_str().unwrap(),
    ]);
    assert_eq!(native.code, Some(0), "native blind solve must succeed: {}", native.stdout);
    let native_center = extract_field_center(&native.stdout).expect("native field.center must parse");

    // ASTAP-compatible mode: `-r 180`, no `-ra`/`-spd` -- exactly what
    // AstroOps' own blind invocation sends (this module's own doc).
    let astap = run(&[
        "-f",
        blind_path.to_str().unwrap(),
        "-r",
        "180",
        "-d",
        db_dir.to_str().unwrap(),
    ]);
    assert_eq!(
        astap.code,
        Some(0),
        "ASTAP-mode blind solve must also succeed: stdout={} stderr={}",
        astap.stdout,
        astap.stderr
    );
    let ini_path = blind_path.with_extension("ini");
    let ini = std::fs::read_to_string(&ini_path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", ini_path.display()));
    assert!(ini.starts_with("PLTSOLVD=T"), "ini={ini}");
    let (nx, ny) = frame_dims(&blind_bytes);
    // The `.ini`'s OWN image-centre sky position -- not its bare `CRVAL`,
    // which is the seed point, not comparable across entry points in
    // general (`ini_image_center`'s own doc).
    let astap_center = ini_image_center(&ini, nx, ny).expect("ini WCS must parse");

    // Both entry points solved the IDENTICAL bytes -- their reported field
    // centres must land on the same real sky position, not merely each be
    // "close to the truth" independently. A tight tolerance here is the
    // actual point of this test (the two-entry-point gap), so it is far
    // tighter than the 30" acceptance criterion checked against ground
    // truth below.
    let agreement_sep = angsep_arcsec(native_center.0, native_center.1, astap_center.0, astap_center.1);
    assert!(
        agreement_sep < 5.0,
        "native and ASTAP-mode blind solves disagree by {agreement_sep:.3}\" -- \
         native={native_center:?} astap_center={astap_center:?}"
    );

    // Corroboration against the frame's own real, independently-recorded
    // ASTAP ground truth (also an image centre: `REFERENCE_CRVAL` is this
    // exact frame's own real `CRVAL1`/`CRVAL2`, and ASTAP always centres
    // `CRPIX` on the true geometric centre -- `real_frames.rs`'s own doc
    // makes the same observation for this frame).
    let truth_sep = angsep_arcsec(astap_center.0, astap_center.1, REFERENCE_CRVAL.0, REFERENCE_CRVAL.1);
    assert!(
        truth_sep < ACCEPTANCE_TOLERANCE_ARCSEC,
        "ASTAP-mode blind solve landed {truth_sep:.2}\" from the frame's own real ASTAP-solved centre"
    );
}
