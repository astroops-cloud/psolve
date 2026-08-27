//! Fix round 1: the XPIXSZ/binning retry (`cmd_solve::solve_with_binning_
//! retry`) must fire through **both** production entry points, not just
//! native `psolve solve`. This is the ASTAP-compatible `-f` dispatch path's
//! half of that coverage -- the one that actually matters per spec 8.1:
//! `ingest.identify.astap_solve` builds exactly this argv, so this is the
//! interface AstroOps' drop-in integration uses. Mirrors
//! `cli_solve_binning_retry.rs`'s fixture and reasoning exactly; see that
//! file for the XPIXSZ/XBINNING arithmetic in detail.

use psolve_core::fit::Wcs;
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
    let d = std::env::temp_dir().join(format!("psolve-astap-binning-retry-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap_or_else(|e| panic!("creating scratch dir {}: {e}", d.display()));
    ScratchDir(d)
}

struct RunResult {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

fn run_in(dir: &Path, args: &[&str]) -> RunResult {
    let o = Command::new(bin()).current_dir(dir).args(args).output().unwrap_or_else(|e| panic!("spawning psolve: {e}"));
    RunResult {
        code: o.status.code(),
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
    }
}

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
    ((a >> 11) as f64 / (1u64 << 53) as f64, (b >> 11) as f64 / (1u64 << 53) as f64)
}

const NX: usize = 640;
const NY: usize = 480;
// Same reference optics as cli_solve_binning_retry.rs: a physical 2.9um
// pixel at 243mm focal length is 2.4614 "/px unbinned, so at XBINNING=2 the
// TRUE per-file-pixel scale (what NAXIS1/2's own already-binned grid needs)
// is exactly double that.
const FOCALLEN_MM: f64 = 243.0;
const PHYSICAL_PIX_UM: f64 = 2.9;
const XBINNING: u32 = 2;
const TRUE_SCALE_ARCSEC: f64 = 206.265 * PHYSICAL_PIX_UM * XBINNING as f64 / FOCALLEN_MM; // ~4.9228
// What this rig's driver actually writes: the pixel size ALREADY multiplied
// by binning -- the header-derived scale computed from this is XBINNING x
// too coarse, exactly the ambiguity the retry exists for.
const WRITTEN_XPIXSZ_UM: f64 = PHYSICAL_PIX_UM * XBINNING as f64;

fn truth_wcs(ra0: f64, dec0: f64) -> Wcs {
    let s = TRUE_SCALE_ARCSEC / 3600.0;
    Wcs { crval: [ra0, dec0], crpix: [NX as f64 / 2.0, NY as f64 / 2.0], cd: [[-s, 0.0], [0.0, s]] }
}

/// A FITS frame with `n` Gaussian stars at the TRUE (binned) scale, an
/// `XPIXSZ`/`XBINNING`/`FOCALLEN` triple in the "already-binned" convention,
/// and a CSV catalogue from the same stars' true sky positions. Each star
/// gets a strictly distinct peak (no `--saturation` equivalent exists in
/// ASTAP's flag grammar to override the repeated-maximum saturation
/// heuristic, so the fixture is built to never let pixels tie in the first
/// place -- same reasoning as `astap_exit_codes.rs`'s own `build_fixture`).
fn build_fixture(ra0: f64, dec0: f64, n: usize) -> (Vec<u8>, String) {
    let w = truth_wcs(ra0, dec0);
    let margin = 40.0;
    let mut pix = Vec::new();
    for i in 0..n {
        let (u, v) = scatter(i);
        pix.push((margin + u * (NX as f64 - 2.0 * margin), margin + v * (NY as f64 - 2.0 * margin)));
    }

    let mut img = vec![1000f64; NX * NY];
    for (i, v) in img.iter_mut().enumerate() {
        *v += ((i * 2654435761usize) % 97) as f64 * 0.4;
    }
    let sigma = 1.8f64;
    let mut csv = String::from("ra,dec,pmra,pmdec,phot_g_mean_mag\n");
    for (k, &(cx, cy)) in pix.iter().enumerate() {
        let peak = 8000.0 - k as f64 * 20.0;
        let r = 5i64;
        for dy in -r..=r {
            for dx in -r..=r {
                let x = cx.round() as i64 + dx;
                let y = cy.round() as i64 + dy;
                if x < 0 || y < 0 || x >= NX as i64 || y >= NY as i64 {
                    continue;
                }
                let ex = x as f64 - cx;
                let ey = y as f64 - cy;
                let val = peak * (-(ex * ex + ey * ey) / (2.0 * sigma * sigma)).exp();
                img[y as usize * NX + x as usize] += val;
            }
        }
        let (ra, dec) = w.pix_to_radec(cx, cy);
        csv.push_str(&format!("{ra:.8},{dec:.8},0,0,{:.2}\n", 12.0 + (k % 10) as f64 * 0.1));
    }

    let cards = [
        "SIMPLE  =                    T".to_string(),
        "BITPIX  =                   16".to_string(),
        "NAXIS   =                    2".to_string(),
        format!("NAXIS1  = {NX:>20}"),
        format!("NAXIS2  = {NY:>20}"),
        "BZERO   =                32768".to_string(),
        format!("FOCALLEN= {FOCALLEN_MM:>20.4}"),
        format!("XPIXSZ  = {WRITTEN_XPIXSZ_UM:>20.4}"),
        format!("XBINNING= {XBINNING:>20}"),
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
    for v in &img {
        let clamped = v.clamp(0.0, 65535.0) as u16;
        let stored = (clamped as i32 - 32768) as i16;
        out.extend_from_slice(&stored.to_be_bytes());
    }
    while !out.len().is_multiple_of(2880) {
        out.push(0);
    }
    (out, csv)
}

/// Build the frame+catalogue fixture and a real `psolve index build`, inside
/// `d`. Returns the frame's bare filename (relative to `d`) and the `-d`
/// database directory.
fn setup(d: &Path, ra0: f64, dec0: f64) -> (String, PathBuf) {
    let (fits_bytes, csv) = build_fixture(ra0, dec0, 60);
    let frame_name = "field.fits";
    std::fs::write(d.join(frame_name), &fits_bytes).unwrap();

    let input = d.join("cat");
    std::fs::create_dir_all(&input).unwrap();
    std::fs::write(input.join("a.csv"), csv).unwrap();

    let db_dir = d.join("db");
    std::fs::create_dir_all(&db_dir).unwrap();
    let idx = db_dir.join("t.psidx");
    let o = Command::new(bin())
        .args(["index", "build", "--input"])
        .arg(&input)
        .arg("--out")
        .arg(&idx)
        .args(["--max-mag", "20", "--nside", "64"])
        .output()
        .unwrap();
    assert!(o.status.success(), "index build failed: {}", String::from_utf8_lossy(&o.stderr));

    (frame_name.to_string(), db_dir)
}

/// The regression this fix round exists for: through the `-f` ASTAP dispatch
/// path (the interface `ingest.identify.astap_solve` actually drives), a
/// bin-2 frame whose `XPIXSZ` is already-binned must still solve -- exit 0,
/// `PLTSOLVD=T` -- via the same scale/binning retry native mode already had.
/// Before this fix round, `astap_cmd` called `solve_prepared` directly with
/// `SolveOptions::default()` and no retry, so this exact fixture (solvable
/// through `psolve solve`) still failed through `-f`.
#[test]
fn a_pre_multiplied_xpixsz_solves_in_astap_mode_via_the_binning_retry() {
    let dir = scratch_dir("basic");
    let (ra0, dec0) = (150.0, -10.0);
    let (frame, db_dir) = setup(dir.path(), ra0, dec0);
    let ra_hours = ra0 / 15.0;
    let spd_deg = dec0 + 90.0;

    let r = run_in(
        dir.path(),
        &[
            "-f", &frame,
            "-ra", &ra_hours.to_string(),
            "-spd", &spd_deg.to_string(),
            "-r", "3",
            "-d", db_dir.to_str().unwrap(),
        ],
    );
    assert_eq!(r.code, Some(0), "stdout: {}\nstderr: {}", r.stdout, r.stderr);
    assert!(
        r.stderr.contains("retrying once"),
        "the binning retry must be logged to stderr through -f dispatch too: {}",
        r.stderr
    );

    let ini = std::fs::read_to_string(dir.path().join("field.ini"))
        .unwrap_or_else(|e| panic!("reading field.ini: {e}"));
    assert!(ini.starts_with("PLTSOLVD=T\n"), "ini was: {ini}");
    let crval1_line = ini.lines().find(|l| l.starts_with("CRVAL1=")).expect("CRVAL1 line");
    let crval1: f64 = crval1_line.trim_start_matches("CRVAL1=").trim().parse().expect("CRVAL1 must parse");
    assert!((crval1 - ra0).abs() < 0.01, "CRVAL1 {crval1} vs truth {ra0}");
}

/// An unbinned frame (`XBINNING` absent -- the ordinary case) must solve on
/// the first attempt with no retry logged, through `-f` dispatch exactly as
/// through native mode -- the gate must not fire when there is no binning
/// ambiguity to retry against.
#[test]
fn an_unbinned_frame_solves_in_astap_mode_with_no_retry() {
    let dir = scratch_dir("unbinned");
    let (ra0, dec0) = (150.0, -10.0);

    // Same fixture shape as astap_exit_codes.rs's own success test: physical
    // (not pre-multiplied) XPIXSZ, no XBINNING card at all.
    let w = Wcs {
        crval: [ra0, dec0],
        crpix: [NX as f64 / 2.0, NY as f64 / 2.0],
        cd: [[-2.4614 / 3600.0, 0.0], [0.0, 2.4614 / 3600.0]],
    };
    let margin = 40.0;
    let mut pix = Vec::new();
    for i in 0..60 {
        let (u, v) = scatter(i);
        pix.push((margin + u * (NX as f64 - 2.0 * margin), margin + v * (NY as f64 - 2.0 * margin)));
    }
    let mut img = vec![1000f64; NX * NY];
    for (i, v) in img.iter_mut().enumerate() {
        *v += ((i * 2654435761usize) % 97) as f64 * 0.4;
    }
    let sigma = 1.8f64;
    let mut csv = String::from("ra,dec,pmra,pmdec,phot_g_mean_mag\n");
    for (k, &(cx, cy)) in pix.iter().enumerate() {
        let peak = 8000.0 - k as f64 * 20.0;
        let r = 5i64;
        for dy in -r..=r {
            for dx in -r..=r {
                let x = cx.round() as i64 + dx;
                let y = cy.round() as i64 + dy;
                if x < 0 || y < 0 || x >= NX as i64 || y >= NY as i64 {
                    continue;
                }
                let ex = x as f64 - cx;
                let ey = y as f64 - cy;
                let val = peak * (-(ex * ex + ey * ey) / (2.0 * sigma * sigma)).exp();
                img[y as usize * NX + x as usize] += val;
            }
        }
        let (ra, dec) = w.pix_to_radec(cx, cy);
        csv.push_str(&format!("{ra:.8},{dec:.8},0,0,{:.2}\n", 12.0 + (k % 10) as f64 * 0.1));
    }
    let cards = [
        "SIMPLE  =                    T".to_string(),
        "BITPIX  =                   16".to_string(),
        "NAXIS   =                    2".to_string(),
        format!("NAXIS1  = {NX:>20}"),
        format!("NAXIS2  = {NY:>20}"),
        "BZERO   =                32768".to_string(),
        "FOCALLEN=                243.0".to_string(),
        "XPIXSZ  =                  2.9".to_string(),
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
    for v in &img {
        let clamped = v.clamp(0.0, 65535.0) as u16;
        let stored = (clamped as i32 - 32768) as i16;
        out.extend_from_slice(&stored.to_be_bytes());
    }
    while !out.len().is_multiple_of(2880) {
        out.push(0);
    }

    let frame_name = "field.fits";
    std::fs::write(dir.path().join(frame_name), &out).unwrap();
    let input = dir.path().join("cat");
    std::fs::create_dir_all(&input).unwrap();
    std::fs::write(input.join("a.csv"), csv).unwrap();
    let db_dir = dir.path().join("db");
    std::fs::create_dir_all(&db_dir).unwrap();
    let idx = db_dir.join("t.psidx");
    let o = Command::new(bin())
        .args(["index", "build", "--input"])
        .arg(&input)
        .arg("--out")
        .arg(&idx)
        .args(["--max-mag", "20", "--nside", "64"])
        .output()
        .unwrap();
    assert!(o.status.success(), "index build failed: {}", String::from_utf8_lossy(&o.stderr));

    let ra_hours = ra0 / 15.0;
    let spd_deg = dec0 + 90.0;
    let r = run_in(
        dir.path(),
        &[
            "-f", frame_name,
            "-ra", &ra_hours.to_string(),
            "-spd", &spd_deg.to_string(),
            "-r", "3",
            "-d", db_dir.to_str().unwrap(),
        ],
    );
    assert_eq!(r.code, Some(0), "stdout: {}\nstderr: {}", r.stdout, r.stderr);
    assert!(
        !r.stderr.contains("retrying once"),
        "an unbinned frame must never trigger the binning retry: {}",
        r.stderr
    );
    let ini = std::fs::read_to_string(dir.path().join("field.ini")).unwrap();
    assert!(ini.starts_with("PLTSOLVD=T\n"), "ini was: {ini}");
}
