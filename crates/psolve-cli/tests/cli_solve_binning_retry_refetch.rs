//! The catalogue half of the XPIXSZ/XBINNING retry.
//!
//! `cli_solve_binning_retry.rs` already proves the retry corrects the SCALE.
//! It cannot catch this defect: its index holds only the 60 in-field stars,
//! so the disc being twice too wide costs nothing -- there is no other star
//! to fetch instead. Real frames are not like that. This fixture puts bright
//! decoy stars OUTSIDE the true field but INSIDE the inflated disc, so a
//! disc derived from the doubled scale spends the catalogue budget on stars
//! that cannot appear in the frame, exactly as measured on 791 real bin-2
//! sv405 frames (all 0% before this fix).

use psolve_core::fit::Wcs;
use std::process::Command;

fn bin() -> std::path::PathBuf {
    let mut p = std::env::current_exe().unwrap();
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("psolve")
}

fn tmpdir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir()
        .join(format!("psolve-retry-refetch-{tag}-{}", std::process::id()));
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
const FOCALLEN_MM: f64 = 243.0;
const PHYSICAL_PIX_UM: f64 = 2.9;
const XBINNING: u32 = 2;
/// The TRUE per-file-pixel scale: the frame's grid is already on-chip binned.
const TRUE_SCALE_ARCSEC: f64 = 206.265 * PHYSICAL_PIX_UM * XBINNING as f64 / FOCALLEN_MM;
/// What this rig's driver writes: the pixel size ALREADY multiplied by
/// binning. `pixel_scale_arcsec` multiplies by XBINNING again, so the
/// header-derived scale is 2x too coarse and the derived radius 2x too wide.
const WRITTEN_XPIXSZ_UM: f64 = PHYSICAL_PIX_UM * XBINNING as f64;

fn truth_wcs(ra0: f64, dec0: f64) -> Wcs {
    let s = TRUE_SCALE_ARCSEC / 3600.0;
    Wcs { crval: [ra0, dec0], crpix: [NX as f64 / 2.0, NY as f64 / 2.0], cd: [[-s, 0.0], [0.0, s]] }
}

/// True field half-diagonal in degrees, from the TRUE scale.
fn true_half_diagonal_deg() -> f64 {
    let w = NX as f64 * TRUE_SCALE_ARCSEC / 3600.0;
    let h = NY as f64 * TRUE_SCALE_ARCSEC / 3600.0;
    (w * w + h * h).sqrt() / 2.0
}

/// The frame, plus a catalogue CSV holding both the in-field stars and a
/// ring of BRIGHTER decoys outside the true field.
///
/// Decoys sit between 1.4x and 1.9x the true half-diagonal: outside the
/// correct disc (half-diagonal x 1.10) and inside the doubled one, so they
/// are fetched only when the radius is wrong. They are brighter than every
/// real star, so `brightest_in_disc` prefers them and the budget is spent
/// before a single in-field star is reached.
///
/// `cat_field_seed` decorrelates the catalogue's in-field stars from the
/// image's. `0` is the honest fixture: the same positions in both, so the
/// frame solves once the disc is right. A different value gives the
/// catalogue a DIFFERENT random field of the same size and brightness in
/// the same patch of sky -- so both discs are populated exactly as before
/// and the retry still refetches, but no quad can ever match and the solve
/// fails at both scales. That is the failure-path fixture: it exercises the
/// refetch without exercising a successful solve.
fn build_fixture(
    ra0: f64,
    dec0: f64,
    n_field: usize,
    n_decoy: usize,
    cat_field_seed: usize,
) -> (Vec<u8>, String) {
    let w = truth_wcs(ra0, dec0);
    let margin = 40.0;
    let mut pix = Vec::new();
    let mut cat_pix = Vec::new();
    for i in 0..n_field {
        let (u, v) = scatter(i);
        pix.push((margin + u * (NX as f64 - 2.0 * margin), margin + v * (NY as f64 - 2.0 * margin)));
        let (cu, cv) = scatter(cat_field_seed + i);
        cat_pix
            .push((margin + cu * (NX as f64 - 2.0 * margin), margin + cv * (NY as f64 - 2.0 * margin)));
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
                img[y as usize * NX + x as usize] +=
                    peak * (-(ex * ex + ey * ey) / (2.0 * sigma * sigma)).exp();
            }
        }
    }
    for (k, &(cx, cy)) in cat_pix.iter().enumerate() {
        let (ra, dec) = w.pix_to_radec(cx, cy);
        // Real stars: mag 12.0-12.9.
        csv.push_str(&format!("{ra:.8},{dec:.8},0,0,{:.2}\n", 12.0 + (k % 10) as f64 * 0.1));
    }

    // Decoys: brighter (mag 6-8), outside the true field, inside the doubled disc.
    let hd = true_half_diagonal_deg();
    for i in 0..n_decoy {
        let (u, v) = scatter(100_000 + i);
        let theta = u * std::f64::consts::TAU;
        let rho = hd * (1.4 + 0.5 * v);
        let dec = dec0 + rho * theta.sin();
        let ra = ra0 + rho * theta.cos() / dec0.to_radians().cos().abs().max(1e-6);
        csv.push_str(&format!("{ra:.8},{dec:.8},0,0,{:.2}\n", 6.0 + (i % 20) as f64 * 0.1));
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
        out.extend_from_slice(&((clamped as i32 - 32768) as i16).to_be_bytes());
    }
    while !out.len().is_multiple_of(2880) {
        out.push(0);
    }
    (out, csv)
}

fn setup(
    d: &std::path::Path,
    ra0: f64,
    dec0: f64,
    cat_field_seed: usize,
) -> (std::path::PathBuf, std::path::PathBuf) {
    let (fits_bytes, csv) = build_fixture(ra0, dec0, 60, 4000, cat_field_seed);
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

/// The refetch's own stderr line, split into `(n_refetched, corrected_deg,
/// n_first, first_deg)`. Parsing it is the point: those four numbers are the
/// only externally visible proof the second query was issued at the
/// corrected radius, and Task 4's corpus run reads the same line.
fn parse_refetch_line(stderr: &str) -> (f64, f64, f64, f64) {
    let line = stderr
        .lines()
        .find(|l| l.contains("refetched the catalogue"))
        .unwrap_or_else(|| panic!("no refetch happened -- stderr:\n{stderr}"));
    // Take the segment after the FIRST " -- " so the frame path (which may
    // itself hold digits) cannot contribute numbers.
    let body = line.split(" -- ").nth(1).unwrap_or_else(|| panic!("malformed: {line}"));
    let nums: Vec<f64> = body
        .split(|c: char| !(c.is_ascii_digit() || c == '.'))
        .filter(|t| !t.is_empty())
        .filter_map(|t| t.parse::<f64>().ok())
        .collect();
    assert_eq!(nums.len(), 4, "expected 4 numbers in {body:?}");
    (nums[0], nums[1], nums[2], nums[3])
}

/// `"catalog":{"concentration":X,...}` from the emitted JSON -- the field
/// that must describe the disc the reported outcome actually came from.
/// `None` for a JSON `null`.
fn parse_catalog_concentration(stdout: &str) -> Option<f64> {
    let key = "\"catalog\":{\"concentration\":";
    let at = stdout.find(key).unwrap_or_else(|| panic!("no catalog block: {stdout}"));
    let rest = &stdout[at + key.len()..];
    let end = rest.find(',').unwrap();
    let v = &rest[..end];
    if v == "null" {
        return None;
    }
    Some(v.parse::<f64>().unwrap_or_else(|_| panic!("bad concentration {v:?}")))
}

#[test]
fn a_binned_frame_solves_when_decoys_would_swamp_an_overwide_disc() {
    let d = tmpdir("native");
    let (ra0, dec0) = (150.0, -10.0);
    let (f, idx) = setup(&d, ra0, dec0, 0);

    // No --scale and no --radius: the CLI derives the wrong (doubled) scale,
    // derives a doubled radius from it, fetches a disc full of decoys, fails,
    // and must then refetch at the corrected radius rather than retrying the
    // corrected scale against the same swamped catalogue.
    let o = Command::new(bin())
        .args(["solve"])
        .arg(&f)
        .arg("--index")
        .arg(&idx)
        .args(["--hint", &format!("{ra0},{dec0}")])
        .args(["--cat-limit", "300"])
        .args(["--saturation", "60000"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&o.stdout).to_string();
    let stderr = String::from_utf8_lossy(&o.stderr).to_string();

    assert_eq!(o.status.code(), Some(0), "stderr: {stderr}\nstdout: {stdout}");
    assert!(stdout.contains("\"solved\":true"), "stdout: {stdout}");
    assert!(
        stdout.contains("\"scale_source\":\"header/binning-retry\""),
        "the retried scale must be what solved it: {stdout}"
    );

    // The refetch itself, on stderr: a second query at half the radius,
    // returning far fewer stars than the swamped first disc. Without this
    // the test passes on a retry that merely corrected the scale.
    let (n_second, corrected, n_first, first) = parse_refetch_line(&stderr);
    assert!(
        (corrected * 2.0 - first).abs() < 1e-3,
        "the corrected radius must be first/XBINNING: {corrected} vs {first}"
    );
    assert!(
        n_second < n_first,
        "the corrected disc must be smaller than the swamped one: {n_second} vs {n_first}"
    );

    // And the JSON must describe THAT disc, not the discarded first fetch.
    // The two are far apart (first ~1.64, refetch ~0.63), so a loose bound
    // is both sufficient and robust: this fires the moment the reported
    // concentration reverts to the first fetch's.
    let conc = parse_catalog_concentration(&stdout)
        .unwrap_or_else(|| panic!("catalog concentration must be reported: {stdout}"));
    assert!(
        conc < 1.0,
        "catalog.concentration must be the REFETCHED disc's (~0.63), not the swamped \
first fetch's (~1.64); got {conc}\nstdout: {stdout}"
    );
    let _ = std::fs::remove_dir_all(&d);
}

/// Fix round 1, item 2: the retry can refetch and still fail, and the
/// failure JSON must report the disc the failure actually came from.
/// Whoever is reading `catalog.concentration` on a frame that did NOT solve
/// is the person most misled by the first fetch's number -- they are trying
/// to work out whether the disc was the problem.
#[test]
fn a_failed_retry_reports_the_refetched_disc_not_the_first_one() {
    let d = tmpdir("native-fail");
    let (ra0, dec0) = (150.0, -10.0);
    // Catalogue field stars drawn from a different seed than the image's:
    // both discs are populated exactly as in the solving case, so the
    // refetch still fires, but no quad can match at either scale.
    let (f, idx) = setup(&d, ra0, dec0, 50_000);

    let o = Command::new(bin())
        .args(["solve"])
        .arg(&f)
        .arg("--index")
        .arg(&idx)
        .args(["--hint", &format!("{ra0},{dec0}")])
        .args(["--cat-limit", "300"])
        .args(["--saturation", "60000"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&o.stdout).to_string();
    let stderr = String::from_utf8_lossy(&o.stderr).to_string();

    // Not solved is a normal outcome: exit 1, `solved:false`.
    assert_eq!(o.status.code(), Some(1), "stderr: {stderr}\nstdout: {stdout}");
    assert!(stdout.contains("\"solved\":false"), "stdout: {stdout}");

    let (n_second, corrected, n_first, first) = parse_refetch_line(&stderr);
    assert!(
        (corrected * 2.0 - first).abs() < 1e-3,
        "the corrected radius must be first/XBINNING: {corrected} vs {first}"
    );
    assert!(n_second < n_first, "{n_second} vs {n_first}");

    let conc = parse_catalog_concentration(&stdout)
        .unwrap_or_else(|| panic!("catalog concentration must be reported: {stdout}"));
    assert!(
        conc < 1.0,
        "on the FAILURE path too, catalog.concentration must be the REFETCHED disc's \
(~0.63), not the first fetch's (~1.64); got {conc}\nstdout: {stdout}"
    );
    let _ = std::fs::remove_dir_all(&d);
}

/// The same fixture through the ASTAP-compatible surface (`psolve -f ...`).
///
/// This is the test that keeps the fix from being invisible in production: a
/// fix of exactly this shape reached `cmd_solve.rs` alone on 2026-08-14 and
/// left this dispatch stale, and `ingest.identify.astap_solve` builds exactly
/// this argv -- so a refetch wired only into native mode does not reach the
/// 791 real bin-2 frames at all. Mirrors `blind_solve.rs`'s pattern of
/// running one frame through both entry points and requiring agreement.
#[test]
fn the_astap_entry_point_refetches_too() {
    let d = tmpdir("astap");
    let (ra0, dec0) = (150.0, -10.0);
    let (f, idx) = setup(&d, ra0, dec0, 0);

    // ASTAP mode resolves its index from the directory holding the .psidx.
    let db_dir = idx.parent().unwrap().to_path_buf();
    // -ra is HOURS, -spd is dec + 90. -r 180 is AstroOps' own blind form;
    // the header narrows it, so the cap does not bind here.
    let o = Command::new(bin())
        .arg("-f")
        .arg(&f)
        .args(["-ra", &format!("{}", ra0 / 15.0)])
        .args(["-spd", &format!("{}", dec0 + 90.0)])
        .args(["-r", "180"])
        .arg("-d")
        .arg(&db_dir)
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&o.stderr).to_string();
    assert_eq!(o.status.code(), Some(0), "ASTAP mode must solve it too. stderr: {stderr}");

    let ini = f.with_extension("ini");
    let text = std::fs::read_to_string(&ini).unwrap_or_else(|e| panic!("reading {ini:?}: {e}"));
    assert!(text.contains("PLTSOLVD=T"), "the .ini must record a solve: {text}");

    // The refetch itself, parsed from the same stderr line native mode
    // emits: the proof is the four numbers, not merely that a solve
    // happened -- the frame could in principle solve on the scale retry
    // alone against some other catalogue.
    let (n_second, corrected, n_first, first) = parse_refetch_line(&stderr);
    assert!(
        (corrected * 2.0 - first).abs() < 1e-3,
        "the corrected radius must be first/XBINNING: {corrected} vs {first}"
    );
    assert!(
        n_second < n_first,
        "the corrected disc must be smaller than the swamped one: {n_second} vs {n_first}"
    );
    let _ = std::fs::remove_dir_all(&d);
}

/// `-r` is a CEILING in ASTAP mode, not a caller-chosen disc -- so unlike
/// native `--radius` it must not suppress the refetch, and it must still
/// bound the refetched disc.
///
/// `-r 0.8` is chosen to bind the FIRST fetch (header-derived 1.2034 deg)
/// but not the corrected one (0.6017 deg). That separates the two candidate
/// implementations by a number: dividing the already-capped radius would
/// refetch at 0.8/2 = 0.4000, while dividing the uncapped header radius and
/// re-applying the ceiling gives 0.6017. `CatalogRefetch::radius_header_deg`
/// carries the uncapped value for exactly this reason, and this test is what
/// stops a later "simplification" from collapsing the two fields into one.
#[test]
fn the_astap_r_flag_stays_a_ceiling_on_the_refetched_disc() {
    let d = tmpdir("astap-cap");
    let (ra0, dec0) = (150.0, -10.0);
    let (f, idx) = setup(&d, ra0, dec0, 0);
    let db_dir = idx.parent().unwrap().to_path_buf();

    let o = Command::new(bin())
        .arg("-f")
        .arg(&f)
        .args(["-ra", &format!("{}", ra0 / 15.0)])
        .args(["-spd", &format!("{}", dec0 + 90.0)])
        .args(["-r", "0.8"])
        .arg("-d")
        .arg(&db_dir)
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&o.stderr).to_string();
    assert_eq!(o.status.code(), Some(0), "ASTAP mode must solve it too. stderr: {stderr}");

    let (_, corrected, _, first) = parse_refetch_line(&stderr);
    // The cap bound the first fetch, so `first` is `-r` verbatim, not the
    // header's 1.2034.
    assert!((first - 0.8).abs() < 1e-3, "-r must bound the FIRST fetch: {first}");
    // ... and the corrected radius came from the UNCAPPED header value
    // halved (~0.6017), not from halving the capped 0.8 (which would be
    // 0.4000). Both are below the ceiling, so only the arithmetic tells
    // them apart.
    assert!(
        corrected > 0.5 && corrected < 0.7,
        "the corrected radius must be the uncapped header radius / XBINNING (~0.6017), \
not the capped one / XBINNING (0.4000); got {corrected}"
    );
    assert!(corrected <= 0.8 + 1e-9, "the refetched disc must still respect -r: {corrected}");
    let _ = std::fs::remove_dir_all(&d);
}

/// And when `-r` is narrower than the CORRECTED radius, the ceiling still
/// wins: the retry must not widen the disc back out to what the header
/// implies. `-r 0.4` binds both fetches (0.4 either way), so the two discs
/// are identical and no second query is issued at all -- the refetch's own
/// "same disc" short-circuit. A caller asking for a deliberately narrow
/// search gets exactly that, retry or no retry.
#[test]
fn an_r_narrower_than_the_corrected_radius_is_still_the_ceiling() {
    let d = tmpdir("astap-narrow");
    let (ra0, dec0) = (150.0, -10.0);
    let (f, idx) = setup(&d, ra0, dec0, 0);
    let db_dir = idx.parent().unwrap().to_path_buf();

    let o = Command::new(bin())
        .arg("-f")
        .arg(&f)
        .args(["-ra", &format!("{}", ra0 / 15.0)])
        .args(["-spd", &format!("{}", dec0 + 90.0)])
        .args(["-r", "0.4"])
        .arg("-d")
        .arg(&db_dir)
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&o.stderr).to_string();

    // The scale retry still runs -- only the catalogue half is short-
    // circuited, and only because the corrected disc equals the first one.
    assert!(
        stderr.contains("scale / XBINNING"),
        "the scale retry must still fire: {stderr}"
    );
    assert!(
        !stderr.contains("refetched the catalogue"),
        "no second query should be issued when the cap binds both fetches to the same \
disc: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&d);
}
