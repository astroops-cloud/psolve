//! End-to-end coverage of `psolve quad-index build`: a small synthetic star
//! field, a sweep restricted to a tight RA/dec box (`--min-ra`/`--max-ra`/
//! `--min-dec`/`--max-dec`) so the test runs in milliseconds rather than
//! sweeping the whole sky, and the milestone's own non-negotiables --
//! determinism regardless of `--jobs`, and a header whose
//! `star_index_fingerprint` matches the source `.psidx`.

use psolve_index::builder::Builder;
use psolve_index::quad_format::QuadHeader;
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
    let d = std::env::temp_dir().join(format!("psolve-cli-quadindex-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A dense, deterministic synthetic star field in a 4x4 deg patch centred
/// on (101, 20) -- big enough that a sweep restricted to the inner 2x2 deg
/// box (`bounds()` below) finds real stars at every band from 0.25 to 2
/// deg, without needing to touch anything near the poles or the whole sky.
fn write_star_index(dir: &std::path::Path) -> std::path::PathBuf {
    let mut b = Builder::new(64, 20.0, 2016.0, "quadindex-test").unwrap();
    let mut s: u64 = 0xA11CE;
    let mut nxt = || {
        s = s.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = s;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        ((z ^ (z >> 31)) >> 11) as f64 / (1u64 << 53) as f64
    };
    for i in 0..600u32 {
        let ra = 101.0 + (nxt() - 0.5) * 4.0;
        let dec = 20.0 + (nxt() - 0.5) * 4.0;
        let mag = 8.0 + (i as f32) * 0.015;
        b.push(ra, dec, mag, 0.0, 0.0);
    }
    let mut buf = std::io::Cursor::new(Vec::new());
    b.finish(&mut buf).unwrap();
    let p = dir.join("stars.psidx");
    std::fs::write(&p, buf.into_inner()).unwrap();
    p
}

/// The tight box every test below restricts the sweep to.
const BOUNDS: [&str; 8] =
    ["--min-ra", "100", "--max-ra", "102", "--min-dec", "19", "--max-dec", "21"];

#[test]
fn builds_a_psqidx_with_quads_a_header_that_round_trips_and_a_verified_digest() {
    let d = tmpdir("basic");
    let star_index = write_star_index(&d);
    let out = d.join("out.psqidx");

    let o = Command::new(bin())
        .args(["quad-index", "build", "--star-index"])
        .arg(&star_index)
        .arg("--out")
        .arg(&out)
        .args(BOUNDS)
        .args(["--name", "test-quads"])
        .output()
        .unwrap();
    assert!(o.status.success(), "stderr: {}", String::from_utf8_lossy(&o.stderr));
    assert!(out.exists());

    let stdout = String::from_utf8_lossy(&o.stdout);
    assert!(stdout.trim_start().starts_with('{'), "stdout must be JSON, got: {stdout}");
    assert!(stdout.contains("\"n_quads\""));
    assert!(stdout.contains("\"per_band\""));
    assert!(stdout.contains("\"clamped\":0"), "a genuine quad_code output should not clamp");

    let bytes = std::fs::read(&out).unwrap();
    let header = QuadHeader::from_bytes(&bytes).unwrap();
    assert_eq!(header.name_str(), "test-quads");
    assert_eq!(header.n_bands, 6);
    assert_eq!(header.band_scales_deg(), vec![0.25f32, 0.5, 1.0, 2.0, 4.0, 8.0]);
    assert!(header.n_quads > 0, "the dense synthetic field must produce at least one quad");

    let base = header.records_offset as usize;
    let region_len = header.n_quads as usize * psolve_index::quad_format::QUAD_RECORD_BYTES;
    let region = &bytes[base..base + region_len];
    assert_eq!(
        psolve_index::sha256::sha256(region),
        header.records_sha256,
        "records_sha256 must verify against the actual record region"
    );
}

#[test]
fn the_header_fingerprint_matches_the_source_psidx() {
    let d = tmpdir("fingerprint");
    let star_index = write_star_index(&d);
    let out = d.join("out.psqidx");

    let o = Command::new(bin())
        .args(["quad-index", "build", "--star-index"])
        .arg(&star_index)
        .arg("--out")
        .arg(&out)
        .args(BOUNDS)
        .output()
        .unwrap();
    assert!(o.status.success(), "stderr: {}", String::from_utf8_lossy(&o.stderr));

    let source = psolve_index::reader::Index::open(&star_index).unwrap();
    let bytes = std::fs::read(&out).unwrap();
    let header = QuadHeader::from_bytes(&bytes).unwrap();
    assert_eq!(&header.star_index_fingerprint[..], &source.header().records_sha256[..8]);
}

#[test]
fn per_band_counts_sum_to_n_quads_and_every_quad_lands_in_a_valid_band() {
    let d = tmpdir("perband");
    let star_index = write_star_index(&d);
    let out = d.join("out.psqidx");

    Command::new(bin())
        .args(["quad-index", "build", "--star-index"])
        .arg(&star_index)
        .arg("--out")
        .arg(&out)
        .args(BOUNDS)
        .output()
        .unwrap();

    let bytes = std::fs::read(&out).unwrap();
    let header = QuadHeader::from_bytes(&bytes).unwrap();

    // Re-derive the band table directly rather than trusting stdout, so this
    // test also proves the file's OWN band table (not just the JSON report)
    // is self-consistent.
    let tab_off = QuadHeader::band_table_offset() as usize;
    let mut tab = Vec::with_capacity(header.n_bands as usize + 1);
    for i in 0..=header.n_bands as usize {
        let s = tab_off + i * 8;
        tab.push(u64::from_le_bytes(bytes[s..s + 8].try_into().unwrap()));
    }
    assert_eq!(tab[0], 0);
    assert_eq!(*tab.last().unwrap(), header.n_quads);
    for w in tab.windows(2) {
        assert!(w[1] >= w[0], "band table must be non-decreasing");
    }
}

#[test]
fn identical_inputs_produce_byte_identical_output_regardless_of_jobs() {
    let d = tmpdir("determinism");
    let star_index = write_star_index(&d);

    let run = |tag: &str, jobs: &str| -> Vec<u8> {
        let out = d.join(format!("{tag}.psqidx"));
        let o = Command::new(bin())
            .args(["quad-index", "build", "--star-index"])
            .arg(&star_index)
            .arg("--out")
            .arg(&out)
            .args(BOUNDS)
            .args(["--jobs", jobs])
            .output()
            .unwrap();
        assert!(o.status.success(), "stderr: {}", String::from_utf8_lossy(&o.stderr));
        std::fs::read(&out).unwrap()
    };

    let single_a = run("single-a", "1");
    let single_b = run("single-b", "1");
    assert_eq!(single_a, single_b, "two single-threaded builds must be byte-identical");

    let multi = run("multi", "4");
    assert_eq!(single_a, multi, "thread count must not change the output");

    let many = run("many", "8");
    assert_eq!(single_a, many, "thread count must not change the output");
}

#[test]
fn missing_star_index_flag_exits_2() {
    let d = tmpdir("missing-star-index");
    let o = Command::new(bin())
        .args(["quad-index", "build", "--out"])
        .arg(d.join("x.psqidx"))
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2));
}

#[test]
fn missing_out_flag_exits_2() {
    let d = tmpdir("missing-out");
    let star_index = write_star_index(&d);
    let o = Command::new(bin())
        .args(["quad-index", "build", "--star-index"])
        .arg(&star_index)
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2));
}

#[test]
fn a_nonexistent_star_index_exits_3() {
    let d = tmpdir("bad-star-index");
    let o = Command::new(bin())
        .args(["quad-index", "build", "--star-index", "/nonexistent/nowhere.psidx", "--out"])
        .arg(d.join("x.psqidx"))
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(3));
}

#[test]
fn jobs_zero_exits_2() {
    let d = tmpdir("jobs-zero");
    let star_index = write_star_index(&d);
    let o = Command::new(bin())
        .args(["quad-index", "build", "--star-index"])
        .arg(&star_index)
        .arg("--out")
        .arg(d.join("x.psqidx"))
        .args(BOUNDS)
        .args(["--jobs", "0"])
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2));
}

#[test]
fn a_flipped_declination_range_exits_2() {
    let d = tmpdir("flipped-dec");
    let star_index = write_star_index(&d);
    let o = Command::new(bin())
        .args(["quad-index", "build", "--star-index"])
        .arg(&star_index)
        .arg("--out")
        .arg(d.join("x.psqidx"))
        .args(["--min-dec", "40", "--max-dec", "-40"])
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2));
}

#[test]
fn a_quote_in_name_exits_2() {
    let d = tmpdir("bad-name");
    let star_index = write_star_index(&d);
    let o = Command::new(bin())
        .args(["quad-index", "build", "--star-index"])
        .arg(&star_index)
        .arg("--out")
        .arg(d.join("x.psqidx"))
        .args(BOUNDS)
        .args(["--name", "foo\"bar"])
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2));
}

#[test]
fn a_tiny_bounding_box_far_from_any_star_writes_a_zero_quad_but_valid_file() {
    let d = tmpdir("empty-region");
    let star_index = write_star_index(&d);
    let out = d.join("empty.psqidx");
    let o = Command::new(bin())
        .args(["quad-index", "build", "--star-index"])
        .arg(&star_index)
        .arg("--out")
        .arg(&out)
        .args(["--min-ra", "250", "--max-ra", "251", "--min-dec", "-10", "--max-dec", "-9"])
        .output()
        .unwrap();
    assert!(o.status.success(), "stderr: {}", String::from_utf8_lossy(&o.stderr));
    let bytes = std::fs::read(&out).unwrap();
    let header = QuadHeader::from_bytes(&bytes).unwrap();
    assert_eq!(header.n_quads, 0, "a region with no catalogue stars must yield zero quads, not fail");
}

// -- quad-index info --

/// Builds a real `.psqidx` (same fixture as the `build` tests above) and
/// returns both paths, ready for `quad-index info --star-index <star_index>
/// <out>`.
fn built_quad_index(tag: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let d = tmpdir(tag);
    let star_index = write_star_index(&d);
    let out = d.join("out.psqidx");
    let o = Command::new(bin())
        .args(["quad-index", "build", "--star-index"])
        .arg(&star_index)
        .arg("--out")
        .arg(&out)
        .args(BOUNDS)
        .args(["--name", "info-test"])
        .output()
        .unwrap();
    assert!(o.status.success(), "stderr: {}", String::from_utf8_lossy(&o.stderr));
    (star_index, out)
}

#[test]
fn info_reports_the_header_and_per_band_counts_as_json() {
    let (star_index, out) = built_quad_index("info-basic");
    let o = Command::new(bin())
        .args(["quad-index", "info", "--star-index"])
        .arg(&star_index)
        .arg(&out)
        .output()
        .unwrap();
    assert!(o.status.success(), "stderr: {}", String::from_utf8_lossy(&o.stderr));
    let s = String::from_utf8_lossy(&o.stdout);
    assert!(s.trim_start().starts_with('{'), "stdout must be JSON: {s}");
    for key in [
        "\"name\"",
        "\"nside\"",
        "\"n_quads\"",
        "\"n_bands\"",
        "\"epoch\"",
        "\"mag_limit\"",
        "\"band_scales_deg\"",
        "\"per_band\"",
        "\"sha256\"",
        "\"star_index_fingerprint\"",
    ] {
        assert!(s.contains(key), "missing {key} in {s}");
    }
    assert!(s.contains("info-test"));
    assert!(s.contains("\"n_bands\":6"));

    // Cross-check per_band against the file's own band table, the same way
    // `per_band_counts_sum_to_n_quads_and_every_quad_lands_in_a_valid_band`
    // does for `build`'s own report.
    let bytes = std::fs::read(&out).unwrap();
    let header = QuadHeader::from_bytes(&bytes).unwrap();
    let tab_off = QuadHeader::band_table_offset() as usize;
    let mut tab = Vec::with_capacity(header.n_bands as usize + 1);
    for i in 0..=header.n_bands as usize {
        let s = tab_off + i * 8;
        tab.push(u64::from_le_bytes(bytes[s..s + 8].try_into().unwrap()));
    }
    for (b, w) in tab.windows(2).enumerate() {
        let count = w[1] - w[0];
        assert!(
            s.contains(&format!("{{\"band\":{b},\"count\":{count}}}")),
            "per_band must report band {b}'s true count {count} from the file's own band table, got: {s}"
        );
    }
}

#[test]
fn info_digest_ok_is_null_not_false_when_verify_was_not_requested() {
    let (star_index, out) = built_quad_index("info-no-verify");
    let o = Command::new(bin())
        .args(["quad-index", "info", "--star-index"])
        .arg(&star_index)
        .arg(&out)
        .output()
        .unwrap();
    assert!(o.status.success(), "stderr: {}", String::from_utf8_lossy(&o.stderr));
    let s = String::from_utf8_lossy(&o.stdout);
    assert!(s.contains("\"digest_ok\":null"), "got: {s}");
    assert!(s.contains("\"verified\":false"));
}

#[test]
fn info_verify_passes_on_a_good_index() {
    let (star_index, out) = built_quad_index("info-verify-ok");
    let o = Command::new(bin())
        .args(["quad-index", "info", "--star-index"])
        .arg(&star_index)
        .arg(&out)
        .arg("--verify")
        .output()
        .unwrap();
    assert!(o.status.success(), "stderr: {}", String::from_utf8_lossy(&o.stderr));
    let s = String::from_utf8_lossy(&o.stdout);
    assert!(s.contains("\"digest_ok\":true"));
    assert!(s.contains("\"verified\":true"));
}

#[test]
fn info_verify_detects_corruption_of_a_copy_and_exits_3() {
    let (star_index, out) = built_quad_index("info-verify-bad");
    // Corrupt a COPY on disk, never any original fixture: flip the last byte
    // of the file, which lands in the record region for a build dense
    // enough to reach band 5 (this fixture's own `n_quads > 0` assertion in
    // the `build` tests already establishes that).
    let mut bytes = std::fs::read(&out).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 0xff;
    std::fs::write(&out, &bytes).unwrap();

    let o = Command::new(bin())
        .args(["quad-index", "info", "--star-index"])
        .arg(&star_index)
        .arg(&out)
        .arg("--verify")
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(3), "a corrupted record region must fail --verify");
    assert!(o.stdout.is_empty(), "a failed verify must write nothing to stdout");
    assert!(!o.stderr.is_empty(), "the reason belongs on stderr");
}

#[test]
fn info_rejects_a_fingerprint_mismatch_against_the_wrong_star_index() {
    let (_star_index, out) = built_quad_index("info-fingerprint");
    // A different, unrelated star index -- same shape, different content, so
    // its records_sha256 (and thus fingerprint) differs from the one this
    // .psqidx was actually built against.
    let d = tmpdir("info-fingerprint-other");
    let mut b = psolve_index::builder::Builder::new(64, 20.0, 2016.0, "other").unwrap();
    b.push(200.0, -40.0, 10.0, 0.0, 0.0);
    b.push(200.01, -40.01, 11.0, 0.0, 0.0);
    let mut buf = std::io::Cursor::new(Vec::new());
    b.finish(&mut buf).unwrap();
    let other_star_index = d.join("other.psidx");
    std::fs::write(&other_star_index, buf.into_inner()).unwrap();

    let o = Command::new(bin())
        .args(["quad-index", "info", "--star-index"])
        .arg(&other_star_index)
        .arg(&out)
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(3), "a mispaired star index must be refused, not silently opened");
    assert!(o.stdout.is_empty());
}

#[test]
fn info_on_a_psidx_file_handed_as_the_psqidx_exits_3() {
    // `.psidx` and `.psqidx` intentionally do not share a magic
    // (`quad_format.rs`'s module doc) -- confirm the CLI path honours that,
    // not just the reader's own unit tests.
    let (star_index, _out) = built_quad_index("info-wrong-format");
    let o = Command::new(bin())
        .args(["quad-index", "info", "--star-index"])
        .arg(&star_index)
        .arg(&star_index) // the .psidx itself, standing in for the .psqidx path
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(3));
}

#[test]
fn info_without_star_index_flag_exits_2() {
    let (_star_index, out) = built_quad_index("info-missing-star-index");
    let o = Command::new(bin()).args(["quad-index", "info"]).arg(&out).output().unwrap();
    assert_eq!(o.status.code(), Some(2));
}

#[test]
fn info_without_a_path_exits_2() {
    let (star_index, _out) = built_quad_index("info-missing-path");
    let o = Command::new(bin())
        .args(["quad-index", "info", "--star-index"])
        .arg(&star_index)
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2));
}

#[test]
fn info_on_a_missing_file_exits_3() {
    let (star_index, _out) = built_quad_index("info-missing-file");
    let o = Command::new(bin())
        .args(["quad-index", "info", "--star-index"])
        .arg(&star_index)
        .arg("/nonexistent/none.psqidx")
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(3));
}
