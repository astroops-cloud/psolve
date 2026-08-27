//! Pins the solve JSON's `build` field (spec §7.2): present and non-empty on
//! both a successful and a failed solve, and honestly `"unknown"` -- never
//! malformed, never fabricated, never a stale value from an earlier build --
//! when `git` cannot identify the source tree.
//!
//! The "`git` unavailable" case is simulated by literally building
//! `psolve-cli` from a fresh copy of the workspace sources with **no
//! `.git`** anywhere in the copy's ancestry (a temp directory, matching how
//! every other test in this crate isolates its scratch files). `build.rs`'s
//! `run_git` collapses "`git` missing from `PATH`" and "`git` present but
//! this is not a repository" to the exact same `None` return -- the file's
//! own doc comment says so -- so exercising the "not a repository" half
//! through a real `cargo build` exercises the same code path a source
//! tarball with no `git` binary at all would hit, without needing to hide
//! `git` from `PATH` (which on this machine sits in the same directory as
//! `cargo`/`rustc`, so removing it would break the build for an unrelated
//! reason).

use std::path::Path;
use std::process::Command;

fn bin() -> std::path::PathBuf {
    let mut p = std::env::current_exe().unwrap();
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("psolve")
}

fn tmpdir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("psolve-build-id-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn workspace_root() -> std::path::PathBuf {
    // This crate's manifest is <root>/crates/psolve-cli.
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().parent().unwrap().to_path_buf()
}

/// Copy just enough of the workspace to build `psolve-cli` -- the two
/// manifests and the three crate source trees -- into `dest`, deliberately
/// leaving `.git` behind. `cp -R` rather than a hand-rolled walker: every
/// test in this crate already shells out for its real work (building
/// indexes, running the compiled binary); copying a source tree is no
/// different in kind.
fn copy_workspace_without_git(dest: &Path) {
    let root = workspace_root();
    std::fs::create_dir_all(dest).unwrap();
    for f in ["Cargo.toml", "Cargo.lock"] {
        let o = Command::new("cp").arg(root.join(f)).arg(dest.join(f)).output().unwrap();
        assert!(o.status.success(), "copying {f}: {}", String::from_utf8_lossy(&o.stderr));
    }
    let crates_dest = dest.join("crates");
    std::fs::create_dir_all(&crates_dest).unwrap();
    for c in ["psolve-cli", "psolve-core", "psolve-index"] {
        let o = Command::new("cp")
            .args(["-R"])
            .arg(root.join("crates").join(c))
            .arg(crates_dest.join(c))
            .output()
            .unwrap();
        assert!(o.status.success(), "copying crates/{c}: {}", String::from_utf8_lossy(&o.stderr));
    }
    assert!(!dest.join(".git").exists(), "the copy must not carry a .git directory");
}

/// Build `psolve-cli` from `manifest_dir` (a `copy_workspace_without_git`
/// destination) into its own `target/`, and return the compiled binary's
/// path. `--offline`: this only needs crates already resolved in the
/// checked-in `Cargo.lock` / already present in the local registry cache,
/// and must not depend on network access being available in CI.
fn build_no_git_binary(manifest_dir: &Path) -> std::path::PathBuf {
    let target_dir = manifest_dir.join("target");
    let o = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string()))
        .args(["build", "--offline", "-p", "psolve-cli"])
        .arg("--manifest-path")
        .arg(manifest_dir.join("Cargo.toml"))
        .arg("--target-dir")
        .arg(&target_dir)
        .output()
        .unwrap();
    assert!(
        o.status.success(),
        "building the no-.git copy failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
    target_dir.join("debug").join("psolve")
}

/// Minimal valid FITS with no stars -- copied verbatim from `cli_solve.rs`
/// (each integration test binary in this crate stands on its own; see that
/// file's own copy for why).
fn blank_fits(path: &Path) {
    let cards = [
        "SIMPLE  =                    T",
        "BITPIX  =                   16",
        "NAXIS   =                    2",
        "NAXIS1  =                   64",
        "NAXIS2  =                   64",
        "BZERO   =                32768",
    ];
    let mut s = String::new();
    for c in cards {
        s.push_str(&format!("{c:<80}"));
    }
    s.push_str(&format!("{:<80}", "END"));
    while !s.len().is_multiple_of(2880) {
        s.push(' ');
    }
    let mut out = s.into_bytes();
    out.extend(std::iter::repeat_n(0u8, 64 * 64 * 2));
    while !out.len().is_multiple_of(2880) {
        out.push(0);
    }
    std::fs::write(path, out).unwrap();
}

/// A tiny but valid index, built through whichever `psolve` binary is
/// passed in -- index building touches no `git` state, so either the normal
/// test binary or the no-`.git` binary built above works identically here.
fn make_index(psolve: &Path, d: &Path) -> std::path::PathBuf {
    let input = d.join("cat");
    std::fs::create_dir_all(&input).unwrap();
    let mut csv = String::from("ra,dec,pmra,pmdec,phot_g_mean_mag\n");
    for i in 0..200 {
        let t = i as f64;
        csv.push_str(&format!(
            "{:.6},{:.6},0,0,{:.2}\n",
            (100.0 + (t * 0.013) % 1.5),
            (20.0 + (t * 0.011) % 1.0),
            10.0 + (i % 40) as f64 * 0.1
        ));
    }
    std::fs::write(input.join("a.csv"), csv).unwrap();
    let out = d.join("t.psidx");
    let o = Command::new(psolve)
        .args(["index", "build", "--input"])
        .arg(&input)
        .arg("--out")
        .arg(&out)
        .args(["--max-mag", "20", "--nside", "64"])
        .output()
        .unwrap();
    assert!(o.status.success(), "index build failed: {}", String::from_utf8_lossy(&o.stderr));
    out
}

/// Pull `"build":"..."` out of a solve JSON line.
fn extract_build(json: &str) -> Option<String> {
    let key = "\"build\":\"";
    let start = json.find(key)? + key.len();
    let end = json[start..].find('"')? + start;
    Some(json[start..end].to_string())
}

#[test]
fn a_failed_solve_carries_a_non_empty_build_id_and_the_resolved_index() {
    let d = tmpdir("failed");
    let idx = make_index(&bin(), &d);
    let f = d.join("blank.fits");
    blank_fits(&f);
    let o = Command::new(bin())
        .args(["solve"])
        .arg(&f)
        .arg("--index")
        .arg(&idx)
        .args(["--hint", "100.0,20.0"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&o.stdout).to_string();
    assert!(stdout.contains("\"solved\":false"), "stdout: {stdout}");

    let build = extract_build(&stdout).unwrap_or_else(|| panic!("no build field in: {stdout}"));
    assert!(!build.is_empty(), "build must be non-empty: {stdout}");

    // This is the exact fix for the second half of the incident: index must
    // now appear on a failure path that had one resolved, not only on
    // success.
    assert!(stdout.contains("\"index\":{\"name\":"), "stdout: {stdout}");
}

#[test]
fn a_no_hint_failure_before_solving_still_carries_the_already_resolved_index() {
    // NO_HINT is reached after --index is opened but before the solve
    // itself runs -- exactly the "index resolved, solve not yet attempted"
    // case the spec calls out.
    let d = tmpdir("nohint");
    let idx = make_index(&bin(), &d);
    let f = d.join("blank.fits");
    blank_fits(&f);
    let o = Command::new(bin()).args(["solve"]).arg(&f).arg("--index").arg(&idx).output().unwrap();
    let stdout = String::from_utf8_lossy(&o.stdout).to_string();
    assert!(stdout.contains("\"reason\":\"NO_HINT\""), "stdout: {stdout}");
    assert!(stdout.contains("\"index\":{\"name\":"), "stdout: {stdout}");
    assert!(extract_build(&stdout).is_some_and(|b| !b.is_empty()), "stdout: {stdout}");
}

#[test]
fn a_missing_index_error_before_index_resolution_carries_no_index_field() {
    // The exit-3 "cannot open index" path never reaches JSON at all (it is
    // an eprintln! usage/config error), so stdout is empty and there is
    // nothing to assert about `index` there -- this test pins that it stays
    // that way rather than growing a placeholder.
    let d = tmpdir("noindex");
    let f = d.join("blank.fits");
    blank_fits(&f);
    let o = Command::new(bin())
        .args(["solve"])
        .arg(&f)
        .args(["--index", "/nonexistent/none.psidx", "--hint", "100.0,20.0"])
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(3));
    assert!(o.stdout.is_empty(), "an index-open failure must emit no JSON at all: {:?}", o.stdout);
}

/// The slow one: a real `cargo build` from a `.git`-less copy of the
/// workspace. Confirms `build.rs` falls back to the honest `"unknown"`
/// rather than failing the build or emitting something malformed/fabricated.
#[test]
fn a_build_with_no_git_repository_falls_back_to_unknown() {
    let d = tmpdir("no-git-build");
    let copy = d.join("workspace");
    copy_workspace_without_git(&copy);
    let psolve = build_no_git_binary(&copy);
    assert!(psolve.exists(), "the no-.git build did not produce a binary at {psolve:?}");

    let run_dir = d.join("run");
    std::fs::create_dir_all(&run_dir).unwrap();
    let idx = make_index(&psolve, &run_dir);
    let f = run_dir.join("blank.fits");
    blank_fits(&f);

    let o = Command::new(&psolve)
        .args(["solve"])
        .arg(&f)
        .arg("--index")
        .arg(&idx)
        .args(["--hint", "100.0,20.0"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&o.stdout).to_string();
    assert!(stdout.contains("\"solved\":false"), "stdout: {stdout}");
    assert_eq!(
        extract_build(&stdout).as_deref(),
        Some("unknown"),
        "a source tree with no .git must report build:\"unknown\", not fail, \
         not omit the field, and not fabricate a value: {stdout}"
    );
    // Still valid JSON, still carries the fields a failure must carry.
    let t = stdout.trim();
    assert!(t.starts_with('{') && t.ends_with('}'), "stdout was: {stdout}");
    assert!(!t.contains("NaN") && !t.contains(":inf"), "invalid JSON tokens: {stdout}");
}

/// `--version` must be a recognised command that exits 0.
///
/// It was not, until 2026-08-24, and the absence cost two people an hour each:
/// this repo's own `deb` CI job used it as the post-install smoke test and
/// failed loudly, and the AstroOps container Dockerfile used it as a build
/// probe, ignored the exit code, and silently baked an unverified binary. The
/// second is the worse failure -- an unrecognised flag that a caller does not
/// check reads exactly like a successful check.
///
/// Pinned here rather than left to the usage banner because a binary that
/// ships in a `.deb` and a container image is expected to answer this, and
/// because "it prints the version somewhere in `--help`" is not the same
/// contract.
#[test]
fn version_flags_are_recognised_and_carry_the_build_id() {
    for flag in ["--version", "-V", "version"] {
        let out = std::process::Command::new(bin())
            .arg(flag)
            .output()
            .expect("psolve must be runnable");
        assert!(
            out.status.success(),
            "`psolve {flag}` exited {:?}; before 2026-08-24 this was `unknown command` and \
exit 2, which is what broke two callers",
            out.status.code()
        );
        let s = String::from_utf8_lossy(&out.stdout);
        assert!(s.starts_with("psolve "), "`psolve {flag}` printed {s:?}");
        assert!(
            s.contains(env!("CARGO_PKG_VERSION")),
            "`psolve {flag}` must carry the crate version: {s:?}"
        );
        // The build id is the field that actually moves between builds --
        // `psolve` alone is 0.1.0 on every build ever made. A consumer once
        // cached 2,000 solve results keyed on a version that never moved.
        assert!(
            s.contains('(') && s.contains(')'),
            "`psolve {flag}` must carry the build id in parentheses: {s:?}"
        );
    }
}
