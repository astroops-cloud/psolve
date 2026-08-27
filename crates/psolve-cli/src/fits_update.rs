//! The `-update` write path: rewrite a solved WCS directly into an existing
//! FITS file's header.
//!
//! This is the one place in the whole crate that touches a frame's **pixel
//! data** -- irreplaceable recorded photons from a specific night, not
//! reconstructible data -- and the only place that rewrites an existing FITS
//! file at all. It is not, however, the only place that can destroy
//! irreplaceable bytes: ASTAP mode's `.ini`/`.wcs` sidecar writes overwrite
//! real recorded ASTAP output when they land beside a frame. Those writes go
//! through the same two safety switches this module owns, via
//! [`refuse_if_readonly_output`]; the rest of the rules below (temp copy,
//! verified rename, block-count refusal) are specific to the pixel data and
//! apply to `-update` alone. A FITS file is a header of
//! fixed 2880-byte blocks immediately followed by pixel data, with no
//! separator between them: if the header grows by even one card past a
//! block boundary, every byte of pixel data shifts. That happened for real
//! once in this project's history and silently corrupted four archive
//! frames. The rules here exist solely to make that impossible again:
//!
//!  1. `psolve-core` never touches a filesystem (see its own module doc,
//!     `psolve-core/src/lib.rs`) -- every byte here is read and written by
//!     `psolve-cli` alone. `psolve-core` gains no write path from this file.
//!  2. **The `.psolve-readonly` guarantee, stated exactly.**
//!
//!     > **A `.psolve-readonly` marker anywhere on the target's canonical
//!     > (physical) ancestor chain always refuses the write.**
//!
//!     That is unconditional. It does not depend on how the path was
//!     spelled, on the process's working directory, on any environment
//!     variable, or on any symlink anywhere: the canonical chain is
//!     [`std::fs::canonicalize`]'s output -- the real directories the
//!     target's bytes actually live in, walked to the filesystem root -- and
//!     canonicalization failing is itself a refusal (fail closed), never a
//!     reason to fall back to an unresolved path. **This is the guarantee to
//!     rely on. To protect a tree, put the marker in the tree the frames
//!     physically live in.**
//!
//!     Two further chains are walked as **additional, best-effort**
//!     coverage, for markers placed in a tree that is *not* the file's
//!     physical location (a locally-named directory whose contents are a
//!     symlink into a mounted share -- this project's own layout):
//!
//!     - *physical-lexical* -- the path as given, made absolute against
//!       [`std::env::current_dir`] (`getcwd(3)`) if it was relative, with
//!       no symlink resolved in the joined path itself. Available whenever
//!       the cwd can be read, which is essentially always.
//!     - *logical-lexical* -- the path as given, made absolute against
//!       `$PWD` instead. Available only when `$PWD` passes every check in
//!       [`logical_lexical_chain`]; otherwise it is omitted (and, for a
//!       relative path, a warning is printed -- rule 8).
//!
//!     Why the third chain exists at all: `getcwd(3)` -- which both
//!     [`std::fs::canonicalize`] and [`std::env::current_dir`] are built
//!     on -- returns the kernel's *physical*, symlink-resolved current
//!     directory. **No operating system records a logical working
//!     directory.** The as-typed cwd exists only as a shell convention:
//!     `cd` sets `$PWD` to the directory as typed, preserving symlinks. So
//!     for a relative path invoked from inside a symlinked directory (`cd
//!     ~/astroops/archive && psolve … frame.fits -update`), the canonical
//!     and physical-lexical chains both resolve to the same place, both
//!     already past the symlink, and neither can see a marker placed on the
//!     symlinked tree's own name. Only `$PWD` can reconstruct it -- and
//!     `$PWD` is ordinary environment input that may be absent, stale, or
//!     hostile, so it is used only under the constraints in
//!     [`logical_lexical_chain`], and never as the basis of a guarantee.
//!
//!     The ancestor walk (rule 7) checks every chain that is available and
//!     refuses if *any* carries a marker -- the guard's evidence is a union,
//!     not a choice.
//!
//!     **Shape enumeration, and what is honestly covered.** Every
//!     invocation is one of eight cells across three independent axes:
//!     **A** (path given absolute, or relative), **S** (the path text itself
//!     passes through a symlinked component, or not), **C** (the process's
//!     cwd is itself reached through a symlink, or not). `A` = absolute is
//!     unaffected by `C` at all (an absolute path is never joined with the
//!     cwd), so the two `abs` rows below stand for two cells each.
//!
//!     The canonical guarantee above holds in **all eight cells, always**.
//!     This table is only about the *additional* coverage: whether a marker
//!     placed on an as-typed tree that is not the file's physical location
//!     is seen.
//!
//!     | A | S | C | as-typed-tree markers | by which chain |
//!     |---|---|---|---|---|
//!     | abs | no | n/a | **unconditional** (there is no second tree: the path text *is* the physical path) | physical-lexical |
//!     | abs | yes | n/a | **unconditional** (no cwd is involved, so the as-typed chain is fully known from the argument) | physical-lexical |
//!     | rel | no | no | **unconditional** (the physical cwd *is* the as-typed cwd) | physical-lexical |
//!     | rel | yes | no | **unconditional** (physical cwd = as-typed cwd, and the path text's own symlink is preserved by the join) | physical-lexical |
//!     | rel | no | yes | **conditional on `$PWD`** -- covered when `$PWD` is present, absolute, `..`-free and verifies by device+inode; **otherwise out of scope** | logical-lexical |
//!     | rel | yes | yes | the path-text segment is unconditional (physical-lexical preserves it); the *as-typed cwd* segment is **conditional on `$PWD`** on the same terms, **otherwise out of scope** | physical-lexical + logical-lexical |
//!
//!     **Six of the eight cells are covered unconditionally. Two -- both
//!     `relative` + `symlinked cwd` -- are covered only conditionally, and
//!     are otherwise out of scope.** The reason they are out of scope, not
//!     merely unimplemented: the kernel does not record a logical working
//!     directory, so when `$PWD` is missing or untrustworthy there is *no
//!     information anywhere in the process* from which the as-typed path
//!     could be reconstructed. No amount of additional chain-walking can
//!     close this; three previous attempts each closed one demonstrated
//!     shape and left a sibling open. Concretely, these all fall in the
//!     out-of-scope part of those two cells and **will proceed with a
//!     lexical-only marker above the file** (each is pinned by a test in
//!     `tests/fits_update.rs`, asserting exactly this documented behaviour):
//!
//!     - `$PWD` unset -- a launcher (cron, systemd, any parent that
//!       `chdir()`ed then `exec()`ed) does not set it; only shells do.
//!     - `$PWD` stale -- e.g. Python's `os.chdir()` + `subprocess`, which
//!       leaves `os.environ['PWD']` pointing at the old directory.
//!     - `$PWD` naming the same directory through a *different, unmarked*
//!       twin symlink -- it verifies by device+inode (it really is this
//!       directory) but yields the wrong as-typed chain. Device+inode
//!       equality proves `$PWD` is *a* name for the cwd; nothing can prove
//!       it is *the name the user typed*.
//!     - `$PWD="."` -- rejected outright by the absolute-path check in
//!       [`logical_lexical_chain`], since it would otherwise pass the
//!       device+inode test and then yield a chain with no ancestors at all.
//!
//!     In every one of those cases a marker on the **canonical** chain still
//!     refuses, per the guarantee at the top of this rule.
//!
//!     One deliberately accepted, documented side effect of not resolving
//!     `..` in the lexical chains: a path like `a/../b/frame.fits` yields
//!     `a` as one of the syntactic ancestors `Path::parent()` walks over
//!     (since `.parent()` strips components without resolving `..`), so a
//!     marker at `a/.psolve-readonly` refuses the write even though the
//!     resolved target `b/frame.fits` is not really inside `a` at all. This
//!     is over-inclusive, never under-inclusive -- it can only make the
//!     guard refuse a write that a stricter, `..`-aware walk would have
//!     allowed, never the reverse -- so it is left as-is rather than
//!     "fixed" with path normalization this guard does not otherwise need.
//!     (`$PWD` itself is held to the stricter standard and is *rejected*
//!     when it contains `..`, because there the over-inclusion would be
//!     built from untrusted input rather than from the user's own argument.)
//!  3. Never write in place. A full new copy is written to a temp file
//!     beside the target and `fsync`ed there before anything else happens
//!     -- that `sync_all` is a hard failure and gates the rest, exactly
//!     like every other check here. The temp file is then
//!     [`std::fs::rename`]d over the target, which is atomic on the same
//!     filesystem and alone already survives `kill -9` (the old directory
//!     entry or the new one exists afterwards, never a half-written mix).
//!     This project also attempts to `fsync` the target's parent directory
//!     after a successful rename, which is what closes the remaining gap
//!     against a power loss or kernel panic (which can otherwise persist a
//!     directory-entry change ahead of the file data it points to) -- but
//!     that directory `fsync` is treated as best-effort, not a hard
//!     failure: see rule 5.
//!  4. Before the rename, the temp file is reparsed from a **fresh
//!     `std::fs::read`** (not the in-memory buffer that built it): its data
//!     unit must start at the same byte offset as the original's, and its
//!     pixel bytes must be byte-identical to the original's. Any mismatch
//!     deletes the temp file and returns an error -- the rename never
//!     happens. Note what this re-read does and does not prove: on a normal
//!     filesystem it is very likely served from the page cache, not the
//!     media, so it validates the *logic* that produced the bytes (the
//!     header packing, the pixel-slice bookkeeping) -- it is not, by
//!     itself, evidence the bytes have reached durable storage. The temp
//!     file's own `sync_all` (rule 3) is what makes that claim, and it does
//!     so before the rename, while failing it is still cheap and safe.
//!  5. A failure to `fsync` the target's parent directory *after* a
//!     successful rename is deliberately **not** a hard failure: by that
//!     point the write has already happened (the rename returned `Ok`), so
//!     reporting `Err` here would describe a completed, irreversible write
//!     as though nothing had happened -- a caller that reads that `Err` as
//!     "safe to retry" or "the frame is still unsolved" would be misled
//!     about a destructive operation that already occurred, which is the
//!     more dangerous direction to be wrong in. This matters in practice,
//!     not just in theory: this project's astro imaging data lives on
//!     SMB-mounted NAS shares, and directory `fsync` returns "operation not
//!     supported" on that filesystem every time, for every frame stored
//!     there -- treating it as fatal would make every NAS-hosted `-update`
//!     report failure after successfully rewriting the file. On a
//!     filesystem that cannot `fsync` a directory, the rename's durability
//!     is that filesystem's own responsibility, not something this program
//!     can add; this only logs a warning to stderr and still returns
//!     success.
//!  6. If the new header would need more 2880-byte blocks than the original
//!     had, the write is refused outright rather than shifting the data
//!     unit to make room, or truncating the header to force a fit.
//!  7. Refused before a single byte is written: `PSOLVE_READONLY` (any
//!     non-empty value, read with [`std::env::var_os`] so a non-UTF-8 value
//!     still refuses rather than silently passing as "unset"), a
//!     `.psolve-readonly` marker file found on *any* of the target's
//!     available ancestor chains (see rule 2 -- the canonical chain always,
//!     which is where the guarantee lives; plus physical-lexical, and
//!     logical-lexical when `$PWD` allows it; any one carrying a marker
//!     refuses), or the target file itself having every
//!     write bit clear in its Unix mode
//!     (`chmod a-w` / `chmod 444`). That last check is narrower than it
//!     might sound, and is exactly what it checks and no more: it catches
//!     "no one can write this file", not "this program specifically
//!     should not". A perfectly writable `0600` file is not protected by
//!     it, and `rename` still silently replaces such a file's inode
//!     outright -- the new inode gets the temp file's own mode (not the
//!     original's exact bits), any custom ownership or extended
//!     attributes on the original are gone, and a hardlinked second name
//!     of the same original inode keeps pointing at the old, unmodified
//!     header. This project chooses to refuse on the all-write-bits-clear
//!     case rather than to carry the old metadata onto the temp file more
//!     generally: preserving ownership in particular generally requires
//!     privileges this process may not have, and a best-effort carry-over
//!     that can itself fail quietly is a worse guarantee than "a read-only
//!     frame stays read-only."
//!  8. When the target is given as a **relative** path and the
//!     logical-lexical chain could not be built (rule 2's two conditional
//!     cells, in their out-of-scope form), a warning is printed to stderr
//!     naming the file and the reason, at the moment the guard decides to
//!     let the write proceed. The user is the only party who can tell
//!     whether their marker sits on a tree this program cannot see, and this
//!     is the moment that matters -- so the reduced coverage is reported
//!     rather than left silent. It is a warning, not a refusal: refusing
//!     every relative invocation whose `$PWD` is unset would break every
//!     non-shell launcher (cron, systemd, `subprocess`) for a hazard that
//!     only exists when a marker has been placed on a symlinked tree, and
//!     the canonical guarantee (rule 2) is unaffected either way.
//!
//! [`update_header_in_place`] is wired into `astap_cmd`'s dispatch in
//! `main.rs`, behind the explicit `-update` flag (Task 10, M3 progress
//! ledger) -- default off, exactly as this module's safety model requires.
//! It remains fully exercised by `tests/fits_update.rs`.
//!
//! Rules 1, 2 and 7's two switches are **not** exclusive to `-update`:
//! [`refuse_if_readonly_output`] exposes them by path, and `astap_cmd` calls
//! it before every `.ini`/`.wcs` sidecar write on both its success and its
//! failure path. A sidecar write is not a pixel-data rewrite, so rules 3-6
//! do not apply to it -- but it does overwrite recorded ASTAP output that
//! cannot be reconstructed, so the safety switches a user is told to rely on
//! must cover it, and do.

use std::fs::File;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use psolve_core::fit::Wcs;
use psolve_core::fits::FitsHeader;

use crate::sidecar;

const CARD: usize = 80;
const BLOCK: usize = 2880;

/// Why a `-update` write did not happen. Every variant is a refusal or a
/// hard failure that ran *before* the rename -- there is no partial-write
/// outcome here by design. (A failure to `fsync` the target's parent
/// directory *after* a successful rename is not one of these variants at
/// all: see the module doc, rule 5 -- by that point the write already
/// happened, so it is reported as a stderr warning alongside `Ok(())`, not
/// as an `Err`.)
#[derive(Debug)]
pub enum FitsUpdateError {
    /// `PSOLVE_READONLY` is set, a `.psolve-readonly` marker exists on any
    /// of the target's available ancestor chains (canonical -- always, and
    /// the one that carries this module's guarantee; plus physical-lexical
    /// and, conditionally, logical-lexical -- module doc, rule 2), or the
    /// target itself is not writable by its Unix mode.
    ReadOnly(String),
    /// The path could not be resolved to a canonical, symlink-free form
    /// (most commonly: it does not exist). Fail-closed counterpart of the
    /// canonicalization rule 2 -- never falls back to the unresolved path.
    UnresolvedPath(String),
    /// The file's bytes did not parse as a FITS header at all.
    NotFits(String),
    /// The new header needs more 2880-byte blocks than the original file
    /// had. Refused rather than risked: see the module doc, rule 6.
    HeaderGrew { original_blocks: usize, needed_blocks: usize },
    /// The temp file's reparsed data unit did not land at the same offset,
    /// or its pixel bytes differ from the original's. Should be
    /// unreachable given how the temp file is constructed below, but is
    /// checked for real rather than assumed -- see the module doc, rule 4.
    Verify(String),
    /// Any other filesystem failure: reading the original, writing or
    /// syncing the temp file, or the rename itself. Does NOT cover a failed
    /// `fsync` of the target's parent directory after a successful rename
    /// -- see this enum's own doc comment and the module doc's rule 5.
    Io(String),
}

impl std::fmt::Display for FitsUpdateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FitsUpdateError::ReadOnly(msg) => write!(f, "refusing to write, read-only: {msg}"),
            FitsUpdateError::UnresolvedPath(msg) => {
                write!(f, "refusing to update: could not resolve the real path: {msg}")
            }
            FitsUpdateError::NotFits(msg) => write!(f, "refusing to update: not a FITS header: {msg}"),
            FitsUpdateError::HeaderGrew { original_blocks, needed_blocks } => write!(
                f,
                "refusing to update: the new header needs {needed_blocks} 2880-byte block(s) but \
                 the original header only has {original_blocks}; writing it would shift the pixel \
                 data, so nothing was written"
            ),
            FitsUpdateError::Verify(msg) => {
                write!(f, "refusing to update, verification of the rewritten file failed: {msg}")
            }
            FitsUpdateError::Io(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for FitsUpdateError {}

/// Rewrite `path`'s FITS header in place with the solved WCS `w`: any of the
/// solution's keywords the header already carries (e.g. from an earlier
/// solve) are replaced in place; the rest are appended. Every other card and
/// every pixel byte is left exactly as it was. See the module doc for the
/// full safety model this follows to guarantee that.
pub fn update_header_in_place(path: &Path, w: &Wcs) -> Result<(), FitsUpdateError> {
    update_header_in_place_reporting(path, w, &mut default_warn)
}

/// The default warning sink: stderr, in this crate's `psolve: warning: …`
/// convention. Split out only so [`update_header_in_place_reporting`] can be
/// handed a capturing sink by tests.
fn default_warn(msg: &str) {
    eprintln!("psolve: warning: {msg}");
}

/// [`update_header_in_place`] with its stderr warnings redirected to `warn`.
///
/// Two of this module's documented behaviours are *warnings alongside a
/// successful or permitted operation* rather than `Err` values -- the
/// reduced-coverage notice of rule 8 and the non-fatal directory-`fsync`
/// failure of rule 5 -- so neither is observable through the return type.
/// Threading the sink through is what lets `tests/fits_update.rs` assert
/// that they are actually emitted; an integration test cannot capture the
/// process's real stderr, and a test that asserts only the `Ok` half of
/// "succeeds *and* warns" silently stops checking the half that was the
/// whole point.
// `pub(crate)`, not private: see the doc comment above.
pub(crate) fn update_header_in_place_reporting(
    path: &Path,
    w: &Wcs,
    warn: &mut dyn FnMut(&str),
) -> Result<(), FitsUpdateError> {
    // Rule 2: resolve up front. `real_path` (canonical) is used for every
    // filesystem operation below -- reading, the temp file's location, and
    // the rename -- so those always land on the bytes that actually exist,
    // and it is also the chain the module's one unconditional guarantee is
    // made of. `physical_lexical_path` and `logical_chain` exist purely so
    // the ancestor walk in `refuse_if_readonly` can *additionally* check the
    // path the way it was given/typed, which `real_path` alone cannot: see
    // the module doc, rule 2, for what that adds and what it cannot.
    let physical_lexical_path = absolute_lexical_path(path)?;
    let logical_chain = logical_lexical_chain(path);
    let real_path = std::fs::canonicalize(path)
        .map_err(|e| FitsUpdateError::UnresolvedPath(format!("{}: {e}", path.display())))?;

    refuse_if_readonly(&real_path, &physical_lexical_path, logical_chain.path())?;
    refuse_if_not_writable(&real_path)?;

    // Rule 8: the guards have just decided to let this write proceed, and
    // for this invocation they did so with less than full as-typed coverage.
    // Say so now, while it can still mean something to whoever placed the
    // marker.
    if let LogicalChain::Unavailable(reason) = logical_chain {
        warn(&format!(
            "{} was given as a relative path, but the directory it was typed relative to could \
             not be reconstructed ({reason}). A .psolve-readonly marker placed on a *symlinked* \
             parent of the current directory cannot be seen in that case -- the kernel records \
             only the physical working directory. A marker anywhere on {}'s real (canonical) \
             directory chain is always honoured and is unaffected by this",
            path.display(),
            real_path.display()
        ));
    }

    let original = std::fs::read(&real_path)
        .map_err(|e| FitsUpdateError::Io(format!("reading {}: {e}", real_path.display())))?;
    let header = FitsHeader::parse(&original).map_err(|e| FitsUpdateError::NotFits(e.to_string()))?;
    let original_blocks = header.data_offset / BLOCK;
    let pixel_data = &original[header.data_offset..];

    let raw_cards = raw_header_cards(&original, header.data_offset);
    let merged = merge_wcs_cards(raw_cards, w);
    let new_header = pack_header(&merged, header.data_offset).map_err(|needed_blocks| {
        FitsUpdateError::HeaderGrew { original_blocks, needed_blocks }
    })?;

    let mut new_file = Vec::with_capacity(new_header.len() + pixel_data.len());
    new_file.extend_from_slice(&new_header);
    new_file.extend_from_slice(pixel_data);

    commit_new_file(&real_path, &new_file, header.data_offset, pixel_data, warn)
}

/// Write `new_file` to a temp file beside `real_path`, durably (rule 3),
/// verify it against `expected_offset`/`expected_pixels` (rule 4), and
/// rename it over `real_path` on success. On any failure up to and
/// including the rename itself, the temp file is removed (via
/// [`TempGuard`]) and `real_path` is left untouched. After a successful
/// rename, `real_path`'s parent directory is `fsync`ed on a best-effort
/// basis (rule 5): a failure there goes to `warn` (stderr in production),
/// not returned as
/// `Err` -- the write has already happened by that point, and misreporting
/// a completed rewrite as a failure is the more dangerous direction to be
/// wrong in (a caller might otherwise treat the frame as still unsolved and
/// retry, when the header was in fact already rewritten).
///
/// Split out from [`update_header_in_place`] so this commit/cleanup
/// sequence -- including the verify-failure path, which should be
/// unreachable on correct input and is otherwise hard to exercise for real
/// -- can be driven directly and deterministically by tests, by handing it
/// `new_file` bytes that do not match `expected_pixels`. That runs the
/// exact same code production does, just with a deliberately corrupted
/// input standing in for the kind of corruption this function exists to
/// catch.
// `pub(crate)`, not private: `tests/fits_update.rs` drives the
// verify-failure path directly through this exact commit/cleanup logic (see
// its doc comment above), by handing it bytes that deliberately do not
// match `expected_pixels`.
pub(crate) fn commit_new_file(
    real_path: &Path,
    new_file: &[u8],
    expected_offset: usize,
    expected_pixels: &[u8],
    warn: &mut dyn FnMut(&str),
) -> Result<(), FitsUpdateError> {
    let tmp_path = temp_path_beside(real_path);
    // Unarmed until `write_temp_durably` confirms `File::create` actually
    // created the path -- arming any earlier would have the guard try to
    // remove a path that a failed `create` never brought into existence.
    let mut guard = TempGuard::new(tmp_path.clone());

    write_temp_durably(&tmp_path, new_file, &mut guard)?;
    verify_temp(&tmp_path, expected_offset, expected_pixels)?;

    std::fs::rename(&tmp_path, real_path).map_err(|e| {
        FitsUpdateError::Io(format!("renaming {} onto {}: {e}", tmp_path.display(), real_path.display()))
    })?;
    // The rename succeeded: the temp path no longer exists (it *is* now
    // `real_path`), so there is nothing left for the guard to clean up.
    guard.disarm();

    // Rule 5: the write already happened above. A directory that cannot be
    // `fsync`ed (this project's SMB-mounted NAS shares return "operation
    // not supported" for every call) is not a reason to report a completed,
    // irreversible write as a failure.
    if let Err(e) = fsync_parent(real_path) {
        warn(&format!(
            "{} was updated successfully, but its directory could not be synced ({e}); on this \
             filesystem the rename's durability is guaranteed by the filesystem itself, not by \
             this program",
            real_path.display()
        ));
    }

    Ok(())
}

/// `path`, made absolute against [`std::env::current_dir`] (the *physical*,
/// symlink-resolved cwd -- `getcwd(3)`) if it was relative, with no symlink
/// resolved in the joined path itself. The physical-lexical form from the
/// module doc's rule 2 -- the chain that unconditionally covers as-typed
/// markers in six of the eight shape cells; see there for what it cannot
/// cover (the two `relative` + `symlinked cwd` cells) and for the
/// documented, deliberately accepted over-inclusion a `..` component in
/// `path` can cause here (never under-inclusion).
fn absolute_lexical_path(path: &Path) -> Result<PathBuf, FitsUpdateError> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        let cwd = std::env::current_dir()
            .map_err(|e| FitsUpdateError::Io(format!("reading the current directory: {e}")))?;
        Ok(cwd.join(path))
    }
}

/// Whether the logical-lexical chain (module doc, rule 2) exists for this
/// invocation, and if not, why not -- the "why not" is what rule 8's warning
/// tells the user.
enum LogicalChain {
    /// The path was given absolute: no working directory is joined at all,
    /// so this chain is not merely unavailable but *not applicable*. The
    /// as-typed chain is fully known from the argument itself, and rule 2's
    /// table marks both `abs` rows unconditionally covered for that reason.
    NotApplicable,
    /// Built from a `$PWD` that passed every check below.
    Built(PathBuf),
    /// `path` is relative but no trustworthy as-typed working directory
    /// could be reconstructed. Carries a short human-readable reason.
    Unavailable(&'static str),
}

impl LogicalChain {
    fn path(&self) -> Option<&Path> {
        match self {
            LogicalChain::Built(p) => Some(p),
            _ => None,
        }
    }
}

/// `path`, made absolute against `$PWD` instead of `getcwd(3)`, if `path`
/// is relative and `$PWD` clears every bar below. The logical-lexical form
/// from the module doc's rule 2 -- best-effort additional coverage for the
/// "relative path invoked from inside a symlinked cwd" shape, since
/// `getcwd(3)` (what both [`std::fs::canonicalize`] and
/// [`absolute_lexical_path`] use) returns the *physical* cwd, already past
/// any symlink, while a real shell's `$PWD` preserves it.
///
/// `$PWD` is ordinary environment input a caller can set to anything, so it
/// must clear all of:
///
/// 1. **Set and non-empty.** A launcher that `chdir()`ed and `exec()`ed
///    (cron, systemd, `subprocess`) sets no `$PWD` at all; only shells do.
/// 2. **Absolute.** A relative `$PWD` cannot be an ancestor chain. This is
///    specifically what rejects `$PWD="."`, which would otherwise sail
///    through the device+inode check below (`.` *is* the current directory,
///    trivially) and then yield `./frame.fits`, a chain whose only ancestor
///    is `.` itself -- no ancestors at all, silently contributing nothing
///    while looking like coverage.
/// 3. **Free of `.` and `..` components.** A `..` would make the walk visit
///    syntactic ancestors the user never named, built out of untrusted
///    input rather than out of their own argument (see the module doc's
///    note on why the same over-inclusion *is* accepted for `path` itself).
/// 4. **Actually this directory**, by device+inode against `.` -- never by
///    string, which a symlink or a `..` would defeat. This is what rejects a
///    stale `$PWD` left behind by e.g. Python's `os.chdir()`.
///
/// Passing all four proves `$PWD` is *a* name for the current directory. It
/// does **not** prove it is *the* name the user typed -- a second, unmarked
/// symlink to the same directory passes every check above and yields the
/// wrong as-typed chain. Nothing can close that gap, because no OS records a
/// logical working directory; the module doc's rule 2 states it as an
/// out-of-scope residual rather than papering over it, and the canonical
/// chain's guarantee is unaffected by it.
fn logical_lexical_chain(path: &Path) -> LogicalChain {
    if path.is_absolute() {
        return LogicalChain::NotApplicable;
    }
    let Some(pwd) = std::env::var_os("PWD").filter(|v| !v.is_empty()).map(PathBuf::from) else {
        return LogicalChain::Unavailable("$PWD is unset or empty, as it is under any launcher that is not a shell");
    };
    if !pwd.is_absolute() {
        return LogicalChain::Unavailable("$PWD is not an absolute path");
    }
    if pwd
        .components()
        .any(|c| matches!(c, std::path::Component::CurDir | std::path::Component::ParentDir))
    {
        return LogicalChain::Unavailable("$PWD contains a '.' or '..' component");
    }
    match same_directory(&pwd, Path::new(".")) {
        Some(true) => LogicalChain::Built(pwd.join(path)),
        Some(false) => LogicalChain::Unavailable("$PWD names a different directory than the process is actually in"),
        None => LogicalChain::Unavailable("$PWD could not be corroborated against the current directory"),
    }
}

/// Whether `a` and `b` name the same directory on disk, compared by
/// device+inode (never by string equality, which a symlink or a `..` would
/// defeat). `None` means "could not be determined" (either path failed to
/// `stat`, or -- off Unix, where there is no portable device+inode API in
/// `std` -- this is simply never able to corroborate anything), which
/// callers must treat as "not verified", not as "verified false".
#[cfg(unix)]
fn same_directory(a: &Path, b: &Path) -> Option<bool> {
    use std::os::unix::fs::MetadataExt;
    let ma = std::fs::metadata(a).ok()?;
    let mb = std::fs::metadata(b).ok()?;
    Some(ma.dev() == mb.dev() && ma.ino() == mb.ino())
}

#[cfg(not(unix))]
fn same_directory(_a: &Path, _b: &Path) -> Option<bool> {
    None
}

/// `PSOLVE_READONLY` (any non-empty value) or a `.psolve-readonly` marker
/// found by walking `real_path`'s ancestors, `physical_lexical_path`'s
/// ancestors, or (when it could be built) `logical_lexical_path`'s
/// ancestors refuses the write before anything is read or written. Every
/// available chain is checked and any one carrying a marker is enough to
/// refuse.
///
/// The chains are **not** equals. `real_path`'s walk is the module's one
/// unconditional guarantee (module doc, rule 2): a marker on the canonical
/// ancestor chain refuses, always, in every invocation shape. The other two
/// are additional best-effort coverage for markers placed on a tree that is
/// not the file's physical location, and the third is not always available
/// at all. See the module doc's rule 2 table for exactly which shapes get
/// which.
fn refuse_if_readonly(
    real_path: &Path,
    physical_lexical_path: &Path,
    logical_lexical_path: Option<&Path>,
) -> Result<(), FitsUpdateError> {
    // `var_os`, not `var`: `var` returns `Err` for a set-but-non-UTF-8
    // value, and `if let Ok(v)` would silently treat that as "unset" --
    // fails open on exactly the kind of malformed input a hard safety
    // switch must fail closed on instead.
    if let Some(v) = std::env::var_os("PSOLVE_READONLY") {
        if !v.is_empty() {
            return Err(FitsUpdateError::ReadOnly(format!("PSOLVE_READONLY is set (value {v:?})")));
        }
    }
    let candidates = [Some(real_path), Some(physical_lexical_path), logical_lexical_path];
    for candidate in candidates.into_iter().flatten() {
        if let Some(marker) = marker_in_ancestors(candidate.parent()) {
            return Err(FitsUpdateError::ReadOnly(format!("{} exists", marker.display())));
        }
    }
    Ok(())
}

/// The same two safety switches [`update_header_in_place`] applies -- the
/// `PSOLVE_READONLY` environment variable and a `.psolve-readonly` marker on
/// any available ancestor chain -- applied to **a file this process is about
/// to create or overwrite**, before a single byte is written.
///
/// This is [`refuse_if_readonly`] reached by path rather than by the three
/// chains `update_header_in_place_reporting` has already computed: the exact
/// same union of canonical, physical-lexical and logical-lexical chains, the
/// same fail-closed canonicalization, not a second, weaker re-derivation.
///
/// The one thing that differs is how the canonical chain is obtained, and it
/// has to: an output file usually does not exist yet, so
/// [`std::fs::canonicalize`] on the target itself would fail for the ordinary
/// case. Its **parent directory** is canonicalized instead and the file name
/// rejoined -- the real directory the bytes would land in, which is exactly
/// what the ancestor walk needs (it starts from `.parent()` anyway). If the
/// target does already exist, it is canonicalized directly, so an existing
/// sidecar reached through a symlink is still resolved to where it really
/// lives. A parent that cannot be canonicalized is itself a refusal, never a
/// fallback to the unresolved path.
///
/// `pub(crate)`, not private: `main.rs`'s ASTAP-mode dispatch calls this
/// before **every** `.ini`/`.wcs` sidecar write, on both the success and the
/// failure path. Those writes destroy real recorded ASTAP output when they
/// land beside a frame (`~/astroops` holds 46 `.ini` and 13 `.wcs` files
/// sitting next to their frames, the byte-exact ground truth this
/// milestone's fixtures were transcribed from, none of it reconstructible),
/// so they are covered by the documented safety switches rather than left
/// outside them.
pub(crate) fn refuse_if_readonly_output(path: &Path) -> Result<(), FitsUpdateError> {
    let physical_lexical_path = absolute_lexical_path(path)?;
    let logical_chain = logical_lexical_chain(path);
    let real_path = canonical_output_path(path)?;
    refuse_if_readonly(&real_path, &physical_lexical_path, logical_chain.path())
}

/// The canonical (physical) form of a path that may not exist yet: the path
/// itself when it does, otherwise its canonicalized parent directory with the
/// file name rejoined. Fail-closed -- a parent that cannot be resolved is an
/// error, never a silent fallback to the path as given.
fn canonical_output_path(path: &Path) -> Result<PathBuf, FitsUpdateError> {
    if let Ok(p) = std::fs::canonicalize(path) {
        return Ok(p);
    }
    let parent = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        // A bare file name (`frame.ini`) has `Some("")` as its parent, and a
        // path with no parent at all is the filesystem root; the current
        // directory is the right chain to walk in the first case and a
        // harmless no-op in the second.
        _ => Path::new("."),
    };
    let real_parent = std::fs::canonicalize(parent)
        .map_err(|e| FitsUpdateError::UnresolvedPath(format!("{}: {e}", parent.display())))?;
    Ok(match path.file_name() {
        Some(name) => real_parent.join(name),
        None => real_parent,
    })
}

/// Walk `dir` and every ancestor above it looking for a `.psolve-readonly`
/// marker, returning the first one found.
fn marker_in_ancestors(dir: Option<&Path>) -> Option<PathBuf> {
    let mut dir = dir;
    while let Some(d) = dir {
        let marker = d.join(".psolve-readonly");
        // `symlink_metadata` (lstat), not `Path::exists` (stat, follows the
        // final symlink): `exists()` returns `false` -- indistinguishable
        // from "no marker" -- on a permission error or a dangling symlink,
        // either of which would otherwise let a marker silently fail to
        // protect anything.
        if std::fs::symlink_metadata(&marker).is_ok() {
            return Some(marker);
        }
        dir = d.parent();
    }
    None
}

/// Refuse when `real_path` is not writable by its own Unix mode (e.g.
/// `chmod a-w`). `rename` replaces the target's inode outright and would
/// otherwise silently discard that protection permanently -- see the
/// module doc's rule 7 for why this project refuses here rather than
/// attempting to carry the old metadata onto the temp file.
fn refuse_if_not_writable(real_path: &Path) -> Result<(), FitsUpdateError> {
    let meta = std::fs::metadata(real_path)
        .map_err(|e| FitsUpdateError::Io(format!("reading metadata for {}: {e}", real_path.display())))?;
    if meta.permissions().readonly() {
        return Err(FitsUpdateError::ReadOnly(format!(
            "{} is not writable (mode denies write); a rename would silently discard that \
             protection, so refusing instead of overwriting it",
            real_path.display()
        )));
    }
    Ok(())
}

/// Split the header region (`bytes[..data_offset]`) into raw, byte-exact
/// 80-byte cards, stopping before `END`. Deliberately independent of
/// `FitsHeader`'s own card list (which strips comments and quoting, and so
/// is lossy) -- every card this returns is reproduced byte-for-byte in the
/// rewritten file unless [`merge_wcs_cards`] specifically replaces it.
///
/// `pub(crate)`, not private: `main.rs`'s ASTAP-mode dispatch reuses this
/// same byte-exact card scan to build the `.wcs` sidecar's pass-through
/// original-header section, rather than a second, lossy re-derivation from
/// `FitsHeader`'s parsed card list.
pub(crate) fn raw_header_cards(bytes: &[u8], data_offset: usize) -> Vec<[u8; CARD]> {
    let mut cards = Vec::new();
    let mut off = 0usize;
    while off + CARD <= data_offset && off + CARD <= bytes.len() {
        let mut c = [b' '; CARD];
        c.copy_from_slice(&bytes[off..off + CARD]);
        if card_key(&c) == "END" {
            break;
        }
        cards.push(c);
        off += CARD;
    }
    cards
}

/// `pub(crate)`, not private: `main.rs`'s ASTAP-mode dispatch reuses this to
/// identify `BITPIX`/`NAXIS`/`NAXIS1`/`NAXIS2` cards while building the
/// `.wcs` sidecar's pass-through original-header section.
pub(crate) fn card_key(card: &[u8; CARD]) -> String {
    String::from_utf8_lossy(&card[..8]).trim().to_string()
}

/// A card's full text (key, `=`, value, and any comment), trailing spaces
/// trimmed -- used only to recognise psolve's own solve-marker `COMMENT`
/// card by its exact content (see [`merge_wcs_cards`]), never to identify
/// any other card, which is why this stays narrower than [`card_key`].
fn card_text(card: &[u8; CARD]) -> String {
    String::from_utf8_lossy(card).trim_end().to_string()
}

/// Merge the solved WCS's cards ([`sidecar::wcs_solution_cards`]) into
/// `original`, byte-exactly: a card whose key already exists is replaced in
/// place -- a re-solve overwrites the earlier solution rather than leaving
/// two conflicting `CRVAL1` cards where this project's own
/// `FitsHeader::get` would silently keep reading the stale first one.
///
/// The merge *policy* lives in exactly one place for the whole crate,
/// [`sidecar::merge_cards_by_key`] (see there for the full rules --
/// repeatable `HISTORY`/`COMMENT`, and the collapse of psolve's own
/// solve-marker `COMMENT` to exactly one). This function is only the
/// byte-exact `[u8; CARD]` binding of it: it supplies [`card_key`] and
/// [`card_text`] as the accessors, so every original card is carried through
/// untouched rather than round-tripped through `String`. The `.wcs` sidecar
/// writers call the same policy over text cards.
///
/// `END` is filtered out here rather than passed through: it is not part of
/// `original` and [`pack_header`] writes its own, exactly once.
fn merge_wcs_cards(original: Vec<[u8; CARD]>, w: &Wcs) -> Vec<[u8; CARD]> {
    let solution: Vec<[u8; CARD]> = sidecar::wcs_solution_cards(w)
        .iter()
        .map(|text| sidecar::pad_or_truncate_card(text))
        .filter(|bytes| card_key(bytes) != "END")
        .collect();
    sidecar::merge_cards_by_key(original, solution, card_key, card_text)
}

/// Pack `cards` plus a trailing `END` card into exactly `target_len` bytes
/// (always the *original* file's header length, in whole 2880-byte blocks),
/// padded with blank cards. Holding `target_len` fixed is the one invariant
/// that guarantees the data unit which follows never moves. If the cards do
/// not fit, nothing is written -- the number of 2880-byte blocks that would
/// have been needed is returned instead, for the caller to report. A result
/// that lands at *exactly* `target_len` (no slack at all) is accepted, not
/// treated as "too close": only strictly exceeding it is a refusal.
fn pack_header(cards: &[[u8; CARD]], target_len: usize) -> Result<Vec<u8>, usize> {
    let mut out = Vec::with_capacity(target_len.max((cards.len() + 1) * CARD));
    for c in cards {
        out.extend_from_slice(c);
    }
    out.extend_from_slice(&sidecar::pad_or_truncate_card("END"));
    if out.len() > target_len {
        return Err(out.len().div_ceil(BLOCK));
    }
    out.resize(target_len, b' ');
    Ok(out)
}

/// Write `bytes` to `tmp_path` durably: create, write, `sync_all`, then
/// drop (closing the fd) -- data blocks are confirmed on stable storage
/// before this returns, rather than only sitting in the page cache. See the
/// module doc, rule 3. `guard` is armed the moment `File::create` succeeds
/// -- from that point on the path exists and is this function's (and then
/// its caller's) responsibility to clean up on any later failure; it is
/// deliberately not armed before that, so a failed `create` never leaves
/// the guard trying to remove a path it never brought into existence.
fn write_temp_durably(tmp_path: &Path, bytes: &[u8], guard: &mut TempGuard) -> Result<(), FitsUpdateError> {
    let mut f = File::create(tmp_path)
        .map_err(|e| FitsUpdateError::Io(format!("creating {}: {e}", tmp_path.display())))?;
    guard.arm();
    f.write_all(bytes)
        .map_err(|e| FitsUpdateError::Io(format!("writing {}: {e}", tmp_path.display())))?;
    f.sync_all()
        .map_err(|e| FitsUpdateError::Io(format!("syncing {}: {e}", tmp_path.display())))?;
    Ok(())
}

/// `fsync` the directory containing `path`, so a preceding `rename` into or
/// within it is durable, not just visible (module doc, rule 3). This
/// function itself still reports a genuine failure as `Err` -- it is its
/// caller, `commit_new_file`, whose contract (module doc, rule 5) downgrades
/// that `Err` to a non-fatal stderr warning; this function makes no such
/// allowance on its own.
///
/// `pub(crate)`, not private: `tests/fits_update.rs` unit-tests this
/// function's own error-reporting directly, since `commit_new_file`'s
/// downgrade would otherwise make the `Err` path invisible from outside.
pub(crate) fn fsync_parent(path: &Path) -> Result<(), FitsUpdateError> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let d = File::open(dir)
        .map_err(|e| FitsUpdateError::Io(format!("opening directory {} to sync: {e}", dir.display())))?;
    d.sync_all()
        .map_err(|e| FitsUpdateError::Io(format!("syncing directory {}: {e}", dir.display())))?;
    Ok(())
}

/// Reread the freshly-written temp file with a fresh [`std::fs::read`]
/// (never trusted from the in-memory buffer that built it) and confirm both
/// halves of the hazard did not happen: the data unit starts at the same
/// offset, and every pixel byte is identical to the original's. See the
/// module doc, rule 4, for what this re-read does and does not prove.
fn verify_temp(
    tmp_path: &Path,
    expected_offset: usize,
    expected_pixels: &[u8],
) -> Result<(), FitsUpdateError> {
    let bytes = std::fs::read(tmp_path)
        .map_err(|e| FitsUpdateError::Io(format!("reading back {}: {e}", tmp_path.display())))?;
    let header = FitsHeader::parse(&bytes)
        .map_err(|e| FitsUpdateError::Verify(format!("rewritten file no longer parses as FITS: {e}")))?;
    if header.data_offset != expected_offset {
        return Err(FitsUpdateError::Verify(format!(
            "data unit moved: was at byte {expected_offset}, is now at byte {}",
            header.data_offset
        )));
    }
    if &bytes[header.data_offset..] != expected_pixels {
        return Err(FitsUpdateError::Verify("pixel bytes changed".to_string()));
    }
    Ok(())
}

/// A unique-per-call name in the *target's* directory -- same filesystem as
/// the target, which is what makes the final `rename` atomic. Every temp
/// file this module ever creates has `.psolve-tmp` in its name, which is
/// also what a caller (or a human) looks for to spot a stray one; a healthy
/// run never leaves one behind (see `tests/fits_update.rs`).
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

fn temp_path_beside(path: &Path) -> PathBuf {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("frame.fits");
    let n = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp_name = format!(".psolve-tmp-{}-{n}-{name}", std::process::id());
    path.with_file_name(tmp_name)
}

/// Holds a temp file path and removes it on [`Drop`], but only once
/// [`arm`] has been called -- and no longer, once [`disarm`] has been
/// called after that. Starts unarmed: constructing a `TempGuard` records
/// the path a temp file *will* have, before anything has necessarily been
/// created there, so an early `Drop` (e.g. `File::create` itself failing)
/// must not attempt to remove a path that was never brought into
/// existence. Replaces what were three hand-copied `let _ =
/// std::fs::remove_file(...)` calls, one per fallible step between "the
/// temp file exists" and "it has been renamed away" -- with this, a future
/// edit that adds a new fallible step in that window is cleaned up
/// automatically by `?`'s early return running the guard's destructor,
/// rather than depending on remembering to add a cleanup call at that
/// specific new point.
///
/// [`arm`]: TempGuard::arm
/// [`disarm`]: TempGuard::disarm
struct TempGuard {
    path: PathBuf,
    armed: bool,
}

impl TempGuard {
    fn new(path: PathBuf) -> Self {
        TempGuard { path, armed: false }
    }

    /// Call once the temp file is known to exist on disk (immediately
    /// after the `create` call that brings it into existence succeeds):
    /// from this point on, `Drop` will remove it unless [`disarm`] is
    /// called first.
    ///
    /// [`disarm`]: TempGuard::disarm
    fn arm(&mut self) {
        self.armed = true;
    }

    /// Call once the temp file has been renamed away (or otherwise no
    /// longer needs cleaning up): the guard's `Drop` becomes a no-op.
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for TempGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}
