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
