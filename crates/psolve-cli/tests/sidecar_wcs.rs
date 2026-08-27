//! Byte-exact tests for the ASTAP `.wcs` sidecar writer -- two structurally
//! different formats (ground-truth doc
//! `docs/superpowers/2026-08-14-astap-format-facts.md` §2), both exercised
//! here against real bytes:
//!
//! - The **default** style (`format_wcs_text`): FITS-card-styled text, LF
//!   after each card, not padded to 2880 -- what 100% of real production
//!   `.wcs` files on this machine actually are. Tested byte-exactly against
//!   `tests/fixtures/reference.wcs`, a direct copy of a real production
//!   file (`~/astroops/library/prawn/lights/S/...wcs`).
//! - The `-wcs`-flag style (`format_wcs_fits_block`): a true FITS block --
//!   zero newlines, every card exactly 80 bytes, padded to a whole multiple
//!   of 2880. No real production file on this machine uses this style (a
//!   grep of every real `CMDLINE`/`COMMENT cmdline:` string found zero
//!   occurrences of `-wcs`), so `tests/fixtures/reference-block.wcs` was
//!   reproduced instead by running the real `astap_cli` binary with `-wcs`
//!   against a copy of a real production FITS file, entirely inside the
//!   scratch directory -- `~/astroops` was only ever read from, never
//!   written to (see task-8-report.md for the exact invocation and
//!   verification against the ground-truth doc's own §2b measurements).
//!
//! `main.rs` does `mod`-declare `sidecar` now (Task 9), but that makes it
//! part of the `psolve` *binary*'s module tree, not something this
//! integration test crate can `use` -- `psolve-cli` has no `[lib]` target to
//! link against. `#[path = "../src/sidecar.rs"]` pulls the source in
//! directly instead; see `sidecar.rs`'s own module doc for the rest of that
//! story, including why some of its functions still carry
//! `#[allow(dead_code)]`.
#![allow(clippy::excessive_precision)]

#[path = "../src/sidecar.rs"]
mod sidecar;

use psolve_core::fit::Wcs;
use sidecar::{
    astap_float, astap_wcs_float, format_ini_failure, format_ini_success, format_wcs_fits_block,
    format_wcs_text,
};

/// The real fixtures' CRVAL/CRPIX/CD -- the same solve as
/// `tests/fixtures/reference.ini`/`reference.wcs`/`reference-block.wcs`, so
/// every fixture in this file is cross-checked against the same underlying
/// numbers.
///
/// `crpix` here is the real fixtures' `CRPIX1`/`CRPIX2` (1920.5, 1080.5)
/// MINUS 1: this `Wcs` represents what a real psolve solve would hand to
/// `wcs_solution_cards` (psolve-core's 0-based pixel convention -- see
/// `sidecar.rs`'s "CRPIX convention" module doc), and the byte-exact
/// assertions below only hold if `wcs_solution_cards`'s own `+ 1.0`
/// recovers the real fixtures' 1-based bytes from this 0-based input.
fn wcs_fixture() -> Wcs {
    Wcs {
        crval: [2.5423046742390622E+002, -4.0311880588850023E+001],
        crpix: [1.9195000000000000E+003, 1.0795000000000000E+003],
        cd: [
            [3.5245253250848707E-004, 5.8334097357301367E-004],
            [-5.8335417754934037E-004, 3.5236170894630648E-004],
        ],
    }
}

fn fixture_bytes(name: &str) -> Vec<u8> {
    std::fs::read(format!("tests/fixtures/{name}"))
        .unwrap_or_else(|e| panic!("reading tests/fixtures/{name}: {e}"))
}

fn fixture_text(name: &str) -> String {
    String::from_utf8(fixture_bytes(name))
        .unwrap_or_else(|e| panic!("tests/fixtures/{name} is not valid UTF-8: {e}"))
}

/// The pass-through prefix of the real default-style fixture: every card
/// before ASTAP's own CTYPE1/solution section (ground-truth doc §2a) --
/// what `format_wcs_text`'s `original_header` parameter expects. Real ASTAP
/// downgrades `BITPIX`/`NAXIS` and drops `NAXIS1`/`NAXIS2` from the source
/// FITS file's header before writing a `.wcs` (confirmed by diffing this
/// fixture's header against its source FITS file's -- see
/// `format_wcs_text`'s doc comment), so this fixture is already in that
/// downgraded shape, matching what a real caller must supply.
fn original_header_fixture() -> String {
    let real = fixture_text("reference.wcs");
    let cutoff = real
        .lines()
        .position(|l| l.starts_with("CTYPE1"))
        .expect("fixture must contain a CTYPE1 card");
    real.lines().take(cutoff).collect::<Vec<_>>().join("\n")
}

/// The pass-through header of an **already-solved** frame: the whole real
/// fixture up to (not including) its `END` card, so every WCS keyword ASTAP
/// itself wrote -- `CTYPE`/`CUNIT`/`CRPIX`/`CRVAL`/`CDELT`/`CROTA`/`CD`/
/// `PLTSOLVD`, plus its own `COMMENT cmdline:` block -- is present as a
/// *stale* card ahead of psolve's own.
///
/// This is the realistic case, not an edge case: the frame this fixture came
/// from carries exactly these cards in its own FITS header (ASTAP wrote them
/// there with `-update`), and every one of the 9495 frames in this
/// deployment's corpus is already solved the same way.
/// [`original_header_fixture`] deliberately truncates before `CTYPE1`, which
/// is why the byte-exact tests above could never see a duplicate-key bug.
fn already_solved_header_fixture() -> String {
    let real = fixture_text("reference.wcs");
    real.lines().take_while(|l| !l.starts_with("END")).collect::<Vec<_>>().join("\n")
}

/// A WCS whose every derived value differs from the real fixture's, so an
/// assertion that a card carries "psolve's value" can actually fail if the
/// stale card survived. (`CD` is deliberately a different rotation as well
/// as a different scale -- scaling alone would leave `CROTA1`/`CROTA2`
/// unchanged and two of the checks below vacuous.)
fn distinct_wcs() -> Wcs {
    Wcs {
        crval: [123.456789012345, 12.345678901234],
        crpix: [1023.0, 767.0],
        cd: [[1.0e-4, 2.0e-4], [-2.0e-4, 1.0e-4]],
    }
}

/// Every WCS key both a source header and [`wcs_solution_cards`] can carry.
const WCS_KEYS: [&str; 16] = [
    "CTYPE1", "CTYPE2", "CUNIT1", "CRPIX1", "CRPIX2", "CRVAL1", "CRVAL2", "CDELT1", "CDELT2",
    "CROTA1", "CROTA2", "CD1_1", "CD1_2", "CD2_1", "CD2_2", "PLTSOLVD",
];

// ---------------------------------------------------------------------
// astap_wcs_float: the 12-mantissa-digit number format
// ---------------------------------------------------------------------

/// Byte-exact against the real `.wcs` fixture's own CRPIX/CRVAL/CDELT/CD
/// lines (ground-truth doc §2a).
#[test]
fn astap_wcs_float_matches_real_wcs_bytes() {
    assert_eq!(astap_wcs_float(1920.5), " 1.920500000000E+003");
    assert_eq!(astap_wcs_float(254.23046742390622), " 2.542304674239E+002");
    assert_eq!(astap_wcs_float(-40.311880588850023), "-4.031188058885E+001");
    assert_eq!(astap_wcs_float(6.8154932258843713e-4), " 6.815493225884E-004");
    assert_eq!(astap_wcs_float(-5.8335417754934037e-4), "-5.833541775493E-004");
}

/// 12 mantissa digits (13 total digit characters: 1 leading + 12 after the
/// point) and a 3-digit zero-padded exponent, for a spread of magnitudes --
/// not just the one fixture value.
#[test]
fn astap_wcs_float_has_twelve_mantissa_digits_and_a_three_digit_exponent() {
    for v in [1.0_f64, -1.0, 1e-4, 1e100, -1e-100, 0.0] {
        let s = astap_wcs_float(v);
        let e = s.find('E').expect("must have an exponent");
        assert_eq!(s.len() - e, 5, "{s} must end in E±NNN");
        let mantissa_digits = s[..e].chars().filter(|c| c.is_ascii_digit()).count();
        assert_eq!(mantissa_digits, 13, "{s}: want 1 leading + 12 mantissa digits");
    }
}

/// Same value, printed by both formatters, must differ -- otherwise the
/// digit count was never actually wired to 12.
#[test]
fn astap_wcs_float_differs_from_the_sixteen_digit_ini_style() {
    let v = 254.23046742390622;
    assert_ne!(astap_wcs_float(v), astap_float(v));
}

#[test]
#[should_panic]
fn astap_wcs_float_panics_on_non_finite() {
    astap_wcs_float(f64::NAN);
}

/// `sidecar.rs` also exports the `.ini` writers (`format_ini_success`,
/// `format_ini_failure`); byte-exact coverage for those lives in
/// `tests/sidecar_ini.rs`, which has its own independent `#[path]`-included
/// copy of this same module. Referencing them here too keeps
/// `clippy --all-targets -D warnings`'s per-binary `dead_code` check
/// honest -- see `sidecar_ini.rs`'s reciprocal test for the full
/// explanation.
///
/// This doubles as a real regression guard, not just lint appeasement:
/// factoring `cdelt_crota` out from under `format_ini_success` (so this
/// module's `.wcs` writer could share it) must not have changed the `.ini`
/// writer's output, which this repeats the real byte-exact fixture check
/// for.
#[test]
fn the_ini_writers_are_unaffected_by_the_cdelt_crota_refactor() {
    let real = fixture_text("reference.ini");
    let cmdline = real
        .lines()
        .find(|l| l.starts_with("CMDLINE="))
        .expect("fixture must have a CMDLINE line")
        .trim_start_matches("CMDLINE=");
    assert_eq!(format_ini_success(&wcs_fixture(), cmdline), real);

    let f = format_ini_failure("astap_cli -f x.fits", "Not enough stars.");
    assert_eq!(f, "\nPLTSOLVD=F\nCMDLINE=astap_cli -f x.fits\nERROR=Not enough stars.\n");
}

// ---------------------------------------------------------------------
// format_wcs_text: the default style
// ---------------------------------------------------------------------

/// The default style is newline-terminated text, not padded to a 2880-byte
/// block boundary (ground-truth doc §2a: every real production `.wcs` file
/// on this machine is this style).
#[test]
fn the_default_wcs_is_newline_terminated_text_and_is_not_block_padded() {
    let s = format_wcs_text(&wcs_fixture(), &original_header_fixture());
    assert!(s.contains('\n'), "the default style is newline-terminated text");
    assert_ne!(s.len() % 2880, 0, "the default style is not padded to 2880");
    assert!(s.contains("CRVAL1"));
}

/// The pass-through header is reproduced verbatim, one card per line, ahead
/// of ASTAP's own CTYPE1/solution section.
#[test]
fn the_pass_through_header_is_reproduced_byte_for_byte() {
    let header = original_header_fixture();
    let s = format_wcs_text(&wcs_fixture(), &header);
    for (got, want) in s.lines().zip(header.lines()) {
        assert_eq!(got, want);
    }
}

/// Every structured card this module generates must be byte-identical to
/// the real fixture's corresponding card -- key, 12-digit value, and
/// comment all included, force-padded to exactly 80 bytes exactly as real
/// ASTAP's own writer does even in the newline-terminated default style
/// (ground-truth doc §2a). The 12-digit derivation itself is pinned
/// separately above; this test pins the surrounding FITS-card layout
/// (column positions, quoting, comment text).
#[test]
fn structured_solution_cards_match_the_real_fixture_byte_for_byte() {
    let real = fixture_text("reference.wcs");
    let s = format_wcs_text(&wcs_fixture(), &original_header_fixture());
    for key in [
        "CTYPE1", "CTYPE2", "CUNIT1", "CRPIX1", "CRPIX2", "CRVAL1", "CRVAL2", "CDELT1",
        "CDELT2", "CROTA1", "CROTA2", "CD1_1", "CD1_2", "CD2_1", "CD2_2",
    ] {
        let real_line = real
            .lines()
            .find(|l| l.starts_with(key))
            .unwrap_or_else(|| panic!("real fixture must have a {key} card"));
        let got_line = s
            .lines()
            .find(|l| l.starts_with(key))
            .unwrap_or_else(|| panic!("generated output must have a {key} card"));
        assert_eq!(got_line, real_line, "{key} card must match the real fixture byte-for-byte");
        assert_eq!(got_line.len(), 80, "{key} card must be exactly 80 bytes");
    }
}

/// PLTSOLVD's `KEY=<value field>` portion matches the real fixture exactly;
/// its comment deliberately does not. psolve is not literally
/// `ASTAP_CLI v2026.06.29` -- claiming to be would be a needless
/// impersonation in a field AstroOps' own parser never reads (it parses
/// `KEY=value`, not FITS comments; ground-truth doc §2a).
#[test]
fn pltsolvd_card_matches_key_and_value_but_not_the_astap_version_comment() {
    let real = fixture_text("reference.wcs");
    let s = format_wcs_text(&wcs_fixture(), &original_header_fixture());
    let real_line = real.lines().find(|l| l.starts_with("PLTSOLVD")).expect("real PLTSOLVD card");
    let got_line = s.lines().find(|l| l.starts_with("PLTSOLVD")).expect("generated PLTSOLVD card");
    assert_eq!(got_line.len(), 80);
    assert_eq!(&got_line[..30], &real_line[..30], "PLTSOLVD key=value must match exactly");
    assert_ne!(got_line, real_line, "the ASTAP version-string comment is deliberately not spoofed");
}

/// The 12-mantissa-digit style, isolated from the surrounding FITS-card
/// text (key digits like `CRVAL1`'s own `1`, and comment text, are not part
/// of the number and must not be counted as if they were).
#[test]
fn wcs_values_use_twelve_mantissa_digits_not_the_ini_sixteen() {
    let s = format_wcs_text(&wcs_fixture(), &original_header_fixture());
    let line = s.lines().find(|l| l.starts_with("CRVAL1")).expect("CRVAL1 card must be present");
    let value = astap_wcs_float(wcs_fixture().crval[0]);
    assert!(line.contains(&value), "CRVAL1 card must contain the 12-digit value, got: {line}");
    let ini_style = astap_float(wcs_fixture().crval[0]);
    assert!(!line.contains(&ini_style), "must not use the .ini's 16-digit style");
}

/// A `.wcs` with no solve-status COMMENT at all would be a third,
/// undocumented shape; this module always writes at least one, ending in
/// `END`.
#[test]
fn the_text_style_ends_in_end_after_at_least_one_comment_card() {
    let s = format_wcs_text(&wcs_fixture(), &original_header_fixture());
    let comment_idx = s.find("\nCOMMENT").expect("must have a COMMENT card");
    let end_idx = s.find("\nEND").expect("must have an END card");
    assert!(comment_idx < end_idx, "COMMENT must come before END");
}

// ---------------------------------------------------------------------
// Already-solved frames: exactly one card per key, and it is psolve's.
//
// The bug this pins (final-review C2): the `.wcs` writers used to append
// `wcs_solution_cards` after the pass-through verbatim, so an already-solved
// frame -- which is every frame in this deployment -- got two of each key
// with ASTAP's stale card FIRST. First-match wins in cfitsio's `ffgky`,
// astropy's `Header[key]` and this project's own `FitsHeader::get`, so Siril
// and N.I.N.A. would silently read ASTAP's old solution out of psolve's own
// sidecar. Real ASTAP's `.wcs` for this same frame has exactly one `CRPIX1`.
// ---------------------------------------------------------------------

/// One card per WCS key, and the value is psolve's, not the source
/// header's.
#[test]
fn an_already_solved_header_yields_exactly_one_card_per_wcs_key() {
    let w = distinct_wcs();
    let s = format_wcs_text(&w, &already_solved_header_fixture());

    for key in WCS_KEYS {
        let hits: Vec<&str> = s.lines().filter(|l| l.starts_with(key)).collect();
        assert_eq!(hits.len(), 1, "{key}: want exactly one card, got {}: {hits:?}", hits.len());
    }

    let (cdelt1, cdelt2) = (w.cd[0][0].hypot(w.cd[0][1]), w.cd[1][0].hypot(w.cd[1][1]));
    let deg = 180.0 / std::f64::consts::PI;
    for (key, want) in [
        ("CRPIX1", w.crpix[0] + 1.0),
        ("CRPIX2", w.crpix[1] + 1.0),
        ("CRVAL1", w.crval[0]),
        ("CRVAL2", w.crval[1]),
        ("CDELT1", cdelt1),
        ("CDELT2", cdelt2),
        ("CROTA1", (-w.cd[0][1]).atan2(w.cd[0][0]) * deg),
        ("CROTA2", w.cd[1][0].atan2(w.cd[1][1]) * deg),
        ("CD1_1", w.cd[0][0]),
        ("CD1_2", w.cd[0][1]),
        ("CD2_1", w.cd[1][0]),
        ("CD2_2", w.cd[1][1]),
    ] {
        let line = s.lines().find(|l| l.starts_with(key)).expect("card must be present");
        assert!(
            line.contains(&astap_wcs_float(want)),
            "{key} must carry psolve's own value {}, got: {line}",
            astap_wcs_float(want)
        );
    }

    // The source header's own stale numbers must be gone entirely, not just
    // outranked by a later duplicate.
    for stale in [" 1.920500000000E+003", " 2.542304674239E+002", "-5.885977836767E+001"] {
        assert!(!s.contains(stale), "a stale source-header value survived: {stale}");
    }

    // `PLTSOLVD` is replaced too -- the source card says ASTAP solved it.
    let pltsolvd = s.lines().find(|l| l.starts_with("PLTSOLVD")).expect("PLTSOLVD card");
    assert!(pltsolvd.contains("psolve"), "PLTSOLVD must be psolve's card, got: {pltsolvd}");
    assert!(!pltsolvd.contains("ASTAP_CLI"), "ASTAP's own PLTSOLVD card must not survive");
}

/// Replacing WCS keys must not touch anything else: the capture header's own
/// cards, and ASTAP's repeatable `COMMENT` block, all pass through
/// untouched, while psolve's own solve-marker `COMMENT` appears exactly
/// once.
#[test]
fn merging_an_already_solved_header_leaves_every_other_card_alone() {
    let header = already_solved_header_fixture();
    let s = format_wcs_text(&distinct_wcs(), &header);

    let astap_comments = |t: &str| t.lines().filter(|l| l.starts_with("COMMENT cmdline:")).count();
    assert_eq!(
        astap_comments(&s),
        astap_comments(&header),
        "ASTAP's own COMMENT cards are repeatable and must pass through untouched"
    );
    assert_eq!(
        s.lines().filter(|l| l.trim_end() == "COMMENT Astrometric solution by psolve").count(),
        1,
        "psolve's own solve marker must appear exactly once"
    );

    // Every non-WCS card of the source header survives byte for byte.
    for line in header.lines().filter(|l| !WCS_KEYS.iter().any(|k| l.starts_with(k))) {
        assert!(s.lines().any(|got| got == line), "a non-WCS source card was lost: {line:?}");
    }

    assert!(s.lines().next_back().is_some_and(|l| l.starts_with("END")), "END must still come last");
}

/// The `-wcs` FITS-block style shares the same merge, so it must not
/// duplicate keys either.
#[test]
fn the_fits_block_style_also_yields_one_card_per_key_for_a_solved_header() {
    let b = format_wcs_fits_block(&distinct_wcs(), &already_solved_header_fixture());
    let block = String::from_utf8(b).expect("block must be ASCII");
    for key in WCS_KEYS {
        let n = block.as_bytes().chunks(80).filter(|c| c.starts_with(key.as_bytes())).count();
        assert_eq!(n, 1, "{key}: want exactly one card in the block style, got {n}");
    }
}

// ---------------------------------------------------------------------
// format_wcs_fits_block: the -wcs flag style
// ---------------------------------------------------------------------

/// `-wcs` is a real FITS block: no newlines at all, an exact multiple of
/// 2880, and every card exactly 80 bytes.
///
/// Finding `END` must be card-aligned, not a raw substring search: the real
/// pass-through header this fixture carries includes an `EXTEND` card,
/// which *contains* the substring `"END"` well before the real `END` card
/// -- a naive `text.find("END")` (the brief's original sketch) matches
/// there instead. Real bytes exposed that, so the search below looks for a
/// card that IS `END`, not a card that merely contains it.
#[test]
fn the_wcs_flag_emits_a_true_fits_block() {
    let b = format_wcs_fits_block(&wcs_fixture(), &original_header_fixture());
    assert_eq!(b.len() % 2880, 0, "must be a whole number of 2880-byte blocks");
    assert!(!b.contains(&b'\n'), "a FITS block contains no newlines");
    for card in b.chunks(80) {
        assert_eq!(card.len(), 80);
        assert!(card.is_ascii(), "FITS cards are ASCII");
    }
    let end = b
        .chunks(80)
        .position(|c| c.starts_with(b"END") && c[3..].iter().all(|&x| x == b' '))
        .expect("must contain an END card")
        * 80;
    assert!(
        b[end + 3..].iter().all(|&c| c == b' '),
        "everything after END must be blank padding"
    );
}

/// Trap 1: a card whose natural text would exceed 80 bytes must be
/// truncated, not spill into the next card. A long pass-through header line
/// (e.g. the real fixture's genuinely-84-byte COMMENT card, ground-truth
/// doc §2a) is exactly the kind of input that could do this.
#[test]
fn an_overlong_pass_through_card_is_truncated_not_overflowed() {
    let long_line = "COMMENT ".to_string() + &"x".repeat(100);
    let header = format!("SIMPLE  =                    T\n{long_line}");
    let b = format_wcs_fits_block(&wcs_fixture(), &header);
    let cards: Vec<&[u8]> = b.chunks(80).collect();
    assert_eq!(cards[1].len(), 80, "the overlong card must still be exactly 80 bytes");
    assert_eq!(
        &cards[1][..8],
        b"COMMENT ",
        "truncation must cut from the end, not corrupt the start of the next card"
    );
    // The next card must be the untouched start of the WCS solution
    // section, not a fragment of the overflowed COMMENT text.
    assert!(cards[2].starts_with(b"CTYPE1"), "overflow must not spill into the next card");
}

/// Trap 2: everything after `END` must be spaces, never NUL -- some
/// writers pad with zero bytes, which is not FITS-conformant and would
/// break a strict reader.
#[test]
fn padding_after_end_is_spaces_never_nul() {
    let b = format_wcs_fits_block(&wcs_fixture(), &original_header_fixture());
    assert!(!b.contains(&0u8), "padding must never use NUL bytes");
}

/// Structural sanity check on the checked-in `-wcs`-flag fixture itself,
/// pinning the facts this module's design was reverse-engineered from (no
/// real production file on this machine uses this style -- see this file's
/// module doc comment for how it was reproduced).
#[test]
fn the_real_wcs_flag_fixture_is_a_conformant_fits_block() {
    let b = fixture_bytes("reference-block.wcs");
    assert_eq!(b.len(), 8640, "3 x 2880-byte blocks, matching the real -wcs run");
    assert_eq!(b.len() % 2880, 0);
    assert!(!b.contains(&b'\n'));
    let cards: Vec<&[u8]> = b.chunks(80).collect();
    assert_eq!(cards.len(), 108);
    let end_idx = cards.iter().position(|c| c.starts_with(b"END")).expect("must have an END card");
    assert_eq!(end_idx, 88, "matches the real -wcs run's END card position");
    for c in &cards[end_idx + 1..] {
        assert_eq!(*c, [b' '; 80], "cards after END must be blank, not NUL-padded");
    }
}

/// The same structured cards `format_wcs_text` writes, just repacked with
/// no newlines and forced to exactly 80 bytes each -- the two formatters
/// must not structurally diverge on the WCS solution itself, only on
/// packing.
#[test]
fn the_fits_block_carries_the_same_solution_cards_as_the_text_style() {
    let header = original_header_fixture();
    let text = format_wcs_text(&wcs_fixture(), &header);
    let block = format_wcs_fits_block(&wcs_fixture(), &header);
    let block_text = String::from_utf8(block).expect("block must be ASCII");
    for key in ["CRVAL1", "CRVAL2", "CRPIX1", "CRPIX2", "CD1_1", "CD2_2", "PLTSOLVD"] {
        let text_line = text.lines().find(|l| l.starts_with(key)).expect("text style must have this card");
        let card = block_text
            .as_bytes()
            .chunks(80)
            .map(|c| std::str::from_utf8(c).unwrap())
            .find(|c| c.starts_with(key))
            .unwrap_or_else(|| panic!("block style must have a {key} card"));
        assert_eq!(card, text_line, "{key} must be identical content, just repacked");
    }
}
