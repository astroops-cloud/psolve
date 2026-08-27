//! Build-time identifier for the solve JSON's `build` field.
//!
//! Why this exists: a downstream consumer cached 2,000 `psolve solve`
//! results keyed in part on `"psolve":"0.1.0"` -- the crate version. psolve
//! was rebuilt from an edited working tree several times in one day; 202 of
//! those 2,000 cached outcomes had actually changed, but the declared
//! version had not moved, because nothing bumps `Cargo.toml`'s `version` on
//! every behaviour-changing commit. `build` is the field meant to move
//! instead: it is derived from `git` at compile time, so a source change
//! without a version bump still produces a different `build` value, and a
//! dirty working tree (uncommitted edits) is distinguishable from a clean
//! build of the same commit.
//!
//! This lives in `psolve-cli`, not `psolve-core`: `psolve-core`'s
//! `no_filesystem` guard tokenises whole words, including inside comments,
//! and `process` is on its forbidden list (see that crate's lib.rs) --
//! shelling out to `git` from a `psolve-core` build script would trip it.
//! `psolve-cli` carries no such guard.
//!
//! No new dependency: everything here is `std::process::Command` plus
//! `println!` to talk to Cargo, exactly as `build.rs` scripts always could.

use std::process::Command;

fn main() {
    let build_id = git_build_id().unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=PSOLVE_BUILD_ID={build_id}");

    // `git describe --dirty`'s suffix depends on the *content* of the whole
    // working tree, not just which commit HEAD points at -- there is no
    // small, fixed set of paths whose mtimes could be watched instead that
    // would stay correct as files anywhere in the repo (not just this crate)
    // are added, removed or edited. Cargo's documented behaviour for
    // `rerun-if-changed` naming a path that does not exist is to always
    // consider the build script out of date, so this reruns -- and
    // re-shells out to `git` -- on every build. `git describe` itself costs
    // low-single-digit milliseconds, so the always-rerun cost is not worth
    // trading away for the risk of a fixed watch-list going stale, which is
    // exactly the caching defect this field exists to fix, reproduced in the
    // build script itself.
    println!("cargo:rerun-if-changed=BUILD_ID_ALWAYS_RERUN");
}

/// `git describe --tags --always --dirty`, falling back to a bare short SHA,
/// or `None` if `git` is absent, this is not a repository (e.g. a source
/// tarball with no `.git`), or the repository has no commits at all. Never
/// panics and never fails the build -- an inability to identify the build is
/// not a reason to refuse to produce one; the caller turns `None` into the
/// honest, non-fabricated `"unknown"`.
fn git_build_id() -> Option<String> {
    if let Some(id) = run_git(["describe", "--tags", "--always", "--dirty"]) {
        return Some(id);
    }
    // `git describe` can fail outright (e.g. a shallow clone with no tags
    // reachable the way it wants) even though `git` itself is present and
    // this is a real repository; a bare short SHA is still honest and still
    // strictly better than a fabricated or stale value.
    run_git(["rev-parse", "--short", "HEAD"])
}

/// Run `git <args>`, in the package root (Cargo's documented CWD for build
/// scripts), and return trimmed stdout on success. `None` on any failure:
/// `git` missing from `PATH`, not a repository, non-UTF-8 output, or a
/// non-zero exit -- all collapse to the same "could not identify" outcome.
fn run_git<const N: usize>(args: [&str; N]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let s = String::from_utf8(output.stdout).ok()?;
    let s = s.trim();
    if s.is_empty() { None } else { Some(s.to_string()) }
}
