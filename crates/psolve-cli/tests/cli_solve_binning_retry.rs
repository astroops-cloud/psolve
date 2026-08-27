//! Defect 3: a rig that writes the ALREADY-BINNED pixel size into `XPIXSZ`
//! (this project's sv405 rig does, on 399 of 400 sampled frames) makes
//! `pixel_scale_arcsec`'s `XPIXSZ * XBINNING` double-apply the binning
//! factor, deriving a plate scale `XBINNING`x too coarse -- 0 of 400 real
//! affected frames solved at that scale, ~67% solve at the correct one. This
//! builds a synthetic XBINNING=2 frame with exactly that convention (a
//! physical 2.9um pixel written as `XPIXSZ=5.8`, the on-chip-binned value)
//! and checks the CLI retries once at `scale / binning` with no `--scale`
//! flag needed, reporting which scale actually solved it.

use psolve_core::fit::Wcs;
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
    let d = std::env::temp_dir().join(format!("psolve-binning-retry-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
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
// FOCALLEN/physical-pixel pair chosen so the physical (unbinned) scale is
// 2.4614 "/px -- the same reference value psolve-core's own fits.rs tests
// use -- so at XBINNING=2 the TRUE per-file-pixel scale (what the frame's
// own NAXIS1/2 grid, already on-chip-binned, actually needs) is exactly
// double that.
const FOCALLEN_MM: f64 = 243.0;
const PHYSICAL_PIX_UM: f64 = 2.9;
const XBINNING: u32 = 2;
const TRUE_SCALE_ARCSEC: f64 = 206.265 * PHYSICAL_PIX_UM * XBINNING as f64 / FOCALLEN_MM; // ~4.9228
// What this rig's driver actually writes: the pixel size ALREADY multiplied
// by binning. `pixel_scale_arcsec` does not know that and multiplies by
// XBINNING again, so the header-derived scale comes out XBINNING x too
// coarse (~9.8456 "/px here).
const WRITTEN_XPIXSZ_UM: f64 = PHYSICAL_PIX_UM * XBINNING as f64;

fn truth_wcs(ra0: f64, dec0: f64) -> Wcs {
    let s = TRUE_SCALE_ARCSEC / 3600.0;
    Wcs { crval: [ra0, dec0], crpix: [NX as f64 / 2.0, NY as f64 / 2.0], cd: [[-s, 0.0], [0.0, s]] }
}

/// A FITS frame with `n` Gaussian stars at the TRUE (binned) scale, an
/// `XPIXSZ`/`XBINNING`/`FOCALLEN` triple in the "already-binned" convention,
/// and a CSV catalogue built from the same stars' true sky positions.
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
        let peak = 8000.0 - (k % 20) as f64 * 150.0;
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

fn setup(d: &std::path::Path, ra0: f64, dec0: f64) -> (std::path::PathBuf, std::path::PathBuf) {
    let (fits_bytes, csv) = build_fixture(ra0, dec0, 60);
    let f = d.join("field.fits");
    std::fs::write(&f, &fits_bytes).unwrap();

    let input = d.join("cat");
    std::fs::create_dir_all(&input).unwrap();
    std::fs::write(input.join("a.csv"), csv).unwrap();
    let idx = d.join("t.psidx");
    let o = Command::new(bin())
        .args(["index", "build", "--input"])
        .arg(&input)
        .arg("--out")
        .arg(&idx)
        .args(["--max-mag", "20", "--nside", "64"])
        .output()
        .unwrap();
    assert!(o.status.success(), "index build failed: {}", String::from_utf8_lossy(&o.stderr));
    (f, idx)
}

#[test]
fn a_pre_multiplied_xpixsz_solves_via_the_binning_retry_with_no_scale_flag() {
    let d = tmpdir("basic");
    let (ra0, dec0) = (150.0, -10.0);
    let (f, idx) = setup(&d, ra0, dec0);

    // Deliberately no --scale: the CLI must derive the (wrong) header scale,
    // fail, retry at scale/binning, and solve -- with no flag telling it to.
    let o = Command::new(bin())
        .args(["solve"])
        .arg(&f)
        .arg("--index")
        .arg(&idx)
        .args(["--hint", &format!("{ra0},{dec0}")])
        .args(["--saturation", "60000"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&o.stdout).to_string();
    let stderr = String::from_utf8_lossy(&o.stderr).to_string();
    assert_eq!(o.status.code(), Some(0), "stderr: {stderr}\nstdout: {stdout}");

    assert!(stdout.contains("\"solved\":true"), "stdout: {stdout}");
    assert!(
        stdout.contains("\"scale_source\":\"header/binning-retry\""),
        "the JSON must say the retried scale is what solved it: {stdout}"
    );
    assert!(
        stderr.contains("retrying once"),
        "the retry must be logged to stderr: {stderr}"
    );
}

#[test]
fn an_explicit_scale_is_honoured_and_never_retried() {
    // The same header, but the caller asserts the (correct) scale directly.
    // No retry must happen even though the header alone would derive the
    // wrong one, and scale_source must say so.
    let d = tmpdir("explicit");
    let (ra0, dec0) = (150.0, -10.0);
    let (f, idx) = setup(&d, ra0, dec0);

    let o = Command::new(bin())
        .args(["solve"])
        .arg(&f)
        .arg("--index")
        .arg(&idx)
        .args(["--hint", &format!("{ra0},{dec0}")])
        .args(["--scale", &TRUE_SCALE_ARCSEC.to_string()])
        .args(["--saturation", "60000"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&o.stdout).to_string();
    let stderr = String::from_utf8_lossy(&o.stderr).to_string();
    assert_eq!(o.status.code(), Some(0), "stderr: {stderr}\nstdout: {stdout}");
    assert!(stdout.contains("\"solved\":true"), "stdout: {stdout}");
    assert!(
        stdout.contains("\"scale_source\":\"explicit\""),
        "an explicit --scale must be reported as such: {stdout}"
    );
    assert!(
        !stderr.contains("retrying once"),
        "an explicit --scale must never trigger the binning retry: {stderr}"
    );
}
