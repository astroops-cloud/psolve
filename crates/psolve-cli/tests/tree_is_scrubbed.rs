//! Pins the published tree free of the classes of leak that recur.
//!
//! This repo is public and this file is in it, so the guard must not name the
//! values it protects: a test that spells out the coordinate it forbids
//! publishes the coordinate. The observation-site cards are therefore pinned
//! POSITIVELY -- they must equal their redacted form -- and everything else is
//! a generic class rather than a private literal.
//!
//! It scans `git ls-files`, so it sees what a stranger clones, not the working
//! directory, which also holds gitignored in-repo worktrees. Like
//! `fixtures_are_tracked.rs` it PANICS rather than skips when `git` is missing:
//! a guard that quietly does nothing on the machine that matters is worse than
//! no guard.

use std::process::Command;

/// (pattern, what to use instead) -- the message is the fix, so a failure does
/// not send anyone hunting for the convention.
const FORBIDDEN_PATTERNS: &[(&str, &str)] = &[
    ("/Users/", "a macOS home directory -- use /home/user (same width: FITS cards must not move)"),
    ("192.168.", "an RFC1918 address -- use <LAN>"),
    (".lan.", "a LAN-only domain -- use the public URL"),
];

/// The three observation-site cards in their redacted form, byte for byte.
/// Pinned positively so this file never has to name the real values.
const REDACTED_SITE_CARDS: &[&str] = &[
    "SITEELEV=     000.000000000000 / [m] Observation site elevation",
    "SITELAT =           -00.000000 / [deg] Observation site latitude",
    "SITELONG=           000.000000 / [deg] Observation site longitude",
];

/// This file necessarily contains the patterns it forbids.
const SELF: &str = "crates/psolve-cli/tests/tree_is_scrubbed.rs";

fn repo_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("crates/<pkg> is two levels below the repo root")
        .to_path_buf()
}

fn tracked_files() -> Vec<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo_root())
        .args(["ls-files", "-z"])
        .output()
        .expect("git ls-files -- git must be installed to run this suite");
    assert!(out.status.success(), "git ls-files failed");
    String::from_utf8_lossy(&out.stdout)
        .split('\0')
        .filter(|s| !s.is_empty() && *s != SELF)
        .map(str::to_string)
        .collect()
}

#[test]
fn no_tracked_file_carries_a_personal_path_or_private_address() {
    let root = repo_root();
    let mut hits: Vec<String> = Vec::new();
    for rel in tracked_files() {
        let Ok(bytes) = std::fs::read(root.join(&rel)) else { continue };
        let body = String::from_utf8_lossy(&bytes);
        for (pattern, fix) in FORBIDDEN_PATTERNS {
            if body.contains(pattern) {
                hits.push(format!("{rel}: {pattern:?} -- {fix}"));
            }
        }
    }
    assert!(
        hits.is_empty(),
        "{} tracked file(s) carry a scrubbed pattern:\n{}",
        hits.len(),
        hits.join("\n")
    );
}

#[test]
fn the_observation_site_cards_are_redacted() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    for name in ["reference.wcs", "reference-block.wcs"] {
        let body = std::fs::read_to_string(dir.join(name)).expect(name);
        for card in REDACTED_SITE_CARDS {
            assert!(
                body.contains(card),
                "{name} does not carry the redacted card {card:?} -- either the scrub \
                 was not applied or a substitution changed the card's width"
            );
        }
    }
}

/// The byte-exact sidecar tests only hold if the cards stay 80 bytes and the
/// block stays padded. Scrubbing is where that gets broken, so it is asserted
/// next to the scrub rather than left to a distant test.
#[test]
fn the_fits_block_fixture_is_still_whole() {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/reference-block.wcs");
    let n = std::fs::metadata(&p).expect("reference-block.wcs").len();
    assert_eq!(n % 2880, 0, "reference-block.wcs is {n} bytes, not a whole multiple of 2880");
    assert_eq!(n, 8640, "reference-block.wcs changed size -- a substitution was not same-width");
}
