//! ASTAP-compatible command-line surface.
//!
//! AstroOps invokes the real `astap_cli` binary with a single-dash flag
//! grammar completely different from psolve's own `--long` native flags
//! (`crate::cmd_solve`). This module parses that grammar so psolve can be a
//! drop-in replacement: same argv in, same semantics out. Two unit
//! conversions are the entire point of this module and are confirmed against
//! real recorded ASTAP invocations on this machine, not inferred from the
//! flag names -- see `hint_degrees` below.
//!
//! `psolve-cli` has no `[lib]` target (only `[[bin]]`), so an external
//! `tests/` integration test cannot link `parse_astap`/`hint_degrees` -- there
//! is nothing to link against. Unit tests live in the co-located `mod tests`
//! at the bottom of this file, matching the pattern already used by
//! `cmd_solve.rs` for the same reason. (Black-box tests that spawn the
//! compiled binary, e.g. `tests/cli_solve.rs`, still work for the mode
//! dispatch in `main.rs` -- see `tests/astap_cli.rs`.)

/// A parsed ASTAP-mode invocation. Field types mirror ASTAP's own units
/// where ASTAP has one (`ra_hours`, `spd_deg`) rather than pre-converting, so
/// that a reader comparing this struct against a real `CMDLINE=` string can
/// check it directly.
#[derive(Debug, Clone)]
pub struct AstapArgs {
    pub file: String,
    pub radius_deg: f64,
    pub ra_hours: Option<f64>,
    pub spd_deg: Option<f64>,
    pub fov_deg: Option<f64>,
    pub db_dir: Option<String>,
    pub out_base: Option<String>,
    pub update: bool,
    pub wcs_fits_block: bool,
    // `-z`/`-s`/`-t`/`-m` are parsed and validated so a well-formed real
    // ASTAP invocation is never rejected on their account (and so they
    // round-trip byte-exact into `cmdline`, which the `.ini`'s `CMDLINE=`
    // key echoes back), but Task 10's dispatch does not consume them: none
    // of `-z` (downsample -- psolve-core has no downsampling stage at all),
    // `-t` (ASTAP's own quad-match tolerance), or `-m` (ASTAP's own minimum
    // star size in arcsec) has an established, verified equivalent among
    // psolve-core's own tunables (`MatchParams::code_tol`,
    // `ExtractParams::min_pix`, etc. measure different, non-interchangeable
    // things), and guessing a mapping without the kind of ground-truth
    // confirmation `hint_degrees` required for `-ra`/`-spd` risks silently
    // degrading the solve rate Task 11's agreement run gates on. `-s`
    // (max stars) is the closest case -- it plausibly maps to
    // `ExtractParams::keep` -- but is left unwired for the same reason:
    // unverified against real behaviour, not requested by this task's
    // brief, and safer deferred than guessed. Silently discarding an
    // explicitly-passed flag is not silent to the OPERATOR, though:
    // `main.rs`'s `astap_cmd` (`warn_about_unwired_flags`) prints a
    // `psolve: warning:` line to stderr naming exactly which of these four
    // were present in this invocation, every time any of them is -- fix
    // round 1 of the M3 Task 10 review (a source comment here is not a
    // signal a pipeline operator watching stderr can see).
    #[allow(dead_code)]
    pub downsample: Option<u32>,
    #[allow(dead_code)]
    pub max_stars: Option<usize>,
    #[allow(dead_code)]
    pub tolerance: Option<f64>,
    #[allow(dead_code)]
    pub min_star_arcsec: Option<f64>,
    /// The full command line ASTAP mode was invoked with: the program path
    /// (`argv[0]`) followed by the argv `parse_astap` was given, joined with
    /// single spaces -- e.g.
    /// `/home/user/astap/astap_cli -f /path/to.fits -r 180 -fov 1.4770 -d
    /// /home/user/astap -update`. Task 7's `.ini` writer reproduces ASTAP's
    /// own `CMDLINE=` key from this byte-exact, and that key includes the
    /// binary path -- not specified in the plan's original struct, added by
    /// a ruling recorded in the M3 progress ledger (`AstapArgs`/Task 7 gap).
    pub cmdline: String,
}

/// Analysis-only flags AstroOps may pass through that take no value.
/// Accepted and ignored per the task brief -- not an error, since erroring
/// would make psolve reject invocations the real `astap_cli` accepts.
const IGNORED_NO_VALUE: &[&str] = &["-log", "-sip", "-check", "-progress"];

/// Analysis-only flags that DO take a value (`astap_cli --help`: `-speed
/// mode[auto/slow]`, `-analyse/-extract/-extract2 snr_min`). The value must
/// still be consumed and discarded, or it would be misread as the next flag
/// or an unexpected bare argument.
const IGNORED_WITH_VALUE: &[&str] = &["-speed", "-analyse", "-extract", "-extract2"];

/// Consume `args[*i]` (assumed to equal `flag`) and its following value,
/// advancing `*i` past both. A flag at the end of argv with nothing after it
/// is a malformed invocation, not an empty default -- silently accepting it
/// is exactly the "-f with no value must not silently pass" failure mode the
/// task brief calls out.
fn take_value(args: &[String], i: &mut usize, flag: &str) -> Result<String, String> {
    let value = args
        .get(*i + 1)
        .ok_or_else(|| format!("psolve: {flag} requires a value"))?
        .clone();
    *i += 2;
    Ok(value)
}

fn parse_f64(v: &str, flag: &str) -> Result<f64, String> {
    match v.parse::<f64>() {
        Ok(x) if x.is_finite() => Ok(x),
        _ => Err(format!("psolve: {flag} must be a finite number, got {v:?}")),
    }
}

fn parse_u32(v: &str, flag: &str) -> Result<u32, String> {
    v.parse::<u32>().map_err(|_| format!("psolve: {flag} must be a non-negative integer, got {v:?}"))
}

fn parse_usize(v: &str, flag: &str) -> Result<usize, String> {
    v.parse::<usize>().map_err(|_| format!("psolve: {flag} must be a positive integer, got {v:?}"))
}

/// Parse an ASTAP-mode argv (`args` is the tail after the program name --
/// the same convention `main.rs` already uses for native mode; `program` is
/// that program name/path, `argv[0]`). `program` is a separate, explicit
/// parameter rather than read from `std::env::args()` inside this function,
/// so the parser stays pure and testable against a fixed arg slice, matching
/// every other test in this module. Unknown flags (including psolve's own
/// `--index` and other native `--long` flags) are a hard error: the two
/// surfaces must not blend, so a caller that mixes them finds out
/// immediately rather than having one side silently ignored.
pub fn parse_astap(program: &str, args: &[String]) -> Result<AstapArgs, String> {
    // ASTAP's own CMDLINE= includes the binary path as the first token (see
    // the `cmdline` field doc). `program` might in principle be empty (an
    // OS that hands back an empty argv[0]) -- joining that in naively would
    // put a leading space on `cmdline` instead of silently misrepresenting
    // it, so fall back to a stable placeholder rather than trusting it blind.
    let program = if program.is_empty() { "psolve" } else { program };
    let mut cmdline_parts: Vec<&str> = Vec::with_capacity(args.len() + 1);
    cmdline_parts.push(program);
    cmdline_parts.extend(args.iter().map(String::as_str));
    let cmdline = cmdline_parts.join(" ");

    let mut file: Option<String> = None;
    let mut radius_deg: Option<f64> = None;
    let mut ra_hours: Option<f64> = None;
    let mut spd_deg: Option<f64> = None;
    let mut fov_deg: Option<f64> = None;
    let mut db_dir: Option<String> = None;
    let mut out_base: Option<String> = None;
    let mut update = false;
    let mut wcs_fits_block = false;
    let mut downsample: Option<u32> = None;
    let mut max_stars: Option<usize> = None;
    let mut tolerance: Option<f64> = None;
    let mut min_star_arcsec: Option<f64> = None;

    let mut i = 0;
    while i < args.len() {
        let tok = args[i].as_str();
        match tok {
            "-f" => file = Some(take_value(args, &mut i, tok)?),
            "-r" => radius_deg = Some(parse_f64(&take_value(args, &mut i, tok)?, tok)?),
            "-fov" => fov_deg = Some(parse_f64(&take_value(args, &mut i, tok)?, tok)?),
            "-ra" => ra_hours = Some(parse_f64(&take_value(args, &mut i, tok)?, tok)?),
            "-spd" => spd_deg = Some(parse_f64(&take_value(args, &mut i, tok)?, tok)?),
            "-s" => max_stars = Some(parse_usize(&take_value(args, &mut i, tok)?, tok)?),
            "-t" => tolerance = Some(parse_f64(&take_value(args, &mut i, tok)?, tok)?),
            "-m" => min_star_arcsec = Some(parse_f64(&take_value(args, &mut i, tok)?, tok)?),
            "-z" => downsample = Some(parse_u32(&take_value(args, &mut i, tok)?, tok)?),
            "-d" | "-D" => db_dir = Some(take_value(args, &mut i, tok)?),
            "-o" => out_base = Some(take_value(args, &mut i, tok)?),
            "-update" => {
                update = true;
                i += 1;
            }
            "-wcs" => {
                wcs_fits_block = true;
                i += 1;
            }
            _ if IGNORED_NO_VALUE.contains(&tok) => i += 1,
            _ if IGNORED_WITH_VALUE.contains(&tok) => {
                take_value(args, &mut i, tok)?;
            }
            other => return Err(format!("psolve: unknown ASTAP flag {other:?}")),
        }
    }

    let file = file.ok_or_else(|| "psolve: -f <FILE> is required in ASTAP mode".to_string())?;
    // ASTAP's own help lists no default for -r (unlike -s/-t/-m/-z, which are
    // documented with one). Every real recorded AstroOps invocation always
    // passes it, and 180 (all-sky) is ASTAP's own convention for "no radius
    // constraint" -- the safe direction when it is genuinely absent.
    let radius_deg = radius_deg.unwrap_or(180.0);

    Ok(AstapArgs {
        file,
        radius_deg,
        ra_hours,
        spd_deg,
        fov_deg,
        db_dir,
        out_base,
        update,
        wcs_fits_block,
        downsample,
        max_stars,
        tolerance,
        min_star_arcsec,
        cmdline,
    })
}

/// Hint in DEGREES, converted from ASTAP's hours/SPD convention.
pub fn hint_degrees(a: &AstapArgs) -> Option<(f64, f64)> {
    match (a.ra_hours, a.spd_deg) {
        // -ra is in hours (x15 -> degrees); -spd is south polar distance,
        // so declination is spd - 90. Both confirmed against real recorded
        // invocations, not inferred from the flag names.
        (Some(h), Some(spd)) => Some((h * 15.0, spd - 90.0)),
        _ => None,
    }
}

/// Search radius implied by a supplied `-fov`, in degrees.
///
/// **`-fov` is the field HEIGHT, not the diagonal.** This function used to be
/// `(fov / 2.0) * 1.10`, which treats it as the diagonal and so produces a
/// disc too small by a factor of `sqrt(1 + (W/H)^2) / 2` -- on a 16:9 sensor,
/// **51% too small from a perfectly correct `-fov`**. Measured on the ATR585M
/// (3840x2160 at 2.46"/px, field 2.624 x 1.476 deg): AstroOps' standard
/// `-fov 1.4770` gave 0.812 deg where the frame's own half-diagonal plus
/// margin is 1.657 deg.
///
/// The 2026-08-15 radius fix made [`search_radius_deg`] prefer the header over
/// a disagreeing `-fov` and added [`fov_radius_mismatch_warning`], which
/// hid this in the common case -- but that fix changed the PRECEDENCE, not
/// this arithmetic, so the defect survived in the fallback taken whenever a
/// header lacks `FOCALLEN`/`XPIXSZ`. There a correct `-fov` still yielded a
/// disc half the size the frame needs, failing as `TOO_FEW_STARS` or
/// `NO_QUAD_MATCH` -- indistinguishable from a bad frame. Found 2026-08-23 by
/// the astroops session, from the field-geometry table in `catalogue.db`.
///
/// Given `NAXIS1`/`NAXIS2` the aspect ratio is known and the half-diagonal is
/// exact. Without them the frame is treated as SQUARE, matching
/// [`crate::cmd_solve::header_radius_deg`]'s own convention for a height-only
/// header: it errs wide, and wide is the safe direction -- a disc larger than
/// the frame costs catalogue budget, a disc smaller than the frame cannot
/// contain the stars needed to solve at all. The old formula erred NARROW
/// from the same input, which is what made it dangerous.
fn fov_radius_deg(fov_height_deg: f64, hdr: Option<&psolve_core::fits::FitsHeader>) -> f64 {
    let aspect = hdr
        .and_then(|h| {
            let nx = h.num("NAXIS1")?;
            let ny = h.num("NAXIS2")?;
            (nx > 0.0 && ny > 0.0).then_some(nx / ny)
        })
        // No dimensions: treat the frame as square (aspect 1.0), erring wide.
        .unwrap_or(1.0);
    (fov_height_deg / 2.0) * (1.0 + aspect * aspect).sqrt() * 1.10
}

/// The radius to actually use for a catalogue search, as distinct from the
/// raw `-r` value ASTAP was invoked with. AstroOps' own blind invocation
/// passes `-r 180` (all-sky) alongside `-fov 1.4770` -- Task 2 established
/// that a search disc much wider than the frame's own footprint dilutes the
/// catalogue with stars no quad can match and costs matches monotonically.
///
/// Precedence, highest first -- and this is the fix for the defect that
/// motivated this function's rewrite: a `-fov` a caller supplies is a guess
/// that may not match the rig the frame was actually taken on (AstroOps
/// passes a single fixed `-fov` for every rig), while the frame's own header
/// knows its actual field size. Preferring the guess over the ground truth is
/// exactly backwards, so the header wins whenever it can be used:
///
/// 1. **Header-derived**, via [`crate::cmd_solve::header_radius_deg`] -- the
///    identical half-diagonal-plus-10%-margin formula native mode uses
///    ([`crate::cmd_solve::default_radius_for`]), not a second copy of it.
///    Used whenever the header carries the `FOCALLEN`/`XPIXSZ` optics
///    keywords `field_width_deg`/`field_height_deg` need.
/// 2. **`-fov`-derived**, when the header lacks those keywords. `-fov` is the
///    field DIAMETER, so half of it plus the same 10% margin is the disc
///    that actually needs to be searched. ASTAP documents `-fov 0` as "auto"
///    ( i.e. absent), so a zero or missing `-fov` does not count here.
/// 3. **`-r` verbatim**, when neither of the above is available.
///
/// Whichever of the above wins, `-r` is *still* honoured as an upper bound on
/// top of it: a caller asking for a deliberately narrow search must get one,
/// even against a wider header- or `-fov`-derived disc. Call
/// [`fov_radius_mismatch_warning`] alongside this to surface the case that
/// made this defect hard to see -- a `-fov` that quietly disagreed with the
/// header used to win outright and no one was told.
pub fn search_radius_deg(a: &AstapArgs, hdr: Option<&psolve_core::fits::FitsHeader>) -> f64 {
    let from_header = crate::cmd_solve::header_radius_deg(hdr);
    let from_fov = a.fov_deg.filter(|f| *f > 0.0).map(|fov| fov_radius_deg(fov, hdr));
    let base = from_header.or(from_fov).unwrap_or(a.radius_deg);
    base.min(a.radius_deg)
}

/// The two numbers [`search_radius_deg`] combines, kept separate for the
/// binning retry's catalogue refetch: the UNCAPPED header-derived radius and
/// the caller's `-r` ceiling.
///
/// The retry divides the header-derived radius by `XBINNING` and re-applies
/// the ceiling. Dividing [`search_radius_deg`]'s already-capped output
/// instead would produce a different, wrong number whenever `-r` bound the
/// first fetch -- with `-r 0.8` against a 1.2034 deg header radius, 0.4000
/// rather than the correct 0.6017. Hence two values rather than one
/// pre-combined one; see [`crate::cmd_solve::CatalogRefetch`].
///
/// The first element is `None` exactly when
/// [`crate::cmd_solve::header_radius_deg`] is -- i.e. when the header lacks
/// the `FOCALLEN`/`XPIXSZ` optics keywords -- and the caller must then
/// suppress the refetch entirely rather than substitute the `-fov`- or
/// `-r`-derived radius [`search_radius_deg`] would fall back to. Two
/// reasons, and the second is the load-bearing one:
///
/// 1. It matches native mode, which passes `header_radius_deg(hdr).map(..)`
///    and so does not refetch without one. The two surfaces must not drift.
/// 2. The correction being applied is `scale / XBINNING`, and it is only
///    valid for a radius that was itself derived from that ambiguous
///    header scale. A `-fov`-derived radius is a caller assertion about the
///    field, unrelated to `XPIXSZ`; halving it would narrow a disc that was
///    never too wide. Without the optics keywords there is nothing to
///    correct, so the retry corrects the scale alone -- today's behaviour.
pub fn header_radius_and_cap(
    a: &AstapArgs,
    hdr: Option<&psolve_core::fits::FitsHeader>,
) -> (Option<f64>, f64) {
    (crate::cmd_solve::header_radius_deg(hdr), a.radius_deg)
}

/// A stderr warning line when a supplied `-fov` implies a search radius more
/// than 25% different from what the frame header itself implies -- `None`
/// when there is nothing to compare (`-fov` absent/zero, or the header lacks
/// the optics keywords [`crate::cmd_solve::header_radius_deg`] needs).
///
/// This is the diagnostic the original defect lacked: [`search_radius_deg`]
/// silently prefers the header over a disagreeing `-fov`, which is the right
/// outcome, but a caller whose fixed `-fov` never matches a given rig should
/// still be told, on every solve, not just be quietly overridden -- that is
/// what would have made "too few stars in the disc" legible as "your -fov is
/// wrong for this camera" instead of a mystery.
pub fn fov_radius_mismatch_warning(
    a: &AstapArgs,
    hdr: Option<&psolve_core::fits::FitsHeader>,
) -> Option<String> {
    let from_header = crate::cmd_solve::header_radius_deg(hdr)?;
    let fov = a.fov_deg.filter(|f| *f > 0.0)?;
    let from_fov = fov_radius_deg(fov, hdr);
    let ratio = from_fov / from_header;
    if (0.75..=1.25).contains(&ratio) {
        return None;
    }
    let pct = (ratio - 1.0) * 100.0;
    Some(format!(
        "psolve: warning: -fov {fov} implies a search radius of {from_fov:.3} deg, but the \
frame header implies {from_header:.3} deg ({pct:+.0}% different) -- using the header-derived \
radius; -fov may not match this rig's actual field of view"
    ))
}

/// ASTAP's own failure string for a missing/unusable star database
/// (ground-truth doc `2026-08-14-astap-format-facts.md` §3c: `-d` pointing
/// nowhere produces `ERROR=No star database found.` and exit code 1,
/// reproduced live against the real binary). [`resolve_index_path`] returns
/// exactly this string on every failure mode below, since it feeds `.ini`'s
/// `ERROR=` key directly (`sidecar::format_ini_failure`) -- there is no
/// separate psolve-specific wording to invent here.
pub const NO_STAR_DATABASE: &str = "No star database found.";

/// Resolve psolve's own index from ASTAP's `-d`/`-D` directory
/// (`AstapArgs.db_dir`). ASTAP's own `-d` names a directory holding its
/// `d50_*`-style star-database files; the psolve equivalent artifact is a
/// single `.psidx` index built by `psolve index build`. When more than one
/// `.psidx` file is present the choice is made deterministic (sorted by
/// path, first wins) rather than depending on whatever order
/// [`std::fs::read_dir`] happens to yield, which the OS does not guarantee.
///
/// Fails -- with the same [`NO_STAR_DATABASE`] string in every case, not a
/// distinct message per cause -- when `db_dir` is absent, does not exist, or
/// contains no `.psidx` file: from ASTAP's perspective (and AstroOps',
/// which only reads the `.ini` this feeds) all three are the same outcome,
/// "no star database is usable here".
pub fn resolve_index_path(db_dir: Option<&str>) -> Result<std::path::PathBuf, String> {
    let dir = db_dir.ok_or_else(|| NO_STAR_DATABASE.to_string())?;
    let mut candidates: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
        .map_err(|_| NO_STAR_DATABASE.to_string())?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|ext| ext.to_str()) == Some("psidx"))
        .collect();
    candidates.sort();
    candidates.into_iter().next().ok_or_else(|| NO_STAR_DATABASE.to_string())
}

/// Auto-discover a `.psqidx` blind-solve quad index in the same `-d`/`-D`
/// directory `resolve_index_path` reads its `.psidx` from -- same
/// sorted-first-match determinism, for the same reason.
///
/// Returns `None`, not `Err`, on every failure to find one: unlike a
/// missing `.psidx` ([`resolve_index_path`]'s [`NO_STAR_DATABASE`] --
/// ASTAP cannot solve at all without a star database), a missing
/// `.psqidx` just means blind solving is unavailable for this invocation.
/// That is the ORDINARY case for every hinted invocation AstroOps sends
/// (a `-d` directory need not carry one at all), so it must not be
/// reported as the same class of error `resolve_index_path` reports for
/// its own, load-bearing artifact.
pub fn resolve_quad_index_path(db_dir: Option<&str>) -> Option<std::path::PathBuf> {
    let dir = db_dir?;
    let mut candidates: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|ext| ext.to_str()) == Some("psqidx"))
        .collect();
    candidates.sort();
    candidates.into_iter().next()
}

#[cfg(test)]
mod tests {
    use super::{
        fov_radius_mismatch_warning, hint_degrees, parse_astap, resolve_index_path,
        resolve_quad_index_path, search_radius_deg, NO_STAR_DATABASE,
    };

    /// Build a bare-bones `FitsHeader` directly from key/value pairs, rather
    /// than through `FitsHeader::parse`'s 2880-byte card grammar -- the
    /// struct's fields (`cards`, `data_offset`) are public exactly so tests
    /// like this one can construct a fixture without a real FITS byte
    /// stream. Mirrors the same pattern `psolve-core::fits`'s own tests use.
    fn fake_header(pairs: &[(&str, &str)]) -> psolve_core::fits::FitsHeader {
        psolve_core::fits::FitsHeader {
            cards: pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            data_offset: 0,
        }
    }

    /// A header carrying just enough optics keywords for `field_width_deg`/
    /// `field_height_deg` to resolve -- no binning, one pixel size shared by
    /// both axes (`XPIXSZ`), matching how `pixel_scale_arcsec` reads it.
    fn header_with_optics(nx: f64, ny: f64, focal_mm: f64, pix_um: f64) -> psolve_core::fits::FitsHeader {
        fake_header(&[
            ("NAXIS1", &nx.to_string()),
            ("NAXIS2", &ny.to_string()),
            ("FOCALLEN", &focal_mm.to_string()),
            ("XPIXSZ", &pix_um.to_string()),
        ])
    }

    /// A fresh, unique scratch directory under the system temp path --
    /// never `~/astroops`, which is strictly read-only project-wide.
    fn scratch_dir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir()
            .join(format!("psolve-astap-args-{tag}-{}-{}", std::process::id(), tag.len()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap_or_else(|e| panic!("creating scratch dir: {e}"));
        d
    }

    #[test]
    fn a_missing_db_dir_argument_is_no_star_database_found() {
        assert_eq!(resolve_index_path(None), Err(NO_STAR_DATABASE.to_string()));
    }

    #[test]
    fn a_db_dir_that_does_not_exist_is_no_star_database_found() {
        let dir = scratch_dir("nonexistent");
        std::fs::remove_dir_all(&dir).ok(); // exists only long enough to get a unique name
        assert_eq!(resolve_index_path(Some(dir.to_str().unwrap())), Err(NO_STAR_DATABASE.to_string()));
    }

    #[test]
    fn a_db_dir_with_no_psidx_file_is_no_star_database_found() {
        let dir = scratch_dir("empty");
        std::fs::write(dir.join("readme.txt"), b"not an index").unwrap();
        let r = resolve_index_path(Some(dir.to_str().unwrap()));
        assert_eq!(r, Err(NO_STAR_DATABASE.to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_db_dir_with_one_psidx_file_resolves_to_it() {
        let dir = scratch_dir("one");
        let idx = dir.join("gaia.psidx");
        std::fs::write(&idx, b"not a real index, just a marker").unwrap();
        let r = resolve_index_path(Some(dir.to_str().unwrap())).expect("must resolve");
        assert_eq!(r, idx);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// More than one `.psidx` in the directory must not be an error and
    /// must not depend on filesystem iteration order -- sorted, first wins.
    #[test]
    fn multiple_psidx_files_resolve_deterministically() {
        let dir = scratch_dir("multiple");
        std::fs::write(dir.join("z-second.psidx"), b"z").unwrap();
        std::fs::write(dir.join("a-first.psidx"), b"a").unwrap();
        let r = resolve_index_path(Some(dir.to_str().unwrap())).expect("must resolve");
        assert_eq!(r, dir.join("a-first.psidx"), "sorted order must pick the alphabetically first file");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Unlike `resolve_index_path`, a missing `.psqidx` is `None`, not an
    /// error -- see this function's own doc for why that distinction
    /// matters (an ordinary hinted invocation has no `.psqidx` at all).
    #[test]
    fn a_missing_db_dir_argument_has_no_quad_index() {
        assert_eq!(resolve_quad_index_path(None), None);
    }

    #[test]
    fn a_db_dir_with_no_psqidx_file_has_no_quad_index() {
        let dir = scratch_dir("no-psqidx");
        std::fs::write(dir.join("gaia.psidx"), b"a star index, not a quad index").unwrap();
        assert_eq!(resolve_quad_index_path(Some(dir.to_str().unwrap())), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_db_dir_with_one_psqidx_file_resolves_to_it() {
        let dir = scratch_dir("one-psqidx");
        let idx = dir.join("gaia.psqidx");
        std::fs::write(&idx, b"not a real quad index, just a marker").unwrap();
        let r = resolve_quad_index_path(Some(dir.to_str().unwrap())).expect("must resolve");
        assert_eq!(r, idx);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Same sorted-first-match determinism `multiple_psidx_files_resolve_
    /// deterministically` pins for `.psidx` -- and, in the same directory,
    /// the two extensions must not interfere with each other's selection.
    #[test]
    fn multiple_psqidx_files_resolve_deterministically_and_do_not_confuse_psidx_selection() {
        let dir = scratch_dir("multiple-psqidx");
        std::fs::write(dir.join("z-second.psqidx"), b"z").unwrap();
        std::fs::write(dir.join("a-first.psqidx"), b"a").unwrap();
        std::fs::write(dir.join("gaia.psidx"), b"a star index").unwrap();
        let r = resolve_quad_index_path(Some(dir.to_str().unwrap())).expect("must resolve");
        assert_eq!(r, dir.join("a-first.psqidx"), "sorted order must pick the alphabetically first file");
        let idx = resolve_index_path(Some(dir.to_str().unwrap())).expect("must resolve");
        assert_eq!(idx, dir.join("gaia.psidx"), "the .psidx selection must be unaffected");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A representative program path/name -- most tests here don't care what
    /// it is, only that it ends up as `cmdline`'s first token. Tests that
    /// care about the exact `CMDLINE=` shape use the real recorded path
    /// directly (see `cmdline_begins_with_the_program_path`).
    const PROG: &str = "astap_cli";

    /// -ra is in HOURS and -spd is dec+90. Getting either wrong produces a
    /// hint that is wrong by a factor of 15 or by 90 degrees -- and the
    /// solver then fails with NO_QUAD_MATCH, which reads as "unsolvable
    /// frame" rather than "the caller mistranslated the units".
    #[test]
    fn ra_is_hours_and_spd_is_declination_plus_ninety() {
        let a = parse_astap(
            PROG,
            &["-f", "x.fits", "-ra", "16.950000", "-spd", "49.666667"].map(String::from),
        )
        .unwrap();
        let (ra_deg, dec_deg) = hint_degrees(&a).expect("both given -> a hint");
        assert!((ra_deg - 254.25).abs() < 1e-9, "16.95 h must be 254.25 deg, got {ra_deg}");
        assert!(
            (dec_deg - (-40.333333)).abs() < 1e-6,
            "spd 49.666667 must be -40.333333 deg, got {dec_deg}"
        );
    }

    #[test]
    fn a_hint_needs_both_ra_and_spd() {
        let only_ra =
            parse_astap(PROG, &["-f", "x.fits", "-ra", "16.95"].map(String::from)).unwrap();
        assert!(hint_degrees(&only_ra).is_none(), "half a hint is not a hint");
    }

    /// The real AstroOps blind invocation must parse exactly.
    #[test]
    fn the_real_blind_invocation_parses() {
        let a = parse_astap(
            PROG,
            &[
                "-f", "/x/y.fits", "-r", "180", "-fov", "1.4770", "-d", "/home/user/astap",
                "-update",
            ]
            .map(String::from),
        )
        .unwrap();
        assert_eq!(a.file, "/x/y.fits");
        assert_eq!(a.radius_deg, 180.0);
        assert_eq!(a.fov_deg, Some(1.4770));
        assert_eq!(a.db_dir.as_deref(), Some("/home/user/astap"));
        assert!(a.update);
        assert!(hint_degrees(&a).is_none(), "-r 180 with no -ra/-spd is a blind solve");
    }

    /// The real AstroOps hinted retry must parse exactly.
    #[test]
    fn the_real_hinted_retry_parses() {
        let a = parse_astap(
            PROG,
            &[
                "-f", "/x/y.fits", "-ra", "16.950000", "-spd", "49.666667", "-r", "15", "-fov",
                "1.4770", "-d", "/home/user/astap", "-update",
            ]
            .map(String::from),
        )
        .unwrap();
        assert_eq!(a.radius_deg, 15.0);
        assert!(hint_degrees(&a).is_some());
        assert!(a.update);
    }

    #[test]
    fn a_missing_input_file_is_an_error_not_a_default() {
        assert!(parse_astap(PROG, &["-r".into(), "180".into()]).is_err());
        assert!(
            parse_astap(PROG, &["-f".into()]).is_err(),
            "-f with no value must not silently pass"
        );
    }

    /// Native and ASTAP surfaces must not blend.
    #[test]
    fn native_flags_are_rejected_in_astap_mode() {
        assert!(parse_astap(PROG, &["-f", "x.fits", "--index", "i.psidx"].map(String::from))
            .is_err());
    }

    /// When -r is absent, ASTAP's own "all-sky" convention (180 deg) is the
    /// resolved default -- pinned here, not just disclosed in a comment.
    #[test]
    fn radius_defaults_to_180_when_r_is_absent() {
        let a = parse_astap(PROG, &["-f", "x.fits"].map(String::from)).unwrap();
        assert_eq!(a.radius_deg, 180.0);
    }

    /// The real blind invocation with no usable header: -r 180 is all-sky,
    /// but -fov 1.4770 bounds the frame to a much smaller disc. Without a
    /// header to prefer, the effective search radius must track -fov, not
    /// the all-sky value that produced it.
    #[test]
    fn effective_radius_prefers_the_field_diameter_over_a_blind_all_sky_r() {
        let a = parse_astap(PROG, &["-f", "x.fits", "-r", "180", "-fov", "1.4770"].map(String::from))
            .unwrap();
        let r = search_radius_deg(&a, None);
        // No header, so no aspect ratio: the frame is treated as square,
        // which errs wide. See `fov_radius_deg`.
        let expected = (1.4770 / 2.0) * 2.0_f64.sqrt() * 1.10;
        assert!((r - expected).abs() < 1e-9, "expected {expected}, got {r}");
    }

    /// The real hinted retry, no usable header: -r 15 is already narrower
    /// than an all-sky search, but still wider than the frame needs once
    /// -fov is known.
    #[test]
    fn effective_radius_prefers_the_field_diameter_over_a_wider_narrow_retry_r() {
        let a = parse_astap(PROG, &["-f", "x.fits", "-r", "15", "-fov", "1.4770"].map(String::from))
            .unwrap();
        let r = search_radius_deg(&a, None);
        let expected = (1.4770 / 2.0) * 2.0_f64.sqrt() * 1.10;
        assert!((r - expected).abs() < 1e-9, "expected {expected}, got {r}");
    }

    /// Without -fov or a header there is nothing to narrow against -- the
    /// caller's -r must be respected as given.
    #[test]
    fn effective_radius_falls_back_to_r_when_fov_is_absent() {
        let a = parse_astap(PROG, &["-f", "x.fits", "-r", "180"].map(String::from)).unwrap();
        assert_eq!(search_radius_deg(&a, None), 180.0);
    }

    /// A tight, deliberately narrow -r smaller than the field-derived value
    /// must not be widened.
    #[test]
    fn effective_radius_never_widens_an_already_narrow_r() {
        let a = parse_astap(PROG, &["-f", "x.fits", "-r", "0.1", "-fov", "1.4770"].map(String::from))
            .unwrap();
        assert_eq!(search_radius_deg(&a, None), 0.1);
    }

    /// The defect this task fixes: a `-fov` far too small for the actual
    /// frame must not win when the header can answer for itself. This
    /// mirrors the real motivating case (AstroOps' fixed `-fov 1.4770` on an
    /// ATR585M frame whose true field is much larger) with synthetic optics
    /// numbers, and checks the result against `default_radius_for` directly
    /// rather than a second hand-derived expected value.
    #[test]
    fn effective_radius_prefers_the_header_over_a_too_small_fov() {
        let hdr = header_with_optics(6252.0, 4176.0, 530.0, 3.76);
        let w = psolve_core::fits::field_width_deg(&hdr).unwrap();
        let h = psolve_core::fits::field_height_deg(&hdr).unwrap();
        let expected = crate::cmd_solve::default_radius_for(w, h);

        let a = parse_astap(PROG, &["-f", "x.fits", "-r", "180", "-fov", "1.4770"].map(String::from))
            .unwrap();
        let r = search_radius_deg(&a, Some(&hdr));
        assert!((r - expected).abs() < 1e-9, "expected header-derived {expected}, got {r}");
        assert!(r > (1.4770 / 2.0) * 1.10, "the too-small -fov must not have won");
    }

    /// A header that lacks the optics keywords must not block the -fov
    /// fallback -- absence of FOCALLEN/XPIXSZ is exactly the "header can't
    /// answer" case the precedence falls through on.
    #[test]
    fn effective_radius_falls_back_to_fov_when_the_header_lacks_optics_keywords() {
        let hdr = fake_header(&[("NAXIS1", "100"), ("NAXIS2", "100")]);
        let a = parse_astap(PROG, &["-f", "x.fits", "-r", "180", "-fov", "1.4770"].map(String::from))
            .unwrap();
        // NAXIS1 == NAXIS2 here, so the aspect ratio really is 1.0.
        let expected = (1.4770 / 2.0) * 2.0_f64.sqrt() * 1.10;
        let r = search_radius_deg(&a, Some(&hdr));
        assert!((r - expected).abs() < 1e-9, "expected {expected}, got {r}");
    }

    /// **`-fov` is the field HEIGHT.** Confirmed the way `hint_degrees`
    /// confirmed `-ra`/`-spd`: against real recorded AstroOps invocations on
    /// two different rigs, not inferred from the flag name.
    ///
    /// | rig | frame | scale | field W x H | diagonal | AstroOps passes |
    /// |---|---|---|---|---|---|
    /// | ATR585M | 3840x2160 | 2.46"/px | 2.624 x 1.476 | 3.011 | `-fov 1.4770` |
    /// | SV405CC bin2 | 2072x1410 | 7.86"/px | 4.524 x 3.079 | 5.472 | `-fov 3.079` |
    ///
    /// Both constants equal the HEIGHT exactly and neither is close to the
    /// diagonal. So a correct `-fov` must imply the same search radius the
    /// frame's own header does -- which is what this asserts, and what the
    /// pre-2026-08-23 `(fov / 2.0) * 1.10` got wrong by a factor of
    /// `sqrt(1 + (W/H)^2) / 2` (51% too small on 16:9).
    #[test]
    fn a_correct_fov_implies_the_same_radius_the_header_does() {
        for (nx, ny, focal_mm, pix_um) in
            [(3840.0, 2160.0, 243.0, 2.90), (2072.0, 1410.0, 243.0, 9.26)]
        {
            let hdr = header_with_optics(nx, ny, focal_mm, pix_um);
            let from_header =
                crate::cmd_solve::header_radius_deg(Some(&hdr)).expect("header has optics");
            let height = psolve_core::fits::field_height_deg(&hdr).expect("header has optics");

            let args = ["-f".to_string(), "x.fits".to_string(), "-fov".to_string(), format!("{height}")];
            let a = parse_astap(PROG, &args).unwrap();

            let from_fov = search_radius_deg(&a, Some(&hdr));
            assert!(
                (from_fov - from_header).abs() < 1e-9,
                "{nx}x{ny}: a -fov equal to the field height ({height}) implied {from_fov} \
but the header implies {from_header}"
            );
            assert!(
                fov_radius_mismatch_warning(&a, Some(&hdr)).is_none(),
                "{nx}x{ny}: a correct -fov must not warn"
            );
        }
    }

    /// -r is an upper bound over EVERY source, including a header-derived
    /// radius -- a deliberately narrow -r must still win.
    #[test]
    fn effective_radius_is_still_capped_by_a_narrow_r_even_with_a_header() {
        let hdr = header_with_optics(6252.0, 4176.0, 530.0, 3.76);
        let a = parse_astap(PROG, &["-f", "x.fits", "-r", "0.1"].map(String::from)).unwrap();
        assert_eq!(search_radius_deg(&a, Some(&hdr)), 0.1);
    }

    /// A WIDE -r must not widen the disc past the header-derived radius --
    /// the opposite direction to the test above, and the one that turned out
    /// to be load-bearing.
    ///
    /// N.I.N.A. invokes ASTAP with `SearchRadius 10` as a matter of course.
    /// Measured on a real cloud-degraded frame (ATR585M/SV555, 2.45"/px,
    /// 1.501 deg half-diagonal), psolve solves at a 1.65 deg disc and
    /// **fails outright** with `NO_QUAD_MATCH` at 10 deg: a disc that wide is
    /// mostly sky the frame does not cover, every catalogue star out there
    /// lowers completeness, and a quad needs all four of its stars on both
    /// sides. ASTAP solves the same frame at `-r 10` unbothered.
    ///
    /// So this ceiling is what keeps a stock N.I.N.A. invocation working:
    /// `min(1.65, 10)` uses the header's own geometry and ignores the 10.
    /// Were `-r` ever treated as the radius rather than a bound, psolve
    /// would fail a frame ASTAP solves, on the exact command line the tool
    /// it replaces is given every night.
    #[test]
    fn a_wide_r_never_widens_the_disc_beyond_the_header_geometry() {
        // 3840x2160 at 2.9 um on 243 mm: the rig this was measured on.
        let hdr = header_with_optics(3840.0, 2160.0, 243.0, 2.9);
        let from_header = crate::cmd_solve::header_radius_deg(Some(&hdr)).unwrap();
        for wide in ["10", "30", "180"] {
            let a = parse_astap(PROG, &["-f", "x.fits", "-r", wide].map(String::from)).unwrap();
            let got = search_radius_deg(&a, Some(&hdr));
            assert!(
                (got - from_header).abs() < 1e-9,
                "-r {wide} widened the disc to {got} deg; the header says {from_header}"
            );
        }
    }

    /// -fov 2.7 (the case that already solved before this fix) must still
    /// resolve to its own fov-derived radius when no header is available --
    /// this fix must not have narrowed anything that already worked.
    #[test]
    fn effective_radius_for_a_generously_sized_fov_is_unchanged_without_a_header() {
        let a = parse_astap(PROG, &["-f", "x.fits", "-r", "30", "-fov", "2.7"].map(String::from))
            .unwrap();
        let expected = (2.7 / 2.0) * 2.0_f64.sqrt() * 1.10;
        assert!((search_radius_deg(&a, None) - expected).abs() < 1e-9);
    }

    /// No warning when there is nothing to compare: no header, no -fov, or a
    /// header missing the optics keywords.
    #[test]
    fn fov_radius_mismatch_warning_is_none_without_enough_to_compare() {
        let a_no_fov = parse_astap(PROG, &["-f", "x.fits", "-r", "180"].map(String::from)).unwrap();
        let hdr = header_with_optics(6252.0, 4176.0, 530.0, 3.76);
        assert!(fov_radius_mismatch_warning(&a_no_fov, Some(&hdr)).is_none(), "no -fov to compare");

        let a_with_fov = parse_astap(PROG, &["-f", "x.fits", "-fov", "1.4770"].map(String::from)).unwrap();
        assert!(fov_radius_mismatch_warning(&a_with_fov, None).is_none(), "no header to compare");

        let bare_hdr = fake_header(&[("NAXIS1", "100"), ("NAXIS2", "100")]);
        assert!(
            fov_radius_mismatch_warning(&a_with_fov, Some(&bare_hdr)).is_none(),
            "header lacks optics keywords"
        );
    }

    /// A -fov that implies a radius far smaller than the header's own must
    /// warn, naming both values.
    #[test]
    fn fov_radius_mismatch_warning_fires_on_a_too_small_fov() {
        let hdr = header_with_optics(6252.0, 4176.0, 530.0, 3.76);
        // 0.5 deg against this header's 1.697 deg field height is a genuine
        // mismatch. Before 2026-08-23 this test used 1.4770, which is only
        // 13% from that header's height -- it read as a "large mismatch" only
        // because the old arithmetic treated -fov as the diagonal and so
        // halved it. A correct -fov must NOT warn.
        let a = parse_astap(PROG, &["-f", "x.fits", "-fov", "0.5"].map(String::from)).unwrap();
        let msg = fov_radius_mismatch_warning(&a, Some(&hdr)).expect("must warn on a large mismatch");
        assert!(msg.contains("0.5"), "must name the supplied -fov: {msg}");
        assert!(msg.to_lowercase().contains("header"), "must name the header: {msg}");
    }

    /// A -fov whose implied radius matches the header's own (within 25%)
    /// must not warn. **Agreement means `-fov` == the field HEIGHT**, not the
    /// diagonal -- see `fov_radius_deg`. This test asserted the diagonal until
    /// 2026-08-23, which is how the semantics defect stayed invisible: the
    /// test was written from the implementation rather than from ASTAP.
    #[test]
    fn fov_radius_mismatch_warning_is_silent_when_fov_agrees_with_the_header() {
        let hdr = header_with_optics(6252.0, 4176.0, 530.0, 3.76);
        let w = psolve_core::fits::field_width_deg(&hdr).unwrap();
        let h = psolve_core::fits::field_height_deg(&hdr).unwrap();
        let _ = w;
        let matching_fov = h;
        let fov_str = format!("{matching_fov}");
        let a = parse_astap(PROG, &["-f", "x.fits", "-fov", fov_str.as_str()].map(String::from))
            .unwrap();
        assert!(fov_radius_mismatch_warning(&a, Some(&hdr)).is_none());
    }

    /// Every value-taking flag must consume its value so it is never
    /// misread as the next flag or an unexpected bare argument -- including
    /// the ignored analysis-only ones that still take one.
    #[test]
    fn ignored_flags_are_accepted_not_errors() {
        let a = parse_astap(
            PROG,
            &[
                "-f", "x.fits", "-log", "-sip", "-check", "-progress", "-speed", "auto",
                "-analyse", "3.5", "-extract", "3.5", "-extract2", "3.5",
            ]
            .map(String::from),
        );
        assert!(a.is_ok(), "analysis-only flags must be accepted, not rejected: {a:?}");
    }

    #[test]
    fn an_unknown_flag_is_an_error() {
        assert!(parse_astap(PROG, &["-f", "x.fits", "-bogus"].map(String::from)).is_err());
    }

    /// -D (a named database abbreviation) is an alternative to -d (a path);
    /// the struct has one slot, so whichever is given fills it.
    #[test]
    fn dash_capital_d_also_fills_db_dir() {
        let a = parse_astap(PROG, &["-f", "x.fits", "-D", "d50"].map(String::from)).unwrap();
        assert_eq!(a.db_dir.as_deref(), Some("d50"));
    }

    /// The recorded cmdline is `program` followed by the joined argv passed
    /// to parse_astap, verbatim -- Task 7's .ini writer depends on this.
    #[test]
    fn cmdline_is_the_joined_original_argv() {
        let raw = ["-f", "x.fits", "-r", "180"];
        let a = parse_astap(PROG, &raw.map(String::from)).unwrap();
        assert_eq!(a.cmdline, format!("{PROG} {}", raw.join(" ")));
    }

    /// The real ground truth this field exists to reproduce:
    /// `CMDLINE=/home/user/astap/astap_cli -f ... -r 180 -fov 1.4770 -d
    /// /home/user/astap -update`. The binary path is part of the string, not
    /// just the flags -- this is what Task 7 writes byte-exact.
    #[test]
    fn cmdline_begins_with_the_program_path() {
        let program = "/home/user/astap/astap_cli";
        let a = parse_astap(
            program,
            &[
                "-f", "/x/y.fits", "-r", "180", "-fov", "1.4770", "-d", "/home/user/astap",
                "-update",
            ]
            .map(String::from),
        )
        .unwrap();
        assert_eq!(
            a.cmdline,
            "/home/user/astap/astap_cli -f /x/y.fits -r 180 -fov 1.4770 \
             -d /home/user/astap -update"
        );
    }

    /// An absent/empty program path must fall back to a stable placeholder,
    /// not panic and not leave a leading space on `cmdline`.
    #[test]
    fn an_empty_program_path_falls_back_rather_than_leaving_a_leading_space() {
        let a = parse_astap("", &["-f", "x.fits"].map(String::from)).unwrap();
        assert!(!a.cmdline.starts_with(' '), "cmdline was {:?}", a.cmdline);
        assert!(a.cmdline.starts_with("psolve "), "cmdline was {:?}", a.cmdline);
    }
}
