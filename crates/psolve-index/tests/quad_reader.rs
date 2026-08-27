//! Coverage for `psolve_index::quad_reader::QuadIndex`, mirroring
//! `tests/reader.rs`'s style. Two things get dedicated tests because they
//! were carried forward from earlier reviews as unenforced until this
//! module existed: the `star_index_fingerprint` pairing check, and
//! `records_offset`/`n_quads` bounds validated against the real file
//! length.

use psolve_index::builder::Builder;
use psolve_index::quad_builder::QuadIndexBuilder;
use psolve_index::quad_format::QuadHeader;
use psolve_index::quad_reader::QuadIndex;
use psolve_index::reader::Index;
use psolve_index::IndexError;
use std::io::Write;

fn tmpdir() -> std::path::PathBuf {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let d = std::env::temp_dir().join(format!("psolve-quadreader-test-{}-{}", std::process::id(), n));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn write_star_index(dir: &std::path::Path, name: &str, stars: &[(f64, f64, f32)]) -> std::path::PathBuf {
    let mut b = Builder::new(64, 20.0, 2016.0, name).unwrap();
    for &(ra, dec, mag) in stars {
        b.push(ra, dec, mag, 0.0, 0.0);
    }
    let mut buf = std::io::Cursor::new(Vec::new());
    b.finish(&mut buf).unwrap();
    let p = dir.join(format!("{name}.psidx"));
    let mut f = std::fs::File::create(&p).unwrap();
    f.write_all(&buf.into_inner()).unwrap();
    p
}

const BANDS: [f32; 3] = [0.25, 0.5, 1.0];

/// Builds a small, real `.psqidx` paired against `star_index` (its
/// fingerprint set from `star_index`'s own digest, as `cmd_quadindex::build`
/// does), with `per_band` quads pushed into each configured band using
/// distinct, deterministic (but not meaningful -- only shape matters here)
/// codes and star references.
fn write_quad_index(
    dir: &std::path::Path,
    tag: &str,
    star_index: &Index,
    per_band: &[usize],
) -> std::path::PathBuf {
    let mut fingerprint = [0u8; 8];
    fingerprint.copy_from_slice(&star_index.header().records_sha256[..8]);
    let mut b = QuadIndexBuilder::new(64, 2016.0, 20.0, tag, fingerprint, &BANDS).unwrap();
    for (band, &count) in per_band.iter().enumerate() {
        for i in 0..count {
            let f = (band * 100 + i) as f64;
            let code = [(f * 0.01) % 1.0, (f * 0.02) % 1.0, (f * 0.03) % 1.0, (f * 0.04) % 1.0];
            let idx = [i as u32, i as u32 + 1, i as u32 + 2, i as u32 + 3];
            b.push(band, code, idx).unwrap();
        }
    }
    let mut buf = std::io::Cursor::new(Vec::new());
    b.finish(&mut buf).unwrap();
    let p = dir.join(format!("{tag}.psqidx"));
    std::fs::write(&p, buf.into_inner()).unwrap();
    p
}

fn some_stars() -> Vec<(f64, f64, f32)> {
    (0..10).map(|i| (10.0 + i as f64 * 0.01, 20.0 + i as f64 * 0.01, 10.0 + i as f32)).collect()
}

/// `QuadIndex` deliberately does not implement `Debug` (it wraps an `Mmap`),
/// so `Result::unwrap_err` -- which requires `T: Debug` -- can't be used
/// directly on `QuadIndex::open`'s return value. This is the same
/// panic-with-a-useful-message shape without that bound.
fn expect_err(r: Result<QuadIndex, IndexError>) -> IndexError {
    match r {
        Ok(_) => panic!("expected an error, got Ok"),
        Err(e) => e,
    }
}

// -- basic open / header / band counts --

#[test]
fn opens_and_reports_the_header_and_per_band_counts() {
    let d = tmpdir();
    let star_path = write_star_index(&d, "stars", &some_stars());
    let star_index = Index::open(&star_path).unwrap();
    let quad_path = write_quad_index(&d, "quads", &star_index, &[3, 0, 5]);

    let qidx = QuadIndex::open(&quad_path, &star_index).unwrap();
    assert_eq!(qidx.header().n_bands, 3);
    assert_eq!(qidx.header().n_quads, 8);
    assert_eq!(qidx.header().band_scales_deg(), vec![0.25f32, 0.5, 1.0]);
    assert_eq!(qidx.header().name_str(), "quads");

    assert_eq!(qidx.band_len(0), 3);
    assert_eq!(qidx.band_len(1), 0);
    assert_eq!(qidx.band_len(2), 5);
    // Out of range is "no quads", not a panic.
    assert_eq!(qidx.band_len(3), 0);
    assert_eq!(qidx.band_len(1000), 0);
}

#[test]
fn quad_lookup_returns_the_pushed_records() {
    let d = tmpdir();
    let star_path = write_star_index(&d, "stars", &some_stars());
    let star_index = Index::open(&star_path).unwrap();
    let quad_path = write_quad_index(&d, "quads", &star_index, &[2, 0, 0]);

    let qidx = QuadIndex::open(&quad_path, &star_index).unwrap();
    let q0 = qidx.quad(0, 0).unwrap();
    assert_eq!(q0.star_idx, [0, 1, 2, 3]);
    let q1 = qidx.quad(0, 1).unwrap();
    assert_eq!(q1.star_idx, [1, 2, 3, 4]);
    assert!(qidx.quad(0, 2).is_none(), "out of range within a band is None, not a panic");
    assert!(qidx.quad(1, 0).is_none(), "an empty band has no quads");
}

// -- digest verification --

#[test]
fn digest_verifies_on_an_unmodified_file() {
    let d = tmpdir();
    let star_path = write_star_index(&d, "stars", &some_stars());
    let star_index = Index::open(&star_path).unwrap();
    let quad_path = write_quad_index(&d, "quads", &star_index, &[4, 2, 1]);

    let qidx = QuadIndex::open(&quad_path, &star_index).unwrap();
    qidx.verify_digest().unwrap();
}

#[test]
fn verify_digest_detects_a_corrupted_record_region() {
    let d = tmpdir();
    let star_path = write_star_index(&d, "stars", &some_stars());
    let star_index = Index::open(&star_path).unwrap();
    let quad_path = write_quad_index(&d, "quads", &star_index, &[4, 2, 1]);

    // Corrupt a COPY, never the original: flip the last byte, which lands in
    // the record region for a non-empty last band.
    let mut bytes = std::fs::read(&quad_path).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 0xff;
    std::fs::write(&quad_path, &bytes).unwrap();

    let qidx = QuadIndex::open(&quad_path, &star_index).unwrap();
    assert!(
        matches!(qidx.verify_digest(), Err(IndexError::ChecksumMismatch)),
        "a flipped byte in the record region must be caught by verify_digest"
    );
}

// -- fingerprint enforcement (carried-forward review item 1) --

#[test]
fn rejects_a_fingerprint_mismatch_against_a_different_star_index() {
    let d = tmpdir();
    let star_a = write_star_index(&d, "star-a", &some_stars());
    let star_b = write_star_index(
        &d,
        "star-b",
        &[(50.0, -10.0, 9.0), (50.01, -10.01, 9.5), (50.02, -10.02, 10.0)],
    );
    let index_a = Index::open(&star_a).unwrap();
    let index_b = Index::open(&star_b).unwrap();
    assert_ne!(
        index_a.header().records_sha256,
        index_b.header().records_sha256,
        "fixture must actually produce two different digests to be a meaningful test"
    );

    let quad_path = write_quad_index(&d, "quads", &index_a, &[2, 0, 0]);

    // Opened against the star index it was built with: succeeds.
    assert!(QuadIndex::open(&quad_path, &index_a).is_ok());

    // Opened against a DIFFERENT star index: must be rejected, not silently
    // resolve star_idx against the wrong catalogue.
    let err = expect_err(QuadIndex::open(&quad_path, &index_b));
    assert!(
        matches!(err, IndexError::FingerprintMismatch { .. }),
        "expected FingerprintMismatch, got {err:?}"
    );
}

// -- .psidx handed to the quad reader (magic check) --

#[test]
fn rejects_a_real_psidx_file_rather_than_misparsing_it() {
    let d = tmpdir();
    let star_path = write_star_index(&d, "stars", &some_stars());
    let star_index = Index::open(&star_path).unwrap();

    // Hand the reader the .psidx itself, not a paired .psqidx.
    let err = expect_err(QuadIndex::open(&star_path, &star_index));
    assert!(
        matches!(err, IndexError::BadMagic),
        "a .psidx file must be rejected by BadMagic, not misparsed as a .psqidx, got {err:?}"
    );
}

// -- offset/length validation against the real file (carried-forward review item 2) --

#[test]
fn rejects_a_truncated_file() {
    let d = tmpdir();
    let star_path = write_star_index(&d, "stars", &some_stars());
    let star_index = Index::open(&star_path).unwrap();
    let quad_path = write_quad_index(&d, "quads", &star_index, &[4, 2, 1]);

    let bytes = std::fs::read(&quad_path).unwrap();
    // Chop off the last quarter of the record region -- short enough to
    // still contain a valid header and band table, so this exercises the
    // record-region length check specifically, not merely a too-short
    // header.
    let truncated = &bytes[..bytes.len() - bytes.len() / 4];
    std::fs::write(&quad_path, truncated).unwrap();

    let err = expect_err(QuadIndex::open(&quad_path, &star_index));
    assert!(matches!(err, IndexError::Truncated { .. }), "expected Truncated, got {err:?}");
}

#[test]
fn rejects_a_records_offset_that_does_not_match_n_bands() {
    let d = tmpdir();
    let star_path = write_star_index(&d, "stars", &some_stars());
    let star_index = Index::open(&star_path).unwrap();
    let quad_path = write_quad_index(&d, "quads", &star_index, &[4, 2, 1]);

    let mut bytes = std::fs::read(&quad_path).unwrap();
    let good = QuadHeader::from_bytes(&bytes).unwrap();
    // Any value other than `records_offset_for(n_bands)` must be rejected.
    // `records_offset_for` always returns a RECORD_ALIGN-aligned value, so
    // moving one RECORD_ALIGN unit earlier is both "does not match n_bands"
    // AND still a nominally page-aligned value -- proving the check is a
    // real cross-check against n_bands, not merely an alignment-bits test.
    let bad_offset = good.records_offset - psolve_index::quad_format::RECORD_ALIGN;
    bytes[56..64].copy_from_slice(&bad_offset.to_le_bytes());
    std::fs::write(&quad_path, &bytes).unwrap();

    let err = expect_err(QuadIndex::open(&quad_path, &star_index));
    assert!(matches!(err, IndexError::BadRange { what: "records_offset", .. }), "got {err:?}");
}

#[test]
fn rejects_a_misaligned_records_offset() {
    let d = tmpdir();
    let star_path = write_star_index(&d, "stars", &some_stars());
    let star_index = Index::open(&star_path).unwrap();
    let quad_path = write_quad_index(&d, "quads", &star_index, &[4, 2, 1]);

    let mut bytes = std::fs::read(&quad_path).unwrap();
    let good = QuadHeader::from_bytes(&bytes).unwrap();
    let misaligned = good.records_offset + 1;
    bytes[56..64].copy_from_slice(&misaligned.to_le_bytes());
    // The record region no longer fits at its old location; extend the
    // buffer so this test exercises the alignment/consistency check
    // specifically, not incidentally hitting the truncation check instead.
    bytes.extend(std::iter::repeat_n(0u8, 4096));
    std::fs::write(&quad_path, &bytes).unwrap();

    let err = expect_err(QuadIndex::open(&quad_path, &star_index));
    assert!(matches!(err, IndexError::BadRange { what: "records_offset", .. }), "got {err:?}");
}

#[test]
fn rejects_an_n_quads_that_would_overflow_or_run_past_the_file() {
    let d = tmpdir();
    let star_path = write_star_index(&d, "stars", &some_stars());
    let star_index = Index::open(&star_path).unwrap();
    let quad_path = write_quad_index(&d, "quads", &star_index, &[4, 2, 1]);

    let mut bytes = std::fs::read(&quad_path).unwrap();
    // n_quads lives at header bytes 24..32. Setting it to u64::MAX makes
    // `n_quads * QUAD_RECORD_BYTES` overflow u64 outright -- this must be
    // rejected via checked arithmetic, not wrap into a small, wrong, and
    // silently-accepted value, and must not attempt to allocate or read
    // anything sized off that count first.
    bytes[24..32].copy_from_slice(&u64::MAX.to_le_bytes());
    std::fs::write(&quad_path, &bytes).unwrap();

    let err = expect_err(QuadIndex::open(&quad_path, &star_index));
    assert!(matches!(err, IndexError::Truncated { .. }), "expected Truncated, got {err:?}");
}

// -- candidates() (Task 4: the blind-solve code-space lookup) --

const CODE_TOL: f64 = 0.02; // matches match_.rs's MatchParams::default().code_tol

/// Deterministic spread of codes across roughly the proven `quad_code`
/// range -- not meant to model real clustering (that's what the real-index
/// benchmark below is for), just varied enough that a "found"/"not found"
/// test isn't trivially degenerate.
fn gen_codes(n: usize) -> Vec<[f64; 4]> {
    (0..n)
        .map(|i| {
            let f = i as f64;
            [
                (f * 0.037).sin() * 0.5 + 0.5,
                (f * 0.071).cos() * 0.5 + 0.5,
                (f * 0.113).sin() * 0.7 + 0.3,
                (f * 0.211).cos() * 0.4 + 0.6,
            ]
        })
        .collect()
}

fn build_quad_index_with_codes(
    dir: &std::path::Path,
    tag: &str,
    star_index: &Index,
    band: usize,
    codes: &[[f64; 4]],
) -> std::path::PathBuf {
    let mut fingerprint = [0u8; 8];
    fingerprint.copy_from_slice(&star_index.header().records_sha256[..8]);
    let mut b = QuadIndexBuilder::new(64, 2016.0, 20.0, tag, fingerprint, &BANDS).unwrap();
    for (i, &code) in codes.iter().enumerate() {
        let idx = [i as u32, i as u32 + 1, i as u32 + 2, i as u32 + 3];
        b.push(band, code, idx).unwrap();
    }
    let mut buf = std::io::Cursor::new(Vec::new());
    b.finish(&mut buf).unwrap();
    let p = dir.join(format!("{tag}.psqidx"));
    std::fs::write(&p, buf.into_inner()).unwrap();
    p
}

fn code_dist2(a: &[f64; 4], b: &[f64; 4]) -> f64 {
    (0..4).map(|i| (a[i] - b[i]).powi(2)).sum()
}

/// Brute-force scan of a band's actual on-disk (quantized, then decoded)
/// codes -- the ground truth `candidates()` is measured against, not the
/// pre-quantization codes passed to the builder. Reads through `QuadIndex`
/// itself so it exercises exactly the same decode path `candidates()` does.
fn brute_force(qidx: &QuadIndex, band: usize, code: [f64; 4], tol: f64) -> Vec<QuadRecordKey> {
    let t2 = tol * tol;
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(r) = qidx.quad(band, i) {
        if code_dist2(&r.code_f64(), &code) <= t2 {
            out.push(QuadRecordKey(r.star_idx));
        }
        i += 1;
    }
    out
}

/// `QuadRecord` has no `Hash` impl (it wraps `[u16; 4]` floats-as-fixed
/// deliberately, not meant for set membership in production code); this
/// thin wrapper over its `star_idx` (unique per pushed record in every test
/// fixture here) is enough to compare candidate sets as sets without
/// touching production code for a test-only need.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct QuadRecordKey([u32; 4]);

fn as_keys(records: Vec<psolve_index::quad_format::QuadRecord>) -> Vec<QuadRecordKey> {
    records.into_iter().map(|r| QuadRecordKey(r.star_idx)).collect()
}

#[test]
fn a_code_present_in_the_index_is_found() {
    let d = tmpdir();
    let star_path = write_star_index(&d, "stars", &some_stars());
    let star_index = Index::open(&star_path).unwrap();
    let codes = gen_codes(50);
    let quad_path = build_quad_index_with_codes(&d, "quads", &star_index, 0, &codes);
    let qidx = QuadIndex::open(&quad_path, &star_index).unwrap();

    // Query with the record's own DECODED (post-quantization) code, so
    // this is immune to the builder's u16 quantization rounding -- the
    // record is trivially within any positive tolerance of itself.
    let target = qidx.quad(0, 7).unwrap();
    let found: std::collections::HashSet<QuadRecordKey> =
        as_keys(qidx.candidates(target.code_f64(), CODE_TOL, 0).collect()).into_iter().collect();
    assert!(
        found.contains(&QuadRecordKey(target.star_idx)),
        "a code that is exactly a stored record's own code must be found"
    );
}

#[test]
fn a_code_far_from_any_stored_code_is_not_found() {
    let d = tmpdir();
    let star_path = write_star_index(&d, "stars", &some_stars());
    let star_index = Index::open(&star_path).unwrap();
    // Every generated code's components land in roughly [-0.2, 1.3]
    // (sin/cos-derived); this query is far outside that in every
    // dimension, well beyond CODE_TOL.
    let codes = gen_codes(200);
    let quad_path = build_quad_index_with_codes(&d, "quads", &star_index, 0, &codes);
    let qidx = QuadIndex::open(&quad_path, &star_index).unwrap();

    let far = [1.95, -0.95, 1.95, -0.95]; // near CODE_MAX/CODE_MIN, still in-range but isolated
    let found: Vec<_> = qidx.candidates(far, CODE_TOL, 0).collect();
    assert!(found.is_empty(), "expected no candidates far from every stored code, got {found:?}");
}

#[test]
fn candidate_set_is_a_superset_of_a_brute_force_scan() {
    let d = tmpdir();
    let star_path = write_star_index(&d, "stars", &some_stars());
    let star_index = Index::open(&star_path).unwrap();
    // Large enough to be meaningful: exercises many quantile-grid cells,
    // not just one or two.
    let codes = gen_codes(2000);
    let quad_path = build_quad_index_with_codes(&d, "quads", &star_index, 0, &codes);
    let qidx = QuadIndex::open(&quad_path, &star_index).unwrap();

    // A spread of query points: some centred on real stored codes (jittered
    // by a fraction of the tolerance, so they are near but not identical to
    // a stored point), some arbitrary points in-range.
    for q_i in 0..300 {
        let query = if q_i % 3 == 0 {
            let f = q_i as f64;
            [
                (f * 0.019).sin() * 0.6 + 0.4,
                (f * 0.043).cos() * 0.6 + 0.4,
                (f * 0.067).sin() * 0.5 + 0.3,
                (f * 0.091).cos() * 0.5 + 0.5,
            ]
        } else {
            let base = qidx.quad(0, (q_i * 7) % 2000).unwrap().code_f64();
            let j = (q_i as f64 % 5.0 - 2.0) * (CODE_TOL * 0.4);
            [base[0] + j, base[1] - j, base[2] + j * 0.5, base[3]]
        };

        let bf: std::collections::HashSet<QuadRecordKey> =
            brute_force(&qidx, 0, query, CODE_TOL).into_iter().collect();
        let got: std::collections::HashSet<QuadRecordKey> =
            as_keys(qidx.candidates(query, CODE_TOL, 0).collect()).into_iter().collect();
        assert!(
            bf.is_subset(&got),
            "query {q_i} ({query:?}): brute force found {} matches, candidates() found only {} \
             -- candidates() must be a superset (missing: {:?})",
            bf.len(),
            got.len(),
            bf.difference(&got).collect::<Vec<_>>()
        );
    }
}

#[test]
fn candidates_are_exactly_the_brute_force_answer_not_a_looser_superset() {
    // Stronger than the contract requires (superset is the minimum bar),
    // but `filter_exact`'s own doc claims this, so it is worth pinning:
    // the grid should only narrow WHICH records get distance-checked, not
    // relax the check itself.
    let d = tmpdir();
    let star_path = write_star_index(&d, "stars", &some_stars());
    let star_index = Index::open(&star_path).unwrap();
    let codes = gen_codes(500);
    let quad_path = build_quad_index_with_codes(&d, "quads", &star_index, 0, &codes);
    let qidx = QuadIndex::open(&quad_path, &star_index).unwrap();

    for q_i in 0..50 {
        let base = qidx.quad(0, (q_i * 11) % 500).unwrap().code_f64();
        let j = (q_i as f64 % 3.0 - 1.0) * (CODE_TOL * 0.5);
        let query = [base[0] + j, base[1], base[2] - j, base[3] + j * 0.3];

        let bf: std::collections::HashSet<QuadRecordKey> =
            brute_force(&qidx, 0, query, CODE_TOL).into_iter().collect();
        let got: std::collections::HashSet<QuadRecordKey> =
            as_keys(qidx.candidates(query, CODE_TOL, 0).collect()).into_iter().collect();
        assert_eq!(bf, got, "query {q_i}: candidates() must equal the brute-force set exactly");
    }
}

#[test]
fn lookup_is_deterministic() {
    let d = tmpdir();
    let star_path = write_star_index(&d, "stars", &some_stars());
    let star_index = Index::open(&star_path).unwrap();
    let codes = gen_codes(300);
    let quad_path = build_quad_index_with_codes(&d, "quads", &star_index, 0, &codes);
    let qidx = QuadIndex::open(&quad_path, &star_index).unwrap();

    let query = qidx.quad(0, 42).unwrap().code_f64();
    let a = as_keys(qidx.candidates(query, CODE_TOL, 0).collect());
    let b = as_keys(qidx.candidates(query, CODE_TOL, 0).collect());
    let c = as_keys(qidx.candidates(query, CODE_TOL, 0).collect());
    assert_eq!(a, b, "repeated calls with the same input must return the same records");
    assert_eq!(b, c, "repeated calls with the same input must return the same records");
}

#[test]
fn a_band_index_out_of_range_returns_empty_rather_than_panicking() {
    let d = tmpdir();
    let star_path = write_star_index(&d, "stars", &some_stars());
    let star_index = Index::open(&star_path).unwrap();
    let codes = gen_codes(50);
    let quad_path = build_quad_index_with_codes(&d, "quads", &star_index, 0, &codes);
    let qidx = QuadIndex::open(&quad_path, &star_index).unwrap();

    // n_bands is 3 (BANDS = [0.25, 0.5, 1.0]); band 1 and 2 are configured
    // but empty (nothing was pushed to them), band 3 is past n_bands
    // entirely, and a very large band number exercises the same path
    // without relying on any particular internal bound.
    for band in [1usize, 2, 3, 1_000_000] {
        let found: Vec<_> = qidx.candidates([0.0, 0.0, 0.0, 0.0], CODE_TOL, band).collect();
        assert!(found.is_empty(), "band {band} should yield no candidates, got {found:?}");
    }
}

#[test]
fn rejects_an_n_quads_moderately_larger_than_the_file_actually_holds() {
    let d = tmpdir();
    let star_path = write_star_index(&d, "stars", &some_stars());
    let star_index = Index::open(&star_path).unwrap();
    let quad_path = write_quad_index(&d, "quads", &star_index, &[4, 2, 1]);

    let mut bytes = std::fs::read(&quad_path).unwrap();
    let good = QuadHeader::from_bytes(&bytes).unwrap();
    // A plausible-looking but too-large count: no arithmetic overflow, just
    // genuinely more records than the file has bytes for.
    bytes[24..32].copy_from_slice(&(good.n_quads + 1_000_000).to_le_bytes());
    std::fs::write(&quad_path, &bytes).unwrap();

    let err = expect_err(QuadIndex::open(&quad_path, &star_index));
    assert!(matches!(err, IndexError::Truncated { .. }), "expected Truncated, got {err:?}");
}
