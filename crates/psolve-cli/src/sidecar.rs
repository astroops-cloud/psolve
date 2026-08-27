//! ASTAP-compatible `.ini` and `.wcs` sidecar formatting.
//!
//! AstroOps parses `filename.ini` after every solve attempt, so the `.ini`
//! writers must reproduce the real `astap_cli` output byte-exactly (ground
//! truth: `docs/superpowers/2026-08-14-astap-format-facts.md` §1, gathered
//! from 28 real `PLTSOLVD=T` and 101 real `PLTSOLVD=F` files). Two `.ini`
//! formats, not one with fields omitted: see `format_ini_failure`'s doc
//! comment for the leading-blank-line quirk that makes them structurally
//! different.
//!
//! `.wcs` is a second, structurally different pair of formats (§2): a
//! default newline-terminated text style (100% of real files on this
//! machine) and a true FITS-block style behind the `-wcs` flag (never
//! observed on disk here, reproduced locally against the real binary
//! instead -- see `format_wcs_fits_block`'s doc comment). Both `.wcs`
//! writers share [`wcs_solution_cards`] with the `.ini` writer's
//! [`cdelt_crota`] derivation so the two sidecars cannot silently drift
//! apart, and share [`astap_float_digits`] (16 digits for `.ini`, 12 for
//! `.wcs`) for the same reason.
//!
//! `psolve-cli` has no `[lib]` target (only `[[bin]]`); `main.rs` now
//! declares `mod sidecar;` (Task 9), so this file is a real part of the
//! `psolve` binary, not test-only code. `fits_update.rs`'s `-update` write
//! path is the first real caller -- of [`wcs_solution_cards`] and
//! [`pad_or_truncate_card`], both `pub(crate)` for exactly that reason.
//!
//! The `.ini`/`.wcs` *file* writers (`format_ini_success`,
//! `format_ini_failure`, `format_wcs_text`, `format_wcs_fits_block`) and the
//! `.ini` number format (`astap_float`) are wired into `astap_cmd`'s
//! dispatch in `main.rs` (M3 progress ledger, Task 10). They are also, as
//! before, directly exercised by the integration test files
//! (`tests/sidecar_ini.rs`, `tests/sidecar_wcs.rs`), which pull this module
//! in directly via `#[path = "../src/sidecar.rs"] mod sidecar;` (the same
//! cross-directory trick `astap_args.rs`'s tests use, since there is no
//! `[lib]` target for an integration test to link against) and call every
//! function from genuine `#[test]` fns.
//!
//! ## CRPIX convention (decided in Task 11's fix round)
//!
//! Every `Wcs` this module receives (`&s.wcs` from a real `Outcome::Solved`,
//! passed straight through by `main.rs`) carries `crpix` in psolve-core's
//! internal, 0-BASED pixel convention -- `extract.rs` centroids blobs over
//! array indices `0..nx`/`0..ny`, and `fit_tan` fits `Wcs.crpix` in that same
//! frame (`cmd_solve.rs`'s `field.center` computation documents the same
//! fact for the native JSON's image-centre calculation). FITS's own
//! convention is 1-BASED: pixel 1 is the first column/row, so `CRPIX1=1.0`
//! names the centre of the first pixel, not its edge.
//!
//! Every function below that writes a `CRPIX1`/`CRPIX2` FITS card --
//! [`format_ini_success`] and [`wcs_solution_cards`] (shared by both `.wcs`
//! writers and by `fits_update.rs`'s `-update` path) -- therefore adds
//! `+ 1.0` to `w.crpix` at the point of formatting. This is the ONE place
//! that conversion happens; `Wcs` itself is never mutated, and every other
//! WCS quantity (`crval`, `cd`, and everything derived from `cd` alone --
//! `cdelt`/`crota`) is convention-agnostic (a pixel *address* is the only
//! thing 0- vs 1-based numbering can disagree about). Getting this wrong
//! shifts every sidecar's and every `-update`'d header's CRPIX by exactly
//! one pixel with no error, no warning, and a solve that still "looks"
//! right -- found via `cmd_solve.rs`'s parallel bug (`field.center` off by
//! half a pixel from evaluating `pix_to_radec` at the FITS-convention centre
//! `nx/2` instead of the 0-based one, `(nx-1)/2`) surfacing as an
//! unexplained ~1.6" systematic term in Task 11's agreement numbers.

use psolve_core::fit::Wcs;

/// ASTAP's own float format: `D.DDDDDDDDDDDDDDDDE±NNN` -- one mantissa digit,
/// decimal point, 16 more mantissa digits, `E`, an explicit exponent sign,
/// and a 3-digit zero-padded exponent. Positive (and zero) values get a
/// single leading space where a negative value's `-` would sit, so every
/// value lines up in a column. Confirmed against real bytes (ground-truth
/// doc §1c/§1a): Rust's own `{:E}` gives `1.9205E3`, nothing like this.
///
/// # Panics
/// On a non-finite `v`. ASTAP's number format has no representation for
/// NaN/infinity, and this function's whole job is to never emit a plausible-
/// looking-but-wrong key -- Task 9 wires this to live solver output, and a
/// stray NaN reaching the writer must fail loudly here rather than produce a
/// `" NaNE+000"`/`" infE+000"` line the AstroOps consumer silently misparses.
/// Not reachable from a successful solve today (`Wcs`'s fields are always
/// finite by construction; see `fit.rs`), so this is a defensive boundary,
/// not a path any current caller can hit.
// `.ini` writing is wired into `astap_cmd`'s dispatch (Task 10). Also
// exercised directly by `tests/sidecar_ini.rs`.
pub fn astap_float(v: f64) -> String {
    astap_float_digits(v, 16)
}

/// The `.wcs` sidecar's number format: identical layout to [`astap_float`]
/// but with **12** mantissa digits instead of 16 (ground-truth doc §2a: the
/// `.ini` writer prints `D.DDDDDDDDDDDDDDDDE±NNN`, the `.wcs` writer prints
/// `D.DDDDDDDDDDDDE±NNN`). Both share [`astap_float_digits`] rather than two
/// independent near-copies of the same formatter -- exactly the kind of
/// duplication that let the `.ini` writer and this one drift apart if
/// maintained separately.
///
/// Verified against real bytes: the real `.ini` and `.wcs` sidecars for the
/// same solve store the *same* underlying `f64`s (confirmed by rounding the
/// `.ini` fixture's 16-digit values to 12 digits and diffing against the
/// real `.wcs` fixture's CRPIX/CRVAL/CDELT/CROTA/CD lines -- exact, byte for
/// byte, including the CDELT/CROTA row-based decomposition from
/// `cdelt_crota`), so no separate rounding investigation was needed here:
/// fewer printed digits of the same value round the same way.
pub fn astap_wcs_float(v: f64) -> String {
    astap_float_digits(v, 12)
}

/// Shared implementation behind [`astap_float`] and [`astap_wcs_float`]:
/// ASTAP's number format is `D.D{digits}E±NNN` -- one mantissa digit,
/// decimal point, `digits` more mantissa digits, `E`, an explicit exponent
/// sign, and a 3-digit zero-padded exponent. Positive (and zero) values get
/// a single leading space where a negative value's `-` would sit, so every
/// value lines up in a column. Confirmed against real bytes (ground-truth
/// doc §1c/§1a/§2a): Rust's own `{:E}` gives `1.9205E3`, nothing like this.
///
/// # Panics
/// On a non-finite `v`. ASTAP's number format has no representation for
/// NaN/infinity, and this function's whole job is to never emit a plausible-
/// looking-but-wrong key -- Task 9 wires this to live solver output, and a
/// stray NaN reaching the writer must fail loudly here rather than produce a
/// `" NaNE+000"`/`" infE+000"` line the AstroOps consumer silently misparses.
/// Not reachable from a successful solve today (`Wcs`'s fields are always
/// finite by construction; see `fit.rs`), so this is a defensive boundary,
/// not a path any current caller can hit.
fn astap_float_digits(v: f64, digits: usize) -> String {
    assert!(v.is_finite(), "astap_float: ASTAP's number format has no non-finite representation, got {v}");
    // `{:.Ne}` gives exactly N digits after the decimal point in the
    // mantissa (verified: round-trips a real fixture's digits exactly, e.g.
    // parsing "6.8154932258843713E-004" and reformatting reproduces every
    // digit) and a bare, unpadded, unsigned-for-positive exponent like `e3`
    // or `e-4` -- the part this function exists to fix up.
    let formatted = format!("{v:.digits$e}");
    let (mantissa, exp) = formatted.split_once('e').unwrap_or((formatted.as_str(), "0"));
    let exp_val: i32 = exp.parse().unwrap_or(0);
    let sign = if exp_val < 0 { '-' } else { '+' };
    let padded_exp = format!("{:03}", exp_val.abs());
    if let Some(stripped) = mantissa.strip_prefix('-') {
        format!("-{stripped}E{sign}{padded_exp}")
    } else {
        format!(" {mantissa}E{sign}{padded_exp}")
    }
}

/// ASTAP's derived CDELT/CROTA from the solved CD matrix. CDELT1/CDELT2/
/// CROTA1/CROTA2 are not part of `Wcs` (which only carries CRVAL/CRPIX/CD)
/// and are not the WCS-standard CDELT+PC pair `cmd_solve.rs` emits for spec
/// section 7.2 -- that pair is column-based (CDELT1 negative, built for a
/// CD = CDELT*PC identity a downstream PC-only consumer needs).
/// Reverse-engineering the real fixture bytes shows ASTAP uses a different,
/// ROW-based, per-axis decomposition instead:
///   CDELT1 = hypot(CD1_1, CD1_2)        CDELT2 = hypot(CD2_1, CD2_2)
///   CROTA1 = atan2(-CD1_2, CD1_1)       CROTA2 = atan2(CD2_1, CD2_2)
/// (both CDELT values always non-negative -- confirmed against the real
/// fixture, where both print positive, unlike the column-based pair). This
/// was verified bit-for-bit against the real fixture's stored doubles: all
/// four values -- CDELT1, CDELT2, CROTA1, CROTA2 -- reproduce ASTAP's own
/// bytes EXACTLY, given one deliberate choice in how the radians->degrees
/// conversion is written. `f64::to_degrees()` (a single multiply by the
/// pre-rounded constant `180/PI`) is 1 ULP off the real file on both CROTA
/// values; `angle * 180.0 / std::f64::consts::PI` (multiply, then divide --
/// two roundings, matching what ASTAP's Pascal `arctan2(...)*180/pi` also
/// does) lands on the real bytes exactly. This was found empirically, not
/// derived: `a / PI * 180.0` (the other association) matches CROTA1 but not
/// CROTA2, so the exact operation order below is load-bearing, not
/// cosmetic -- see `tests` for the pinned evidence.
///
/// Shared by both the `.ini` and `.wcs` writers -- the real fixtures for the
/// same solve carry byte-identical derived values (down to the last of the
/// `.wcs` format's fewer printed digits), so this derivation must live in
/// exactly one place or the two sidecars can silently drift apart.
fn cdelt_crota(w: &Wcs) -> (f64, f64, f64, f64) {
    let cdelt1 = w.cd[0][0].hypot(w.cd[0][1]);
    let cdelt2 = w.cd[1][0].hypot(w.cd[1][1]);
    let crota1 = (-w.cd[0][1]).atan2(w.cd[0][0]) * 180.0 / std::f64::consts::PI;
    let crota2 = w.cd[1][0].atan2(w.cd[1][1]) * 180.0 / std::f64::consts::PI;
    (cdelt1, cdelt2, crota1, crota2)
}

/// Format a solved WCS as ASTAP's success-case `.ini`: 14 keys, fixed order,
/// LF only, trailing newline. No `[section]` headers, no blank lines between
/// keys (ground-truth doc §1a/§1b, verified across all 28 real files).
/// See [`cdelt_crota`] for where CDELT1/CDELT2/CROTA1/CROTA2 come from.
///
/// `w.crpix` is psolve-core's 0-based pixel address; `+ 1.0` here is the
/// crossing into FITS's 1-based `CRPIX1`/`CRPIX2` -- see this module's own
/// doc comment ("CRPIX convention").
// Wired into `astap_cmd`'s dispatch (Task 10). Also exercised directly by
// `tests/sidecar_ini.rs`.
pub fn format_ini_success(w: &Wcs, cmdline: &str) -> String {
    let (cdelt1, cdelt2, crota1, crota2) = cdelt_crota(w);

    format!(
        "PLTSOLVD=T\n\
         CRPIX1={}\n\
         CRPIX2={}\n\
         CRVAL1={}\n\
         CRVAL2={}\n\
         CDELT1={}\n\
         CDELT2={}\n\
         CROTA1={}\n\
         CROTA2={}\n\
         CD1_1={}\n\
         CD1_2={}\n\
         CD2_1={}\n\
         CD2_2={}\n\
         CMDLINE={cmdline}\n",
        astap_float(w.crpix[0] + 1.0),
        astap_float(w.crpix[1] + 1.0),
        astap_float(w.crval[0]),
        astap_float(w.crval[1]),
        astap_float(cdelt1),
        astap_float(cdelt2),
        astap_float(crota1),
        astap_float(crota2),
        astap_float(w.cd[0][0]),
        astap_float(w.cd[0][1]),
        astap_float(w.cd[1][0]),
        astap_float(w.cd[1][1]),
    )
}

/// Format ASTAP's failure-case `.ini`. Structurally different from the
/// success case, not the same format with fields dropped (ground-truth doc
/// §1e, verified against two independent real failure files):
///
/// - Byte 0 of the file is a literal `\n` -- a genuine blank first line,
///   reproduced independently in two unrelated real failures with different
///   `ERROR=` text, so it is an ASTAP quirk (an extra `writeln` on the
///   failure path), not a copy artifact. A consumer that skips line 0 would
///   break on a file that lacked it, so this is reproduced exactly rather
///   than "cleaned up".
/// - `CMDLINE` comes *before* `ERROR` -- the opposite order from what the
///   success case's key list would suggest.
/// - None of CRPIX/CRVAL/CDELT/CROTA/CD appear at all: a failed solve has no
///   WCS to report, not a WCS of zeros.
// Wired into `astap_cmd`'s dispatch (Task 10). Also exercised directly by
// `tests/sidecar_ini.rs`.
pub fn format_ini_failure(cmdline: &str, error: &str) -> String {
    format!("\nPLTSOLVD=F\nCMDLINE={cmdline}\nERROR={error}\n")
}

/// Right-justify a value into FITS's 21-column value field (the columns a
/// numeric or boolean card's value occupies between `=` and ` / <comment>`).
/// Ground-truth doc §2a: e.g. `CRPIX1  =  1.920500000000E+003 / ...` has
/// `  1.920500000000E+003` (21 chars) between the `=` and the ` / `.
fn value_field_right(v: &str) -> String {
    format!("{v:>21}")
}

/// Left-justify a FITS string value in the same 21-column field: a leading
/// space, then `'content'` with content padded to at least 8 chars (the FITS
/// minimum string-value width), matching real ASTAP bytes exactly (ground-
/// truth doc §2a, e.g. `CUNIT1  = 'deg     '           / Unit of...`).
fn value_field_string(content: &str) -> String {
    let quoted = format!("'{content:<8}'");
    format!(" {quoted:<20}")
}

/// One `KEY   =<21-char value> / <comment>` FITS card, force-padded (or --
/// the trap this exists to avoid -- truncated, never silently overflowed)
/// to exactly 80 bytes. Every structured card ASTAP itself writes (as
/// opposed to a free-text COMMENT card) is exactly 80 bytes in the real
/// fixtures, including in the newline-terminated default style -- only
/// COMMENT cards are allowed to be a different length there (ground-truth
/// doc §2a).
fn kv_card(key: &str, value_field: &str, comment: &str) -> String {
    let mut c = format!("{key:<8}={value_field} / {comment}");
    match c.len().cmp(&80) {
        std::cmp::Ordering::Less => c.push_str(&" ".repeat(80 - c.len())),
        std::cmp::Ordering::Greater => c.truncate(80),
        std::cmp::Ordering::Equal => {}
    }
    c
}

/// The ordered list of cards this module generates for a solved `.wcs`:
/// CTYPE/CUNIT, the WCS solution (12 mantissa digits -- [`astap_wcs_float`]),
/// `PLTSOLVD`, a solve `COMMENT`, and `END`. Does NOT include the pass-
/// through original header -- the two formatters combine that differently
/// (see [`format_wcs_text`] and [`format_wcs_fits_block`]), so it stays out
/// of the list they share.
///
/// Real ASTAP appends a `COMMENT cmdline:...` block reporting the exact
/// invocation and solve statistics (elapsed time, offset, mount delta -- see
/// ground-truth doc §2a). That block bundles together two genuinely
/// different kinds of "missing" data, and only one of them is actually
/// unavailable: the elapsed solve time and the mount offset are not
/// computed or stored anywhere in this codebase, so this function truly has
/// no value to put there. The command-line string is not in that category --
/// `AstapArgs.cmdline` already carries the full invocation (program path
/// plus argv, see `astap_args.rs`); it simply is not threaded into this
/// function's signature. A caller that wants a byte-exact
/// `COMMENT cmdline:...` line can pass `AstapArgs.cmdline` in once something
/// needs that; this function does not invent it in the meantime. The single
/// generic `COMMENT` below keeps the section structurally present (a `.wcs`
/// with solution keywords and no COMMENT at all would be a third,
/// undocumented shape) without inventing statistics this function cannot
/// know.
// `pub(crate)`, not private: `fits_update.rs`'s `-update` write path builds
// the same WCS cards this produces for the `.wcs` sidecar, so the two share
// this list rather than risk drifting apart (see the module doc's opening
// paragraph on this file's shared-derivation policy). This is also the
// other of the two places (with `format_ini_success`) that add `+ 1.0` to
// `w.crpix` crossing from psolve-core's 0-based convention into FITS's
// 1-based one -- see the module doc's "CRPIX convention" section. Because
// `fits_update.rs` calls this same function, that `-update`'s in-place
// header write gets the same conversion for free, not a second copy of it.
pub(crate) fn wcs_solution_cards(w: &Wcs) -> Vec<String> {
    let (cdelt1, cdelt2, crota1, crota2) = cdelt_crota(w);
    vec![
        kv_card("CTYPE1", &value_field_string("RA---TAN"), "first parameter RA,    projection TANgential"),
        kv_card("CTYPE2", &value_field_string("DEC--TAN"), "second parameter DEC,  projection TANgential"),
        kv_card("CUNIT1", &value_field_string("deg"), "Unit of coordinates"),
        kv_card("CRPIX1", &value_field_right(&astap_wcs_float(w.crpix[0] + 1.0)), "X of reference pixel"),
        kv_card("CRPIX2", &value_field_right(&astap_wcs_float(w.crpix[1] + 1.0)), "Y of reference pixel"),
        kv_card("CRVAL1", &value_field_right(&astap_wcs_float(w.crval[0])), "RA of reference pixel (deg)"),
        kv_card("CRVAL2", &value_field_right(&astap_wcs_float(w.crval[1])), "DEC of reference pixel (deg)"),
        kv_card("CDELT1", &value_field_right(&astap_wcs_float(cdelt1)), "X pixel size (deg)"),
        kv_card("CDELT2", &value_field_right(&astap_wcs_float(cdelt2)), "Y pixel size (deg)"),
        kv_card("CROTA1", &value_field_right(&astap_wcs_float(crota1)), "Image twist of X axis        (deg)"),
        kv_card("CROTA2", &value_field_right(&astap_wcs_float(crota2)), "Image twist of Y axis        (deg)"),
        kv_card("CD1_1", &value_field_right(&astap_wcs_float(w.cd[0][0])), "CD matrix to convert (x,y) to (Ra, Dec)"),
        kv_card("CD1_2", &value_field_right(&astap_wcs_float(w.cd[0][1])), "CD matrix to convert (x,y) to (Ra, Dec)"),
        kv_card("CD2_1", &value_field_right(&astap_wcs_float(w.cd[1][0])), "CD matrix to convert (x,y) to (Ra, Dec)"),
        kv_card("CD2_2", &value_field_right(&astap_wcs_float(w.cd[1][1])), "CD matrix to convert (x,y) to (Ra, Dec)"),
        kv_card("PLTSOLVD", &value_field_right("T"), "Astrometric solved by psolve"),
        PSOLVE_COMMENT_TEXT.to_string(),
        format!("{:<80}", "END"),
    ]
}

/// The exact text of the `COMMENT` card [`wcs_solution_cards`] adds to mark
/// a psolve solve. Shared by [`merge_cards_by_key`] (to recognise and
/// collapse psolve's own earlier card on a re-solve) and by the card list
/// itself, so the recogniser can never drift from what is actually emitted.
///
/// `pub(crate)`, not private: `fits_update.rs`'s `-update` write path passes
/// it to the same merge.
pub(crate) const PSOLVE_COMMENT_TEXT: &str = "COMMENT Astrometric solution by psolve";

/// A text card's FITS keyword: its first 8 bytes, trimmed. Byte-based (not
/// `&str` slicing) so a pass-through card carrying non-ASCII bytes can never
/// panic on a multi-byte UTF-8 boundary; FITS cards are ASCII by spec, so
/// real input never exercises that.
fn card_key_str(card: &str) -> String {
    let b = card.as_bytes();
    let n = b.len().min(8);
    String::from_utf8_lossy(&b[..n]).trim().to_string()
}

/// A text card's full content with trailing spaces trimmed -- used only to
/// recognise psolve's own solve-marker `COMMENT` by its exact text, never to
/// identify any other card.
fn card_text_str(card: &str) -> String {
    card.trim_end().to_string()
}

/// Replace-by-key merge of a solved WCS's cards into an existing card list.
/// **The single implementation of this policy in the crate**: both `.wcs`
/// writers below and `fits_update.rs`'s `-update` in-place header rewrite
/// call it, over their two different card representations (`String` here,
/// byte-exact `[u8; 80]` there) via the `key_of`/`text_of` accessors.
///
/// It exists because appending unconditionally is silently wrong on an
/// already-solved frame -- and in this deployment *every* frame is already
/// solved, since ASTAP wrote its own solution into each one. A header that
/// already carries `CRPIX1` would end up with two, the stale one first, and
/// first-match-wins in cfitsio's `ffgky`, astropy's `Header[key]` and this
/// project's own `FitsHeader::get`: the consumer would silently read the old
/// solution out of psolve's own output. So:
///
/// - A keyed card whose key already exists **replaces that card in place**
///   (position preserved); otherwise it is appended.
/// - `HISTORY` and blank-keyword cards are genuinely repeatable by FITS
///   convention and are always appended.
/// - `COMMENT` cards are repeatable too and are appended -- with one
///   deliberate exception: psolve's own solve marker
///   ([`PSOLVE_COMMENT_TEXT`]) has **every** existing copy removed and
///   exactly one re-appended, so a header that already carried duplicates
///   self-heals to one rather than never converging.
/// - `END` is always appended, never treated as a key to replace: it is a
///   terminator, not a keyword. (`fits_update.rs` filters `END` out before
///   calling this at all, since `pack_header` writes its own.)
pub(crate) fn merge_cards_by_key<T>(
    mut original: Vec<T>,
    solution: impl IntoIterator<Item = T>,
    key_of: impl Fn(&T) -> String,
    text_of: impl Fn(&T) -> String,
) -> Vec<T> {
    for card in solution {
        let key = key_of(&card);
        if key.is_empty() || key == "HISTORY" || key == "END" {
            original.push(card);
            continue;
        }
        if key == "COMMENT" {
            if text_of(&card) == PSOLVE_COMMENT_TEXT {
                original.retain(|c| !(key_of(c) == "COMMENT" && text_of(c) == PSOLVE_COMMENT_TEXT));
            }
            original.push(card);
            continue;
        }
        match original.iter().position(|c| key_of(c) == key) {
            Some(pos) => original[pos] = card,
            None => original.push(card),
        }
    }
    original
}

/// The complete ordered card list a `.wcs` sidecar carries: the caller's
/// pass-through of the original capture header (one already-formatted card
/// per line) with this solve's own WCS cards merged in by
/// [`merge_cards_by_key`] -- **not** concatenated, which would leave an
/// already-solved frame's stale `CRPIX1`/`CRVAL1`/… ahead of psolve's own.
///
/// Cards are kept at their natural length here (the text style writes them
/// that way; the FITS-block style pads them itself), so a pass-through card
/// is reproduced byte for byte.
fn merged_wcs_cards(w: &Wcs, original_header: &str) -> Vec<String> {
    let original: Vec<String> = original_header.lines().map(String::from).collect();
    merge_cards_by_key(original, wcs_solution_cards(w), |c| card_key_str(c), |c| card_text_str(c))
}

/// Format a solved WCS as ASTAP's **default** `.wcs` sidecar: the style
/// every real production file on this machine actually is (ground-truth doc
/// §2a) -- FITS-card-styled **text**, LF after each card, and deliberately
/// **not** padded to a 2880-byte block boundary. `original_header` is the
/// caller's pass-through of the original capture header, one already-
/// formatted card per line, which this solve's WCS cards are **merged**
/// into by key rather than appended after (see [`merged_wcs_cards`]: an
/// already-solved frame's own stale `CRPIX1` must not survive ahead of
/// psolve's). (ASTAP's own `.wcs` writer downgrades
/// `BITPIX`/`NAXIS` and drops `NAXIS1`/`NAXIS2` from the source FITS file's
/// header before writing them here, since a `.wcs` describes no pixel data
/// of its own -- confirmed by diffing the real `.wcs` fixture's header
/// prefix against its source FITS file's; that transform is the caller's
/// job, this function only appends what comes after it).
///
/// Real ASTAP lets some of its own COMMENT cards run long (one observed
/// fixture has an 84-byte COMMENT card) or short (a final, un-padded 57-byte
/// one) -- this function reproduces that latitude by writing every card at
/// its natural length, padding only the structured cards `.wcs`'s own
/// writer always pads to 80 (`kv_card` and the literal `END` line).
// Wired into `astap_cmd`'s dispatch (Task 10). Also exercised directly by
// `tests/sidecar_wcs.rs`.
pub fn format_wcs_text(w: &Wcs, original_header: &str) -> String {
    let mut out = String::new();
    for card in merged_wcs_cards(w, original_header) {
        out.push_str(&card);
        out.push('\n');
    }
    out
}

/// Format a solved WCS as ASTAP's `-wcs`-flag `.wcs` sidecar: a genuine FITS
/// header block. Reproduced locally against the real `astap_cli` binary
/// (ground-truth doc §2b, and this repo's fixture
/// `tests/fixtures/reference-block.wcs`, generated the same way): **zero
/// newlines**, every card force-padded or truncated to **exactly 80 bytes**
/// -- the same underlying key-merged cards [`format_wcs_text`] writes (see
/// [`merged_wcs_cards`]), just repacked -- and the whole thing padded with blank (all-space, never NUL) 80-byte
/// cards out to a **whole multiple of 2880 bytes**.
///
/// No real production file on this machine uses this style (every real
/// `.wcs` file found under `~/astroops` is the default text style;
/// see [`format_wcs_text`]'s doc comment) -- `-wcs` is a documented ASTAP
/// flag this function supports for completeness, not something AstroOps'
/// actual traffic exercises today.
// Wired into `astap_cmd`'s dispatch (Task 10). Also exercised directly by
// `tests/sidecar_wcs.rs`.
pub fn format_wcs_fits_block(w: &Wcs, original_header: &str) -> Vec<u8> {
    let cards = merged_wcs_cards(w, original_header);

    let mut out = Vec::with_capacity(cards.len() * 80);
    for card in &cards {
        out.extend_from_slice(&pad_or_truncate_card(card));
    }
    // FITS-block padding out to the next 2880-byte boundary. Deliberately
    // spaces (b' '), never NUL: some writers pad with zero bytes, which is
    // not FITS-conformant and would break a strict reader (ground-truth doc
    // §2b: the real block's padding cards are all-space).
    while !out.len().is_multiple_of(2880) {
        out.extend_from_slice(&[b' '; 80]);
    }
    out
}

/// Force one logical card to exactly 80 bytes: pad a short one with spaces,
/// and -- the FITS-block trap this exists to close -- truncate a long one
/// rather than let it silently overflow into the next card's 80 bytes.
/// Operates on raw bytes (not `char`s) so a card built from untrusted
/// pass-through header text can never panic on a multi-byte UTF-8 boundary;
/// the result is intentionally allowed to end in a split byte sequence
/// (FITS cards are ASCII by spec, so real input never exercises this).
// `pub(crate)`: `fits_update.rs`'s `-update` write path reuses this to build
// the same 80-byte cards it merges into an existing header, rather than a
// near-duplicate copy of the same padding/truncation rule.
pub(crate) fn pad_or_truncate_card(card: &str) -> [u8; 80] {
    let mut bytes = [b' '; 80];
    let src = card.as_bytes();
    let n = src.len().min(80);
    bytes[..n].copy_from_slice(&src[..n]);
    bytes
}
