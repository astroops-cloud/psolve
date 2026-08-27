//! Pins WHICH tests silently do nothing without the rig data.
//!
//! Several integration tests need artefacts that live only on this machine --
//! the multi-GB indexes under `~/astroops/data`, the frame archive under
//! `~/astroops/library`. By convention (see `CLAUDE.md`) they **skip with an
//! `eprintln!` rather than fail** when it is absent, so the suite still runs
//! on a machine without them. That convention is right, and it has a cost:
//! on a CI runner, where none of that data exists, those tests pass without
//! testing anything.
//!
//! So a green pipeline means "compiles, clippy-clean, data-independent tests
//! pass". It does NOT mean the agreement run passes, that blind solving works
//! against a real index, or that sidecar bytes still match `astap_cli`.
//!
//! **Why this is a test and not a line in the CI log.** A printed count in a
//! passing job is read by nobody -- the astroops session had exactly this
//! happen to a drift check that printed something true for weeks. The gap has
//! to be a failure, not an output. So the set below is pinned: add a
//! rig-dependent test and this fails until you add it here, which forces the
//! choice -- commit a fixture, or consciously widen the gap. Growing the gap
//! is allowed; growing it silently is not.
//!
//! This is the same shape as `psolve-core/tests/no_filesystem.rs`: a
//! structural guard that fails loudly rather than a convention people are
//! trusted to remember.

use std::path::{Path, PathBuf};

/// Test files permitted to skip themselves when the rig data is absent.
///
/// Adding an entry is a deliberate act. Before you do, ask whether the test
/// could use a committed fixture instead -- `sidecar_wcs.rs` needs real
/// `astap_cli` output and gets it from `tests/fixtures/`, so it is NOT in
/// this list despite describing `~/astroops` provenance in its module doc.
const RIG_DEPENDENT: &[&str] = &[
    "psolve-cli/tests/blind_measure_tolerances.rs",
    "psolve-cli/tests/blind_solve.rs",
    "psolve-cli/tests/real_frames.rs",
    "psolve-index/tests/blind_candidates_real_index.rs",
];

/// The marker of the convention: an `eprintln!` whose message begins
/// "skipping". Matched as a token so a doc comment mentioning the word does
/// not count.
const MARKER: &str = "eprintln!(\"skipping";

fn crates_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("crates/ is the parent").to_path_buf()
}

fn test_files() -> Vec<(String, String)> {
    let mut out = Vec::new();
    let crates = crates_dir();
    let mut krate: Vec<_> = std::fs::read_dir(&crates)
        .expect("read crates/")
        .filter_map(Result::ok)
        .map(|e| e.path())
        .collect();
    krate.sort();
    for k in krate {
        let tests = k.join("tests");
        if !tests.is_dir() {
            continue;
        }
        let mut files: Vec<_> = std::fs::read_dir(&tests)
            .expect("read a tests/ dir")
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "rs"))
            .collect();
        files.sort();
        for f in files {
            let rel = format!(
                "{}/tests/{}",
                k.file_name().expect("crate dir name").to_string_lossy(),
                f.file_name().expect("file name").to_string_lossy()
            );
            let body = std::fs::read_to_string(&f).expect("read a test file");
            out.push((rel, body));
        }
    }
    out
}

#[test]
fn the_set_of_tests_that_skip_without_rig_data_is_pinned() {
    let mut found: Vec<String> = test_files()
        .into_iter()
        .filter(|(_, body)| body.contains(MARKER))
        .map(|(rel, _)| rel)
        .collect();
    found.sort();

    let mut expected: Vec<String> = RIG_DEPENDENT.iter().map(|s| (*s).to_string()).collect();
    expected.sort();

    let added: Vec<_> = found.iter().filter(|f| !expected.contains(f)).collect();
    let removed: Vec<_> = expected.iter().filter(|e| !found.contains(e)).collect();

    assert!(
        added.is_empty(),
        "these tests skip themselves when the rig data is absent but are not listed in \
RIG_DEPENDENT: {added:?}\n\n\
That means CI now covers less than it did, silently. Either give the test a committed \
fixture so it runs everywhere, or add it to RIG_DEPENDENT to record that the gap grew \
on purpose."
    );
    assert!(
        removed.is_empty(),
        "these are listed in RIG_DEPENDENT but no longer skip without rig data: \
{removed:?}\n\nGood news, most likely -- remove them from the list."
    );
}

/// The list is only meaningful if the paths in it exist; a typo would silently
/// shrink the pinned set to whatever still matched.
#[test]
fn every_pinned_path_exists() {
    let crates = crates_dir();
    for rel in RIG_DEPENDENT {
        let p = crates.join(rel);
        assert!(p.is_file(), "RIG_DEPENDENT names {rel}, which is not a file at {}", p.display());
    }
}
