//! Byte-exact tests for the ASTAP `.ini` sidecar writer, because AstroOps
//! parses these files -- a "close enough" exponent or a missing alignment
//! space is a file the consumer silently misreads.
//!
//! `psolve-cli` has no `[lib]` target (only `[[bin]]`). `main.rs` does
//! `mod`-declare `src/sidecar.rs` now (Task 9), but that only makes it part
//! of the `psolve` *binary*'s module tree -- an integration test crate like
//! this one still cannot `use` it without a `[lib]` target to link against.
//! `#[path]` pulls the source file directly into this test binary instead,
//! matching `psolve-index`'s
//! `include_str!("../../psolve-index/tests/fixtures/...")` precedent for
//! reaching across a directory boundary without a library target. See
//! `sidecar.rs`'s own module doc for why some of its functions still carry
//! `#[allow(dead_code)]` (they are wired into `astap_cmd`'s dispatch in a
//! later task) even though this file exercises every one of them.
//!
//! `clippy::excessive_precision` is silenced crate-wide below: every literal
//! in this file is a digit-for-digit transcription of a real ASTAP byte
//! sequence (fixture values, expected `astap_float` output), typed with all
//! 17 significant digits on purpose so a reader can diff them against the
//! ground-truth doc directly. Clippy's suggested truncation parses to the
//! identical `f64`, but "minimal" digits would hide the point of a
//! byte-exactness test.
#![allow(clippy::excessive_precision)]

#[path = "../src/sidecar.rs"]
mod sidecar;

use psolve_core::fit::Wcs;
use sidecar::{astap_float, astap_wcs_float, format_ini_failure, format_ini_success, format_wcs_fits_block, format_wcs_text};

/// The real success fixture's CRVAL/CRPIX/CD, transcribed from
/// `tests/fixtures/reference.ini` (itself a byte-exact copy of a real
/// `astap_cli` solve; see the ground-truth doc's §1a).
///
/// `crpix` here is the real fixture's `CRPIX1`/`CRPIX2` (1920.5, 1080.5)
/// MINUS 1: this `Wcs` represents what a real psolve solve would hand to
/// `format_ini_success` (psolve-core's 0-based pixel convention -- see
/// `sidecar.rs`'s "CRPIX convention" module doc), and the byte-exact
/// assertion below only holds if `format_ini_success`'s own `+ 1.0`
/// recovers the real fixture's 1-based bytes from this 0-based input. Using
/// the fixture's raw 1-based value here would test the formatter against an
/// input no real solve produces, and silently hide a missing `+ 1.0`.
fn wcs_from_the_fixture_values() -> Wcs {
    Wcs {
        crval: [2.5423046742390622E+002, -4.0311880588850023E+001],
        crpix: [1.9195000000000000E+003, 1.0795000000000000E+003],
        cd: [
            [3.5245253250848707E-004, 5.8334097357301367E-004],
            [-5.8335417754934037E-004, 3.5236170894630648E-004],
        ],
    }
}

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!("tests/fixtures/{name}"))
        .unwrap_or_else(|e| panic!("reading tests/fixtures/{name}: {e}"))
}

/// Byte-exact, because AstroOps parses these. A "close enough" exponent or a
/// missing alignment space is a file the consumer silently misreads.
#[test]
fn astap_float_matches_real_astap_bytes() {
    assert_eq!(astap_float(1920.5), " 1.9205000000000000E+003");
    assert_eq!(astap_float(254.23046742390622), " 2.5423046742390622E+002");
    assert_eq!(astap_float(-40.311880588850023), "-4.0311880588850023E+001");
    assert_eq!(astap_float(6.8154932258843713e-4), " 6.8154932258843713E-004");
    assert_eq!(astap_float(-5.8335417754934037e-4), "-5.8335417754934037E-004");
}

/// A non-finite value must never become a plausible-looking-but-wrong key
/// (e.g. `" NaNE+000"`) -- Task 9 wires this to live solver output, so this
/// is the boundary that keeps a stray NaN from reaching the AstroOps
/// consumer as a malformed sidecar it silently misparses.
#[test]
#[should_panic]
fn a_non_finite_value_panics_rather_than_emitting_a_malformed_key() {
    astap_float(f64::NAN);
}

#[test]
#[should_panic]
fn infinity_also_panics_rather_than_emitting_a_malformed_key() {
    astap_float(f64::INFINITY);
}

/// Three-digit zero-padded exponent -- Rust's `{:E}` gives `E-4`, ASTAP
/// gives `E-004`.
#[test]
fn the_exponent_is_always_three_digits() {
    for v in [1.0_f64, -1.0, 1e-4, 1e100, -1e-100, 0.0] {
        let s = astap_float(v);
        let e = s.find('E').expect("must have an exponent");
        assert_eq!(s.len() - e, 5, "{s} must end in E±NNN");
    }
}

/// The full 14-key success file, compared against the real fixture --
/// straight byte-exact equality. All four derived values (CDELT1, CDELT2,
/// CROTA1, CROTA2) reproduce ASTAP's own bytes exactly; see
/// `format_ini_success`'s doc comment for the `* 180.0 / PI` operation-order
/// detail that CROTA1/CROTA2 depend on.
#[test]
fn the_success_file_matches_the_real_fixture_byte_for_byte() {
    let real = fixture("reference.ini");
    let cmdline = real
        .lines()
        .find(|l| l.starts_with("CMDLINE="))
        .expect("fixture must have a CMDLINE line")
        .trim_start_matches("CMDLINE=");
    let wcs = wcs_from_the_fixture_values();
    assert_eq!(format_ini_success(&wcs, cmdline), real);
}

/// CDELT1/CDELT2/CROTA1/CROTA2 all derive from the CD matrix via `f64` math
/// (`hypot`/`atan2`) and all four reproduce ASTAP's own bytes exactly, given
/// the specific `* 180.0 / PI` association `format_ini_success` uses for the
/// degree conversion. Pinned individually, not just as a side effect of the
/// whole-file test above, since these are the values that took the most
/// investigation to get right.
#[test]
fn cdelt_and_crota_derived_from_cd_match_the_real_fixture_exactly() {
    let wcs = wcs_from_the_fixture_values();
    let cdelt1 = wcs.cd[0][0].hypot(wcs.cd[0][1]);
    let cdelt2 = wcs.cd[1][0].hypot(wcs.cd[1][1]);
    let crota1 = (-wcs.cd[0][1]).atan2(wcs.cd[0][0]) * 180.0 / std::f64::consts::PI;
    let crota2 = wcs.cd[1][0].atan2(wcs.cd[1][1]) * 180.0 / std::f64::consts::PI;
    assert_eq!(astap_float(cdelt1), " 6.8154932258843713E-004");
    assert_eq!(astap_float(cdelt2), " 6.8151366119530501E-004");
    assert_eq!(astap_float(crota1), "-5.8859778367665449E+001");
    assert_eq!(astap_float(crota2), "-5.8866887820396883E+001");
}

/// The degree-conversion pitfall itself, pinned directly: `f64::to_degrees()`
/// (a single multiply by the pre-rounded constant `180/PI`) is 1 ULP off the
/// real bytes on both CROTA values here, even though the preceding `atan2`
/// call agrees with ASTAP bit-for-bit. `* 180.0 / PI` (multiply, then
/// divide) is what closes the gap -- and the reversed association,
/// `/ PI * 180.0`, only closes it for CROTA1, not CROTA2, so this is not a
/// case where any "more careful" reassociation would do.
#[test]
fn to_degrees_is_one_ulp_off_but_mul_then_div_matches_exactly() {
    let wcs = wcs_from_the_fixture_values();
    let a1 = (-wcs.cd[0][1]).atan2(wcs.cd[0][0]);
    let a2 = wcs.cd[1][0].atan2(wcs.cd[1][1]);

    assert_ne!(
        astap_float(a1.to_degrees()),
        "-5.8859778367665449E+001",
        "to_degrees() is expected to be 1 ULP off, not a match"
    );
    assert_ne!(
        astap_float(a2.to_degrees()),
        "-5.8866887820396883E+001",
        "to_degrees() is expected to be 1 ULP off, not a match"
    );
    assert_eq!(astap_float(a1 * 180.0 / std::f64::consts::PI), "-5.8859778367665449E+001");
    assert_eq!(astap_float(a2 * 180.0 / std::f64::consts::PI), "-5.8866887820396883E+001");
}

/// The failure file starts with a literal blank line. It looks like a stray
/// writeln in ASTAP, but a consumer that skips line 0 would break on a file
/// that lacked it, so reproduce it exactly.
#[test]
fn the_failure_file_starts_with_a_blank_line_and_orders_cmdline_before_error() {
    let s = format_ini_failure("astap_cli -f x.fits", "Not enough stars.");
    assert!(s.starts_with('\n'), "byte 0 must be a newline");
    assert_eq!(s, "\nPLTSOLVD=F\nCMDLINE=astap_cli -f x.fits\nERROR=Not enough stars.\n");
    assert!(!s.contains("CRVAL"), "a failed solve writes no WCS keys");
}

/// The real failure fixture (`ERROR=No star database found.`), matched
/// byte-for-byte -- unlike the success case, nothing here is derived, so
/// there is no ULP gap to account for.
#[test]
fn the_real_failure_fixture_matches_format_ini_failure_byte_for_byte() {
    let real = fixture("reference-failure.ini");
    let cmdline = real
        .lines()
        .find(|l| l.starts_with("CMDLINE="))
        .expect("fixture must have a CMDLINE line")
        .trim_start_matches("CMDLINE=");
    let error = real
        .lines()
        .find(|l| l.starts_with("ERROR="))
        .expect("fixture must have an ERROR line")
        .trim_start_matches("ERROR=");
    assert_eq!(format_ini_failure(cmdline, error), real);
}

/// `sidecar.rs` also exports the `.wcs` writers (`astap_wcs_float`,
/// `format_wcs_text`, `format_wcs_fits_block`); byte-exact coverage for
/// those lives in `tests/sidecar_wcs.rs`, which has its own independent
/// `#[path]`-included copy of this same module (see this file's module doc
/// comment). Referencing them here too keeps `clippy --all-targets -D
/// warnings`'s per-binary `dead_code` check honest: each of the two test
/// binaries compiles the whole of `sidecar.rs` on its own, so a function
/// only ever called from the *other* binary is genuinely dead code from
/// this one's point of view.
///
/// This doubles as a real cross-check, not just lint appeasement: the
/// `.wcs` writer's 12-digit CRVAL1 must round from the very same `crval`
/// the `.ini` writer's 16-digit CRVAL1 (pinned above) rounds from.
#[test]
fn the_wcs_writers_share_this_fixtures_crval_with_the_ini_writer() {
    let wcs = wcs_from_the_fixture_values();
    assert_eq!(astap_wcs_float(wcs.crval[0]), " 2.542304674239E+002");

    let header = "SIMPLE  =                    T";
    let text = format_wcs_text(&wcs, header);
    assert!(
        text.contains("CRVAL1  =  2.542304674239E+002"),
        "the .wcs writer's CRVAL1 card must carry the same value the .ini writer's does"
    );

    let block = format_wcs_fits_block(&wcs, header);
    assert_eq!(block.len() % 2880, 0, "the FITS-block style must be block-padded");
}
