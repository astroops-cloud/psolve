use psolve_index::builder::Builder;
use psolve_index::healpix::ang2pix_nest;
use psolve_index::reader::Index;
use std::io::Write;

fn write_index(dir: &std::path::Path, nside: u32, stars: &[(f64, f64, f32)]) -> std::path::PathBuf {
    let mut b = Builder::new(nside, 20.0, 2016.0, "test").unwrap();
    for &(ra, dec, mag) in stars {
        b.push(ra, dec, mag, 0.0, 0.0);
    }
    let mut buf = std::io::Cursor::new(Vec::new());
    b.finish(&mut buf).unwrap();
    let p = dir.join("test.psidx");
    let mut f = std::fs::File::create(&p).unwrap();
    f.write_all(&buf.into_inner()).unwrap();
    p
}

// NOTE: the original helper keyed the temp dir on `std::process::id()` alone,
// which is constant for every test in this binary. `cargo test` runs test
// functions concurrently on threads within that one process, so all 8 tests
// would share one directory and one `test.psidx` filename and stomp on each
// other. An atomic counter makes every call (even repeated calls from the
// same test) land in its own directory, which is simpler and more robust
// than threading a name through every call site.
fn tmpdir() -> std::path::PathBuf {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let d = std::env::temp_dir().join(format!("psolve-test-{}-{}", std::process::id(), n));
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn opens_and_reports_the_header() {
    let d = tmpdir();
    let p = write_index(&d, 8, &[(10.0, 20.0, 11.0), (10.1, 20.1, 12.0)]);
    let idx = Index::open(&p).unwrap();
    assert_eq!(idx.header().n_records, 2);
    assert_eq!(idx.header().nside, 8);
    assert_eq!(idx.header().name_str(), "test");
}

#[test]
fn digest_verifies() {
    let d = tmpdir();
    let p = write_index(&d, 8, &[(1.0, 2.0, 11.0), (3.0, 4.0, 12.0)]);
    Index::open(&p).unwrap().verify_digest().unwrap();
}

#[test]
fn rejects_a_corrupted_record_region() {
    let d = tmpdir();
    let p = write_index(&d, 8, &[(1.0, 2.0, 11.0), (3.0, 4.0, 12.0)]);
    let mut bytes = std::fs::read(&p).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 0xff;
    std::fs::write(&p, &bytes).unwrap();
    let idx = Index::open(&p).unwrap();
    assert!(idx.verify_digest().is_err(), "corruption must be detected");
}

#[test]
fn cell_lookup_returns_the_stars_in_that_cell() {
    let d = tmpdir();
    let stars = [(10.0, 20.0, 11.0), (10.01, 20.01, 12.0), (200.0, -40.0, 13.0)];
    let p = write_index(&d, 64, &stars);
    let idx = Index::open(&p).unwrap();
    let c = ang2pix_nest(64, 10.0, 20.0);
    assert!(idx.cell_len(c) >= 1);
    let far = ang2pix_nest(64, 200.0, -40.0);
    assert_eq!(idx.cell_len(far), 1);
}

#[test]
fn brightest_in_disc_is_sorted_and_limited() {
    let d = tmpdir();
    // 60 stars in a tight patch, magnitudes deliberately shuffled.
    let stars: Vec<(f64, f64, f32)> = (0..60)
        .map(|i| (100.0 + (i % 8) as f64 * 0.02, 20.0 + (i / 8) as f64 * 0.02,
                  9.0 + ((i * 13) % 60) as f32 * 0.1))
        .collect();
    let p = write_index(&d, 64, &stars);
    let idx = Index::open(&p).unwrap();
    let got = idx.brightest_in_disc(100.08, 20.08, 1.0, 10);
    assert_eq!(got.len(), 10, "limit must be honoured");
    for w in got.windows(2) {
        assert!(w[0].mag() <= w[1].mag(), "result must be brightest-first");
    }
    let all = idx.brightest_in_disc(100.08, 20.08, 1.0, 10_000);
    assert!(all.len() >= 10);
    assert!(all[0].mag() <= all[all.len() - 1].mag());
}

fn angsep_deg(ra1: f64, dec1: f64, ra2: f64, dec2: f64) -> f64 {
    let (r1, d1, r2, d2) =
        (ra1.to_radians(), dec1.to_radians(), ra2.to_radians(), dec2.to_radians());
    let (dr, dd) = (r2 - r1, d2 - d1);
    let a = (dd / 2.0).sin().powi(2) + d1.cos() * d2.cos() * (dr / 2.0).sin().powi(2);
    2.0 * a.sqrt().min(1.0).asin().to_degrees()
}

#[test]
fn brightest_in_disc_every_result_is_within_the_radius() {
    let d = tmpdir();
    let stars: Vec<(f64, f64, f32)> = (0..40)
        .map(|i| {
            let ra = 200.0 + ((i % 20) as f64 - 10.0) * 0.3;
            let dec = -13.0 + ((i / 20) as f64 - 0.5) * 0.6;
            (ra, dec, 9.0 + (i % 10) as f32 * 0.3)
        })
        .collect();
    let p = write_index(&d, 64, &stars);
    let idx = Index::open(&p).unwrap();
    let got = idx.brightest_in_disc(200.0, -13.0, 1.0, 1000);
    assert!(!got.is_empty(), "expected at least one star in range");
    for r in &got {
        let sep = angsep_deg(200.0, -13.0, r.ra_deg(), r.dec_deg());
        assert!(sep <= 1.0 + 1e-6, "star at separation {sep} deg exceeds the 1.0 deg radius");
    }
}

#[test]
fn brightest_in_disc_excludes_an_out_of_field_star_even_when_it_is_brighter() {
    // nside 64's cell padding is ~1.03 deg, so a query radius of 0.5 deg still
    // searches cells reaching out to ~1.53 deg. A star at 0.8 deg true
    // separation falls inside that padded search but outside the requested
    // radius, and must not displace the genuinely in-field (fainter) stars.
    let d = tmpdir();
    let mut stars = vec![(100.0, 20.8, 1.0f32)]; // bright, 0.8 deg north of centre
    for i in 0..5 {
        stars.push((100.0 + i as f64 * 0.05, 20.0 + i as f64 * 0.02, 12.0));
    }
    let p = write_index(&d, 64, &stars);
    let idx = Index::open(&p).unwrap();
    let got = idx.brightest_in_disc(100.0, 20.0, 0.5, 10);
    assert_eq!(got.len(), 5, "the out-of-radius bright star must not be counted");
    for r in &got {
        assert!(
            r.mag() >= 11.0,
            "the brighter out-of-field star leaked into the result: mag {}",
            r.mag()
        );
        let sep = angsep_deg(100.0, 20.0, r.ra_deg(), r.dec_deg());
        assert!(sep <= 0.5 + 1e-6, "returned star at separation {sep} deg exceeds radius 0.5");
    }
}

#[test]
fn disc_far_from_any_star_is_empty() {
    let d = tmpdir();
    let p = write_index(&d, 64, &[(10.0, 20.0, 11.0)]);
    let idx = Index::open(&p).unwrap();
    assert!(idx.brightest_in_disc(200.0, -60.0, 0.5, 100).is_empty());
}

#[test]
fn stars_in_disc_includes_a_star_just_inside_and_excludes_one_just_outside() {
    let d = tmpdir();
    // Centre (100.0, 20.0), radius 0.5 deg. One star at 0.49 deg separation
    // (due north, so separation == the dec delta) must be included; one at
    // 0.51 deg must not.
    let stars = [
        (100.0, 20.49, 10.0f32), // inside
        (100.0, 20.51, 10.0f32), // outside
    ];
    let p = write_index(&d, 64, &stars);
    let idx = Index::open(&p).unwrap();
    let got = idx.stars_in_disc(100.0, 20.0, 0.5, 20.0);
    assert_eq!(got.len(), 1, "expected exactly the inside star, got {got:?}");
    let sep = angsep_deg(100.0, 20.0, got[0].ra_deg(), got[0].dec_deg());
    assert!(sep <= 0.5 + 1e-6, "returned star at separation {sep} exceeds the radius");
}

#[test]
fn stars_in_disc_magnitude_cut_is_inclusive() {
    let d = tmpdir();
    let stars = [(100.0, 20.0, 12.0f32), (100.01, 20.0, 12.5f32), (100.02, 20.0, 13.0f32)];
    let p = write_index(&d, 64, &stars);
    let idx = Index::open(&p).unwrap();
    let got = idx.stars_in_disc(100.0, 20.0, 1.0, 12.5);
    let mags: Vec<f32> = got.iter().map(|r| r.mag()).collect();
    assert!(mags.contains(&12.0), "mag 12.0 should pass a 12.5 cap");
    assert!(mags.contains(&12.5), "mag exactly at the cap must be INCLUDED (inclusive cut)");
    assert!(!mags.contains(&13.0), "mag 13.0 must be excluded by a 12.5 cap");
}

#[test]
fn stars_in_disc_is_the_same_set_as_brightest_in_disc_with_a_huge_limit() {
    let d = tmpdir();
    // A shuffled magnitude spread across several cells so both the k-way
    // merge and the plain per-cell scan have real work to do.
    let stars: Vec<(f64, f64, f32)> = (0..80)
        .map(|i| {
            let ra = 150.0 + (i % 10) as f64 * 0.15;
            let dec = -30.0 + (i / 10) as f64 * 0.15;
            (ra, dec, 8.0 + ((i * 17) % 40) as f32 * 0.1)
        })
        .collect();
    let p = write_index(&d, 64, &stars);
    let idx = Index::open(&p).unwrap();

    let bright = idx.brightest_in_disc(150.5, -29.5, 2.0, 1_000_000);
    let all = idx.stars_in_disc(150.5, -29.5, 2.0, 999.0);

    assert_eq!(bright.len(), all.len(), "same disc must yield the same count");
    let mut bright_sorted: Vec<(u32, i32)> =
        bright.iter().map(|r| (r.ra_scaled, r.dec_scaled)).collect();
    let mut all_sorted: Vec<(u32, i32)> = all.iter().map(|r| (r.ra_scaled, r.dec_scaled)).collect();
    bright_sorted.sort_unstable();
    all_sorted.sort_unstable();
    assert_eq!(bright_sorted, all_sorted, "must be the same SET of stars, order may differ");
}

#[test]
fn stars_in_disc_handles_ra_wrap_at_zero_three_sixty() {
    let d = tmpdir();
    // A star just west of 0 (i.e. just below 360) and one just east of 0,
    // both close to a centre placed exactly at 0.0 deg RA.
    let stars = [(359.9, 10.0, 11.0f32), (0.1, 10.0, 11.0f32), (180.0, 10.0, 11.0f32)];
    let p = write_index(&d, 64, &stars);
    let idx = Index::open(&p).unwrap();
    let got = idx.stars_in_disc(0.0, 10.0, 1.0, 20.0);
    assert_eq!(got.len(), 2, "both stars straddling the 0/360 wrap must be found, got {got:?}");
    for r in &got {
        let sep = angsep_deg(0.0, 10.0, r.ra_deg(), r.dec_deg());
        assert!(sep <= 1.0 + 1e-6, "star at {sep} deg exceeds the 1.0 deg radius");
    }
}

#[test]
fn stars_in_disc_over_a_pole_works() {
    let d = tmpdir();
    // Stars scattered around the north pole at different RAs -- near the
    // pole RA is nearly degenerate, so this exercises the same wrap-adjacent
    // logic as the RA-wrap test but via declination instead.
    let stars = [
        (0.0, 89.8, 11.0f32),
        (90.0, 89.8, 11.0f32),
        (180.0, 89.8, 11.0f32),
        (270.0, 89.8, 11.0f32),
        (45.0, 88.5, 11.0f32), // still within 2 deg of the pole
    ];
    let p = write_index(&d, 64, &stars);
    let idx = Index::open(&p).unwrap();
    let got = idx.stars_in_disc(0.0, 90.0, 2.0, 20.0);
    assert_eq!(got.len(), 5, "every star within 2 deg of the pole must be found, got {got:?}");
    for r in &got {
        let sep = angsep_deg(0.0, 90.0, r.ra_deg(), r.dec_deg());
        assert!(sep <= 2.0 + 1e-6, "star at {sep} deg exceeds the 2.0 deg radius");
    }
}

#[test]
fn rejects_a_records_offset_that_does_not_match_records_offset_for() {
    // `records_offset` and `nside` are independent header fields; the builder
    // always derives one from the other, but nothing enforced that on the
    // read path until now. A corrupt header could otherwise point
    // `records_offset` at the wrong place and every `cell()`/`star()` slice
    // would silently read the wrong bytes instead of failing to open.
    let d = tmpdir();
    let p = write_index(&d, 8, &[(1.0, 2.0, 11.0)]);
    let mut bytes = std::fs::read(&p).unwrap();
    // records_offset lives at header bytes 36..44 -- see Header::to_bytes.
    let mut v = u64::from_le_bytes(bytes[36..44].try_into().unwrap());
    v += 4096; // still page-aligned and plausible-looking, but wrong for nside 8
    bytes[36..44].copy_from_slice(&v.to_le_bytes());
    std::fs::write(&p, &bytes).unwrap();
    match Index::open(&p) {
        Ok(_) => panic!("a records_offset that disagrees with records_offset_for must not open"),
        Err(e) => assert!(
            format!("{e}").contains("records_offset"),
            "expected a records_offset mismatch error, got: {e}"
        ),
    }
}

#[test]
fn rejects_a_non_index_file() {
    let d = tmpdir();
    let p = d.join("junk.psidx");
    std::fs::write(&p, b"this is not an index at all, not even close").unwrap();
    assert!(Index::open(&p).is_err());
}

#[test]
fn rejects_a_truncated_file() {
    let d = tmpdir();
    let p = write_index(&d, 8, &[(1.0, 2.0, 11.0)]);
    let bytes = std::fs::read(&p).unwrap();
    std::fs::write(&p, &bytes[..bytes.len() - 8]).unwrap();
    assert!(Index::open(&p).is_err(), "truncated index must not open");
}

// Centre used by the stratified_in_disc tests below, and a builder for the
// lopsided fixture they all share: one dense HEALPix cell holding many
// bright stars, plus several neighbouring cells holding fainter ones. At
// nside 64 a query radius of 2.0 deg spans many cells, so brightest-N would
// draw its whole answer from the dense cell alone while stratification must
// not.
const CENTRE_RA: f64 = 100.0;
const CENTRE_DEC: f64 = 20.0;

fn build_lopsided_index(d: &std::path::Path) -> Index {
    let mut stars: Vec<(f64, f64, f32)> = Vec::new();
    // Dense clump: 60 bright stars packed into a few arcminutes, all
    // landing in the same (or very few) nside-64 cells.
    for i in 0..60 {
        let ra = CENTRE_RA + (i % 8) as f64 * 0.005;
        let dec = CENTRE_DEC + (i / 8) as f64 * 0.005;
        stars.push((ra, dec, 8.0 + (i % 10) as f32 * 0.05));
    }
    // Sparser neighbours: fainter stars spread out to ~1.8 deg so they land
    // in distinct cells from the clump and from each other.
    for i in 0..40 {
        let angle = (i as f64) * 0.5; // radians, spirals outward across cells
        let r = 0.3 + (i as f64 % 12.0) * 0.13; // up to ~1.86 deg
        let ra = CENTRE_RA + r * angle.cos() / CENTRE_DEC.to_radians().cos().max(0.1);
        let dec = CENTRE_DEC + r * angle.sin();
        stars.push((ra, dec, 14.0 + (i % 5) as f32 * 0.2));
    }
    let p = write_index(d, 64, &stars);
    Index::open(&p).unwrap()
}

/// Number of distinct HEALPix cells (at the index's own nside) the given
/// records fall in.
fn distinct_cells(idx: &Index, recs: &[psolve_index::record::StarRecord]) -> usize {
    let nside = idx.header().nside;
    let cells: std::collections::HashSet<u64> =
        recs.iter().map(|r| ang2pix_nest(nside, r.ra_deg(), r.dec_deg())).collect();
    cells.len()
}

#[test]
fn stratified_in_disc_spreads_across_cells_rather_than_taking_the_brightest() {
    let d = tmpdir();
    let idx = build_lopsided_index(&d);
    let got = idx.stratified_in_disc(CENTRE_RA, CENTRE_DEC, 2.0, 40);
    let cells_hit = distinct_cells(&idx, &got);
    assert!(cells_hit > 1, "all {} stars came from one cell", got.len());
}

#[test]
fn stratified_in_disc_returns_the_full_limit_when_stars_exist() {
    let d = tmpdir();
    let idx = build_lopsided_index(&d);
    let got = idx.stratified_in_disc(CENTRE_RA, CENTRE_DEC, 2.0, 40);
    assert_eq!(got.len(), 40, "sparse cells must donate their budget");
}

#[test]
fn stratified_in_disc_never_returns_a_star_outside_the_radius() {
    let d = tmpdir();
    let idx = build_lopsided_index(&d);
    for s in idx.stratified_in_disc(CENTRE_RA, CENTRE_DEC, 1.0, 100) {
        let sep = angsep_deg(CENTRE_RA, CENTRE_DEC, s.ra_deg(), s.dec_deg());
        assert!(sep <= 1.0 + 1e-6, "star at separation {sep} deg exceeds the 1.0 deg radius");
    }
}

#[test]
fn stratified_in_disc_is_a_subset_of_stars_in_disc() {
    let d = tmpdir();
    let idx = build_lopsided_index(&d);
    let all: std::collections::HashSet<(u32, i32)> = idx
        .stars_in_disc(CENTRE_RA, CENTRE_DEC, 2.0, f32::MAX)
        .iter()
        .map(|s| (s.ra_scaled, s.dec_scaled))
        .collect();
    for s in idx.stratified_in_disc(CENTRE_RA, CENTRE_DEC, 2.0, 40) {
        assert!(all.contains(&(s.ra_scaled, s.dec_scaled)));
    }
}

#[test]
fn stratified_in_disc_result_is_sorted_brightest_first() {
    let d = tmpdir();
    let idx = build_lopsided_index(&d);
    let got = idx.stratified_in_disc(CENTRE_RA, CENTRE_DEC, 2.0, 40);
    for w in got.windows(2) {
        assert!(w[0].mag() <= w[1].mag(), "result must be brightest-first");
    }
}

#[test]
fn a_limit_larger_than_the_disc_returns_everything_in_it() {
    let d = tmpdir();
    let idx = build_lopsided_index(&d);
    let a = idx.stratified_in_disc(CENTRE_RA, CENTRE_DEC, 2.0, usize::MAX).len();
    let b = idx.stars_in_disc(CENTRE_RA, CENTRE_DEC, 2.0, f32::MAX).len();
    assert_eq!(a, b);
}

// Not one of the brief's 8 tests: added alongside a bounds-safety fix in
// `Index::open` (see task-7 report). A cell table whose final cumulative
// offset doesn't match `n_records` would otherwise let `cell()` slice past
// the record region on a corrupt-but-not-truncated file.
#[test]
fn rejects_a_cell_table_whose_final_offset_does_not_match_record_count() {
    let d = tmpdir();
    let p = write_index(&d, 8, &[(1.0, 2.0, 11.0), (3.0, 4.0, 12.0)]);
    let mut bytes = std::fs::read(&p).unwrap();
    let npix = psolve_index::healpix::npix(8);
    let tab_off = psolve_index::format::Header::cell_table_offset() as usize;
    let last = tab_off + npix as usize * 8;
    let mut v = u64::from_le_bytes(bytes[last..last + 8].try_into().unwrap());
    v += 1;
    bytes[last..last + 8].copy_from_slice(&v.to_le_bytes());
    std::fs::write(&p, &bytes).unwrap();
    assert!(Index::open(&p).is_err(), "cell table final offset must match n_records");
}

// -- brightest_in_disc_indexed / star_at (Task 2: .psqidx builder support) --

#[test]
fn brightest_in_disc_indexed_returns_the_same_stars_as_brightest_in_disc() {
    let d = tmpdir();
    let stars: Vec<(f64, f64, f32)> = (0..40)
        .map(|i| (100.0 + (i % 8) as f64 * 0.02, 20.0 + (i / 8) as f64 * 0.02, 10.0 + i as f32 * 0.1))
        .collect();
    let p = write_index(&d, 64, &stars);
    let idx = Index::open(&p).unwrap();

    let plain = idx.brightest_in_disc(100.1, 20.1, 1.0, 12);
    let indexed = idx.brightest_in_disc_indexed(100.1, 20.1, 1.0, 12);
    assert_eq!(plain.len(), indexed.len());
    for (p, (_, i)) in plain.iter().zip(indexed.iter()) {
        assert_eq!(*p, *i, "the indexed variant must return the identical star records, in the identical order");
    }
}

#[test]
fn brightest_in_disc_indexed_global_indices_are_distinct() {
    let d = tmpdir();
    let stars: Vec<(f64, f64, f32)> = (0..30)
        .map(|i| (150.0 + (i % 6) as f64 * 0.05, -10.0 + (i / 6) as f64 * 0.05, 9.0 + i as f32 * 0.05))
        .collect();
    let p = write_index(&d, 64, &stars);
    let idx = Index::open(&p).unwrap();
    let got = idx.brightest_in_disc_indexed(150.1, -9.9, 1.0, 30);
    let mut ids: Vec<u32> = got.iter().map(|(i, _)| *i).collect();
    let before = ids.len();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), before, "every returned star must carry a distinct global index");
}

#[test]
fn star_at_resolves_the_global_index_returned_by_the_indexed_disc_query() {
    let d = tmpdir();
    let stars: Vec<(f64, f64, f32)> = (0..20)
        .map(|i| (10.0 + (i % 5) as f64 * 0.03, 5.0 + (i / 5) as f64 * 0.03, 8.0 + i as f32 * 0.2))
        .collect();
    let p = write_index(&d, 64, &stars);
    let idx = Index::open(&p).unwrap();
    let got = idx.brightest_in_disc_indexed(10.1, 5.1, 1.0, 20);
    assert!(!got.is_empty());
    for (global_idx, rec) in &got {
        let resolved = idx.star_at(*global_idx).expect("global index from this same index must resolve");
        assert_eq!(resolved, *rec, "star_at must resolve back to the exact star the disc query returned");
    }
}

#[test]
fn star_at_rejects_an_out_of_range_index() {
    let d = tmpdir();
    let p = write_index(&d, 8, &[(1.0, 2.0, 11.0), (3.0, 4.0, 12.0)]);
    let idx = Index::open(&p).unwrap();
    assert!(idx.star_at(2).is_none(), "n_records is 2, so index 2 is out of range");
    assert!(idx.star_at(u32::MAX).is_none());
    assert!(idx.star_at(0).is_some());
}

#[test]
fn brightest_in_disc_indexed_respects_the_true_radius_not_just_cell_membership() {
    let d = tmpdir();
    // One star well inside the radius, one just outside -- same shape as
    // `brightest_in_disc_excludes_an_out_of_field_star_even_when_it_is_brighter`.
    let stars = [(50.0, 0.0, 15.0), (50.5, 0.0, 1.0)];
    let p = write_index(&d, 64, &stars);
    let idx = Index::open(&p).unwrap();
    let got = idx.brightest_in_disc_indexed(50.0, 0.0, 0.1, 10);
    assert_eq!(got.len(), 1, "the far, brighter star must be excluded by true separation");
    assert_eq!(got[0].1.mag(), 15.0);
}
