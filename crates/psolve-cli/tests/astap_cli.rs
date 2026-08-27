//! Black-box tests for the ASTAP-compatible mode dispatch in `main.rs`.
//!
//! `psolve-cli` has no `[lib]` target, so these spawn the compiled binary
//! (the same pattern `cli_solve.rs`/`cli_build.rs` already use) rather than
//! calling `astap_cmd`/`parse_astap` directly. Unit coverage for the parser
//! itself (`AstapArgs`, `parse_astap`, `hint_degrees`, `search_radius_deg`,
//! `resolve_index_path`) lives in `src/astap_args.rs`'s co-located `mod
//! tests`; this file exercises the dispatch: does `-f` in argv actually
//! route into ASTAP mode, does a usage error there get ASTAP's own exit
//! code (`1`, not native mode's `2`), and does a well-formed real recorded
//! invocation reach the real solve/sidecar/exit-code machinery. A full
//! solve-to-success run (real frame, real index, `.ini`/`.wcs` sidecars,
//! `-update`, and the COMMENT-card idempotency fix) lives in
//! `tests/astap_exit_codes.rs`, which is the file the M3 Task 10 brief
//! names -- this file stays focused on dispatch and usage-error routing.

use std::process::Command;

fn bin() -> std::path::PathBuf {
    let mut p = std::env::current_exe().unwrap();
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join(format!("psolve{}", std::env::consts::EXE_SUFFIX))
}

fn scratch_dir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("psolve-astap-cli-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A directory that really exists and holds no `.psidx`.
///
/// This was a hardcoded path to real ASTAP's own install directory, so "a
/// real directory that holds no index" was true on one machine and false
/// everywhere else -- CI included, where the tests passed regardless.
///
/// Measured 2026-08-27, and it is less than it looks: the callers'
/// `"No star database found."` fires identically for a nonexistent directory,
/// an empty one, and one holding an unreadable `.psidx`. It changes only when
/// a RESOLVABLE index is present, at which point the run gets as far as
/// reading the frame (`cannot read /nonexistent/y.fits`). So this temp dir
/// buys portability and a comment that is true on every machine; it does NOT
/// strengthen the assertion. What the callers actually pin is "dispatch
/// reached index resolution instead of failing to parse", which is what their
/// own doc comments claim.
fn empty_db_dir(tag: &str) -> std::path::PathBuf {
    scratch_dir(tag)
}

/// Native mode must behave exactly as it did before ASTAP mode existed --
/// the `-f` scan in `main.rs` must not fire for an invocation that never
/// mentions `-f` at all.
#[test]
fn a_native_invocation_is_unaffected_by_astap_mode_detection() {
    let o = Command::new(bin()).args(["solve"]).output().unwrap();
    assert_eq!(o.status.code(), Some(2), "native usage error must still be exit 2");
    let s = String::from_utf8_lossy(&o.stderr);
    assert!(s.contains("<FILE> is required"), "stderr was: {s}");
}

#[test]
fn native_unknown_command_still_prints_usage_and_exits_2() {
    let o = Command::new(bin()).args(["bogus"]).output().unwrap();
    assert_eq!(o.status.code(), Some(2));
    let s = String::from_utf8_lossy(&o.stderr);
    assert!(s.contains("unknown command"), "stderr was: {s}");
}

/// The mere presence of `-f` anywhere in argv must enter ASTAP mode -- here
/// proven by the ASTAP-specific error message appearing (an unknown ASTAP
/// flag), which native mode would never produce. A malformed ASTAP
/// invocation is a usage error, but ASTAP mode's own exit-code scheme has no
/// usage-error code distinct from "everything else": it collapses to `1`,
/// not native mode's `2` (Task 10; see `main.rs`'s `astap_cmd` doc comment).
#[test]
fn a_dash_f_invocation_enters_astap_mode() {
    let o = Command::new(bin()).args(["-f", "/nonexistent/x.fits", "-bogus"]).output().unwrap();
    assert_eq!(o.status.code(), Some(1));
    let s = String::from_utf8_lossy(&o.stderr);
    assert!(s.contains("unknown ASTAP flag"), "stderr was: {s}");
}

/// The two surfaces must not blend: a native `--index` flag alongside ASTAP
/// mode's `-f` is rejected, not silently accepted as an extra.
#[test]
fn dash_f_with_native_index_flag_is_rejected() {
    let o = Command::new(bin())
        .args(["-f", "/nonexistent/x.fits", "--index", "i.psidx"])
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(1));
    let s = String::from_utf8_lossy(&o.stderr);
    assert!(s.contains("unknown ASTAP flag"), "stderr was: {s}");
}

/// A value-taking flag with nothing after it is a usage error, not a panic
/// -- and, like every other ASTAP-mode usage error, exit `1`.
#[test]
fn a_flag_with_no_value_exits_1_not_a_panic() {
    let o = Command::new(bin()).args(["-f"]).output().unwrap();
    assert_eq!(o.status.code(), Some(1), "stderr: {}", String::from_utf8_lossy(&o.stderr));
    let s = String::from_utf8_lossy(&o.stderr);
    assert!(s.contains("requires a value"), "stderr was: {s}");
}

/// An unrecognized single-dash flag is a usage error, not a panic.
#[test]
fn an_unknown_single_dash_flag_exits_1_not_a_panic() {
    let o = Command::new(bin())
        .args(["-f", "/nonexistent/x.fits", "-nonsense", "1"])
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(1), "stderr: {}", String::from_utf8_lossy(&o.stderr));
    let s = String::from_utf8_lossy(&o.stderr);
    assert!(s.contains("unknown ASTAP flag"), "stderr was: {s}");
}

/// `-z`/`-s`/`-t`/`-m` are accepted (never rejected as an unknown flag) but
/// not applied to the solve -- `AstapArgs`'s own field doc explains why no
/// mapping is guessed. That must not be silent to the operator: ASTAP mode
/// prints no `--help` of its own reachable from a `-f` invocation, so a
/// `psolve: warning:` line on stderr, naming exactly the flags given, is the
/// only place a pipeline watching stderr (as AstroOps does) could see it.
/// Fix round 1 of the M3 Task 10 review.
#[test]
fn unwired_flags_warn_on_stderr_naming_only_the_ones_given() {
    let o = Command::new(bin())
        .args(["-f", "/nonexistent/x.fits", "-d", "/nonexistent-db", "-s", "300", "-t", "0.01"])
        .output()
        .unwrap();
    let s = String::from_utf8_lossy(&o.stderr);
    assert!(s.contains("psolve: warning:"), "stderr was: {s}");
    assert!(s.contains("-s") && s.contains("-t"), "must name the flags actually given: {s}");
    assert!(!s.contains("-z,") && !s.contains(", -z") && !s.contains("-m,"), "must not name flags that were not given: {s}");
}

/// No `-z`/`-s`/`-t`/`-m` at all must mean no unwired-flags warning --
/// checked by the specific wording, not by the presence of ANY
/// `psolve: warning:` line, since other unrelated warnings (e.g. a sidecar
/// the process could not write to `/nonexistent/...`) legitimately share
/// that prefix and must not make this test look like it is asserting more
/// than it is.
#[test]
fn no_warning_when_none_of_the_four_flags_are_given() {
    let o = Command::new(bin())
        .args(["-f", "/nonexistent/x.fits", "-d", "/nonexistent-db"])
        .output()
        .unwrap();
    let s = String::from_utf8_lossy(&o.stderr);
    assert!(!s.contains("accepted but not applied"), "stderr was: {s}");
}

/// The real AstroOps blind invocation, verbatim, must be accepted by
/// dispatch (parsed successfully, not rejected as a usage error) and must
/// reach the real solve machinery -- proven here by the failure reason
/// being the DATABASE ("No star database found.", from `-d` naming a real
/// directory that holds no psolve `.psidx` index), not a parse error. The
/// frame path does not exist either, but index resolution runs first, so
/// that never gets checked in this invocation.
#[test]
fn the_real_blind_invocation_is_accepted_by_dispatch_and_reaches_the_solve_path() {
    let o = Command::new(bin())
        .args([
            "-f",
            "/nonexistent/y.fits",
            "-r",
            "180",
            "-fov",
            "1.4770",
            "-d",
            empty_db_dir("blind-db").to_str().unwrap(),
            "-update",
        ])
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(1), "stderr: {}", String::from_utf8_lossy(&o.stderr));
    let s = String::from_utf8_lossy(&o.stderr);
    assert!(s.contains("No star database found."), "stderr was: {s}");
}

/// The real AstroOps hinted retry, verbatim, must likewise be accepted by
/// dispatch and reach the same database-resolution failure -- proving the
/// `-ra`/`-spd`/`-r`/`-fov` combination this invocation carries parses
/// cleanly (the unit-level hours/SPD conversion itself is pinned directly in
/// `astap_args.rs`'s own tests).
#[test]
fn the_real_hinted_retry_is_accepted_by_dispatch_and_reaches_the_solve_path() {
    let o = Command::new(bin())
        .args([
            "-f",
            "/nonexistent/y.fits",
            "-ra",
            "16.950000",
            "-spd",
            "49.666667",
            "-r",
            "15",
            "-fov",
            "1.4770",
            "-d",
            empty_db_dir("hinted-db").to_str().unwrap(),
            "-update",
        ])
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(1), "stderr: {}", String::from_utf8_lossy(&o.stderr));
    let s = String::from_utf8_lossy(&o.stderr);
    assert!(s.contains("No star database found."), "stderr was: {s}");
}

/// `AstapArgs.cmdline`, as reported through the real compiled binary's
/// failure `.ini` (`CMDLINE=`), must begin with the program path, not just
/// the flags -- the same fact `astap_args.rs`'s own
/// `cmdline_begins_with_the_program_path` unit test pins at the parser
/// level, checked here end to end through the actual sidecar file the
/// binary writes.
#[test]
fn the_written_ini_cmdline_begins_with_the_program_path() {
    let dir = scratch_dir("cmdline-e2e");
    let out_base = dir.join("out");
    let o = Command::new(bin())
        .args(["-f", "/nonexistent/y.fits", "-r", "180", "-d", "/nonexistent-db", "-o"])
        .arg(&out_base)
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(1), "stderr: {}", String::from_utf8_lossy(&o.stderr));

    let ini = std::fs::read_to_string(out_base.with_extension("ini"))
        .unwrap_or_else(|e| panic!("reading the failure .ini: {e}"));
    let bin_path = bin();
    let bin_str = bin_path.to_string_lossy();
    let cmdline_line = ini
        .lines()
        .find(|l| l.starts_with("CMDLINE="))
        .unwrap_or_else(|| panic!("no CMDLINE= line in {ini:?}"));
    assert_eq!(
        cmdline_line,
        format!(
            "CMDLINE={bin_str} -f /nonexistent/y.fits -r 180 -d /nonexistent-db -o {}",
            out_base.display()
        )
    );
    assert!(ini.contains("ERROR=No star database found."), "ini was: {ini:?}");

    let _ = std::fs::remove_dir_all(&dir);
}
