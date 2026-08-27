use psolve_index::builder::Builder;
use psolve_index::format::{Header, RECORD_ALIGN};
use psolve_index::healpix::{ang2pix_nest, npix};
use psolve_index::record::{StarRecord, RECORD_BYTES};
use std::io::Cursor;

fn build(nside: u32, stars: &[(f64, f64, f32)]) -> Vec<u8> {
    let mut b = Builder::new(nside, 20.0, 2016.0, "test-index").unwrap();
    for &(ra, dec, mag) in stars {
        b.push(ra, dec, mag, 0.0, 0.0);
    }
    let mut buf = Cursor::new(Vec::new());
    b.finish(&mut buf).unwrap();
    buf.into_inner()
}

fn read_cell_table(bytes: &[u8], nside: u32) -> Vec<u64> {
    let off = Header::cell_table_offset() as usize;
    let n = (npix(nside) + 1) as usize;
    (0..n)
        .map(|i| {
            let s = off + i * 8;
            u64::from_le_bytes(bytes[s..s + 8].try_into().unwrap())
        })
        .collect()
}

fn cell_records(bytes: &[u8], nside: u32, cell: u64) -> Vec<StarRecord> {
    let h = Header::from_bytes(bytes).unwrap();
    let tab = read_cell_table(bytes, nside);
    let (a, b) = (tab[cell as usize], tab[cell as usize + 1]);
    (a..b)
        .map(|i| {
            let s = (h.records_offset + i * RECORD_BYTES as u64) as usize;
            StarRecord::from_bytes(bytes[s..s + RECORD_BYTES].try_into().unwrap())
        })
        .collect()
}

#[test]
fn header_reports_what_was_written() {
    let bytes = build(4, &[(10.0, 10.0, 12.0), (20.0, 20.0, 13.0)]);
    let h = Header::from_bytes(&bytes).unwrap();
    assert_eq!(h.n_records, 2);
    assert_eq!(h.nside, 4);
    assert_eq!(h.epoch, 2016.0);
    assert_eq!(h.name_str(), "test-index");
    assert_eq!(h.records_offset % RECORD_ALIGN, 0);
}

#[test]
fn cell_table_is_monotonic_and_spans_every_record() {
    let stars: Vec<(f64, f64, f32)> = (0..500)
        .map(|i| (i as f64 * 0.7 % 360.0, (i as f64 * 0.37 % 170.0) - 85.0, 10.0 + (i % 9) as f32))
        .collect();
    let bytes = build(8, &stars);
    let tab = read_cell_table(&bytes, 8);
    assert_eq!(tab[0], 0);
    assert_eq!(*tab.last().unwrap(), 500);
    for w in tab.windows(2) {
        assert!(w[1] >= w[0], "cell table must be non-decreasing");
    }
}

#[test]
fn every_star_lands_in_its_own_healpix_cell() {
    let stars: Vec<(f64, f64, f32)> = (0..300)
        .map(|i| (i as f64 * 1.13 % 360.0, (i as f64 * 0.61 % 176.0) - 88.0, 12.0))
        .collect();
    let bytes = build(8, &stars);
    for &(ra, dec, _) in &stars {
        let cell = ang2pix_nest(8, ra, dec);
        let recs = cell_records(&bytes, 8, cell);
        assert!(
            recs.iter().any(|r| (r.ra_deg() - ra).abs() < 1e-4 && (r.dec_deg() - dec).abs() < 1e-4),
            "star ({ra},{dec}) missing from cell {cell}"
        );
    }
}

#[test]
fn records_are_sorted_brightest_first_within_each_cell() {
    // All in one small patch so they share cells, with magnitudes out of order.
    let stars: Vec<(f64, f64, f32)> = (0..200)
        .map(|i| (100.0 + (i % 10) as f64 * 0.01, 20.0 + (i / 10) as f64 * 0.01,
                  20.0 - (i * 7 % 200) as f32 * 0.05))
        .collect();
    let bytes = build(64, &stars);
    let tab = read_cell_table(&bytes, 64);
    let mut checked = 0;
    for cell in 0..npix(64) {
        if tab[cell as usize + 1] - tab[cell as usize] < 2 {
            continue;
        }
        let recs = cell_records(&bytes, 64, cell);
        for w in recs.windows(2) {
            assert!(w[0].mag_milli <= w[1].mag_milli, "cell {cell} not brightest-first");
        }
        checked += 1;
    }
    assert!(checked > 0, "test data produced no multi-star cell");
}

#[test]
fn record_digest_matches_the_record_region() {
    let bytes = build(4, &[(1.0, 2.0, 11.0), (3.0, 4.0, 12.0)]);
    let h = Header::from_bytes(&bytes).unwrap();
    let start = h.records_offset as usize;
    let end = start + (h.n_records as usize) * RECORD_BYTES;
    assert_eq!(h.records_sha256, psolve_index::sha256::sha256(&bytes[start..end]));
}

#[test]
fn empty_build_is_valid() {
    let bytes = build(4, &[]);
    let h = Header::from_bytes(&bytes).unwrap();
    assert_eq!(h.n_records, 0);
    let tab = read_cell_table(&bytes, 4);
    assert!(tab.iter().all(|&v| v == 0));
}

#[test]
fn rejects_invalid_nside() {
    assert!(Builder::new(63, 20.0, 2016.0, "x").is_err());
}

#[test]
fn stars_without_proper_motion_are_kept_not_skipped() {
    // Gaia DR3 has ~340M two-parameter sources: real position, no proper
    // motion. Skipping them would discard about a fifth of the catalogue,
    // so a non-finite PM must NOT disqualify a star. Position and magnitude
    // are required; proper motion is optional.
    let mut b = Builder::new(8, 20.0, 2016.0, "pm-test").unwrap();
    b.push(10.0, 20.0, 12.0, f32::NAN, f32::NAN);
    b.push(11.0, 21.0, 12.5, 1.0, 2.0);
    let mut buf = Cursor::new(Vec::new());
    let stats = b.finish(&mut buf).unwrap();
    assert_eq!(stats.written, 2, "a star with no proper motion must still be indexed");
    assert_eq!(stats.skipped, 0);
}

#[test]
fn a_star_with_no_usable_position_is_skipped_and_counted() {
    // The other side of the same rule: position and magnitude are required.
    let mut b = Builder::new(8, 20.0, 2016.0, "skip-test").unwrap();
    b.push(f64::NAN, 20.0, 12.0, 0.0, 0.0);
    b.push(10.0, f64::NAN, 12.0, 0.0, 0.0);
    b.push(10.0, 20.0, f32::NAN, 0.0, 0.0);
    b.push(10.0, 20.0, 12.0, 0.0, 0.0);
    let mut buf = Cursor::new(Vec::new());
    let stats = b.finish(&mut buf).unwrap();
    assert_eq!(stats.written, 1);
    assert_eq!(stats.skipped, 3, "unusable rows must be counted, not silently dropped");
}
