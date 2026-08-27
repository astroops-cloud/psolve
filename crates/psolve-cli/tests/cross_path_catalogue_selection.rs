//! Task 3's own regression: both catalogue-fetch call sites --
//! `cmd_solve.rs`'s native `psolve solve` and `main.rs`'s ASTAP-compatible
//! `psolve -f` dispatch -- must fetch the catalogue neighbourhood via
//! `Index::stratified_in_disc`, not `Index::brightest_in_disc`. This is the
//! exact shape the 2026-08-14 scale/binning retry got wrong: that fix landed
//! in `cmd_solve.rs` only, and `main.rs`'s ASTAP dispatch -- the interface
//! `ingest.identify.astap_solve` in the sibling astroops repo actually calls
//! -- kept the old, unfixed behaviour. Review caught it only by running the
//! same frame through both paths and comparing. This file is that same
//! comparison, made permanent.
//!
//! The fixture is engineered, not scavenged, so the divergence is
//! deterministic rather than "on a sparse field the two selections pick
//! nearly the same stars" (the brief's own warning): the synthetic catalogue
//! packs 350 catalogue-only "clump" stars into one HEALPix cell ~1 deg from
//! the pointing (think a globular cluster core sitting in the search disc
//! but off-frame -- it has no image counterpart at all) and 30 fainter
//! stars whose positions exactly match 30 real Gaussian sources rendered
//! into the FITS frame, in a different cell near the pointing centre. Every
//! clump star is brighter than every match star.
//!
//! At the auto catalogue depth (`cat_limit_for(30)` clamps to the 300
//! floor), `brightest_in_disc` -- pure global brightness order -- fills the
//! entire 300-star budget from the clump (350 > 300) and returns ZERO of the
//! 30 real matches, so the solve has nothing to match against
//! (`NO_QUAD_MATCH`; confirmed by hand below).  `stratified_in_disc`
//! round-robins per HEALPix cell, so the match cell's 30 stars are all
//! pulled in within the first ~30 rounds, comfortably inside the 300-star
//! budget, and the frame solves cleanly (all 30 detections matched). See
//! `reader.rs`'s `stratified_in_disc`/`brightest_in_disc` doc comments for
//! the general mechanism; this is that mechanism's effect made visible at
//! the CLI surface both production entry points share.
//!
//! `psolve-cli` is a bin-only crate (no `[lib]` target), so this spawns the
//! compiled binary both ways, the same pattern `astap_binning_retry.rs`/
//! `cli_solve_binning_retry.rs` use for the sibling wiring bug.
//!
//! **Verified load-bearing by hand**, with a freshly rebuilt binary each
//! time (cargo's mtime-based staleness check does not always notice a
//! `sed`-restored file, which cost real time while tuning this fixture --
//! `touch` the sources before rebuilding if you repeat this): reverting
//! *either* call site alone to `brightest_in_disc` reproduces exactly the
//! asymmetry this file exists to catch -- reverting `main.rs` alone: native
//! solves, ASTAP mode exits 1 with `ERROR=Not enough stars.`; reverting
//! `cmd_solve.rs` alone: ASTAP mode solves (exit 0), native reports
//! `"solved":false`. Reverting both: both fail deterministically
//! (`NO_QUAD_MATCH`). Only with both call sites on `stratified_in_disc` do
//! both entry points solve this fixture.

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
    let d = std::env::temp_dir()
        .join(format!("psolve-cross-path-catalogue-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d)
        .unwrap_or_else(|e| panic!("creating scratch dir {}: {e}", d.display()));
    ScratchDir(d)
}

/// Same xorshift-style scatter as `astap_binning_retry.rs`/
/// `cli_solve_binning_retry.rs` -- deterministic, well-spread pseudo-random
/// unit pairs with no external RNG dependency.
fn scatter(i: u64) -> (f64, f64) {
    let mut z = i.wrapping_mul(0x9E37_79B9_7F4A_7C15);
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
// A physical 2.9um pixel at 243mm focal length -- the same reference optics
// astap_binning_retry.rs/cli_solve_binning_retry.rs use -- gives 2.4614"/px.
// No XBINNING card at all: this fixture is not exercising the binning
// retry, so it stays out of that ambiguity entirely.
const FOCALLEN_MM: f64 = 243.0;
const PHYSICAL_PIX_UM: f64 = 2.9;
const SCALE_ARCSEC: f64 = 206.265 * PHYSICAL_PIX_UM / FOCALLEN_MM; // ~2.4614

const RA0: f64 = 200.0;
const DEC0: f64 = -20.0;
const SEARCH_RADIUS_DEG: f64 = 2.0;
const N_MATCH: usize = 30;
const N_CLUMP: usize = 350;
const CLUMP_BOX_DEG: f64 = 0.02;

/// 30 real image detections (rendered as Gaussian sources, matched by exact
/// catalogue counterparts) plus 350 catalogue-only "clump" stars ~1 deg away
/// that have no pixels in the frame at all. The clump stars are all brighter
/// (mag ~7.8-8.0) than every match star (mag ~11.9-11.9+), and both groups
/// land in different HEALPix nside=64 cells (checked by hand against
/// `psolve_index::healpix::ang2pix_nest` while designing this fixture: the
/// clump's 0.02 deg box at RA0 + ~1.0/cos(DEC0) stays inside a single cell
/// distinct from the match stars' cell near the pointing centre -- 29 of 30
/// match stars land in one cell, 1 in a neighbouring one, all 350 clump
/// stars in a third). Returns `(fits_bytes, catalogue_csv)`.
fn build_fixture() -> (Vec<u8>, String) {
    let s = SCALE_ARCSEC / 3600.0;
    let crpix = [NX as f64 / 2.0, NY as f64 / 2.0];
    let cd = [[-s, 0.0], [0.0, s]];
    let w = psolve_core::fit::Wcs { crval: [RA0, DEC0], crpix, cd };

    let margin = 40.0;
    let mut match_pix = Vec::new();
    for i in 0..N_MATCH {
        let (u, v) = scatter(i as u64);
        match_pix.push((
            margin + u * (NX as f64 - 2.0 * margin),
            margin + v * (NY as f64 - 2.0 * margin),
        ));
    }

    let mut img = vec![1000f64; NX * NY];
    for (i, v) in img.iter_mut().enumerate() {
        *v += ((i * 2654435761usize) % 97) as f64 * 0.4;
    }
    let sigma = 1.8f64;
    let mut csv = String::from("ra,dec,pmra,pmdec,phot_g_mean_mag\n");
    for (k, &(cx, cy)) in match_pix.iter().enumerate() {
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
        // Strictly distinct per star -- a repeating-magnitude formula ties
        // many records together, and the index's brightest-first sort within
        // a cell then breaks those ties in whatever order its sort happens
        // to leave them, which can shift which specific stars land at a
        // given rank when the total count changes. Not load-bearing for the
        // final fixed-size fixture below, but cheap insurance.
        let mag = 11.9 + k as f64 * 0.0005;
        csv.push_str(&format!("{ra:.8},{dec:.8},0,0,{mag:.4}\n"));
    }

    // The dense, catalogue-only clump: ~1 deg from the pointing, packed into
    // a small box, strictly brighter than every match star above. No pixels
    // are rendered for these -- they exist only to starve
    // `brightest_in_disc`'s global-brightness order.
    let dra = 1.0 / DEC0.to_radians().cos();
    let clump_ra = RA0 + dra;
    let clump_dec = DEC0;
    for k in 0..N_CLUMP {
        let (u, v) = scatter(10_000 + k as u64);
        let ra = clump_ra + (u - 0.5) * CLUMP_BOX_DEG;
        let dec = clump_dec + (v - 0.5) * CLUMP_BOX_DEG;
        let mag = 7.8 + k as f64 * 0.0005;
        csv.push_str(&format!("{ra:.8},{dec:.8},0,0,{mag:.4}\n"));
    }

    let cards = [
        "SIMPLE  =                    T".to_string(),
        "BITPIX  =                   16".to_string(),
        "NAXIS   =                    2".to_string(),
        format!("NAXIS1  = {NX:>20}"),
        format!("NAXIS2  = {NY:>20}"),
        "BZERO   =                32768".to_string(),
        format!("FOCALLEN= {FOCALLEN_MM:>20.4}"),
        format!("XPIXSZ  = {PHYSICAL_PIX_UM:>20.4}"),
    ];
    let mut hdr = String::new();
    for c in &cards {
        hdr.push_str(&format!("{c:<80}"));
    }
    hdr.push_str(&format!("{:<80}", "END"));
    while !hdr.len().is_multiple_of(2880) {
        hdr.push(' ');
    }
    let mut out = hdr.into_bytes();
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

/// Build the fixture and a real `psolve index build` inside `d`. Returns
/// the frame path and the `-d`-style database directory.
fn setup(d: &Path) -> (PathBuf, PathBuf) {
    let (fits_bytes, csv) = build_fixture();
    let frame = d.join("field.fits");
    std::fs::write(&frame, &fits_bytes).unwrap();

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

    (frame, db_dir)
}

/// Both entry points must use the stratified fetch. A fix that reaches only
/// the native path leaves the drop-in interface -- the one AstroOps calls --
/// on the old behaviour, which has happened before in this repo (the
/// 2026-08-14 scale/binning retry).
///
/// No `--cat-limit` on the native side and no equivalent exists in ASTAP's
/// own flag grammar for the ASTAP side: both resolve the SAME automatic
/// `cat_limit_for(usable)` depth (30 usable stars -> the 300 floor), so this
/// is a genuine apples-to-apples comparison of catalogue SELECTION, not of
/// catalogue DEPTH.
#[test]
fn both_entry_points_use_the_same_catalogue_selection() {
    let native_dir = scratch_dir("native");
    let (native_frame, native_db) = setup(native_dir.path());

    let o = Command::new(bin())
        .args(["solve"])
        .arg(&native_frame)
        .args(["--index"])
        .arg(native_db.join("t.psidx"))
        .args(["--hint", &format!("{RA0},{DEC0}")])
        .args(["--radius", &SEARCH_RADIUS_DEG.to_string()])
        .output()
        .expect("running native mode");
    let native_json = String::from_utf8_lossy(&o.stdout).to_string();
    let native_solved = native_json.contains("\"solved\":true");

    let astap_dir = scratch_dir("astap");
    let (astap_frame, astap_db) = setup(astap_dir.path());
    let ra_hours = RA0 / 15.0;
    let spd_deg = DEC0 + 90.0;
    let out_base = astap_dir.path().join("field");
    let o = Command::new(bin())
        .arg("-f")
        .arg(&astap_frame)
        .args(["-ra", &ra_hours.to_string()])
        .args(["-spd", &spd_deg.to_string()])
        .args(["-r", &SEARCH_RADIUS_DEG.to_string()])
        .args(["-d"])
        .arg(&astap_db)
        .args(["-o"])
        .arg(&out_base)
        .output()
        .expect("running ASTAP mode");
    let astap_solved = o.status.code() == Some(0);

    assert!(
        native_solved,
        "native mode did not solve the dense-clump fixture -- it must select \
         catalogue stars spread across cells (stratified_in_disc), not \
         brightest-N globally, or the 30 real match stars are starved out by \
         the 350-star clump: {native_json}"
    );
    assert!(
        astap_solved,
        "ASTAP mode ('-f' dispatch, the interface AstroOps' \
         ingest.identify.astap_solve actually calls) did not solve the same \
         fixture native mode solved -- this is exactly the 2026-08-14 gap: a \
         fix wired into cmd_solve.rs but not main.rs's ASTAP dispatch. \
         stderr: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert_eq!(
        native_solved, astap_solved,
        "native and ASTAP mode disagree on the same frame -- the two entry \
         points must fetch the catalogue identically"
    );
}
