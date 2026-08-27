use psolve_index::gaia::{find_columns, parse_row, read_ecsv, ColumnNames, RowFilter};
use std::io::BufReader;

const SAMPLE: &str = include_str!("fixtures/gaia_sample.csv");

fn header_line() -> &'static str {
    SAMPLE.lines().find(|l| !l.starts_with('#')).unwrap()
}

fn gaia() -> ColumnNames {
    ColumnNames::default()
}

fn mag_only(max_mag: f32) -> RowFilter {
    RowFilter { max_mag, ..RowFilter::default() }
}

#[test]
fn finds_columns_by_name_not_position() {
    let c = find_columns(header_line(), &gaia()).unwrap();
    // Reordering the header must still work -- that is the point of by-name lookup.
    let reordered = "phot_g_mean_mag,dec,ra,pmdec,pmra,source_id";
    let c2 = find_columns(reordered, &gaia()).unwrap();
    assert_ne!(
        (c.ra, c.dec), (c2.ra, c2.dec),
        "indices should differ between the two layouts"
    );
}

#[test]
fn rejects_a_header_missing_a_required_column() {
    let bad = "source_id,ra,dec,pmra,pmdec"; // no phot_g_mean_mag
    assert!(find_columns(bad, &gaia()).is_err());
}

#[test]
fn header_without_source_id_is_accepted() {
    // Reduced shards drop source_id; nothing in the index needs it.
    let c = find_columns("ra,dec,pmra,pmdec,phot_g_mean_mag", &gaia()).unwrap();
    let row = parse_row(&c, "10.0,20.0,1.0,2.0,12.5", 1).unwrap().unwrap();
    assert_eq!(row.source_id, 0);
    assert!((row.ra - 10.0).abs() < 1e-9);
    assert!((row.mag - 12.5).abs() < 1e-6);
}

#[test]
fn a_non_gaia_catalogue_works_via_column_overrides() {
    // A Vizier-style export: different names, same meaning.
    let names = ColumnNames::with_overrides(
        "ra=RAJ2000,dec=DEJ2000,mag=Vmag,pmra=pmRA,pmdec=pmDE",
    )
    .unwrap();
    let csv = "RAJ2000,DEJ2000,Vmag,pmRA,pmDE\n120.5,-33.25,8.75,-4.5,6.25\n";
    let mut got = Vec::new();
    read_ecsv(BufReader::new(csv.as_bytes()), &names, &RowFilter::default(), |r| {
        got.push(r)
    })
    .unwrap();
    assert_eq!(got.len(), 1);
    assert!((got[0].ra - 120.5).abs() < 1e-9);
    assert!((got[0].dec + 33.25).abs() < 1e-9);
    assert!((got[0].mag - 8.75).abs() < 1e-6);
    assert!((got[0].pmdec - 6.25).abs() < 1e-6);
}

#[test]
fn unknown_or_malformed_column_overrides_are_rejected() {
    // A silently-ignored typo would build the index from the wrong column.
    assert!(ColumnNames::with_overrides("magnitude=Vmag").is_err());
    assert!(ColumnNames::with_overrides("ra").is_err());
    assert!(ColumnNames::with_overrides("").is_ok(), "empty spec = defaults");
}

#[test]
fn parses_a_normal_row() {
    let c = find_columns(header_line(), &gaia()).unwrap();
    let line = SAMPLE.lines().filter(|l| !l.starts_with('#')).nth(1).unwrap();
    let row = parse_row(&c, line, 1).unwrap().unwrap();
    assert_eq!(row.source_id, 4295806720);
    assert!((row.ra - 44.99615537864534).abs() < 1e-12);
    assert!((row.dec - 0.005615226341865997).abs() < 1e-12);
    assert!((row.mag - 17.641426).abs() < 1e-5);
    assert!((row.pmra - 12.616485).abs() < 1e-4);
}

// This fixture row (and `null_magnitude_is_skipped_not_an_error` below) uses
// Gaia's literal "null" sentinel, not an empty field -- see the doc comment
// on `is_missing` in gaia.rs. Real Gaia DR3 exports never emit an empty
// field, so a fixture built from empty fields would leave the `null` branch
// of `is_missing` covered by only one test in this whole file. The
// genuinely-empty case is pinned separately in
// `a_malformed_proper_motion_is_an_error_not_a_silent_zero` (PM) and
// `empty_magnitude_is_skipped_not_an_error` below (magnitude), both via
// inline CSV rather than this shared fixture.
#[test]
fn null_proper_motion_becomes_zero() {
    let c = find_columns(header_line(), &gaia()).unwrap();
    let line = SAMPLE.lines().filter(|l| !l.starts_with('#')).nth(3).unwrap();
    let row = parse_row(&c, line, 3).unwrap().unwrap();
    assert_eq!(row.pmra, 0.0);
    assert_eq!(row.pmdec, 0.0);
    assert!((row.mag - 9.5).abs() < 1e-5);
}

#[test]
fn null_magnitude_is_skipped_not_an_error() {
    let c = find_columns(header_line(), &gaia()).unwrap();
    let line = SAMPLE.lines().filter(|l| !l.starts_with('#')).nth(4).unwrap();
    assert!(parse_row(&c, line, 4).unwrap().is_none());
}

#[test]
fn empty_magnitude_is_skipped_not_an_error() {
    // Pins the genuinely-empty-field case (bring-your-own CSVs may use it)
    // independently of the shared fixture, which now uses Gaia's real "null"
    // sentinel for its missing-magnitude row instead.
    let c = find_columns("ra,dec,pmra,pmdec,phot_g_mean_mag", &gaia()).unwrap();
    assert!(parse_row(&c, "10.0,20.0,1.0,1.0,", 1).unwrap().is_none());
}

#[test]
fn read_ecsv_skips_comments_and_applies_the_mag_cut() {
    let mut got = Vec::new();
    let n = read_ecsv(BufReader::new(SAMPLE.as_bytes()), &gaia(), &mag_only(15.0), |r| {
        got.push(r)
    })
    .unwrap();
    // rows: 17.64 (cut), 14.13 (kept), 9.5 (kept), no-mag (skipped)
    assert_eq!(got.len(), 2, "mag cut should keep exactly two rows");
    assert_eq!(n, 2);
    assert!(got.iter().all(|r| r.mag <= 15.0));
}

#[test]
fn read_ecsv_with_a_generous_cut_keeps_all_magnitude_bearing_rows() {
    let mut got = Vec::new();
    read_ecsv(BufReader::new(SAMPLE.as_bytes()), &gaia(), &RowFilter::default(), |r| {
        got.push(r)
    })
    .unwrap();
    assert_eq!(got.len(), 3);
}

#[test]
fn read_ecsv_applies_the_declination_cut() {
    // Sample decs are 0.00562, 0.02105, 0.01988 (plus one row with no mag).
    let mut got = Vec::new();
    read_ecsv(
        BufReader::new(SAMPLE.as_bytes()),
        &gaia(),
        &RowFilter { min_dec: 0.01, ..RowFilter::default() },
        |r| got.push(r),
    )
    .unwrap();
    assert_eq!(got.len(), 2, "min_dec should exclude the 0.00562 row");
    assert!(got.iter().all(|r| r.dec >= 0.01));

    let mut south = Vec::new();
    read_ecsv(
        BufReader::new(SAMPLE.as_bytes()),
        &gaia(),
        &RowFilter { max_dec: 0.01, ..RowFilter::default() },
        |r| south.push(r),
    )
    .unwrap();
    assert_eq!(south.len(), 1, "max_dec should keep only the 0.00562 row");
}

#[test]
fn an_impossible_declination_range_is_rejected() {
    let f = RowFilter { min_dec: 40.0, max_dec: -40.0, ..RowFilter::default() };
    assert!(f.validate().is_err(), "min above max must be rejected");
    let g = RowFilter { min_dec: -200.0, ..RowFilter::default() };
    assert!(g.validate().is_err(), "out-of-range declination must be rejected");
    assert!(RowFilter::default().validate().is_ok());
}

#[test]
fn a_short_row_is_an_error_not_a_panic() {
    let c = find_columns(header_line(), &gaia()).unwrap();
    assert!(parse_row(&c, "1,2,3", 7).is_err());
}

#[test]
fn a_malformed_proper_motion_is_an_error_not_a_silent_zero() {
    let c = find_columns("ra,dec,pmra,pmdec,phot_g_mean_mag", &gaia()).unwrap();
    // Empty stays a legitimate zero...
    let ok = parse_row(&c, "10.0,20.0,,,12.5", 1).unwrap().unwrap();
    assert_eq!(ok.pmra, 0.0);
    // ...but garbage must not be laundered into one.
    assert!(parse_row(&c, "10.0,20.0,N/A,2.0,12.5", 2).is_err());
}

#[test]
fn a_file_with_no_header_row_is_an_error() {
    let only_comments = "# %ECSV 1.0\n# ---\n# delimiter: ','\n";
    assert!(read_ecsv(BufReader::new(only_comments.as_bytes()), &gaia(),
                      &RowFilter::default(), |_| {}).is_err());
    assert!(read_ecsv(BufReader::new("".as_bytes()), &gaia(),
                      &RowFilter::default(), |_| {}).is_err());
}

#[test]
fn gaias_null_sentinel_is_missing_data_not_corruption() {
    // Gaia DR3 writes the literal "null", never an empty field. Verified
    // against live bulk data: 285/2000 rows carry pmra=null.
    let csv = "ra,dec,pmra,pmdec,phot_g_mean_mag\n\
               346.4277237603648,-5.474016898345624,null,null,20.978693\n\
               10.0,20.0,1.5,2.5,12.0\n\
               11.0,21.0,null,null,null\n";
    let mut got = Vec::new();
    let n = read_ecsv(BufReader::new(csv.as_bytes()), &gaia(), &RowFilter::default(), |r| {
        got.push(r)
    })
    .unwrap();
    assert_eq!(n, 2, "null PM must be kept; only the null-magnitude row is skipped");
    assert_eq!(got[0].pmra, 0.0, "null PM becomes zero, like any missing PM");
    assert_eq!(got[0].pmdec, 0.0);
    assert!((got[1].pmra - 1.5).abs() < 1e-6, "real values must still parse");
}

#[test]
fn genuine_garbage_is_still_an_error() {
    // Ruling F10 stands: `null` is Gaia's sentinel, but junk is still junk.
    let c = find_columns("ra,dec,pmra,pmdec,phot_g_mean_mag", &gaia()).unwrap();
    assert!(parse_row(&c, "10.0,20.0,N/A,2.0,12.5", 1).is_err());
    assert!(parse_row(&c, "10.0,20.0,1.0,2.0,not-a-magnitude", 2).is_err());
}
