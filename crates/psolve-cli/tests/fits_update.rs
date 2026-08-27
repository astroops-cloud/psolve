//! Tests for the `-update` write path -- the one genuinely dangerous
//! surface in this crate. A FITS file is a header of fixed 2880-byte blocks
//! immediately followed by pixel data with no separator: if the header
//! grows past a block boundary, every byte of pixel data shifts. Every FITS
//! byte in this file is synthetic and lives under [`scratch_dir`]
//! (`$TMPDIR/psolve-fits-update-<tag>-<pid>`); nothing here ever reads or
//! writes anything under `~/astroops` -- that tree is strictly read-only
//! project-wide (see the M3 task-9 brief), and this file has no path in it
//! that could reach it even by accident.
//!
//! `psolve-cli` has no `[lib]` target, so both source files under test are
//! pulled in directly via `#[path]` (matching `sidecar_ini.rs`/
//! `sidecar_wcs.rs`'s own precedent) rather than linked as a library.
//!
//! `clippy::excessive_precision` is silenced crate-wide, same as
//! `sidecar_ini.rs`/`sidecar_wcs.rs`: [`wcs_fixture`]'s digits are a
//! transcription of a real ASTAP fixture value, typed with all significant
//! digits on purpose.
#![allow(clippy::excessive_precision)]

#[path = "../src/sidecar.rs"]
mod sidecar;
#[path = "../src/fits_update.rs"]
mod fits_update;

use fits_update::{
    commit_new_file, fsync_parent, refuse_if_readonly_output, update_header_in_place,
    update_header_in_place_reporting, FitsUpdateError,
};
use psolve_core::fit::Wcs;
use psolve_core::fits::FitsHeader;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// `PSOLVE_READONLY` and the process's current directory are both
/// process-wide state. `cargo test` runs the `#[test]` fns in this file on
/// multiple threads inside one process by default, so a test that
/// sets/clears the env var, or `chdir`s to exercise a relative path, could
/// otherwise interleave with another test's call into
/// [`update_header_in_place`] (which reads both) mid-flight. Every test
/// below holds this lock for its entire body, which serializes them all
/// with respect to that shared state without forcing serial execution of
/// the whole test binary. `unwrap_or_else(PoisonError::into_inner)` rather
/// than `unwrap()`: one earlier test panicking while holding the lock must
/// not spuriously fail every test that runs after it.
static ENV_LOCK: Mutex<()> = Mutex::new(());

fn lock() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// Changes the process's current directory to `dir` and restores both the
/// previous directory and the previous `$PWD` on drop -- including on an
/// early return from a panicking `assert!`, since `Drop` runs during
/// unwind. Only ever constructed while holding [`ENV_LOCK`] (see its doc
/// comment): both `chdir` and `$PWD` are process-wide.
struct CwdGuard {
    prev_dir: PathBuf,
    prev_pwd: Option<std::ffi::OsString>,
}

/// What a [`CwdGuard`] does to `$PWD` after `chdir`ing. `$PWD` is *not*
/// kernel state -- nothing but a shell ever maintains it -- so every one of
/// these is a real thing some real launcher leaves behind, and the module
/// under test has to cope with all of them.
enum Pwd<'a> {
    /// Leave whatever the test harness's own `$PWD` is: stale, pointing at
    /// the directory `cargo test` was run from. This is the
    /// `os.chdir()`-then-`subprocess` shape.
    LeaveStale,
    /// Set `$PWD` to exactly this value. With the directory as typed, this
    /// emulates a real shell's `cd`, which preserves any symlink in the
    /// path (something `std::env::set_current_dir` alone does *not* do --
    /// that leaves `getcwd(3)`, and so `std::env::current_dir()`, reporting
    /// the resolved physical path). With anything else, it emulates one of
    /// the several ways `$PWD` is routinely *not* the as-typed cwd: stale
    /// (another directory entirely), relative, `"."`, or a different name
    /// for the right directory.
    Set(&'a Path),
    /// Remove `$PWD` entirely: the cron / systemd / any-non-shell-launcher
    /// shape, where the parent `chdir()`ed and `exec()`ed without ever
    /// setting the variable.
    Unset,
}

impl CwdGuard {
    fn change_to(dir: &Path) -> Self {
        Self::change_to_impl(dir, Pwd::LeaveStale)
    }

    fn change_to_with_pwd(dir: &Path, pwd_value: &Path) -> Self {
        Self::change_to_impl(dir, Pwd::Set(pwd_value))
    }

    fn change_to_impl(dir: &Path, pwd: Pwd<'_>) -> Self {
        let prev_dir = std::env::current_dir().unwrap_or_else(|e| panic!("reading cwd: {e}"));
        let prev_pwd = std::env::var_os("PWD");
        std::env::set_current_dir(dir).unwrap_or_else(|e| panic!("chdir to {}: {e}", dir.display()));
        match pwd {
            Pwd::LeaveStale => {}
            Pwd::Set(p) => std::env::set_var("PWD", p),
            Pwd::Unset => std::env::remove_var("PWD"),
        }
        CwdGuard { prev_dir, prev_pwd }
    }
}

/// Runs `f` with a warning sink that records everything the code under test
/// emits, returning both the result and the captured warnings.
///
/// Two of this module's documented behaviours -- rule 8's reduced-coverage
/// notice and rule 5's non-fatal directory-`fsync` failure -- are warnings
/// alongside a permitted operation, so they are invisible in the return
/// type, and an integration test cannot capture the process's real stderr.
/// Asserting only the `Ok` half of "succeeds *and* warns" would silently
/// stop checking the half that is the whole point, which is why
/// `update_header_in_place_reporting` exists.
fn capturing_warnings<T>(f: impl FnOnce(&mut dyn FnMut(&str)) -> T) -> (T, Vec<String>) {
    let mut warnings: Vec<String> = Vec::new();
    let out = f(&mut |m: &str| warnings.push(m.to_string()));
    (out, warnings)
}

impl Drop for CwdGuard {
    fn drop(&mut self) {
        let _ = std::env::set_current_dir(&self.prev_dir);
        match &self.prev_pwd {
            Some(v) => std::env::set_var("PWD", v),
            None => std::env::remove_var("PWD"),
        }
    }
}

/// Removes its directory (recursively) on drop, so a scratch dir never
/// outlives its test even on an early `assert!` failure. Hand-rolled rather
/// than pulling in `tempfile`: `psolve-cli` may depend only on
/// `psolve-index`, `psolve-core`, and `rayon`.
struct ScratchDir(PathBuf);

impl ScratchDir {
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A fresh, empty scratch directory under the system temp path, unique to
/// this test (`tag`) and process. Matches the `psolve-{tag}-{pid}` naming
/// this crate's other integration tests already use (see `cli_info.rs`).
fn scratch_dir(tag: &str) -> ScratchDir {
    let d = std::env::temp_dir().join(format!("psolve-fits-update-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap_or_else(|e| panic!("creating scratch dir {}: {e}", d.display()));
    ScratchDir(d)
}

/// Build synthetic, self-contained FITS bytes: the mandatory
/// SIMPLE/BITPIX/NAXIS/NAXIS1/NAXIS2 cards, then `extra_cards`, then `END`,
/// padded to a whole 2880-byte block, followed by `pixel_data` verbatim.
/// `NAXIS1`/`NAXIS2` are nominal only -- `update_header_in_place` never
/// decodes pixels, it treats everything after the header as opaque bytes to
/// preserve, so `pixel_data` need not match their product.
fn fits_bytes(extra_cards: &[String], pixel_data: &[u8]) -> Vec<u8> {
    let mut cards: Vec<String> = vec![
        "SIMPLE  =                    T".to_string(),
        "BITPIX  =                   16".to_string(),
        "NAXIS   =                    2".to_string(),
        "NAXIS1  =                    2".to_string(),
        "NAXIS2  =                    2".to_string(),
    ];
    cards.extend(extra_cards.iter().cloned());
    cards.push("END".to_string());

    let mut header = String::new();
    for c in &cards {
        header.push_str(&format!("{c:<80}"));
    }
    while !header.len().is_multiple_of(2880) {
        header.push(' ');
    }
    let mut out = header.into_bytes();
    out.extend_from_slice(pixel_data);
    out
}

/// Distinct, non-trivial bytes (not all-zero) so a bug that zeroed or
/// reordered the data unit would actually be caught.
fn pixel_pattern(n: usize) -> Vec<u8> {
    (0..n).map(|i| (i % 251) as u8).collect()
}

/// A representative WCS for `update_header_in_place`'s write-path safety
/// tests: CRVAL/CD values transcribed from a real ASTAP fixture (same
/// values `sidecar_ini.rs`/`sidecar_wcs.rs` use), not arbitrary numbers.
/// This file never asserts the written `CRPIX1`/`CRPIX2` bytes against a
/// specific value (only `CRVAL1`, at line ~318) -- it exercises the safety
/// guards (verify-before-rename, block-boundary refusal, symlink handling,
/// etc.), which do not depend on the CRPIX convention. The byte-exact
/// 0-based-in/1-based-out CRPIX conversion `wcs_solution_cards` performs is
/// pinned in `sidecar_wcs.rs` and `real_frames.rs`, not here.
fn wcs_fixture() -> Wcs {
    Wcs {
        crval: [2.5423046742390622E+002, -4.0311880588850023E+001],
        crpix: [1.9205000000000000E+003, 1.0805000000000000E+003],
        cd: [
            [3.5245253250848707E-004, 5.8334097357301367E-004],
            [-5.8335417754934037E-004, 3.5236170894630648E-004],
        ],
    }
}

/// A frame with a small, ordinary header -- nowhere near a block boundary,
/// so a normal 17-card WCS write has ample room.
fn temp_fits_copy(tag: &str) -> (ScratchDir, PathBuf) {
    let dir = scratch_dir(tag);
    let bytes = fits_bytes(&[], &pixel_pattern(32));
    let path = dir.path().join("frame.fits");
    std::fs::write(&path, &bytes).unwrap_or_else(|e| panic!("writing fixture: {e}"));
    (dir, path)
}

/// A frame whose header already occupies exactly one whole 2880-byte block
/// (36 card slots: 5 mandatory + 30 filler + `END`) and carries none of the
/// WCS keywords yet, so writing the solution's 17 cards needs a second
/// block that was never there -- the exact hazard this module exists to
/// refuse rather than risk.
fn temp_fits_copy_with_full_header(tag: &str) -> (ScratchDir, PathBuf) {
    let dir = scratch_dir(tag);
    let filler: Vec<String> = (0..30).map(|i| format!("COMMENT filler card number {i}")).collect();
    let bytes = fits_bytes(&filler, &pixel_pattern(8));
    assert_eq!(bytes.len(), 2880 + 8, "fixture must be exactly one header block plus pixel data");
    let path = dir.path().join("frame.fits");
    std::fs::write(&path, &bytes).unwrap_or_else(|e| panic!("writing fixture: {e}"));
    (dir, path)
}

fn data_unit_offset(path: &Path) -> usize {
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    FitsHeader::parse(&bytes).unwrap_or_else(|e| panic!("parsing {}: {e}", path.display())).data_offset
}

fn read_data_unit(path: &Path) -> Vec<u8> {
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    let off = FitsHeader::parse(&bytes).unwrap_or_else(|e| panic!("parsing {}: {e}", path.display())).data_offset;
    bytes[off..].to_vec()
}

fn stray_temp_files(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains(".psolve-tmp"))
        .collect()
}

/// `sidecar.rs`'s own `format_ini_success`/`format_ini_failure`/
/// `format_wcs_text`/`format_wcs_fits_block`/`astap_float` are called for
/// real from `main.rs`'s `astap_cmd` (Task 10), but this file pulls
/// `sidecar.rs` in as its own separate `#[path]`-included compilation (see
/// this file's module doc), which does not itself call them -- only
/// `wcs_solution_cards`/`pad_or_truncate_card`, via `merge_wcs_cards`/
/// `pack_header`. Without a reference to the rest, THIS binary's own
/// `clippy --all-targets` pass would flag them as dead code, independent of
/// whether `main.rs` or `tests/sidecar_ini.rs`/`tests/sidecar_wcs.rs` use
/// them -- the same per-binary reasoning those two files' own module docs
/// already state for the identical reason.
#[test]
fn sidecars_file_writers_are_reachable_from_this_binary_too() {
    let wcs = wcs_fixture();
    let ini_ok = sidecar::format_ini_success(&wcs, "astap_cli -f x.fits");
    assert!(ini_ok.starts_with("PLTSOLVD=T"));
    let ini_fail = sidecar::format_ini_failure("astap_cli -f x.fits", "Not enough stars.");
    assert!(ini_fail.starts_with('\n') && ini_fail.contains("ERROR="));
    let text = sidecar::format_wcs_text(&wcs, "SIMPLE  =                    T");
    assert!(text.contains("CRVAL1"));
    let block = sidecar::format_wcs_fits_block(&wcs, "SIMPLE  =                    T");
    assert_eq!(block.len() % 2880, 0);
    assert_eq!(sidecar::astap_float(0.0), " 0.0000000000000000E+000");
}

/// The whole hazard in one test: whatever the header does, the pixels must
/// not move and must not change. Four real archive frames were silently
/// corrupted once by a header rewrite that shifted the data unit.
#[test]
fn updating_a_header_never_moves_or_alters_the_pixel_data() {
    let _g = lock();
    let (dir, path) = temp_fits_copy("moves-pixels");
    let before = read_data_unit(&path);
    let before_offset = data_unit_offset(&path);

    update_header_in_place(&path, &wcs_fixture()).unwrap_or_else(|e| panic!("update failed: {e}"));

    assert_eq!(data_unit_offset(&path), before_offset, "the data unit moved");
    assert_eq!(read_data_unit(&path), before, "pixel bytes changed");
    drop(dir);
}

/// A re-solve must overwrite the earlier WCS in place, not duplicate the
/// keywords -- this project's own `FitsHeader::get` returns the *first*
/// match for a repeated key, so a naive append would leave every reader
/// silently stuck on the stale value.
#[test]
fn updating_replaces_an_existing_wcs_card_instead_of_duplicating_it() {
    let _g = lock();
    let dir = scratch_dir("replace-existing");
    let stale = vec!["CRVAL1  =                  0.0".to_string()];
    let bytes = fits_bytes(&stale, &pixel_pattern(8));
    let path = dir.path().join("frame.fits");
    std::fs::write(&path, &bytes).unwrap();

    update_header_in_place(&path, &wcs_fixture()).unwrap_or_else(|e| panic!("update failed: {e}"));

    let out = std::fs::read(&path).unwrap();
    let h = FitsHeader::parse(&out).unwrap();
    let crval1_count = h.cards.iter().filter(|(k, _)| k == "CRVAL1").count();
    assert_eq!(crval1_count, 1, "must replace the stale CRVAL1, not duplicate it");
    let got = h.num("CRVAL1").unwrap();
    assert!((got - wcs_fixture().crval[0]).abs() < 1e-6, "CRVAL1 was not updated, got {got}");
    drop(dir);
}

/// The bug this pins (carried from Task 9's final review, M3 progress
/// ledger): `merge_wcs_cards` used to treat every `COMMENT` card as
/// unconditionally repeatable, so re-solving the same frame appended a
/// second, byte-identical "COMMENT Astrometric solution by psolve" card
/// instead of replacing the first. The header grows by one card every run
/// on a pipeline that re-solves frames, and eventually hits the
/// `HeaderGrew` refusal.
///
/// `FitsHeader::parse` does not surface `COMMENT` cards at all (it only
/// keeps cards with `=` at byte 8 -- `psolve-core/src/fits.rs`), so this
/// counts raw 80-byte cards directly via `fits_update`'s own byte-exact
/// scanner rather than `FitsHeader::cards`.
#[test]
fn a_resolve_does_not_duplicate_psolves_own_comment_card() {
    let _g = lock();
    let (dir, path) = temp_fits_copy("comment-idempotent");

    update_header_in_place(&path, &wcs_fixture())
        .unwrap_or_else(|e| panic!("first update failed: {e}"));
    update_header_in_place(&path, &wcs_fixture())
        .unwrap_or_else(|e| panic!("second update failed: {e}"));

    let out = std::fs::read(&path).unwrap();
    let data_offset = FitsHeader::parse(&out).unwrap().data_offset;
    let cards = fits_update::raw_header_cards(&out, data_offset);
    let psolve_comments = cards
        .iter()
        .filter(|c| {
            fits_update::card_key(c) == "COMMENT"
                && String::from_utf8_lossy(c.as_slice()).trim_end()
                    == "COMMENT Astrometric solution by psolve"
        })
        .count();
    assert_eq!(psolve_comments, 1, "a re-solve must replace psolve's own COMMENT card, not duplicate it");
    drop(dir);
}

/// The idempotency fix above must be narrow: only psolve's own solve-marker
/// `COMMENT` is special-cased. A `COMMENT` card the original capture
/// software wrote must never be touched or deduplicated away.
#[test]
fn a_resolve_does_not_touch_the_original_capture_softwares_comment_cards() {
    let _g = lock();
    let dir = scratch_dir("comment-preserve-original");
    let original_comment = vec!["COMMENT captured by N.I.N.A.".to_string()];
    let bytes = fits_bytes(&original_comment, &pixel_pattern(8));
    let path = dir.path().join("frame.fits");
    std::fs::write(&path, &bytes).unwrap();

    update_header_in_place(&path, &wcs_fixture())
        .unwrap_or_else(|e| panic!("first update failed: {e}"));
    update_header_in_place(&path, &wcs_fixture())
        .unwrap_or_else(|e| panic!("second update failed: {e}"));

    let out = std::fs::read(&path).unwrap();
    let data_offset = FitsHeader::parse(&out).unwrap().data_offset;
    let cards = fits_update::raw_header_cards(&out, data_offset);
    let comment_cards: Vec<String> = cards
        .iter()
        .filter(|c| fits_update::card_key(c) == "COMMENT")
        .map(|c| String::from_utf8_lossy(c.as_slice()).trim_end().to_string())
        .collect();
    assert_eq!(
        comment_cards.iter().filter(|s| s.contains("N.I.N.A.")).count(),
        1,
        "the original capture software's COMMENT card must survive, exactly once: {comment_cards:?}"
    );
    assert_eq!(
        comment_cards
            .iter()
            .filter(|s| s.as_str() == "COMMENT Astrometric solution by psolve")
            .count(),
        1,
        "psolve's own COMMENT card must be present exactly once, not duplicated: {comment_cards:?}"
    );
    drop(dir);
}

/// Fix round 1 (M3 Task 10 review): the first version of the idempotency fix
/// only replaced the FIRST matching psolve COMMENT card it found, so a
/// header that already carried duplicates -- a hand-edited frame, or a
/// regression predating the fix -- never self-healed: the count stayed
/// exactly where it started no matter how many more times `-update` ran.
/// This pre-seeds a header with three duplicate psolve-marker COMMENT
/// cards (plus one untouched original-capture-software COMMENT and one
/// HISTORY card) and runs a SINGLE `update_header_in_place`, proving the
/// merge collapses all of them to one in one pass, not just from a single
/// prior card.
#[test]
fn a_header_pre_seeded_with_duplicate_psolve_comment_cards_collapses_to_one() {
    let _g = lock();
    let dir = scratch_dir("comment-preseeded-duplicates");
    let extra_cards = vec![
        "COMMENT Astrometric solution by psolve".to_string(),
        "COMMENT Astrometric solution by psolve".to_string(),
        "COMMENT Astrometric solution by psolve".to_string(),
        "COMMENT captured by N.I.N.A.".to_string(),
        "HISTORY frame calibrated 2026-08-01".to_string(),
    ];
    let bytes = fits_bytes(&extra_cards, &pixel_pattern(8));
    let path = dir.path().join("frame.fits");
    std::fs::write(&path, &bytes).unwrap();

    update_header_in_place(&path, &wcs_fixture()).unwrap_or_else(|e| panic!("update failed: {e}"));

    let out = std::fs::read(&path).unwrap();
    let data_offset = FitsHeader::parse(&out).unwrap().data_offset;
    let cards = fits_update::raw_header_cards(&out, data_offset);
    let text_of = |c: &[u8; 80]| String::from_utf8_lossy(c.as_slice()).trim_end().to_string();

    let psolve_comments =
        cards.iter().filter(|c| text_of(c) == "COMMENT Astrometric solution by psolve").count();
    assert_eq!(
        psolve_comments, 1,
        "three pre-seeded duplicates must collapse to exactly one in a single -update, not stay at 3"
    );

    let original_comments =
        cards.iter().filter(|c| fits_update::card_key(c) == "COMMENT" && text_of(c).contains("N.I.N.A.")).count();
    assert_eq!(original_comments, 1, "the original capture software's COMMENT card must be untouched");

    let history_cards =
        cards.iter().filter(|c| fits_update::card_key(c) == "HISTORY").count();
    assert_eq!(history_cards, 1, "the HISTORY card must be untouched");

    drop(dir);
}

/// A header that would need a second 2880-byte block must be refused
/// outright -- not truncated, not written with the data unit shifted.
#[test]
fn a_header_that_would_grow_the_block_count_is_refused_not_truncated() {
    let _g = lock();
    let (dir, path) = temp_fits_copy_with_full_header("grows");
    let before = read_data_unit(&path);

    let err = update_header_in_place(&path, &wcs_fixture()).unwrap_err();

    assert!(format!("{err}").contains("header"), "must refuse, not silently shift data: {err}");
    assert!(matches!(err, FitsUpdateError::HeaderGrew { original_blocks: 1, needed_blocks: 2 }), "got {err:?}");
    assert_eq!(read_data_unit(&path), before, "the refused write must not touch the file at all");
    drop(dir);
}

/// `PSOLVE_READONLY` (any non-empty value) refuses the write before
/// anything is touched.
#[test]
fn psolve_readonly_env_refuses_the_write() {
    let _g = lock();
    let (dir, path) = temp_fits_copy("readonly-env");
    let before = read_data_unit(&path);

    std::env::set_var("PSOLVE_READONLY", "1");
    let r = update_header_in_place(&path, &wcs_fixture());
    std::env::remove_var("PSOLVE_READONLY");

    assert!(matches!(r, Err(FitsUpdateError::ReadOnly(_))), "PSOLVE_READONLY must refuse the write, got {r:?}");
    assert_eq!(read_data_unit(&path), before);
    drop(dir);
}

/// A `.psolve-readonly` marker in the target's own directory refuses the
/// write.
#[test]
fn a_psolve_readonly_marker_file_refuses_the_write() {
    let _g = lock();
    let (dir, path) = temp_fits_copy("readonly-marker");
    std::fs::write(dir.path().join(".psolve-readonly"), b"").unwrap();

    let r = update_header_in_place(&path, &wcs_fixture());

    assert!(matches!(r, Err(FitsUpdateError::ReadOnly(_))), "got {r:?}");
    drop(dir);
}

/// The marker also refuses the write from any ancestor directory, not just
/// the immediate parent -- a marker dropped at the top of an archive tree
/// must protect every frame beneath it.
#[test]
fn a_psolve_readonly_marker_in_an_ancestor_directory_also_refuses_the_write() {
    let _g = lock();
    let dir = scratch_dir("readonly-ancestor");
    let nested = dir.path().join("a").join("b");
    std::fs::create_dir_all(&nested).unwrap();
    std::fs::write(dir.path().join(".psolve-readonly"), b"").unwrap();
    let bytes = fits_bytes(&[], &pixel_pattern(8));
    let path = nested.join("frame.fits");
    std::fs::write(&path, &bytes).unwrap();

    let r = update_header_in_place(&path, &wcs_fixture());

    assert!(matches!(r, Err(FitsUpdateError::ReadOnly(_))), "a marker two directories up must still refuse, got {r:?}");
    drop(dir);
}

// ---------------------------------------------------------------------
// `refuse_if_readonly_output`: the same two switches, applied by path to a
// file that does not exist yet.
//
// This is what ASTAP mode's `.ini`/`.wcs` sidecar writes go through. Before
// it existed, both switches covered only `update_header_in_place`, and a
// sidecar write sailed straight past a marker sitting in the frame's own
// directory -- overwriting real recorded ASTAP output that cannot be
// reconstructed. The end-to-end proof through the compiled binary lives in
// `tests/astap_exit_codes.rs`; these pin the guard itself.
// ---------------------------------------------------------------------

/// The ordinary sidecar case: the output file does not exist yet, so the
/// canonical chain has to come from its *parent* directory. A marker there
/// must still refuse.
#[test]
fn a_marker_refuses_an_output_path_that_does_not_exist_yet() {
    let _g = lock();
    let dir = scratch_dir("output-marker-new-file");
    std::fs::write(dir.path().join(".psolve-readonly"), b"").unwrap();
    let out = dir.path().join("frame.ini");
    assert!(!out.exists(), "the point of this test is a path with nothing at it yet");

    let r = refuse_if_readonly_output(&out);

    assert!(matches!(r, Err(FitsUpdateError::ReadOnly(_))), "got {r:?}");
    drop(dir);
}

/// And from any ancestor, not just the immediate parent -- a marker at the
/// top of an archive tree protects every sidecar beneath it.
#[test]
fn a_marker_in_an_ancestor_refuses_an_output_path_too() {
    let _g = lock();
    let dir = scratch_dir("output-marker-ancestor");
    let nested = dir.path().join("a").join("b");
    std::fs::create_dir_all(&nested).unwrap();
    std::fs::write(dir.path().join(".psolve-readonly"), b"").unwrap();

    let r = refuse_if_readonly_output(&nested.join("frame.wcs"));

    assert!(matches!(r, Err(FitsUpdateError::ReadOnly(_))), "got {r:?}");
    drop(dir);
}

/// `PSOLVE_READONLY` refuses an output path on exactly the same terms.
#[test]
fn psolve_readonly_env_refuses_an_output_path() {
    let _g = lock();
    let dir = scratch_dir("output-readonly-env");
    let out = dir.path().join("frame.ini");

    std::env::set_var("PSOLVE_READONLY", "1");
    let refused = refuse_if_readonly_output(&out);
    std::env::remove_var("PSOLVE_READONLY");
    let allowed = refuse_if_readonly_output(&out);

    assert!(matches!(refused, Err(FitsUpdateError::ReadOnly(_))), "got {refused:?}");
    assert!(allowed.is_ok(), "with the switch cleared the same path must be allowed, got {allowed:?}");
    drop(dir);
}

/// An existing output file is canonicalized directly (not via its parent),
/// so a sidecar reached through a symlinked directory still resolves to the
/// tree it physically lives in -- and a marker there refuses.
#[test]
fn an_existing_output_file_reached_through_a_symlink_resolves_to_its_real_tree() {
    let _g = lock();
    let dir = scratch_dir("output-symlinked-existing");
    let real = dir.path().join("real");
    std::fs::create_dir_all(&real).unwrap();
    std::fs::write(real.join(".psolve-readonly"), b"").unwrap();
    let out = real.join("frame.ini");
    std::fs::write(&out, b"an earlier ASTAP solution").unwrap();
    let link = dir.path().join("link");
    std::os::unix::fs::symlink(&real, &link).unwrap();

    let r = refuse_if_readonly_output(&link.join("frame.ini"));

    assert!(matches!(r, Err(FitsUpdateError::ReadOnly(_))), "got {r:?}");
    drop(dir);
}

/// Fail closed: a directory that cannot be resolved is a refusal, never a
/// fallback to the unresolved path (which would silently walk the wrong
/// ancestors).
#[test]
fn an_output_path_in_an_unresolvable_directory_fails_closed() {
    let _g = lock();
    let dir = scratch_dir("output-unresolvable");

    let r = refuse_if_readonly_output(&dir.path().join("no").join("such").join("dir").join("f.ini"));

    assert!(matches!(r, Err(FitsUpdateError::UnresolvedPath(_))), "got {r:?}");
    drop(dir);
}

/// The control: with no marker and no environment switch, an ordinary
/// output path is allowed. Without this, every assertion above would still
/// pass if the guard simply refused everything.
#[test]
fn an_unprotected_output_path_is_allowed() {
    let _g = lock();
    let dir = scratch_dir("output-unprotected");

    let r = refuse_if_readonly_output(&dir.path().join("frame.ini"));

    assert!(r.is_ok(), "an unprotected directory must not be refused, got {r:?}");
    drop(dir);
}

/// The ancestor walk must also work for a *relative* path: `cd` into a
/// protected directory and pass a bare filename is at least as likely a
/// real invocation as an absolute one (e.g. `cd ~/astroops/archive/... &&
/// psolve solve frame.fits -update`), and `Path::new("frame.fits").parent()`
/// is `Some("")` whose own `.parent()` is `None` -- a walk over the
/// unresolved path as given would stop after one step and never see a
/// marker further up. `update_header_in_place` canonicalizes before
/// walking specifically so this passes.
#[test]
fn a_psolve_readonly_marker_protects_a_relative_path_too() {
    let _g = lock();
    let dir = scratch_dir("readonly-relative");
    let nested = dir.path().join("a").join("b");
    std::fs::create_dir_all(&nested).unwrap();
    std::fs::write(dir.path().join(".psolve-readonly"), b"").unwrap();
    std::fs::write(nested.join("frame.fits"), fits_bytes(&[], &pixel_pattern(8))).unwrap();

    let cwd = CwdGuard::change_to(&nested);
    let r = update_header_in_place(Path::new("frame.fits"), &wcs_fixture());
    drop(cwd);

    assert!(
        matches!(r, Err(FitsUpdateError::ReadOnly(_))),
        "a marker two directories above a relative path's cwd must still refuse, got {r:?}"
    );
    drop(dir);
}

/// A marker on the *real* directory must still protect a file reached
/// through a symlinked parent -- resolved (canonical) form catches this,
/// since a marker at `real/.psolve-readonly` does not lexically appear
/// above `link/frame.fits` even though both name the same file on disk.
/// This is the canonical chain's job; the mirror-image case (a marker on
/// the *lexical* tree, missed by canonicalizing) is covered separately
/// below.
#[cfg(unix)]
#[test]
fn a_psolve_readonly_marker_protects_a_file_reached_through_a_symlinked_parent() {
    let _g = lock();
    let dir = scratch_dir("readonly-symlink");
    let real = dir.path().join("real");
    std::fs::create_dir_all(&real).unwrap();
    std::fs::write(real.join(".psolve-readonly"), b"").unwrap();
    std::fs::write(real.join("frame.fits"), fits_bytes(&[], &pixel_pattern(8))).unwrap();

    let link = dir.path().join("link");
    std::os::unix::fs::symlink(&real, &link).unwrap_or_else(|e| panic!("symlinking: {e}"));
    let path_via_symlink = link.join("frame.fits");

    let r = update_header_in_place(&path_via_symlink, &wcs_fixture());

    assert!(
        matches!(r, Err(FitsUpdateError::ReadOnly(_))),
        "a marker in the real directory must protect access through a symlinked parent, got {r:?}"
    );
    drop(dir);
}

/// The mirror-image hole a canonical-only walk would reopen: a marker
/// placed on a *lexically* organized tree must still protect data that
/// physically lives elsewhere, reached through a symlinked component --
/// this project's own storage layout (`~/astroops` local, imaging data on
/// an SMB-mounted NAS share) is exactly this shape. `astroops/archive` is
/// a symlink to `bigdisk/archive`; the marker sits at
/// `astroops/.psolve-readonly`, which is a *lexical* ancestor of
/// `astroops/archive/frame.fits` but NOT a *canonical* one (canonicalizing
/// resolves straight through to `bigdisk/archive`, which the marker is
/// nowhere above). A canonical-only walk (as introduced, then reverted, in
/// fix round 1) would see `Ok(())` and rewrite the file; the lexical chain
/// added in fix round 2 is what still catches it.
#[cfg(unix)]
#[test]
fn a_psolve_readonly_marker_above_a_symlinked_component_still_refuses() {
    let _g = lock();
    let dir = scratch_dir("readonly-symlink-mirror");
    let astroops = dir.path().join("astroops");
    let bigdisk_archive = dir.path().join("bigdisk").join("archive");
    std::fs::create_dir_all(&astroops).unwrap();
    std::fs::create_dir_all(&bigdisk_archive).unwrap();
    std::fs::write(astroops.join(".psolve-readonly"), b"").unwrap();
    let before = fits_bytes(&[], &pixel_pattern(8));
    std::fs::write(bigdisk_archive.join("frame.fits"), &before).unwrap();

    let archive_link = astroops.join("archive");
    std::os::unix::fs::symlink(&bigdisk_archive, &archive_link).unwrap_or_else(|e| panic!("symlinking: {e}"));
    let path_via_symlink = archive_link.join("frame.fits");

    let r = update_header_in_place(&path_via_symlink, &wcs_fixture());

    assert!(
        matches!(r, Err(FitsUpdateError::ReadOnly(_))),
        "a marker on the lexical tree must still refuse data reached through a symlinked \
         component, even though canonicalizing resolves the symlink away, got {r:?}"
    );
    assert_eq!(
        std::fs::read(bigdisk_archive.join("frame.fits")).unwrap(),
        before,
        "the refused write must not have touched the real file"
    );
    drop(dir);
}

/// The third shape, and the one fix round 2's two-chain walk still missed:
/// `cd` *into* a symlinked directory, then pass a bare relative filename.
/// `std::env::current_dir()` calls `getcwd(3)`, which returns the kernel's
/// PHYSICAL (already symlink-resolved) cwd -- so for this shape, both the
/// canonical chain and the physical-lexical chain resolve to the exact same
/// location, and neither one sees a marker placed on the LOGICAL (as-typed)
/// tree a real shell's `$PWD` would still report. This is `cd
/// ~/astroops/archive && psolve solve frame.fits -update`, this project's
/// own round-1 report's words: "arguably the single most likely real
/// invocation shape." Only the logical-lexical chain (built from a
/// verified `$PWD`) catches it -- see the module doc's rule 2 shape
/// enumeration, row `rel/no/yes`.
#[cfg(unix)]
#[test]
fn a_psolve_readonly_marker_protects_a_relative_path_from_inside_a_symlinked_cwd() {
    let _g = lock();
    let dir = scratch_dir("readonly-logical-cwd");
    let astroops = dir.path().join("astroops");
    let bigdisk_archive = dir.path().join("bigdisk").join("archive");
    std::fs::create_dir_all(&astroops).unwrap();
    std::fs::create_dir_all(&bigdisk_archive).unwrap();
    std::fs::write(astroops.join(".psolve-readonly"), b"").unwrap();
    let before = fits_bytes(&[], &pixel_pattern(8));
    std::fs::write(bigdisk_archive.join("frame.fits"), &before).unwrap();

    let archive_link = astroops.join("archive");
    std::os::unix::fs::symlink(&bigdisk_archive, &archive_link).unwrap_or_else(|e| panic!("symlinking: {e}"));

    // A real shell's `cd astroops/archive` sets $PWD to the symlinked path
    // as typed, not to getcwd(3)'s resolved physical path -- emulate that
    // explicitly, since `std::env::set_current_dir` alone does not touch
    // `$PWD` (that bookkeeping is normally the shell's job, not the OS's).
    let cwd = CwdGuard::change_to_with_pwd(&archive_link, &archive_link);
    let r = update_header_in_place(Path::new("frame.fits"), &wcs_fixture());
    drop(cwd);

    assert!(
        matches!(r, Err(FitsUpdateError::ReadOnly(_))),
        "a marker on the logical (as-typed) tree must still refuse a relative path invoked \
         from inside a symlinked cwd, got {r:?}"
    );
    assert_eq!(
        std::fs::read(bigdisk_archive.join("frame.fits")).unwrap(),
        before,
        "the refused write must not have touched the real file"
    );
    drop(dir);
}

// ---------------------------------------------------------------------------
// The `relative path` + `symlinked cwd` cells of the module doc's rule 2
// table: the only two of the eight shape cells whose as-typed coverage is
// conditional rather than unconditional.
//
// The as-typed cwd is not kernel state. `getcwd(3)` returns the physical,
// symlink-resolved directory, and no OS records a logical one -- `$PWD` is a
// shell convention and nothing more. So when `$PWD` is missing or cannot be
// trusted, there is no information anywhere in the process from which the
// path the user typed could be reconstructed, and a marker on a purely
// as-typed tree is genuinely out of scope. Three previous rounds each closed
// one demonstrated shape here and left a sibling open; the tests below pin
// the *documented* behaviour of the remaining shapes instead, which is:
//
//   - a marker on the canonical chain refuses, always, in every one of them
//     (`the_canonical_chain_always_refuses_however_untrustworthy_pwd_is`);
//   - a marker that exists *only* on the as-typed tree does not refuse, and
//     the reduced coverage is warned about (the three tests after that);
//   - except in the twin-symlink case, where a chain *was* built and simply
//     is not the one the user typed, so there is nothing to warn about --
//     the residual this module cannot detect, stated as such.
//
// A test that asserted a guarantee this code does not make would be worse
// than no test, so none of these assert refusal where the code proceeds.
// ---------------------------------------------------------------------------

/// Where `symlinked_cwd_fixture` puts the `.psolve-readonly` marker.
#[cfg(unix)]
enum MarkerAt {
    /// On the as-typed tree only (`astroops/`), which is a lexical ancestor
    /// of the symlinked cwd but is nowhere above the frame's real location.
    AsTypedTreeOnly,
    /// On the canonical chain (`bigdisk/`, a real ancestor of the frame),
    /// which is nowhere near either as-typed name. This is the placement
    /// the module's one unconditional guarantee covers.
    CanonicalChain,
    /// No marker at all -- for the controls. A guard that refuses
    /// everything would pass every marker test in this file while being
    /// completely useless, so "unmarked shapes still succeed" has to be
    /// asserted as explicitly as "marked shapes refuse".
    Nowhere,
}

/// The shared shape for every test below:
///
/// ```text
/// <scratch>/astroops/                    <- marker here for AsTypedTreeOnly
/// <scratch>/astroops/archive  -> <scratch>/bigdisk/archive   (the "as typed" name)
/// <scratch>/twin              -> <scratch>/bigdisk/archive   (a second, unmarked name)
/// <scratch>/bigdisk/                     <- marker here for CanonicalChain
/// <scratch>/bigdisk/archive/frame.fits   (the real frame)
/// ```
///
/// `twin` deliberately lives *outside* `astroops/` so that nothing above it
/// carries a marker -- it is a name for exactly the same directory that
/// passes every `$PWD` check and still reconstructs the wrong chain.
#[cfg(unix)]
struct SymlinkedCwd {
    dir: ScratchDir,
    /// `<scratch>/astroops/archive`
    as_typed: PathBuf,
    /// `<scratch>/twin`
    twin: PathBuf,
    /// `<scratch>/bigdisk/archive`
    real: PathBuf,
    /// The frame's bytes before the call, for an untouched-file assertion.
    before: Vec<u8>,
}

#[cfg(unix)]
fn symlinked_cwd_fixture(tag: &str, marker: MarkerAt) -> SymlinkedCwd {
    let dir = scratch_dir(tag);
    let astroops = dir.path().join("astroops");
    let bigdisk = dir.path().join("bigdisk");
    let real = bigdisk.join("archive");
    std::fs::create_dir_all(&astroops).unwrap();
    std::fs::create_dir_all(&real).unwrap();

    let marker_dir = match marker {
        MarkerAt::AsTypedTreeOnly => Some(&astroops),
        MarkerAt::CanonicalChain => Some(&bigdisk),
        MarkerAt::Nowhere => None,
    };
    if let Some(d) = marker_dir {
        std::fs::write(d.join(".psolve-readonly"), b"").unwrap();
    }

    let before = fits_bytes(&[], &pixel_pattern(8));
    std::fs::write(real.join("frame.fits"), &before).unwrap();

    let as_typed = astroops.join("archive");
    std::os::unix::fs::symlink(&real, &as_typed).unwrap_or_else(|e| panic!("symlinking: {e}"));
    let twin = dir.path().join("twin");
    std::os::unix::fs::symlink(&real, &twin).unwrap_or_else(|e| panic!("symlinking twin: {e}"));

    SymlinkedCwd { dir, as_typed, twin, real, before }
}

/// The module's one **unconditional** guarantee, executed against every
/// untrustworthy `$PWD` shape there is: *a `.psolve-readonly` marker
/// anywhere on the target's canonical (physical) ancestor chain always
/// refuses the write.*
///
/// Each case below `chdir`s into a symlinked directory and passes a bare
/// relative filename -- the shape whose as-typed coverage is conditional --
/// and then breaks `$PWD` in a different, individually realistic way. The
/// as-typed chain is unreconstructible in every one of them; the guarantee
/// does not care, because it is made of `std::fs::canonicalize`'s output and
/// nothing else. This is the property to rely on, and the reason the
/// module doc says plainly: **to protect a tree, put the marker in the tree
/// the frames physically live in.**
#[cfg(unix)]
#[test]
fn the_canonical_chain_always_refuses_however_untrustworthy_pwd_is() {
    let _g = lock();
    for case in ["unset", "relative", "dot", "stale", "twin", "as-typed"] {
        let fx = symlinked_cwd_fixture(&format!("canonical-guarantee-{case}"), MarkerAt::CanonicalChain);
        let scratch = fx.dir.path().to_path_buf();
        let pwd = match case {
            // cron / systemd / any launcher that chdir()ed then exec()ed.
            "unset" => Pwd::Unset,
            "relative" => Pwd::Set(Path::new("astroops/archive")),
            // Passes the device+inode check trivially, then yields a chain
            // with no ancestors at all -- now rejected before it gets there.
            "dot" => Pwd::Set(Path::new(".")),
            // Python's os.chdir() + subprocess: os.environ['PWD'] is left
            // pointing at the directory the process started in.
            "stale" => Pwd::Set(&scratch),
            // A different, unmarked name for the very same directory:
            // verifies by device+inode, wrong as-typed chain.
            "twin" => Pwd::Set(&fx.twin),
            // The honest, well-behaved shell case, as a control.
            _ => Pwd::Set(&fx.as_typed),
        };

        let cwd = CwdGuard::change_to_impl(&fx.as_typed, pwd);
        let r = update_header_in_place(Path::new("frame.fits"), &wcs_fixture());
        drop(cwd);

        assert!(
            matches!(r, Err(FitsUpdateError::ReadOnly(_))),
            "a marker on the canonical ancestor chain must refuse unconditionally, but with \
             $PWD={case} it did not: got {r:?}"
        );
        assert_eq!(
            std::fs::read(fx.real.join("frame.fits")).unwrap(),
            fx.before,
            "the refused write ($PWD={case}) must not have touched the real file"
        );
        drop(fx);
    }
}

/// `$PWD` **unset** -- cron, systemd, or any launcher that `chdir()`ed and
/// then `exec()`ed, none of which set it; only shells do. With the marker on
/// the as-typed tree *only*, no chain in the process can see it, so the
/// write proceeds. Asserting a refusal here would assert a guarantee the
/// code does not make. What the code *does* guarantee is that it says so:
/// rule 8's warning is the user's notice that this invocation ran with
/// reduced coverage.
#[cfg(unix)]
#[test]
fn pwd_unset_from_a_symlinked_cwd_warns_and_proceeds_past_an_as_typed_only_marker() {
    let _g = lock();
    let fx = symlinked_cwd_fixture("pwd-unset", MarkerAt::AsTypedTreeOnly);

    let cwd = CwdGuard::change_to_impl(&fx.as_typed, Pwd::Unset);
    let (r, warnings) =
        capturing_warnings(|warn| update_header_in_place_reporting(Path::new("frame.fits"), &wcs_fixture(), warn));
    drop(cwd);

    assert!(r.is_ok(), "documented behaviour is warn-and-proceed, not refuse; got {r:?}");
    assert!(
        warnings.iter().any(|w| w.contains("relative path") && w.contains("$PWD is unset or empty")),
        "an unreconstructible as-typed cwd must be warned about, naming the reason: {warnings:?}"
    );
    drop(fx);
}

/// `$PWD` set to a **relative** value. Nothing absolute can be built from
/// it, so the chain is refused before the device+inode check even runs, and
/// the write proceeds past an as-typed-only marker with a warning.
#[cfg(unix)]
#[test]
fn pwd_relative_from_a_symlinked_cwd_warns_and_proceeds_past_an_as_typed_only_marker() {
    let _g = lock();
    let fx = symlinked_cwd_fixture("pwd-relative", MarkerAt::AsTypedTreeOnly);

    let cwd = CwdGuard::change_to_impl(&fx.as_typed, Pwd::Set(Path::new("astroops/archive")));
    let (r, warnings) =
        capturing_warnings(|warn| update_header_in_place_reporting(Path::new("frame.fits"), &wcs_fixture(), warn));
    drop(cwd);

    assert!(r.is_ok(), "documented behaviour is warn-and-proceed, not refuse; got {r:?}");
    assert!(
        warnings.iter().any(|w| w.contains("$PWD is not an absolute path")),
        "a relative $PWD must be rejected as a chain source, with the reason named: {warnings:?}"
    );
    drop(fx);
}

/// `$PWD="."` -- the one that made the device+inode check look stronger than
/// it is. `.` genuinely *is* the current directory, so it passes that check
/// trivially, and the chain it then yields (`./frame.fits`) has no ancestors
/// at all: a chain that contributes nothing while counting as coverage. The
/// absolute-path requirement rejects it outright, so it is now honestly
/// reported as an unavailable chain rather than silently standing in for
/// one.
#[cfg(unix)]
#[test]
fn pwd_dot_is_rejected_rather_than_yielding_an_ancestorless_chain() {
    let _g = lock();
    let fx = symlinked_cwd_fixture("pwd-dot", MarkerAt::AsTypedTreeOnly);

    let cwd = CwdGuard::change_to_impl(&fx.as_typed, Pwd::Set(Path::new(".")));
    let (r, warnings) =
        capturing_warnings(|warn| update_header_in_place_reporting(Path::new("frame.fits"), &wcs_fixture(), warn));
    drop(cwd);

    assert!(r.is_ok(), "documented behaviour is warn-and-proceed, not refuse; got {r:?}");
    assert!(
        warnings.iter().any(|w| w.contains("$PWD is not an absolute path")),
        "$PWD=\".\" must be rejected for not being absolute, not accepted for passing the \
         device+inode check: {warnings:?}"
    );
    drop(fx);
}

/// The residual this module cannot detect, stated as a test rather than
/// left to be rediscovered a fourth time: `$PWD` naming the same directory
/// through a **different, unmarked twin symlink**. It is set, non-empty,
/// absolute, free of `.`/`..`, and passes the device+inode check -- because
/// it really is a name for this directory. It is simply not *the* name the
/// user typed, and no check can tell the difference, because the kernel
/// never recorded which name that was.
///
/// So a chain is built, it is the wrong one, the as-typed-only marker is
/// missed, and the write proceeds -- with **no warning**, since from the
/// program's side nothing was unavailable. That is the honest boundary of
/// what `$PWD` can buy, and why the module doc grounds its guarantee on the
/// canonical chain instead. (The companion case -- the same fixture with the
/// marker on the canonical chain -- refuses, and is covered by
/// `the_canonical_chain_always_refuses_however_untrustworthy_pwd_is`.)
#[cfg(unix)]
#[test]
fn pwd_naming_a_twin_symlink_builds_a_verified_but_wrong_chain_and_proceeds() {
    let _g = lock();
    let fx = symlinked_cwd_fixture("pwd-twin", MarkerAt::AsTypedTreeOnly);

    let cwd = CwdGuard::change_to_impl(&fx.as_typed, Pwd::Set(&fx.twin));
    let (r, warnings) =
        capturing_warnings(|warn| update_header_in_place_reporting(Path::new("frame.fits"), &wcs_fixture(), warn));
    drop(cwd);

    assert!(
        r.is_ok(),
        "a device+inode-verified $PWD naming a twin symlink yields a chain that cannot see the \
         as-typed marker; the documented behaviour is to proceed, got {r:?}"
    );
    assert!(
        warnings.is_empty(),
        "a chain WAS built here -- there is no unavailability to warn about, and warning anyway \
         would misdescribe what happened: {warnings:?}"
    );
    let after = std::fs::read(fx.real.join("frame.fits")).unwrap();
    assert_ne!(after, fx.before, "this shape does write; the test exists to pin that honestly");
    drop(fx);
}

/// **The controls.** Every shape the tests above drive, with **no marker
/// anywhere**, must still write successfully.
///
/// This is not a formality. A guard that simply refused every `-update`
/// would satisfy every marker test in this file -- and the whole point of
/// `-update` is that it works. Three rounds of widening this guard's
/// ancestor walk each added another chain, another lexical form, another
/// source of paths to search for a marker; each one is another chance to
/// accidentally match something and start refusing writes nobody asked to
/// protect. These eleven cases sweep the same shape space the refusal tests
/// do -- both `A` values, both `S` values, both `C` values, and every
/// `$PWD` state -- and assert the opposite outcome, so the guard is pinned
/// from both sides.
#[cfg(unix)]
#[test]
fn every_unmarked_shape_still_writes_successfully() {
    let _g = lock();

    // The four shapes that involve no symlinked cwd, driven directly.
    // (`abs/no`, `abs/yes`, `rel/no/no`, `rel/yes/no` in rule 2's table.)
    {
        let fx = symlinked_cwd_fixture("control-no-symlinked-cwd", MarkerAt::Nowhere);
        let real_frame = fx.real.join("frame.fits");
        let as_typed_frame = fx.as_typed.join("frame.fits");

        // abs / S=no: a plain absolute path, no symlink anywhere.
        update_header_in_place(&real_frame, &wcs_fixture())
            .unwrap_or_else(|e| panic!("control abs/no-symlink must succeed: {e}"));
        // abs / S=yes: absolute, through the `astroops/archive` symlink.
        update_header_in_place(&as_typed_frame, &wcs_fixture())
            .unwrap_or_else(|e| panic!("control abs/symlinked-component must succeed: {e}"));

        // rel / S=no / C=no: cwd is the real directory, bare filename.
        let cwd = CwdGuard::change_to(&fx.real);
        let r = update_header_in_place(Path::new("frame.fits"), &wcs_fixture());
        drop(cwd);
        r.unwrap_or_else(|e| panic!("control rel/no-symlink/plain-cwd must succeed: {e}"));

        // rel / S=yes / C=no: cwd is the real `bigdisk` directory (not
        // reached through any symlink), path text goes back out through the
        // `astroops/archive` symlink.
        let cwd = CwdGuard::change_to(&fx.dir.path().join("bigdisk"));
        let r = update_header_in_place(Path::new("../astroops/archive/frame.fits"), &wcs_fixture());
        drop(cwd);
        r.unwrap_or_else(|e| panic!("control rel/symlinked-component/plain-cwd must succeed: {e}"));

        drop(fx);
    }

    // The six `$PWD` states from inside a symlinked cwd (rule 2's two
    // conditional cells), plus one with a symlinked component in the path
    // text as well -- seven more, none of which may refuse.
    for case in ["unset", "relative", "dot", "stale", "twin", "as-typed", "symlinked-path-too"] {
        let fx = symlinked_cwd_fixture(&format!("control-{case}"), MarkerAt::Nowhere);
        let scratch = fx.dir.path().to_path_buf();
        let pwd = match case {
            "unset" => Pwd::Unset,
            "relative" => Pwd::Set(Path::new("astroops/archive")),
            "dot" => Pwd::Set(Path::new(".")),
            "stale" => Pwd::Set(&scratch),
            "twin" => Pwd::Set(&fx.twin),
            _ => Pwd::Set(&fx.as_typed),
        };
        // The last case adds S=yes on top of C=yes: from the symlinked cwd,
        // reach the same frame back out through the *other* symlink.
        let target: PathBuf = if case == "symlinked-path-too" {
            Path::new("..").join("..").join("twin").join("frame.fits")
        } else {
            PathBuf::from("frame.fits")
        };

        let cwd = CwdGuard::change_to_impl(&fx.as_typed, pwd);
        let r = update_header_in_place(&target, &wcs_fixture());
        drop(cwd);

        r.unwrap_or_else(|e| panic!("unmarked control ($PWD={case}) must still succeed, got: {e}"));
        let after = std::fs::read(fx.real.join("frame.fits")).unwrap();
        assert_ne!(after, fx.before, "unmarked control ($PWD={case}) reported success without writing");
        drop(fx);
    }
}

/// A failed update (header growth) must leave no `.psolve-tmp*` file
/// behind.
#[test]
fn a_failed_update_leaves_no_temp_file_behind() {
    let _g = lock();
    let (dir, path) = temp_fits_copy_with_full_header("no-stray-temp-grew");
    let _ = update_header_in_place(&path, &wcs_fixture());

    let strays = stray_temp_files(dir.path());
    assert!(strays.is_empty(), "a failed update left {} temp file(s) behind: {strays:?}", strays.len());
    drop(dir);
}

/// A refusal on the read-only guard must leave no temp file behind either
/// -- the guard runs before any write, but this is checked rather than
/// assumed.
#[test]
fn a_readonly_refusal_leaves_no_temp_file_behind() {
    let _g = lock();
    let (dir, path) = temp_fits_copy("no-stray-temp-readonly");
    std::env::set_var("PSOLVE_READONLY", "1");
    let _ = update_header_in_place(&path, &wcs_fixture());
    std::env::remove_var("PSOLVE_READONLY");

    let strays = stray_temp_files(dir.path());
    assert!(strays.is_empty(), "a read-only refusal left {} temp file(s) behind: {strays:?}", strays.len());
    drop(dir);
}

/// A successful update leaves exactly the target file behind -- the temp
/// copy is renamed over it, never left sitting alongside it.
#[test]
fn a_successful_update_leaves_no_temp_file_and_renames_over_the_target() {
    let _g = lock();
    let (dir, path) = temp_fits_copy("success-no-stray");

    update_header_in_place(&path, &wcs_fixture()).unwrap_or_else(|e| panic!("update failed: {e}"));

    let entries: Vec<String> =
        std::fs::read_dir(dir.path()).unwrap().filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(entries, vec!["frame.fits".to_string()], "only the target file should remain");
    drop(dir);
}

/// The accept side of the block boundary, pinned exactly: 5 mandatory cards
/// plus 13 filler plus `END` is 19 slots before the merge; adding the 17
/// WCS cards (none of which pre-exist here) plus a fresh `END` lands at
/// exactly 36 slots -- one whole 2880-byte block, with zero slack. This
/// must be accepted, not refused for merely being close to the edge: only
/// exceeding `target_len` is a refusal (`pack_header`'s `out.len() >
/// target_len`, not `>=`).
#[test]
fn a_header_that_exactly_fills_its_last_block_after_merging_is_accepted() {
    let _g = lock();
    let dir = scratch_dir("exact-fit-accepted");
    let filler: Vec<String> = (0..13).map(|i| format!("COMMENT filler card number {i}")).collect();
    let bytes = fits_bytes(&filler, &pixel_pattern(8));
    assert_eq!(bytes.len(), 2880 + 8, "fixture must start as exactly one header block plus pixel data");
    let path = dir.path().join("frame.fits");
    std::fs::write(&path, &bytes).unwrap();

    update_header_in_place(&path, &wcs_fixture())
        .unwrap_or_else(|e| panic!("a merge that exactly fills the last block must be accepted, got: {e}"));

    assert_eq!(data_unit_offset(&path), 2880, "an accepted exact-fit write must not have grown a block");
    drop(dir);
}

/// The refuse side, pinned exactly one card past the same boundary as the
/// test above: one filler card more (14, not 13) needs a second block and
/// must be refused, not squeezed in.
#[test]
fn a_header_exactly_one_card_past_the_boundary_is_refused() {
    let _g = lock();
    let dir = scratch_dir("one-card-over-refused");
    let filler: Vec<String> = (0..14).map(|i| format!("COMMENT filler card number {i}")).collect();
    let bytes = fits_bytes(&filler, &pixel_pattern(8));
    let path = dir.path().join("frame.fits");
    std::fs::write(&path, &bytes).unwrap();

    let err = update_header_in_place(&path, &wcs_fixture()).unwrap_err();

    assert!(
        matches!(err, FitsUpdateError::HeaderGrew { original_blocks: 1, needed_blocks: 2 }),
        "one card past the boundary must be refused for needing a 2nd block, got {err:?}"
    );
    drop(dir);
}

/// `verify_temp` is billed as an independent safety net that should never
/// fire on correct input, which is exactly why it is otherwise hard to
/// exercise for real (the only prior evidence was a sabotage run, reverted,
/// leaving nothing enforceable). Drive it directly and deterministically
/// through the *real* commit/cleanup code (`commit_new_file`, the same
/// function `update_header_in_place` calls) by handing it a `new_file`
/// buffer whose trailing bytes deliberately do not match `expected_pixels`
/// -- simulating the kind of corruption this net exists to catch -- and
/// confirm both required outcomes: the temp file it created is cleaned up,
/// and the real target file is left completely untouched.
#[test]
fn a_corrupted_temp_fails_verification_cleans_up_and_leaves_the_target_untouched() {
    let _g = lock();
    let (dir, path) = temp_fits_copy("verify-fails");
    let before = std::fs::read(&path).unwrap();
    let header_len = FitsHeader::parse(&before).unwrap().data_offset;

    let mut corrupted = before[..header_len].to_vec();
    corrupted.extend_from_slice(&[0xEE; 32]); // does not match the real pixel_pattern(32)

    let real_path = std::fs::canonicalize(&path).unwrap();
    let (r, warnings) =
        capturing_warnings(|warn| commit_new_file(&real_path, &corrupted, header_len, &pixel_pattern(32), warn));
    let err = r.unwrap_err();
    assert!(matches!(err, FitsUpdateError::Verify(_)), "got {err:?}");
    assert!(warnings.is_empty(), "a verify failure is an Err, not a warn-and-proceed: {warnings:?}");

    let strays = stray_temp_files(dir.path());
    assert!(strays.is_empty(), "a verify failure left {} temp file(s) behind: {strays:?}", strays.len());
    assert_eq!(std::fs::read(&path).unwrap(), before, "a verify failure must leave the target untouched");
    drop(dir);
}

/// `fsync_parent` itself still reports a genuine failure as `Err` -- its
/// caller (`commit_new_file`) is the one that downgrades that `Err` to a
/// stderr warning (rule 5), not this function. A nonexistent directory is
/// a portable, deterministic way to make `File::open` fail without
/// depending on a particular filesystem's fsync support (unlike the
/// `ENOTSUP` this project's real SMB-mounted NAS shares return for every
/// directory `fsync` -- see the fix-round-2 report for that verification,
/// done manually against the live mounts rather than as a committed test,
/// since a test cannot depend on a specific machine's mounts being present).
#[test]
fn fsync_parent_reports_a_genuine_failure() {
    // Doesn't touch PSOLVE_READONLY or the cwd, so this lock isn't load-
    // bearing today -- held anyway so "every test in this file holds
    // ENV_LOCK for its whole body" stays true as a property the next
    // person can rely on, not an exception to remember.
    let _g = lock();
    let dir = scratch_dir("fsync-parent-error");
    let missing = dir.path().join("does-not-exist").join("frame.fits");
    let err = fsync_parent(&missing).unwrap_err();
    assert!(matches!(err, FitsUpdateError::Io(_)), "got {err:?}");
    drop(dir);
}

/// The behaviour fix round 2 was actually about, pinned directly and
/// portably: a directory that cannot be `fsync`ed *after* a successful
/// rename must not turn a completed write into a reported failure. Mode
/// `0o311` (write+execute, no read) on the parent directory is a
/// deterministic way to reproduce that split without any mount: it leaves
/// `File::create`/`write_all`/`rename` all working (they need write+
/// execute on the directory, not read) while `File::open(dir)` -- needed to
/// `fsync` the directory itself -- fails with `EACCES` (no read
/// permission). This is a different errno than the real NAS mounts'
/// `ENOTSUP` (see the fix-round-2 report's manual verification against
/// `/home/user/mnt/astro`), but it exercises the exact same code path:
/// `commit_new_file` must still return `Ok` and still warn. Previously
/// nothing asserted this -- only that `fsync_parent` itself can fail, not
/// that its caller tolerates the failure -- so a future edit that made
/// `fsync_parent(real_path)` the tail expression of `commit_new_file`
/// again (restoring the original bug) would have left the whole suite
/// green.
///
/// Two things this test used to get wrong, both fixed here:
///
/// - It asserted `Ok` and the rewrite but never the **warning** -- the
///   `and warns` half of its own name -- because `eprintln!` goes to the
///   process's real stderr, which an integration test cannot capture. It
///   now drives `update_header_in_place_reporting` with a capturing sink
///   and asserts the warning is emitted and says what it is about.
/// - Run as **root**, mode `0o311` denies nothing (the superuser bypasses
///   the permission check), `fsync_parent` succeeds, no warning is emitted,
///   and the test's old assertions -- `Ok` plus a rewritten header, which
///   any healthy run satisfies -- all passed **vacuously**, testing
///   nothing. It now probes whether the mode actually denies the open and
///   fails loudly if it does not, rather than reporting a pass for a
///   scenario it never managed to stage.
#[cfg(unix)]
#[test]
fn an_update_succeeds_and_warns_when_the_parent_directory_cannot_be_synced() {
    let _g = lock();
    use std::os::unix::fs::PermissionsExt;

    let (dir, path) = temp_fits_copy("fsync-parent-denied");

    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o311))
        .unwrap_or_else(|e| panic!("setting 0o311 on {}: {e}", dir.path().display()));
    // Does 0o311 actually deny reading this directory for *this* process?
    // Under an euid that bypasses mode checks (root), it does not -- and
    // then the scenario under test simply cannot be staged. Probe the real
    // behaviour rather than the uid, since the uid is not what matters and
    // `std` exposes no `geteuid` anyway.
    let mode_denies_the_open = std::fs::File::open(dir.path()).is_err();
    let (result, warnings) = if mode_denies_the_open {
        capturing_warnings(|warn| update_header_in_place_reporting(&path, &wcs_fixture(), warn))
    } else {
        (Ok(()), Vec::new())
    };
    // Restore before any further filesystem access -- including this
    // test's own asserts below and `ScratchDir`'s `Drop`, both of which
    // need to read/list the directory.
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755))
        .unwrap_or_else(|e| panic!("restoring permissions on {}: {e}", dir.path().display()));
    assert!(
        mode_denies_the_open,
        "cannot stage a post-rename directory-fsync failure: mode 0o311 did not deny opening {} \
         for this process, which is what happens when the tests run as root. This test would \
         otherwise pass without exercising anything -- run the suite as a normal user",
        dir.path().display()
    );

    assert!(
        result.is_ok(),
        "a directory that cannot be fsync'd must not fail an already-completed write, got {result:?}"
    );
    assert!(
        warnings.iter().any(|w| w.contains("could not be synced")),
        "the completed-but-unsynced write must be reported as a warning, not silently: {warnings:?}"
    );
    let out = std::fs::read(&path).unwrap();
    let h = FitsHeader::parse(&out).unwrap();
    assert_eq!(h.get("PLTSOLVD"), Some("T"), "the header must actually have been rewritten despite the fsync failure");
    drop(dir);
}
