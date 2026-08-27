//! A hinted solve whose hint is grossly wrong must fall back to blind.
//!
//! Measured 2026-08-27 on a real pointing-model build: every one of 26 frames
//! carried a mount pointing 18.8-19.5 degrees from the truth, against psolve's
//! 1.66 degree search radius. psolve failed all of them; given the right
//! position it solved them at 0.36" rms, and given NO position it solved them
//! blind. The capability was already there and simply unreachable, because a
//! hint that exists is trusted even when it is wrong.
//!
//! This is deliberately synthetic rather than rig-dependent: the failure is
//! about the hint, not about the frames, so it needs no telescope data and
//! runs on CI like any other test.

use std::process::Command;

fn bin() -> std::path::PathBuf {
    let mut p = std::env::current_exe().unwrap();
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join(format!("psolve{}", std::env::consts::EXE_SUFFIX))
}

fn tmpdir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("psolve-blind-fallback-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

const NX: usize = 640;
const NY: usize = 480;
const SCALE_ARCSEC: f64 = 2.4614;
const TRUTH_RA: f64 = 101.0;
const TRUTH_DEC: f64 = 20.0;

/// Deterministic hashed scatter -- the generator `cli_solve_success.rs` uses,
/// reproduced here because each integration test binary stands on its own.
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

/// A frame of Gaussian stars plus the catalogue those stars were generated
/// from, both derived from the same TAN WCS so the truth is exact.
fn build_fixture(n: usize) -> (Vec<u8>, String) {
    let s = SCALE_ARCSEC / 3600.0;
    let crpix = [NX as f64 / 2.0, NY as f64 / 2.0];
    let cd = [[-s, 0.0], [0.0, s]];
    let margin = 40.0;

    let mut img = vec![1000f64; NX * NY];
    for (i, v) in img.iter_mut().enumerate() {
        *v += ((i * 2654435761usize) % 97) as f64 * 0.4;
    }
    let mut csv = String::from("ra,dec,pmra,pmdec,phot_g_mean_mag\n");
    let sigma = 1.8f64;
    for k in 0..n {
        let (u, v) = scatter(k);
        let cx = margin + u * (NX as f64 - 2.0 * margin);
        let cy = margin + v * (NY as f64 - 2.0 * margin);
        let peak = 8000.0 - (k % 20) as f64 * 150.0;
        for dy in -5i64..=5 {
            for dx in -5i64..=5 {
                let x = cx.round() as i64 + dx;
                let y = cy.round() as i64 + dy;
                if x < 0 || y < 0 || x >= NX as i64 || y >= NY as i64 {
                    continue;
                }
                let ex = x as f64 - cx;
                let ey = y as f64 - cy;
                img[y as usize * NX + x as usize] +=
                    peak * (-(ex * ex + ey * ey) / (2.0 * sigma * sigma)).exp();
            }
        }
        // pixel -> tangent plane -> sky, the inverse of what the solver fits
        let xi = cd[0][0] * (cx - crpix[0]) + cd[0][1] * (cy - crpix[1]);
        let eta = cd[1][0] * (cx - crpix[0]) + cd[1][1] * (cy - crpix[1]);
        let (xi, eta) = (xi.to_radians(), eta.to_radians());
        let (ra0, dec0) = (TRUTH_RA.to_radians(), TRUTH_DEC.to_radians());
        let denom = dec0.cos() - eta * dec0.sin();
        let ra = ra0 + (xi / denom).atan();
        let dec = ((dec0.sin() + eta * dec0.cos()) / (xi * xi + denom * denom).sqrt()).atan();
        csv.push_str(&format!(
            "{:.8},{:.8},0,0,{:.2}\n",
            ra.to_degrees(),
            dec.to_degrees(),
            12.0 + (k % 10) as f64 * 0.1
        ));
    }

    let cards = [
        "SIMPLE  =                    T".to_string(),
        "BITPIX  =                   16".to_string(),
        "NAXIS   =                    2".to_string(),
        format!("NAXIS1  = {NX:>20}"),
        format!("NAXIS2  = {NY:>20}"),
        "BZERO   =                32768".to_string(),
        // A blind search picks which .psqidx scale bands to query from the
        // frame's plate scale, so a fixture with no optics in its header
        // cannot exercise the band selection at all. 243 mm at 2.9 um is
        // 2.461"/px, matching SCALE_ARCSEC above.
        "FOCALLEN=                243.0".to_string(),
        "XPIXSZ  =                  2.9".to_string(),
        "YPIXSZ  =                  2.9".to_string(),
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
        out.extend_from_slice(&((clamped as i32 - 32768) as i16).to_be_bytes());
    }
    while !out.len().is_multiple_of(2880) {
        out.push(0);
    }
    (out, csv)
}

/// Frame, star index and paired quad index, all in `d`.
fn setup(d: &std::path::Path) -> (std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
    // 60, matching the density `cli_solve_success.rs` uses and the example the
    // quad builder was proven against. Denser fields produce quads far smaller
    // than the smallest indexed band (0.25 deg), which the blind lookup cannot
    // match -- a property of the fixture, not of the fallback.
    let (fits, csv) = build_fixture(60);
    let frame = d.join("field.fits");
    std::fs::write(&frame, &fits).unwrap();

    let input = d.join("cat");
    std::fs::create_dir_all(&input).unwrap();
    std::fs::write(input.join("a.csv"), csv).unwrap();

    let psidx = d.join("t.psidx");
    let o = Command::new(bin())
        .args(["index", "build", "--input"])
        .arg(&input)
        .arg("--out")
        .arg(&psidx)
        .args(["--max-mag", "20", "--nside", "64"])
        .output()
        .unwrap();
    assert!(o.status.success(), "index build: {}", String::from_utf8_lossy(&o.stderr));

    let psqidx = d.join("t.psqidx");
    let o = Command::new(bin())
        .args(["quad-index", "build", "--star-index"])
        .arg(&psidx)
        .arg("--out")
        .arg(&psqidx)
        .args(["--min-ra", "100.0", "--max-ra", "102.0"])
        .args(["--min-dec", "19.0", "--max-dec", "21.0"])
        .output()
        .unwrap();
    assert!(o.status.success(), "quad-index build: {}", String::from_utf8_lossy(&o.stderr));

    (frame, psidx, psqidx)
}



/// The control: the same frame with a CORRECT hint must solve, so a failure
/// below cannot be blamed on the fixture.
#[test]
fn the_fixture_solves_when_the_hint_is_right() {
    let d = tmpdir("control");
    let (frame, psidx, _psqidx) = setup(&d);
    let o = Command::new(bin())
        .args(["solve"])
        .arg(&frame)
        .arg("--index")
        .arg(&psidx)
        .args(["--hint", &format!("{TRUTH_RA},{TRUTH_DEC}")])
        .output()
        .unwrap();
    let s = String::from_utf8_lossy(&o.stdout);
    assert!(s.contains("\"solved\":true"), "control must solve: {s}");
}

/// The point of the exercise: a hint 20 degrees from the truth is worse than
/// no hint at all, because psolve trusts it and searches the wrong sky. With a
/// quad index available the blind path must be REACHED.
///
/// This asserts the decision, not the search. Whether a blind search then
/// succeeds depends on the quad index having usable content for the field, and
/// that is `solve_blind`'s own contract, covered by `blind_solve.rs` against a
/// real index. What is new here -- and what could regress silently -- is that a
/// failed hinted solve now consults it at all.
///
/// The tell is the failure detail. Without the fallback the hinted path
/// reports its own dead end ("no catalogue stars supplied": the disc 20 degrees
/// away is empty). With it, the reported failure comes from the blind search
/// and names image quads and candidate hypotheses -- vocabulary the hinted path
/// does not have.
#[test]
fn a_grossly_wrong_hint_reaches_the_blind_path_when_a_quad_index_is_available() {
    let d = tmpdir("wrong-hint");
    let (frame, psidx, psqidx) = setup(&d);

    let o = Command::new(bin())
        .args(["solve"])
        .arg(&frame)
        .arg("--index")
        .arg(&psidx)
        .arg("--quad-index")
        .arg(&psqidx)
        .args(["--hint", &format!("{},{}", TRUTH_RA + 20.0, TRUTH_DEC)])
        .output()
        .unwrap();
    let s = String::from_utf8_lossy(&o.stdout);
    let err = String::from_utf8_lossy(&o.stderr);

    assert!(
        err.contains("falling back to a blind search"),
        "the fallback must announce itself on stderr, got: {err}"
    );
    assert!(
        s.contains("image quads") && s.contains("hypotheses"),
        "the reported failure must come from the blind search, not the hinted \
         dead end, got: {s}"
    );
    assert!(
        !s.contains("no catalogue stars supplied"),
        "the hinted path's own dead end must not be what is reported once the \
         blind path has run: {s}"
    );
}

/// The fallback must not fire when there is nothing to fall back to: without a
/// quad index the honest answer is still a refusal, not a slower refusal.
#[test]
fn a_wrong_hint_without_a_quad_index_still_refuses() {
    let d = tmpdir("no-quads");
    let (frame, psidx, _psqidx) = setup(&d);
    let o = Command::new(bin())
        .args(["solve"])
        .arg(&frame)
        .arg("--index")
        .arg(&psidx)
        .args(["--hint", &format!("{},{}", TRUTH_RA + 20.0, TRUTH_DEC)])
        .output()
        .unwrap();
    let s = String::from_utf8_lossy(&o.stdout);
    assert!(s.contains("\"solved\":false"), "must still refuse without quads: {s}");
}
