//! psolve — plate solver. M1 ships the `index` subcommand only.
//!
//! Exit codes (spec section 9):
//!   0 success · 1 normal negative outcome · 2 usage/config · 3 index problem

mod astap_args;
mod cmd_index;
mod cmd_quadindex;
mod cmd_solve;
mod fits_update;
mod sidecar;

use psolve_core::solve::{CatalogStar, Outcome, SolveOptions};
use psolve_index::reader::Index;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "\
psolve 0.1.0

USAGE:
    psolve index build --input <DIR> --out <FILE> [OPTIONS]
    psolve index info <FILE>
    psolve index query <INDEX> --ra <DEG> --dec <DEG> --radius <DEG> [OPTIONS]
    psolve quad-index build --star-index <FILE> --out <FILE> [OPTIONS]
    psolve quad-index info --star-index <FILE> <PSQIDX> [--verify]
    psolve solve <FILE> --index <FILE> [OPTIONS]

BUILD OPTIONS:
    --max-mag <F>   faintest magnitude to include     [default: 14]
    --min-dec <D>   southern declination limit, deg   [default: -90]
    --max-dec <D>   northern declination limit, deg   [default: 90]
    --nside <N>     HEALPix nside, power of two       [default: 64]
    --epoch <Y>     catalogue epoch, decimal year     [default: 2016.0 (Gaia DR3)]
    --columns <S>   column name overrides for a non-Gaia catalogue,
                    e.g. ra=RAJ2000,dec=DEJ2000,mag=Vmag
    --name <S>      index name stored in the header   [default: derived]
    --jobs <N>      parallel file readers, >=1        [default: cores]
    --allow-partial accept a build with per-file read failures or an
                    unconfirmed-complete mirror (exit 0 instead of 3);
                    the JSON result still reports what was lost

A fixed observatory never sees the whole sky: --min-dec/--max-dec drop stars
that cannot appear in any frame it takes, shrinking the index accordingly.

QUERY OPTIONS:
    --ra <DEG>      disc centre right ascension, 0..360
    --dec <DEG>     disc centre declination, -90..90
    --radius <DEG>  disc radius, degrees, > 0
    --max-mag <M>   faintest magnitude to include (inclusive)
                                                    [default: the index's own mag_limit]
    --format <F>    csv or ndjson                     [default: csv]

Every catalogue star in the disc to --max-mag is emitted, in index order --
not the brightest N. Rows stream to stdout as they are found; errors and the
summary go to stderr.

QUAD-INDEX BUILD OPTIONS:
    --star-index <FILE> paired star .psidx to build quads from   [required]
    --out <FILE>        output .psqidx path                     [required]
    --name <S>          index name stored in the header   [default: blind-quad-index]
    --jobs <N>          parallel tile sweep threads, >=1   [default: cores]
    --min-ra/--max-ra <DEG>   restrict the sweep's RA range, 0..360, no wrap
                                                            [default: 0..360]
    --min-dec/--max-dec <DEG> restrict the sweep's dec range, -90..90
                                                            [default: -90..90]

Sweeps the sky at six doubling bands (0.25..8 deg), forms quads per tile from
the paired star index, and writes a .psqidx blind-solve quad index. See
psolve-cli::cmd_quadindex's module doc for the tiling, band-assignment, and
per-tile selection rules. Deterministic regardless of --jobs.

QUAD-INDEX INFO OPTIONS:
    --star-index <FILE> the .psqidx's paired star .psidx        [required]
    --verify             recompute the record-region digest and confirm it
                          matches the header (O(file size); not free -- see
                          psolve-index::quad_reader for the reader this uses)

Reports the header fields and per-band quad counts as JSON. A .psqidx cannot
be opened without its paired .psidx: --star-index's own records_sha256 is
checked against the header's star_index_fingerprint, and a mismatch is
refused rather than silently resolving quads against the wrong stars.

SOLVE OPTIONS:
    --hint <RA,DEC>     pointing hint in degrees   [default: from OBJCTRA/OBJCTDEC]
    --quad-index <FILE> .psqidx blind-solve quad index, paired against --index;
                        when no hint is available (no --hint, no OBJCTRA/OBJCTDEC
                        or RA/DEC), solves without one instead of returning
                        NO_HINT                    [default: none]
    --scale <A>         arcsec/pixel               [default: from FOCALLEN/XPIXSZ]
    --radius <D>        catalogue search radius    [default: frame half-diagonal + margin,
                                                      else 2.5, when optics keywords are absent]
    --cat-limit <N>     catalogue stars to fetch   [default: 3x the frame's own usable star
                                                      count, clamped to 300..5000]
    --max-mag <M>       drop catalogue stars fainter than magnitude M
                                                   [default: none -- the index's own depth]
    --saturation <V>    pixel value at/above which a detection is clipped
                                                   [default: derived from the decoded data --
                                                      many pixels sharing the maximum]
    --sigma <F>         detection threshold, multiples of background sigma [default: 5.0]
    --min-pix <N>       minimum connected pixels for a detection    [default: 4]
    --keep <N>          brightest detections kept after extraction  [default: 500]
    --max-ellipticity <F> maximum axis-ratio ellipticity before a detection
                          is rejected as elongated                  [default: 0.6]

Exit codes: 0 solved, 1 not solved, 2 usage/config, 3 index problem.
";

fn main() -> ExitCode {
    let mut argv = std::env::args();
    // `argv.next()` is documented as possibly returning `None` (an OS that
    // hands back an empty argv). A missing program path must not panic and
    // must not silently degrade `AstapArgs.cmdline` into a leading-space
    // artifact -- a stable placeholder is the safe fallback.
    let program = argv.next().unwrap_or_else(|| "psolve".to_string());
    let args: Vec<String> = argv.collect();

    // ASTAP mode: entered whenever argv contains `-f`, ASTAP's own filename
    // flag. Checked first, on the owned argv, before native mode's `&str`
    // match below -- the two surfaces must not blend, and `astap_args`'s
    // parser needs owned `String`s to record `AstapArgs.cmdline` (the
    // program path plus the verbatim argv Task 7's `.ini` writer echoes back
    // as `CMDLINE=`).
    if args.iter().any(|a| a == "-f") {
        return astap_cmd(&program, &args);
    }

    let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

    match refs.as_slice() {
        ["index", "build", rest @ ..] => cmd_index::build(rest),
        ["index", "info", rest @ ..] => cmd_index::info(rest),
        ["index", "query", rest @ ..] => cmd_index::query(rest),
        ["quad-index", "build", rest @ ..] => cmd_quadindex::build(rest),
        ["quad-index", "info", rest @ ..] => cmd_quadindex::info(rest),
        ["solve", rest @ ..] => cmd_solve::solve_cmd(rest),
        ["-h"] | ["--help"] | [] => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
        // `--version` was NOT accepted until 2026-08-24, and its absence cost
        // two people an hour each: this repo's own `deb` CI job used it as the
        // post-install smoke test, and the AstroOps container Dockerfile used
        // it as a build-time probe. Both got `unknown command` and exit 2 --
        // the CI job failed loudly, the Dockerfile ignored the exit code and
        // silently baked an unverified binary.
        //
        // A binary that ships in a .deb and a container image is expected to
        // answer `--version`. Emitting the crate version AND the build id
        // matters more here than usual: `psolve` alone is 0.1.0 on every build
        // ever made, while `build` is derived from `git describe` at compile
        // time, and a downstream consumer once cached 2,000 solve results keyed
        // on the version that never moved.
        ["-V"] | ["--version"] | ["version"] => {
            println!("psolve {} ({})", env!("CARGO_PKG_VERSION"), env!("PSOLVE_BUILD_ID"));
            ExitCode::SUCCESS
        }
        other => {
            eprintln!("psolve: unknown command {other:?}\n\n{USAGE}");
            ExitCode::from(2)
        }
    }
}

/// Minimal flag reader: `--name value`. Returns None if absent.
pub fn flag<'a>(args: &'a [&'a str], name: &str) -> Option<&'a str> {
    args.iter().position(|a| *a == name).and_then(|i| args.get(i + 1)).copied()
}

/// ASTAP-compatible entry point: parses the ASTAP argv, resolves psolve's
/// own index from `-d`/`-D`, runs the real solve, and writes ASTAP's own
/// `.ini`/`.wcs` sidecars -- byte-compatible enough that AstroOps' existing
/// parser (which reads only the `.ini`) cannot tell the two binaries apart.
///
/// **Exit codes deliberately do NOT reuse native mode's `0/1/2/3` scheme**
/// (see this file's module doc for that scheme, which stays exactly as it
/// is for `psolve solve`). ASTAP's own observed behaviour
/// (`docs/superpowers/2026-08-14-astap-format-facts.md` §3c, reproduced live
/// against the real binary) is a two-code scheme: `0` on a successful solve
/// or `--help`, `1` for everything else. Note that the `--help` half of that
/// is real ASTAP's behaviour, not a path through this function: `--help`
/// carries no `-f`, and `-f` is what `main` dispatches on, so `psolve
/// --help` goes to NATIVE mode and prints psolve's own `USAGE` (exit `0`
/// either way). That is a recorded deviation from spec §8.1, which specifies
/// argv0/`--astap-compat` as the triggers -- see `docs/astap-compat.md`'s
/// "Mode detection". This function's every non-success
/// return is `ExitCode::from(1)` -- a malformed invocation, a missing input
/// file, an unresolvable star database, an unsolved frame, and a refused
/// `-update` write all collapse to the same code. That collapse is
/// deliberate, not an oversight: a third distinct code here would be a
/// native convention leaking into a mode whose entire purpose is to be
/// indistinguishable, at the `$?` AstroOps actually branches on, from the
/// real `astap_cli`. In particular this is why a read-only `-update` refusal
/// is `1` here and not native mode's `3` -- the two modes' schemes are
/// intentionally decoupled, not merged.
fn astap_cmd(program: &str, args: &[String]) -> ExitCode {
    let parsed = match astap_args::parse_astap(program, args) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(1);
        }
    };

    // -z/-s/-t/-m are accepted (never rejected as unknown flags, and still
    // echoed byte-exact into CMDLINE=) but not applied to the solve -- see
    // AstapArgs's own field doc for why no mapping is guessed. Discarding a
    // flag a caller explicitly passed with zero user-visible signal is the
    // same "plausible result instead of a loud one" shape this milestone has
    // already fixed twice elsewhere, so it is reported here, unconditionally,
    // before anything else runs: ASTAP mode prints no --help of its own for
    // a -f invocation, so main.rs's USAGE const never reaches a pipeline
    // operator running it, and stderr is the one place AstroOps actually
    // looks.
    warn_about_unwired_flags(&parsed);

    let ini_path = astap_sidecar_base(&parsed).with_extension("ini");

    let index_path = match astap_args::resolve_index_path(parsed.db_dir.as_deref()) {
        Ok(p) => p,
        Err(msg) => {
            return astap_write_failure_and_exit(&ini_path, &parsed.cmdline, &msg, "NO_STAR_DATABASE", &msg)
        }
    };
    // A directory holding no `.psidx` file and a directory holding one that
    // fails to open (truncated, corrupt, wrong format) are the same
    // observable outcome from ASTAP's own vantage point: no usable star
    // database. Both get the identical `NO_STAR_DATABASE` wording.
    let index = match Index::open(&index_path) {
        Ok(i) => i,
        Err(_) => {
            return astap_write_failure_and_exit(
                &ini_path,
                &parsed.cmdline,
                astap_args::NO_STAR_DATABASE,
                "NO_STAR_DATABASE",
                astap_args::NO_STAR_DATABASE,
            )
        }
    };

    let bytes = match std::fs::read(&parsed.file) {
        Ok(b) => b,
        Err(e) => {
            // Real ASTAP's own behaviour for this case (ground-truth doc
            // §3c) is `Error, accessing the file!` on stderr, exit 1, and no
            // `.ini` at all -- there is no output base to have written one
            // to that ASTAP itself would consider authoritative. Mirrored
            // here: no sidecar, just the exit code AstroOps actually reads.
            eprintln!("psolve: cannot read {}: {e}", parsed.file);
            return ExitCode::from(1);
        }
    };
    let hdr = psolve_core::fits::FitsHeader::parse(&bytes).ok();

    // Pointing hint: -ra/-spd if given (astap_args::hint_degrees already
    // does the hours/SPD conversion), else the header's own OBJCTRA/OBJCTDEC
    // -- the same fallback native mode's cmd_solve.rs uses. Real AstroOps
    // invocations usually carry OBJCTRA/OBJCTDEC in the frame's own header,
    // so a "blind" `-r 180`-only invocation with no `-ra`/`-spd` typically
    // resolves a hinted centre from there anyway; `resolved_hint` is `None`
    // only for a frame with sentinel/absent pointing throughout, which is
    // exactly the case blind solving below exists for.
    let resolved_hint = astap_args::hint_degrees(&parsed)
        .or_else(|| hdr.as_ref().and_then(psolve_core::fits::hint_radec));

    // A caller-supplied -fov that disagrees with the frame's own header by
    // more than 25% used to win outright and silently -- this is the
    // diagnostic that would have made that legible instead of presenting as
    // an unexplained "Not enough stars." (see `astap_args::search_radius_deg`
    // and `fov_radius_mismatch_warning`'s own doc comments). Hint-
    // independent, so this fires the same whether the eventual path below
    // is hinted or blind.
    if let Some(warning) = astap_args::fov_radius_mismatch_warning(&parsed, hdr.as_ref()) {
        eprintln!("{warning}");
    }
    let radius_deg = astap_args::search_radius_deg(&parsed, hdr.as_ref());

    let opts_base = SolveOptions { hint: None, catalog_epoch: index.header().epoch, ..SolveOptions::default() };

    // `resolved_hint` decides the branch FIRST, before `prepare()` ever
    // runs -- the same ordering fix `cmd_solve.rs`'s native `solve_cmd`
    // makes for the identical reason (see that function's own comment at
    // this same decision point): a hintless, no-quad-index frame must get
    // the "Not enough stars." (`NO_HINT`) failure unconditionally, not
    // whatever `prepare()` would have failed on first.
    let outcome = match resolved_hint {
        Some((hra, hdec)) => {
            let opts = SolveOptions { hint: Some((hra, hdec)), ..opts_base };
            // Decode + background + extract, once, up front: the catalogue
            // depth is sized from this frame's own star count
            // (`cmd_solve::cat_limit_for` -- Task 4's sparse-frame fix; a
            // fixed depth big enough for a dense frame starves a sparse one,
            // and vice versa), and the same extraction is then handed to the
            // solve. It used to be two full passes over the same pixels,
            // which cost ~67 ms per invocation here with no way to opt out:
            // ASTAP's own grammar has no `--cat-limit` equivalent, so every
            // drop-in invocation paid it.
            match psolve_core::solve::prepare(&bytes, &opts) {
                Err(failed) => failed,
                Ok(prepared) => {
                    let limit = cmd_solve::cat_limit_for(prepared.usable_star_count());
                    // Same decision `cmd_solve.rs`'s native path makes -- this
                    // is the ASTAP-compatible dispatch path, so it must not be
                    // left on stale behaviour the way the 2026-08-14 scale
                    // retry was once left here alone (see
                    // `cmd_solve::select_catalog`'s doc).
                    // `None`: ASTAP's CLI grammar has no magnitude flag, and
                    // inventing one here would make this mode diverge from the
                    // tool it exists to be indistinguishable from.
                    let selection =
                        cmd_solve::select_catalog(&index, hra, hdec, radius_deg, limit, None);
                    let catalog: Vec<CatalogStar> = selection
                        .recs
                        .iter()
                        .map(|r| CatalogStar {
                            ra: r.ra_deg(),
                            dec: r.dec_deg(),
                            mag: r.mag(),
                            pmra: r.pmra_mas_yr(),
                            pmdec: r.pmdec_mas_yr(),
                        })
                        .collect();
                    // Same retry `cmd_solve::solve_cmd` uses for native mode,
                    // and for the same reason: this rig's driver pre-
                    // multiplies XPIXSZ by XBINNING, so the header-derived
                    // scale on a bin-2 frame is 2x too coarse and the first
                    // attempt fails. ASTAP's own flag grammar has no scale-
                    // override flag (`opts.scale_arcsec` above is always the
                    // header-derived default, never a caller assertion), so
                    // `explicit_scale_given` is unconditionally `false` here
                    // -- `solve_with_binning_retry`'s own `XBINNING > 1` gate
                    // is what keeps this from firing on unbinned frames, not
                    // this argument. Fix round 1 found this entry point had
                    // been missed entirely: `ingest.identify.astap_solve`
                    // (spec 8.1's drop-in path) calls exactly this function,
                    // so the 810 real bin-2 sv405 frames that motivated the
                    // retry were still unsolvable through the interface that
                    // actually matters until this call was added. Task 7's
                    // blind path (below) is the second such case: it must not
                    // be wired only into native `solve_cmd` and left stale
                    // here.
                    // The retry must redo the CATALOGUE too, not just the
                    // scale: the first disc's radius came from the same
                    // inflated header scale, so it is `XBINNING` times too
                    // wide and the star budget is spent across `XBINNING^2`
                    // times too much sky. Measured 2026-08-22: 0 of 791 real
                    // bin-2 frames solved with the scale corrected and the
                    // catalogue left alone; 790 of 791 solve once the disc is
                    // right. Those 791 frames are 76.7% of every solve
                    // failure in the deployment, and every one of them
                    // arrives through THIS dispatch -- so the fix does not
                    // reach production until it is wired here, which is
                    // exactly what the 2026-08-14 scale retry got wrong at
                    // this same call site.
                    let (header_radius, radius_cap) =
                        astap_args::header_radius_and_cap(&parsed, hdr.as_ref());
                    let attempt = cmd_solve::solve_with_binning_retry(
                        &parsed.file,
                        &prepared,
                        &catalog,
                        &opts,
                        hdr.as_ref(),
                        false,
                        // `None` when the header lacks the optics keywords,
                        // suppressing the refetch rather than substituting a
                        // fallback radius -- the same rule native mode's
                        // `.map()` expresses, so the two surfaces cannot
                        // drift. See `astap_args::header_radius_and_cap`.
                        header_radius.map(|radius_header_deg| cmd_solve::CatalogRefetch {
                            max_mag: None,
                            index: &index,
                            hint_ra: hra,
                            hint_dec: hdec,
                            // UNCAPPED, with `-r` carried separately in
                            // `radius_cap`: the retry halves this and then
                            // re-applies the ceiling, which is a different
                            // number from halving an already-capped radius
                            // whenever `-r` bound the first fetch.
                            radius_header_deg,
                            radius_cap: Some(radius_cap),
                            limit,
                            // ASTAP's grammar has no radius-assertion flag.
                            // `-r` is a ceiling, applied above, not a
                            // caller-chosen disc -- unlike native `--radius`,
                            // so it must not suppress the refetch the way
                            // that flag does.
                            explicit_radius: false,
                        }),
                        Some(&bytes),
                    );
                    attempt.outcome
                }
            }
        }
        None => match astap_args::resolve_quad_index_path(parsed.db_dir.as_deref())
            .and_then(|p| psolve_index::quad_reader::QuadIndex::open(&p, &index).ok())
        {
            // No hint anywhere AND no usable `.psqidx` in `-d`/`-D` --
            // matching native mode's identical early-return for the same
            // condition (`cmd_solve.rs`). "Not enough stars." is the closer
            // of ASTAP's two documented failure strings for a solve that
            // never got underway; ASTAP's own wording has no separate "no
            // pointing hint" case among the two ever observed. The `.ini`
            // string stays that generic wording -- ASTAP-compatible
            // consumers parse it -- but `psolve_core::error::ReasonCode::
            // NoHint` still goes to stderr as psolve's own machine-readable
            // reason, same as every other compat failure below. A `.psqidx`
            // that exists but fails to open (missing pair, fingerprint
            // mismatch, corrupt) is silently treated the same as one that
            // does not exist -- ASTAP-compatible mode is auto-discovery
            // best-effort throughout (see `resolve_index_path`'s own doc for
            // the star-index side of that same philosophy).
            None => {
                return astap_write_failure_and_exit(
                    &ini_path,
                    &parsed.cmdline,
                    "Not enough stars.",
                    psolve_core::error::ReasonCode::NoHint.as_str(),
                    "no pointing hint: pass -ra/-spd or supply OBJCTRA/OBJCTDEC",
                );
            }
            Some(quad_index) => match psolve_core::solve::prepare(&bytes, &opts_base) {
                Err(failed) => failed,
                Ok(prepared) => {
                    let limit = cmd_solve::cat_limit_for(prepared.usable_star_count());
                    let search = cmd_solve::solve_blind(
                        &parsed.file,
                        &prepared,
                        hdr.as_ref(),
                        &index,
                        &quad_index,
                        radius_deg,
                        limit,
                        // ASTAP mode has no magnitude flag; see the
                        // select_catalog call above.
                        None,
                        &opts_base,
                        false, // no scale-override flag exists in ASTAP's own grammar
                    );
                    eprintln!(
                        "psolve: blind search -- {} image quads, {} hypotheses offered, {} \
candidate transform(s) survived, {} cluster(s) attempted",
                        search.image_quads, search.hypotheses, search.survivors, search.clusters_tried,
                    );
                    search.outcome
                }
            },
        },
    };

    match outcome {
        Outcome::Solved(s) => {
            let wcs_path = astap_sidecar_base(&parsed).with_extension("wcs");
            // Both sidecar paths are cleared BEFORE either is written, not
            // one immediately before its own write: refusing the second
            // after the first has already landed would leave a
            // half-honoured invocation behind, which is the shape this
            // guard exists to prevent.
            if sidecar_write_refused(&ini_path) || sidecar_write_refused(&wcs_path) {
                return ExitCode::from(1);
            }

            let ini = sidecar::format_ini_success(&s.wcs, &parsed.cmdline);
            if let Err(e) = std::fs::write(&ini_path, ini) {
                eprintln!("psolve: warning: could not write {}: {e}", ini_path.display());
            }

            let original_header = astap_wcs_original_header(&bytes, hdr.as_ref());
            let write_result = if parsed.wcs_fits_block {
                std::fs::write(&wcs_path, sidecar::format_wcs_fits_block(&s.wcs, &original_header))
            } else {
                std::fs::write(&wcs_path, sidecar::format_wcs_text(&s.wcs, &original_header))
            };
            if let Err(e) = write_result {
                eprintln!("psolve: warning: could not write {}: {e}", wcs_path.display());
            }

            // Default OFF: only -update enables this, and it is the one
            // path in the crate that rewrites the user's own frame (see
            // fits_update.rs's module doc for the full safety model). A
            // refusal or a hard failure here -- readonly marker,
            // PSOLVE_READONLY, header growth, verify mismatch -- means the
            // invocation as given was not fully honoured, so it is reported
            // as this mode's one failure exit code, 1, not native mode's 3:
            // see this function's own doc comment for why the two schemes
            // are deliberately decoupled. The sidecars above are left in
            // place either way -- the solve itself genuinely succeeded, and
            // they say so honestly regardless of whether the in-place
            // header write also did.
            if parsed.update {
                if let Err(e) = fits_update::update_header_in_place(Path::new(&parsed.file), &s.wcs) {
                    eprintln!("psolve: {e}");
                    return ExitCode::from(1);
                }
            }

            ExitCode::SUCCESS
        }
        // Every solve failure, whatever psolve-core's own reason code says,
        // collapses to ASTAP's other documented failure string: real ASTAP
        // has exactly two (ground-truth doc §1e/§3c), and "Not enough
        // stars." is the one that means "a solve was attempted and did not
        // succeed" rather than "no database was even found". The `.ini`
        // stays that generic wording -- ASTAP-compatible consumers parse it
        // and must not see it change -- but the real `reason`/`detail`
        // psolve-core already computed still goes to stderr, so a radius
        // problem (NO_QUAD_MATCH) is diagnosable as a radius problem instead
        // of reading as a star-count problem.
        Outcome::Failed { reason, detail, .. } => astap_write_failure_and_exit(
            &ini_path,
            &parsed.cmdline,
            "Not enough stars.",
            reason.as_str(),
            &detail,
        ),
    }
}

/// Print a `psolve: warning:` line naming exactly which of `-z`/`-s`/`-t`/
/// `-m` were present in this invocation, if any -- the disclosure
/// `astap_cmd`'s doc comment calls for. Named by their real ASTAP flags
/// (`-z`/`-s`/`-t`/`-m`), not their `AstapArgs` field names, since that is
/// what an operator reading a pipeline's stderr actually typed or
/// generated.
fn warn_about_unwired_flags(a: &astap_args::AstapArgs) {
    let mut ignored: Vec<&str> = Vec::new();
    if a.downsample.is_some() {
        ignored.push("-z");
    }
    if a.max_stars.is_some() {
        ignored.push("-s");
    }
    if a.tolerance.is_some() {
        ignored.push("-t");
    }
    if a.min_star_arcsec.is_some() {
        ignored.push("-m");
    }
    if !ignored.is_empty() {
        eprintln!(
            "psolve: warning: {} accepted but not applied to the solve (no verified psolve \
             equivalent for {} yet)",
            ignored.join(", "),
            if ignored.len() == 1 { "it" } else { "them" }
        );
    }
}

/// `-o`'s output base path when given, else the input file's own path with
/// its extension stripped -- ASTAP's own documented default (`-o`: "Name the
/// output files with this base path & file name"; absent, the sidecars sit
/// beside the input file).
fn astap_sidecar_base(a: &astap_args::AstapArgs) -> PathBuf {
    match &a.out_base {
        Some(o) => PathBuf::from(o),
        None => Path::new(&a.file).with_extension(""),
    }
}

/// Write ASTAP's failure `.ini` (leading blank line, `CMDLINE=` before
/// `ERROR=` -- `sidecar::format_ini_failure`) and return ASTAP's own
/// failure exit code, `1`. A failure to write the sidecar itself is reported
/// to stderr but does not change the exit code: the failure `error` names
/// already happened, and a sidecar the caller cannot write is a second,
/// independent problem, not a reason to hide the first.
///
/// `error` is ASTAP's own observed wording (unchanged, byte-exact -- real
/// consumers parse the `.ini`'s `ERROR=` key, and it only has two documented
/// strings to choose from). `reason_code`/`detail` are psolve's OWN,
/// unshared diagnostic: several distinct internal causes (a too-narrow
/// search disc, too few detected stars, a hint that does not match the
/// field, a shallow catalogue, ...) all collapse to the same "Not enough
/// stars." `.ini` wording, so without a second channel a radius defect and a
/// star-count defect are indistinguishable from the sidecar alone. Printed
/// to stderr, never the `.ini`, so the sidecar format is untouched.
fn astap_write_failure_and_exit(
    ini_path: &Path,
    cmdline: &str,
    error: &str,
    reason_code: &str,
    detail: &str,
) -> ExitCode {
    // The failure path needs the guard just as much as the success path --
    // arguably more. This is the branch a *misconfigured* invocation takes
    // (a bad `-d`, a `-f` naming a file that does not exist), and it runs
    // before the input has even been read, so without this an accidental
    // run inside a protected tree replaced a recorded `PLTSOLVD=T` ASTAP
    // solution with a `PLTSOLVD=F` failure file. The exit code is `1`
    // either way, so refusing costs the caller no new outcome to handle.
    if !sidecar_write_refused(ini_path) {
        let ini = sidecar::format_ini_failure(cmdline, error);
        if let Err(e) = std::fs::write(ini_path, &ini) {
            eprintln!("psolve: warning: could not write {}: {e}", ini_path.display());
        }
    }
    eprintln!("psolve: {error}");
    eprintln!("psolve: reason={reason_code} detail={detail:?}");
    ExitCode::from(1)
}

/// Apply `-update`'s own two safety switches -- `PSOLVE_READONLY` and a
/// `.psolve-readonly` marker on any of the target's ancestor chains -- to a
/// sidecar this process is about to write, reporting a refusal on stderr.
///
/// A `.ini`/`.wcs` write is not a pixel-data rewrite, but it does overwrite
/// files that are exactly as irreplaceable: real recorded ASTAP output
/// sitting beside the frames it describes. `docs/astap-compat.md` tells
/// users the marker is "the guarantee to rely on ... to protect a tree", so
/// it has to protect the tree, not just the one write path the guard
/// happened to be written for.
///
/// The refusal is deliberately not fatal *by itself* in the failure-path
/// caller (which already exits `1`); every caller either returns
/// `ExitCode::from(1)` on `Err` or is already on its way to it, so a refused
/// invocation is never reported as success.
fn sidecar_write_refused(path: &Path) -> bool {
    match fits_update::refuse_if_readonly_output(path) {
        Ok(()) => false,
        Err(e) => {
            eprintln!("psolve: {}: {e}; no sidecar was written", path.display());
            true
        }
    }
}

/// The `.wcs` sidecar's pass-through original-header section: every card of
/// the source FITS file's own header, byte-exact and in order, except
/// `BITPIX` (forced to `8`) and `NAXIS` (forced to `0`, with ASTAP's own
/// "Dimensionality" comment) and `NAXIS1`/`NAXIS2` (dropped entirely). This
/// is the transform `sidecar::format_wcs_text`'s own doc comment names as
/// "the caller's job" -- reverse-engineered from a real `.wcs` file's header
/// prefix diffed against its source FITS file's (ground-truth doc §2a: a
/// `.wcs` describes no pixel data of its own, so ASTAP downgrades the axis
/// keywords that would otherwise describe pixel data that is not there).
///
/// Reuses `fits_update::raw_header_cards`/`card_key` -- the same byte-exact,
/// non-lossy card scanner the `-update` write path uses -- rather than a
/// second, independent re-derivation from `FitsHeader`'s own parsed card
/// list (which strips comments and quoting and, notably, does not even
/// surface `COMMENT` cards at all: `FitsHeader::parse` only keeps cards with
/// `=` at byte 8).
///
/// Falls back to an empty pass-through (the WCS solution keywords only, no
/// original-header section) if the source header could not be parsed at
/// all. Unreachable in practice by the time this is called -- a frame whose
/// header does not parse never reaches a successful `Outcome::Solved` for
/// this function to be called from -- but a defensive fallback on untrusted
/// input is cheaper than a panic.
fn astap_wcs_original_header(bytes: &[u8], hdr: Option<&psolve_core::fits::FitsHeader>) -> String {
    let Some(hdr) = hdr else { return String::new() };
    let raw_cards = fits_update::raw_header_cards(bytes, hdr.data_offset);
    let mut lines = Vec::with_capacity(raw_cards.len());
    for card in &raw_cards {
        match fits_update::card_key(card).as_str() {
            "BITPIX" => lines.push(format!("{:<80}", "BITPIX  =                    8")),
            "NAXIS" => lines.push(format!("{:<80}", "NAXIS   =                    0 / Dimensionality")),
            "NAXIS1" | "NAXIS2" => {}
            _ => lines.push(String::from_utf8_lossy(card).into_owned()),
        }
    }
    lines.join("\n")
}
