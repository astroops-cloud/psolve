//! ASTAP mode's own exit-code scheme, wired end to end (M3 Task 10).
//!
//! Ground truth (`docs/superpowers/2026-08-14-astap-format-facts.md` §3c,
//! reproduced live against the real `astap_cli` binary): a **two-code**
//! scheme, `0` on success/`--help`, `1` for everything else. Native mode's
//! own richer `0/1/2/3` scheme (`main.rs`'s module doc) is untouched --
//! `tests/cli_solve.rs`/`tests/cli_build.rs` already pin it, and the two
//! tests here that re-check it exist only to prove ASTAP-mode wiring did not
//! leak into it.
//!
//! `psolve-cli` has no `[lib]` target, so this spawns the compiled binary
//! (same pattern as every other black-box test file in this crate) rather
//! than calling `astap_cmd` directly.
//!
//! Real frame bytes, a real catalogue, and a real `psolve index build` are
//! used for the success/`-update`/idempotency tests below, all confined to
//! a per-test scratch directory under the system temp path -- nothing here
//! ever reads or writes `~/astroops`, which is strictly read-only
//! project-wide.

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
    let d = std::env::temp_dir().join(format!("psolve-astap-exit-codes-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap_or_else(|e| panic!("creating scratch dir {}: {e}", d.display()));
    ScratchDir(d)
}

struct RunResult {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

/// Run the compiled binary with `args`, from within `dir` -- ASTAP mode's
/// sidecar paths (`.ini`/`.wcs`) are relative to the process's cwd whenever
/// `-o` is absent, exactly like real `astap_cli`, so every test that checks
/// a written sidecar needs a controlled, isolated cwd.
fn run_in(dir: &Path, args: &[&str]) -> RunResult {
    let o = Command::new(bin()).current_dir(dir).args(args).output().unwrap_or_else(|e| panic!("spawning psolve: {e}"));
    RunResult {
        code: o.status.code(),
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
    }
}

/// Same as [`run_in`], but from this test binary's own cwd -- for cases
/// that only ever touch absolute/nonexistent paths and write no sidecar.
fn run(args: &[&str]) -> RunResult {
    let o = Command::new(bin()).args(args).output().unwrap_or_else(|e| panic!("spawning psolve: {e}"));
    RunResult {
        code: o.status.code(),
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
    }
}

// ---------------------------------------------------------------------
// Brief Step 1's three tests, pinned close to verbatim.
// ---------------------------------------------------------------------

/// A drop-in replacement that returns a different exit code is not a
/// drop-in replacement -- AstroOps branches on it.
///
/// The `--help` case is here because real `astap_cli` exits `0` for it and a
/// caller must see the same, but note *how* psolve gets there: `--help`
/// carries no `-f`, and `-f` is what selects ASTAP mode, so this one goes
/// through NATIVE mode and prints psolve's own usage text. The exit code
/// matches; the stdout does not. (Recorded deviation from spec 8.1, which
/// specifies argv0/`--astap-compat` as the mode triggers -- see
/// `docs/astap-compat.md`'s "Mode detection".)
#[test]
fn astap_mode_uses_astap_exit_codes() {
    assert_eq!(run(&["-f", "/nonexistent.fits"]).code, Some(1));
    assert_eq!(run(&["-f", "valid.fits", "-d", "/nonexistent"]).code, Some(1));
    let help = run(&["--help"]);
    assert_eq!(help.code, Some(0));
    assert!(help.stdout.starts_with("psolve"), "--help is served by native mode: {}", help.stdout);
}

/// Native mode keeps its own richer scheme -- ASTAP compatibility must not
/// leak into it. (Also pinned in `tests/cli_solve.rs`/`cli_build.rs`; this
/// repeats the two cases the M3 Task 10 brief names, as the direct control
/// for the ASTAP-mode collapse this file is about.)
///
/// The brief's own illustrative sketch used a nonexistent FRAME path for
/// the exit-3 case (`run_native(&["solve", "/nonexistent.fits", "--index",
/// "i"])`), but `cmd_solve.rs` reads the frame before it opens the index, so
/// a missing frame is exit `2` (a usage error), not `3` -- confirmed against
/// the existing, deliberately-unchanged `cli_solve.rs::a_missing_index_exits_3`
/// control, which uses a real frame and a missing INDEX to reach exit `3`.
/// Reproduced that way here rather than the brief's literal snippet, per
/// this task's own constraint: native mode's codes must not change, and
/// changing them to fit the brief's sketch instead of testing what they
/// actually do would be exactly that.
#[test]
fn native_mode_keeps_its_own_exit_codes() {
    let dir = scratch_dir("native-exit-codes");
    let frame = dir.path().join("blank.fits");
    std::fs::write(&frame, minimal_blank_fits()).unwrap();

    let r = run_in(dir.path(), &["solve", frame.to_str().unwrap(), "--index", "/nonexistent.psidx"]);
    assert_eq!(r.code, Some(3), "a real frame with a missing index must be exit 3: stderr: {}", r.stderr);

    assert_eq!(run(&["solve"]).code, Some(2));
}

/// A minimal, valid FITS header with no pixel data beyond a single 2880-byte
/// block -- enough for `Index::open`/frame-reading to get far enough to
/// exercise the index-open failure path, not a realistic image.
fn minimal_blank_fits() -> Vec<u8> {
    let cards = [
        "SIMPLE  =                    T".to_string(),
        "BITPIX  =                   16".to_string(),
        "NAXIS   =                    2".to_string(),
        "NAXIS1  =                    4".to_string(),
        "NAXIS2  =                    4".to_string(),
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
    out.extend_from_slice(&[0u8; 4 * 4 * 2]);
    while !out.len().is_multiple_of(2880) {
        out.push(0);
    }
    out
}

/// A failed solve in ASTAP mode still writes the failure `.ini` -- leading
/// blank line, `PLTSOLVD=F`, `ERROR=` -- and exits `1`.
#[test]
fn a_failed_solve_in_astap_mode_still_writes_the_failure_ini() {
    let dir = scratch_dir("failure-ini");
    std::fs::write(dir.path().join("unsolvable.fits"), b"not a real fits file").unwrap();

    let r = run_in(dir.path(), &["-f", "unsolvable.fits", "-d", "/nonexistent"]);

    let ini = std::fs::read_to_string(dir.path().join("unsolvable.ini"))
        .unwrap_or_else(|e| panic!("reading unsolvable.ini: {e}; stderr: {}", r.stderr));
    assert!(ini.starts_with('\n') && ini.contains("PLTSOLVD=F") && ini.contains("ERROR="));
    assert_eq!(r.code, Some(1));
}

// ---------------------------------------------------------------------
// Real end-to-end: a synthetic frame + catalogue that actually solves.
// ---------------------------------------------------------------------

const NX: usize = 640;
const NY: usize = 480;
/// 206.265 * XPIXSZ * binning / FOCALLEN, with the real fixture's own
/// FOCALLEN=243.0/XPIXSZ=2.9 (ground-truth doc §2a) -- so the header's own
/// optics keywords derive exactly this scale, since ASTAP mode has no
/// `--scale`-equivalent flag to pass one explicitly.
const SCALE_ARCSEC: f64 = 206.265 * 2.9 / 243.0;

/// Deterministic hashed scatter -- the same generator `psolve-core`'s own
/// synthetic tests and `cli_solve_success.rs` use, reproduced here since
/// there is no `[lib]` target to share it from.
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

/// `crpix` is `[NX/2, NY/2]`, which in psolve-core's own 0-BASED pixel
/// convention is **half a pixel off** the true image centre (that is
/// `[(NX-1)/2, (NY-1)/2]` -- see `sidecar.rs`'s "CRPIX convention" module
/// doc and the `field.center` fix it came out of). Left as-is deliberately:
/// this is a self-consistent synthetic truth -- the catalogue below is
/// generated by pushing pixels through this very WCS -- so the half-pixel
/// offset cancels exactly, and every tolerance here is orders of magnitude
/// wider than it anyway. Do not read "the exact image centre" off this
/// line; it is the pre-fix convention, kept only because changing it would
/// regenerate the fixture for no gain.
fn truth_wcs(ra0: f64, dec0: f64) -> Wcs {
    let s = SCALE_ARCSEC / 3600.0;
    Wcs { crval: [ra0, dec0], crpix: [NX as f64 / 2.0, NY as f64 / 2.0], cd: [[-s, 0.0], [0.0, s]] }
}

/// A FITS frame with `n` Gaussian stars, well clear of the edge margin, plus
/// a CSV catalogue built from those same stars' true sky positions under
/// `truth_wcs`. Each star gets a strictly distinct peak value
/// (`8000 - k*20`, `k` in `0..n`), unlike `cli_solve_success.rs`'s shared
/// `peak % 20` cycle -- ASTAP mode has no `--saturation`-equivalent flag to
/// override `solve()`'s own `default_saturation` heuristic (which treats 8+
/// pixels sharing the image maximum as a saturation ceiling), so this
/// fixture is built to never let more than one pixel share any value in the
/// first place, rather than relying on a flag this mode cannot pass.
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
        "FOCALLEN=                243.0".to_string(),
        "XPIXSZ  =                  2.9".to_string(),
        "COMMENT captured by N.I.N.A.".to_string(),
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

/// Build the frame+catalogue fixture and a real `psolve index build` from
/// it, entirely inside `d`. Returns the frame path (bare filename, relative
/// to `d`) and the directory holding the built `.psidx` -- the ASTAP `-d`
/// argument.
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

/// A matching frame solves in ASTAP mode: exit `0`, a `PLTSOLVD=T` `.ini`
/// with the right `CRVAL`, and a `.wcs` sidecar with the WCS solution cards
/// and a downgraded (`BITPIX=8`/`NAXIS=0`, no `NAXIS1`/`NAXIS2`)
/// pass-through of the original header, per ASTAP's own documented `.wcs`
/// transform (ground-truth doc §2a).
#[test]
fn a_matching_frame_solves_in_astap_mode_and_writes_both_sidecars() {
    let dir = scratch_dir("solve-success");
    let (ra0, dec0) = (150.0, -10.0);
    let (frame, db_dir) = setup(dir.path(), ra0, dec0);
    let ra_hours = ra0 / 15.0;
    let spd_deg = dec0 + 90.0;

    let r = run_in(
        dir.path(),
        &[
            "-f",
            &frame,
            "-ra",
            &ra_hours.to_string(),
            "-spd",
            &spd_deg.to_string(),
            "-r",
            "3",
            "-d",
            db_dir.to_str().unwrap(),
        ],
    );
    assert_eq!(r.code, Some(0), "stdout: {}\nstderr: {}", r.stdout, r.stderr);

    let ini = std::fs::read_to_string(dir.path().join("field.ini"))
        .unwrap_or_else(|e| panic!("reading field.ini: {e}"));
    assert!(ini.starts_with("PLTSOLVD=T\n"), "ini was: {ini}");
    let crval1_line = ini.lines().find(|l| l.starts_with("CRVAL1=")).expect("CRVAL1 line");
    let crval1: f64 = crval1_line.trim_start_matches("CRVAL1=").trim().parse().expect("CRVAL1 must parse");
    assert!((crval1 - ra0).abs() < 0.01, "CRVAL1 {crval1} vs truth {ra0}");

    let wcs = std::fs::read_to_string(dir.path().join("field.wcs"))
        .unwrap_or_else(|e| panic!("reading field.wcs: {e}"));
    assert!(wcs.contains("CRVAL1"), "wcs was: {wcs}");
    assert!(wcs.contains("BITPIX  =                    8"), "BITPIX must be downgraded: {wcs}");
    assert!(wcs.contains("NAXIS   =                    0"), "NAXIS must be downgraded: {wcs}");
    assert!(!wcs.contains("NAXIS1"), "NAXIS1 must be dropped from the .wcs pass-through: {wcs}");
    assert!(!wcs.contains("NAXIS2"), "NAXIS2 must be dropped from the .wcs pass-through: {wcs}");
    assert!(
        wcs.contains("COMMENT captured by N.I.N.A."),
        "the original capture software's own header cards must still pass through: {wcs}"
    );
}

/// The "no hint anywhere" early return in `astap_cmd` (no `-ra`/`-spd`, and
/// this fixture's header carries no `OBJCTRA`/`OBJCTDEC`) writes the
/// `"Not enough stars."` failure `.ini`, end to end through the compiled
/// binary with a REAL, resolvable index -- not the short-circuit-at-
/// `resolve_index_path` shape `a_failed_solve_in_astap_mode_still_writes_
/// the_failure_ini` above covers via `-d /nonexistent`. Fix round 1 of the
/// M3 Task 10 review (Minor): this path had been verified only by hand.
#[test]
fn a_hintless_invocation_with_a_real_index_reports_not_enough_stars() {
    let dir = scratch_dir("no-hint-real-index");
    let (ra0, dec0) = (150.0, -10.0);
    let (frame, db_dir) = setup(dir.path(), ra0, dec0);

    // No -ra/-spd, and build_fixture's own header carries no
    // OBJCTRA/OBJCTDEC -- there is genuinely no hint anywhere.
    let r = run_in(dir.path(), &["-f", &frame, "-r", "3", "-d", db_dir.to_str().unwrap()]);
    assert_eq!(r.code, Some(1), "stderr: {}", r.stderr);

    let ini = std::fs::read_to_string(dir.path().join("field.ini"))
        .unwrap_or_else(|e| panic!("reading field.ini: {e}"));
    assert!(ini.starts_with('\n') && ini.contains("PLTSOLVD=F"), "ini was: {ini}");
    assert!(ini.contains("ERROR=Not enough stars."), "ini was: {ini}");
}

/// The `Outcome::Failed` branch in `astap_cmd` (a real hint, a real index,
/// but the frame itself has nothing to detect) also reports
/// `"Not enough stars."`, end to end through the compiled binary with a
/// REAL, resolvable index and a REAL pointing hint -- proving the failure
/// comes from the solve attempt itself, not from index resolution or the
/// hint check short-circuiting first. Fix round 1 of the M3 Task 10 review
/// (Minor).
#[test]
fn a_starless_frame_with_a_real_hint_and_index_reports_not_enough_stars() {
    let dir = scratch_dir("outcome-failed-real-index");
    let (ra0, dec0) = (150.0, -10.0);
    // A real index still needs a real catalogue to build from -- reuse
    // `setup`'s star-bearing fixture for that, but solve a SEPARATE, zero-star
    // frame at the same pointing so detection itself is what fails.
    let (_frame_with_stars, db_dir) = setup(dir.path(), ra0, dec0);
    let (blank_bytes, _unused_csv) = build_fixture(ra0, dec0, 0);
    std::fs::write(dir.path().join("blank.fits"), &blank_bytes).unwrap();
    let ra_hours = ra0 / 15.0;
    let spd_deg = dec0 + 90.0;

    let r = run_in(
        dir.path(),
        &[
            "-f",
            "blank.fits",
            "-ra",
            &ra_hours.to_string(),
            "-spd",
            &spd_deg.to_string(),
            "-r",
            "3",
            "-d",
            db_dir.to_str().unwrap(),
        ],
    );
    assert_eq!(r.code, Some(1), "stderr: {}", r.stderr);

    let ini = std::fs::read_to_string(dir.path().join("blank.ini"))
        .unwrap_or_else(|e| panic!("reading blank.ini: {e}"));
    assert!(ini.starts_with('\n') && ini.contains("PLTSOLVD=F"), "ini was: {ini}");
    assert!(ini.contains("ERROR=Not enough stars."), "ini was: {ini}");
}

/// `-update` is default OFF: a successful solve with no `-update` must not
/// touch the input frame's bytes at all.
#[test]
fn no_update_flag_means_the_frame_is_never_touched() {
    let dir = scratch_dir("no-update");
    let (ra0, dec0) = (150.0, -10.0);
    let (frame, db_dir) = setup(dir.path(), ra0, dec0);
    let before = std::fs::read(dir.path().join(&frame)).unwrap();
    let ra_hours = ra0 / 15.0;
    let spd_deg = dec0 + 90.0;

    let r = run_in(
        dir.path(),
        &[
            "-f",
            &frame,
            "-ra",
            &ra_hours.to_string(),
            "-spd",
            &spd_deg.to_string(),
            "-r",
            "3",
            "-d",
            db_dir.to_str().unwrap(),
        ],
    );
    assert_eq!(r.code, Some(0), "stderr: {}", r.stderr);

    let after = std::fs::read(dir.path().join(&frame)).unwrap();
    assert_eq!(before, after, "-update was not passed; the frame must be byte-identical");
}

/// `-update` rewrites the header in place on a successful solve, and a
/// SECOND solve of the same, already-updated frame (a re-solve, as a
/// pipeline that revisits frames would do) must not duplicate psolve's own
/// `COMMENT` card -- the Task 9 non-idempotency this task fixes
/// (`fits_update.rs`'s `merge_wcs_cards`).
#[test]
fn update_rewrites_the_header_and_a_resolve_does_not_duplicate_the_comment_card() {
    let dir = scratch_dir("update-idempotent");
    let (ra0, dec0) = (150.0, -10.0);
    let (frame, db_dir) = setup(dir.path(), ra0, dec0);
    let ra_hours = ra0 / 15.0;
    let spd_deg = dec0 + 90.0;
    let args = [
        "-f",
        frame.as_str(),
        "-ra",
        &ra_hours.to_string(),
        "-spd",
        &spd_deg.to_string(),
        "-r",
        "3",
        "-d",
        db_dir.to_str().unwrap(),
        "-update",
    ];

    let r1 = run_in(dir.path(), &args);
    assert_eq!(r1.code, Some(0), "first solve: stderr: {}", r1.stderr);
    let after_first = std::fs::read(dir.path().join(&frame)).unwrap();
    assert!(
        String::from_utf8_lossy(&after_first).contains("Astrometric solution by psolve"),
        "the header must carry psolve's solve-marker COMMENT after -update"
    );

    let r2 = run_in(dir.path(), &args);
    assert_eq!(r2.code, Some(0), "second solve (re-solve): stderr: {}", r2.stderr);
    let after_second = std::fs::read(dir.path().join(&frame)).unwrap();

    let count_comment_cards = |bytes: &[u8]| -> usize {
        bytes
            .chunks(80)
            .filter(|c| {
                c.starts_with(b"COMMENT ")
                    && String::from_utf8_lossy(c).trim_end() == "COMMENT Astrometric solution by psolve"
            })
            .count()
    };
    assert_eq!(
        count_comment_cards(&after_second),
        1,
        "a re-solve must replace psolve's own COMMENT card, not append a second copy"
    );
    // The original capture software's own COMMENT card must have survived
    // both resolves untouched.
    assert!(
        String::from_utf8_lossy(&after_second).contains("COMMENT captured by N.I.N.A."),
        "the original capture software's COMMENT card must not be touched by the idempotency fix"
    );
}

/// End-to-end proof of final-review C2, through the compiled binary: after
/// `-update` has written a solution into the frame's own header, a SECOND
/// run's `.wcs` sidecar -- whose pass-through is now that already-solved
/// header -- must carry exactly one card per WCS key, and it must be
/// psolve's value, not the header's stale one.
///
/// Every frame in this deployment is in this state (ASTAP wrote its own
/// solution into all 9495 of them), so this is the ordinary case. The
/// duplicate-key form put ASTAP's stale card first, and first-match-wins in
/// cfitsio, astropy and this project's own `FitsHeader::get`.
#[test]
fn a_resolve_of_an_updated_frame_writes_one_wcs_card_per_key() {
    let dir = scratch_dir("wcs-no-duplicate-keys");
    let (ra0, dec0) = (150.0, -10.0);
    let (frame, db_dir) = setup(dir.path(), ra0, dec0);
    let ra_hours = ra0 / 15.0;
    let spd_deg = dec0 + 90.0;
    let args = [
        "-f", frame.as_str(),
        "-ra", &ra_hours.to_string(),
        "-spd", &spd_deg.to_string(),
        "-r", "3",
        "-d", db_dir.to_str().unwrap(),
        "-update",
    ];

    let r1 = run_in(dir.path(), &args);
    assert_eq!(r1.code, Some(0), "first solve: stderr: {}", r1.stderr);
    // The frame's own header is now solved -- that is what makes the second
    // run's pass-through interesting.
    assert!(
        String::from_utf8_lossy(&std::fs::read(dir.path().join(&frame)).unwrap()).contains("CRPIX1"),
        "the first -update must have put a WCS into the frame's header"
    );

    let r2 = run_in(dir.path(), &args);
    assert_eq!(r2.code, Some(0), "second solve: stderr: {}", r2.stderr);

    let wcs = std::fs::read_to_string(dir.path().join("field.wcs")).unwrap();
    for key in [
        "CTYPE1", "CTYPE2", "CUNIT1", "CRPIX1", "CRPIX2", "CRVAL1", "CRVAL2", "CDELT1", "CDELT2",
        "CROTA1", "CROTA2", "CD1_1", "CD1_2", "CD2_1", "CD2_2", "PLTSOLVD",
    ] {
        let hits: Vec<&str> = wcs.lines().filter(|l| l.starts_with(key)).collect();
        assert_eq!(hits.len(), 1, "{key}: want exactly one card in the .wcs, got {hits:?}");
    }

    // And it is this solve's own value: the `.ini` and the `.wcs` describe
    // the same solution, to the `.wcs` format's own 12 mantissa digits.
    let ini = std::fs::read_to_string(dir.path().join("field.ini")).unwrap();
    let ini_crval1: f64 = ini
        .lines()
        .find_map(|l| l.strip_prefix("CRVAL1="))
        .and_then(|v| v.trim().parse().ok())
        .expect("CRVAL1 in the .ini");
    let wcs_crval1: f64 = wcs
        .lines()
        .find(|l| l.starts_with("CRVAL1"))
        .and_then(|l| l.split('=').nth(1))
        .and_then(|v| v.split('/').next())
        .and_then(|v| v.trim().parse().ok())
        .expect("CRVAL1 in the .wcs");
    assert!(
        (ini_crval1 - wcs_crval1).abs() < 1e-9,
        "the .wcs must report this solve's CRVAL1 ({ini_crval1}), got {wcs_crval1}"
    );
}

/// A read-only refusal during `-update` is ASTAP mode's own failure code,
/// `1` -- NOT native mode's `3` (the exit-code collision Task 10 resolves;
/// see `main.rs`'s `astap_cmd` doc comment). The solve itself still succeeds
/// and the sidecars are still written -- only the in-place header write is
/// refused.
///
/// The marker protects the *frame's* directory only, and `-o` sends the
/// sidecars to an unprotected one. That separation is now load-bearing: the
/// sidecar writes go through the same read-only guard (final-review C1), so
/// a marker over both would refuse the sidecars first and this test would
/// never reach the `-update` refusal it exists to pin. The
/// marker-over-everything shape is covered by
/// `a_marker_refuses_the_success_path_sidecar_writes` below.
#[test]
fn a_readonly_refusal_during_update_is_astap_exit_code_1_not_native_3() {
    let dir = scratch_dir("readonly-update");
    let (ra0, dec0) = (150.0, -10.0);
    let (frame, db_dir) = setup(dir.path(), ra0, dec0);

    // The frame moves into its own protected subdirectory; the sidecars go
    // to a sibling that carries no marker.
    let protected = dir.path().join("protected");
    std::fs::create_dir_all(&protected).unwrap();
    std::fs::rename(dir.path().join(&frame), protected.join(&frame)).unwrap();
    std::fs::write(protected.join(".psolve-readonly"), b"").unwrap();
    let out_dir = dir.path().join("out");
    std::fs::create_dir_all(&out_dir).unwrap();

    let before = std::fs::read(protected.join(&frame)).unwrap();
    let ra_hours = ra0 / 15.0;
    let spd_deg = dec0 + 90.0;

    let r = run_in(
        dir.path(),
        &[
            "-f",
            protected.join(&frame).to_str().unwrap(),
            "-o",
            out_dir.join("field").to_str().unwrap(),
            "-ra",
            &ra_hours.to_string(),
            "-spd",
            &spd_deg.to_string(),
            "-r",
            "3",
            "-d",
            db_dir.to_str().unwrap(),
            "-update",
        ],
    );
    assert_eq!(r.code, Some(1), "a readonly refusal in ASTAP mode must be exit 1, not native mode's 3");
    assert!(r.stderr.contains("read-only") || r.stderr.contains("readonly"), "stderr was: {}", r.stderr);

    let after = std::fs::read(protected.join(&frame)).unwrap();
    assert_eq!(before, after, "a refused -update must leave the frame byte-identical");

    // The solve itself succeeded and the sidecar says so honestly, even
    // though the in-place header write was refused.
    let ini = std::fs::read_to_string(out_dir.join("field.ini")).unwrap();
    assert!(ini.starts_with("PLTSOLVD=T\n"), "the solve succeeded independently of the refused -update");
}

// ---------------------------------------------------------------------
// The sidecar writes obey the same two read-only switches (final-review
// C1).
//
// Before this, `.ini`/`.wcs` were written with a bare `std::fs::write` that
// consulted neither switch, on BOTH the success and the failure path. With
// `PSOLVE_READONLY=1` set and a marker in the frame's own directory, a
// misconfigured invocation (a bad `-d`, or a `-f` naming a file that does
// not exist) still silently replaced a recorded `PLTSOLVD=T` ASTAP solution
// with a `PLTSOLVD=F` failure file. `~/astroops` holds 46 `.ini` and 13
// `.wcs` files sitting beside their frames -- recorded ASTAP output, the
// ground truth this milestone's byte-exact fixtures were transcribed from,
// none of it reconstructible.
//
// Four tests: {marker, PSOLVE_READONLY} x {success path, failure path}.
// ---------------------------------------------------------------------

/// A marker in the frame's own directory refuses the **success** path's
/// sidecar writes: no `.ini`, no `.wcs`, exit `1` -- even though the solve
/// itself succeeded and no `-update` was asked for.
#[test]
fn a_marker_refuses_the_success_path_sidecar_writes() {
    let dir = scratch_dir("marker-success-sidecars");
    let (ra0, dec0) = (150.0, -10.0);
    let (frame, db_dir) = setup(dir.path(), ra0, dec0);
    std::fs::write(dir.path().join(".psolve-readonly"), b"").unwrap();
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

    assert_eq!(r.code, Some(1), "a refused sidecar write must exit 1: stderr: {}", r.stderr);
    assert!(r.stderr.contains("read-only"), "the refusal must say why: stderr: {}", r.stderr);
    assert!(!dir.path().join("field.ini").exists(), ".ini must not have been written");
    assert!(!dir.path().join("field.wcs").exists(), ".wcs must not have been written");
}

/// The same marker refuses the **failure** path's sidecar write. This is the
/// demonstrated data-loss shape: a bad `-d` refuses before the input is even
/// read, and the failure `.ini` it would write lands on -- and destroys -- a
/// recorded `PLTSOLVD=T` solution sitting at the same path.
#[test]
fn a_marker_refuses_the_failure_path_sidecar_write() {
    let dir = scratch_dir("marker-failure-sidecar");
    std::fs::write(dir.path().join("frame.fits"), b"not a real fits file").unwrap();
    let recorded = "PLTSOLVD=T\nCRPIX1= 1.9205000000000000E+003\n";
    std::fs::write(dir.path().join("frame.ini"), recorded).unwrap();
    std::fs::write(dir.path().join(".psolve-readonly"), b"").unwrap();

    let r = run_in(dir.path(), &["-f", "frame.fits", "-d", "/nonexistent"]);

    assert_eq!(r.code, Some(1));
    assert!(r.stderr.contains("read-only"), "stderr was: {}", r.stderr);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("frame.ini")).unwrap(),
        recorded,
        "the recorded ASTAP solution must survive byte-for-byte"
    );
}

/// `PSOLVE_READONLY` refuses the success path's sidecar writes on exactly
/// the same terms. Set only on the spawned child -- never on the test
/// process, which would race every other test in this binary.
#[test]
fn psolve_readonly_refuses_the_success_path_sidecar_writes() {
    let dir = scratch_dir("env-success-sidecars");
    let (ra0, dec0) = (150.0, -10.0);
    let (frame, db_dir) = setup(dir.path(), ra0, dec0);
    let ra_hours = ra0 / 15.0;
    let spd_deg = dec0 + 90.0;

    let o = Command::new(bin())
        .current_dir(dir.path())
        .env("PSOLVE_READONLY", "1")
        .args([
            "-f", &frame,
            "-ra", &ra_hours.to_string(),
            "-spd", &spd_deg.to_string(),
            "-r", "3",
            "-d", db_dir.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&o.stderr).into_owned();
    assert_eq!(o.status.code(), Some(1), "stderr: {stderr}");
    assert!(stderr.contains("PSOLVE_READONLY"), "stderr was: {stderr}");
    assert!(!dir.path().join("field.ini").exists(), ".ini must not have been written");
    assert!(!dir.path().join("field.wcs").exists(), ".wcs must not have been written");
}

/// And the failure path, again with a recorded solution in the line of fire.
#[test]
fn psolve_readonly_refuses_the_failure_path_sidecar_write() {
    let dir = scratch_dir("env-failure-sidecar");
    std::fs::write(dir.path().join("frame.fits"), b"not a real fits file").unwrap();
    let recorded = "PLTSOLVD=T\nCRPIX1= 1.9205000000000000E+003\n";
    std::fs::write(dir.path().join("frame.ini"), recorded).unwrap();

    let o = Command::new(bin())
        .current_dir(dir.path())
        .env("PSOLVE_READONLY", "1")
        .args(["-f", "frame.fits", "-d", "/nonexistent"])
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&o.stderr).into_owned();
    assert_eq!(o.status.code(), Some(1), "stderr: {stderr}");
    assert!(stderr.contains("PSOLVE_READONLY"), "stderr was: {stderr}");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("frame.ini")).unwrap(),
        recorded,
        "the recorded ASTAP solution must survive byte-for-byte"
    );
}

/// The exact invocation shape the final review demonstrated: `-f` naming a
/// file that does not exist, so the refusal at `resolve_index_path` runs
/// before any input is read -- and the `missing.ini` it would drop beside a
/// nonexistent frame is refused too.
#[test]
fn a_marker_refuses_the_sidecar_for_an_input_that_does_not_exist() {
    let dir = scratch_dir("marker-missing-input");
    std::fs::write(dir.path().join(".psolve-readonly"), b"").unwrap();

    let r = run_in(dir.path(), &["-f", "missing.fits", "-d", "/nonexistent"]);

    assert_eq!(r.code, Some(1));
    assert!(!dir.path().join("missing.ini").exists(), "no sidecar may be written into a protected tree");
}

/// `PSOLVE_READONLY` refuses the same way, exit `1`, set only for the
/// spawned child process -- never the test process itself, which would race
/// every other test in this binary.
#[test]
fn psolve_readonly_env_during_update_is_also_astap_exit_code_1() {
    let dir = scratch_dir("readonly-env-update");
    let (ra0, dec0) = (150.0, -10.0);
    let (frame, db_dir) = setup(dir.path(), ra0, dec0);
    let ra_hours = ra0 / 15.0;
    let spd_deg = dec0 + 90.0;

    let o = Command::new(bin())
        .current_dir(dir.path())
        .env("PSOLVE_READONLY", "1")
        .args([
            "-f",
            &frame,
            "-ra",
            &ra_hours.to_string(),
            "-spd",
            &spd_deg.to_string(),
            "-r",
            "3",
            "-d",
            db_dir.to_str().unwrap(),
            "-update",
        ])
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(1));
}
