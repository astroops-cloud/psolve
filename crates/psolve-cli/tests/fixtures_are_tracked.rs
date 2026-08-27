use std::path::Path;

/// The fixtures these tests compare against must be in git. .gitignore
/// ignores *.ini and *.wcs because those are solver output; the fixture
/// directory is the exception, and an un-negated ignore rule would let a
/// reference file exist only on the machine that generated it.
#[test]
fn sidecar_fixtures_are_not_gitignored() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let repo_root = Path::new(manifest_dir)
        .parent()
        .and_then(Path::parent)
        .expect("must be able to find repo root from manifest dir");

    for name in [
        "reference.ini",
        "reference-failure.ini",
        "reference.wcs",
        "reference-block.wcs",
    ] {
        let out = std::process::Command::new("git")
            .args(["check-ignore", "-q", &format!("crates/psolve-cli/tests/fixtures/{name}")])
            .current_dir(repo_root)
            .output()
            .expect("git must be runnable");
        assert!(
            !out.status.success(),
            "{name} is gitignored; committed sidecar fixtures would vanish"
        );
    }
}

/// The reference sidecars must contain no CR bytes.
///
/// They are byte-exact copies of real `astap_cli` output on Unix, and the
/// sidecar tests compare psolve's own bytes against them. Git's
/// `core.autocrlf=true` -- the default on Windows -- rewrites LF to CRLF on
/// checkout, at which point those tests compare against something ASTAP never
/// wrote. `.gitattributes` sets `* -text` to prevent it; this test is what
/// notices if that file is ever removed, narrowed, or overridden by a local
/// `core.autocrlf` setting.
///
/// Measured 2026-08-27: without `.gitattributes`, two `sidecar_ini.rs` tests
/// fail on a `windows-latest` runner with `\r\n` on the fixture side.
#[test]
fn the_reference_fixtures_carry_no_carriage_returns() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut checked = 0;
    for entry in std::fs::read_dir(&dir).expect("reading tests/fixtures") {
        let path = entry.expect("a fixture dir entry").path();
        let Some(ext) = path.extension().and_then(|e| e.to_str()) else { continue };
        if ext != "ini" && ext != "wcs" {
            continue;
        }
        let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("reading {path:?}: {e}"));
        let crs = bytes.iter().filter(|b| **b == b'\r').count();
        assert_eq!(
            crs, 0,
            "{path:?} contains {crs} CR byte(s) -- git rewrote the line endings on \
             checkout, so this fixture is no longer what astap_cli wrote. Check \
             .gitattributes (`* -text`) and your core.autocrlf setting."
        );
        checked += 1;
    }
    assert!(checked >= 4, "expected at least 4 sidecar fixtures, found {checked}");
}
