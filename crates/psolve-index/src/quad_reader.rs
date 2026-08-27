//! Read-only, mmap-backed `.psqidx` access. Mirrors `reader.rs`'s shape
//! exactly (mmap the whole file once at open, decode the offset table into a
//! `Vec` up front, slice records out of the map on demand) for the same
//! reason: there is no write path here, by design (spec section 4), and a
//! `.psqidx` is exactly as untrusted on open as a `.psidx` is.
//!
//! Two things this reader enforces that nothing did before it existed:
//!
//! - **The `star_index_fingerprint` pairing.** `quad_format.rs`'s module doc
//!   ("Record layout: star-index references, not embedded positions")
//!   explains why a `.psqidx`'s `star_idx` references are meaningless
//!   without the exact `.psidx` they were built against, and why a silent
//!   mispairing produces confident garbage rather than an error. `open`
//!   therefore takes the paired `Index` as a required argument, not an
//!   optional one -- there is no way to construct a `QuadIndex` at all
//!   without a fingerprint check having already passed, so no caller
//!   (including a future lookup path) can accidentally skip it.
//! - **Offset/length cross-checks against the real file.** `QuadHeader`'s
//!   own parser (`quad_format.rs`) takes a bare byte slice and has no file
//!   length to check against; it can decode a header whose `records_offset`
//!   and `n_bands`/`n_quads` are individually well-formed but jointly
//!   describe a record region that does not fit in the file it came from.
//!   This module has that file-length context, so it is the one place that
//!   can and must reject such a file before any record is sliced out of the
//!   map -- same rationale `reader.rs`'s own `open` gives for its
//!   equivalent check.

use crate::blind_grid::{self, BandGrid};
use crate::error::IndexError;
use crate::quad_format::{QuadHeader, QuadRecord, QUAD_RECORD_BYTES};
use crate::reader::Index;
use crate::sha256::sha256;
use memmap2::Mmap;
use std::fs::File;
use std::path::Path;
use std::sync::OnceLock;

pub struct QuadIndex {
    map: Mmap,
    header: QuadHeader,
    /// n_bands+1 cumulative record offsets, decoded once at open.
    band_table: Vec<u64>,
    /// One lazily-built quantile grid per band, indexed by band number.
    /// `open()` only allocates the empty slots (cheap: `n_bands` is at most
    /// `MAX_BANDS` = 8); the actual grid for a band is built on that band's
    /// first `candidates()` call and cached from then on. See
    /// `blind_grid.rs`'s module doc for why a grid (not a kd-tree, the
    /// other candidate this was measured against) and why this is safe to
    /// build once and reuse rather than per lookup -- the whole reason it
    /// is NOT built eagerly here is that `info`/`doctor`-style callers that
    /// never call `candidates()` (e.g. `quad-index info`) should not pay a
    /// ~1-2s build cost they have no use for.
    grids: Vec<OnceLock<BandGrid>>,
}

impl QuadIndex {
    /// Opens `path` as a `.psqidx`, paired against `star_index` -- the
    /// already-open `.psidx` this file's `star_idx` references are meant to
    /// resolve against. Rejects, rather than silently accepting, any of:
    /// bad magic (including a real `.psidx` handed to this reader -- the two
    /// formats intentionally do not share one, see `quad_format.rs`'s module
    /// doc), an unsupported version, a `star_index_fingerprint` that does
    /// not match `star_index`, a `records_offset` inconsistent with
    /// `n_bands`, or a file too short to hold the band table and record
    /// region the header claims.
    pub fn open(path: &Path, star_index: &Index) -> Result<QuadIndex, IndexError> {
        let file = File::open(path)?;
        // SAFETY: the index is a read-only, immutable artifact. A concurrent
        // truncation would be UB; we accept that as we accept it for any mmap
        // (same rationale as `reader::Index::open`).
        let map = unsafe { Mmap::map(&file)? };
        let header = QuadHeader::from_bytes(&map)?;

        // The whole reason `star_index_fingerprint` exists: a `.psqidx`
        // built against one `.psidx` and opened here against a different
        // one would otherwise resolve every quad's star references into the
        // wrong stars, with no error at all. Checked before anything else
        // touches the record region.
        let mut expected_fingerprint = [0u8; 8];
        expected_fingerprint.copy_from_slice(&star_index.header().records_sha256[..8]);
        if header.star_index_fingerprint != expected_fingerprint {
            return Err(IndexError::FingerprintMismatch {
                expected: expected_fingerprint,
                found: header.star_index_fingerprint,
            });
        }

        // `records_offset` and `n_bands` are two independent header fields;
        // the builder always derives one from the other via
        // `records_offset_for`, but nothing on the read path enforced that
        // until now -- same defect class `reader::Index::open` fixed for
        // `.psidx`'s `records_offset`/`nside`. Because `records_offset_for`
        // always returns a `RECORD_ALIGN`-aligned value, this equality check
        // also rejects a misaligned `records_offset` outright: there is no
        // legitimate misaligned value it could otherwise match.
        let expected_records_offset = QuadHeader::records_offset_for(header.n_bands);
        if header.records_offset != expected_records_offset {
            return Err(IndexError::BadRange {
                what: "records_offset",
                reason: format!(
                    "expected {expected_records_offset} for n_bands {} (per records_offset_for), \
                     found {}",
                    header.n_bands, header.records_offset
                ),
            });
        }

        let tab_off = QuadHeader::band_table_offset();
        let tab_bytes = QuadHeader::band_table_bytes(header.n_bands);

        // The header is untrusted: `records_offset` and `n_quads` are two
        // independent fields, and a hostile or corrupt file can set them
        // inconsistently (e.g. a huge n_quads with a tiny file). Bound the
        // file length against BOTH the full band table and the full record
        // region before any slicing, using checked arithmetic so a
        // maliciously large n_quads can't wrap instead of erroring, and
        // can't be allocated for either -- this file is only ever mmap'd,
        // never copied into a fresh `Vec` sized off an untrusted count.
        let record_span = header
            .n_quads
            .checked_mul(QUAD_RECORD_BYTES as u64)
            .ok_or(IndexError::Truncated { expected: u64::MAX, actual: map.len() as u64 })?;
        let need_records = header
            .records_offset
            .checked_add(record_span)
            .ok_or(IndexError::Truncated { expected: u64::MAX, actual: map.len() as u64 })?;
        let need_table = tab_off + tab_bytes;
        let need = need_records.max(need_table);
        if (map.len() as u64) < need {
            return Err(IndexError::Truncated { expected: need, actual: map.len() as u64 });
        }

        let tab_off = tab_off as usize;
        let mut band_table = Vec::with_capacity((header.n_bands + 1) as usize);
        let mut prev = 0u64;
        for i in 0..(header.n_bands + 1) as usize {
            let s = tab_off + i * 8;
            // `need` already guarantees this range is in bounds, but this
            // still goes through `get` + `try_into` rather than direct
            // indexing: this is untrusted input and must never panic, even
            // if the bound above is ever loosened by a future edit.
            let bytes: [u8; 8] = map
                .get(s..s + 8)
                .and_then(|b| b.try_into().ok())
                .ok_or(IndexError::Truncated { expected: need_table, actual: map.len() as u64 })?;
            let v = u64::from_le_bytes(bytes);
            // The table must be non-decreasing: `band()`/`band_len()` slice
            // and subtract on these values without re-checking, so a
            // corrupt (but not merely truncated) file with a decreasing
            // entry would otherwise underflow `band_len`'s subtraction or
            // hand `band()` a start past its end.
            if v < prev {
                return Err(IndexError::BadRange {
                    what: "band table",
                    reason: format!("entry {i} ({v}) is less than entry {} ({prev})", i - 1),
                });
            }
            prev = v;
            band_table.push(v);
        }
        // The last entry is the running total after every band, so it must
        // equal n_quads. Without this, a band's end offset could exceed
        // n_quads and `band()` would slice past the record region (still
        // within `need`'s bound in the worst case, but reading quads that
        // were never meant to belong to that band).
        if prev != header.n_quads {
            return Err(IndexError::BadRange {
                what: "band table",
                reason: format!("final offset {prev} does not match n_quads {}", header.n_quads),
            });
        }

        let grids = (0..header.n_bands as usize).map(|_| OnceLock::new()).collect();
        Ok(QuadIndex { map, header, band_table, grids })
    }

    pub fn header(&self) -> &QuadHeader {
        &self.header
    }

    fn record_bytes(&self) -> &[u8] {
        let s = self.header.records_offset as usize;
        let e = s + self.header.n_quads as usize * QUAD_RECORD_BYTES;
        &self.map[s..e]
    }

    /// Recompute the record digest and compare. O(file size) -- for `info
    /// --verify`/`doctor`, not the solve path.
    pub fn verify_digest(&self) -> Result<(), IndexError> {
        if sha256(self.record_bytes()) == self.header.records_sha256 {
            Ok(())
        } else {
            Err(IndexError::ChecksumMismatch)
        }
    }

    /// Quads pushed to `band` at build time. `0` for a `band` at or past
    /// `n_bands` -- out of range is "no quads", not a panic, same contract
    /// `reader::Index::cell_len` uses for an out-of-range cell.
    pub fn band_len(&self, band: usize) -> usize {
        if band + 1 >= self.band_table.len() {
            return 0;
        }
        (self.band_table[band + 1] - self.band_table[band]) as usize
    }

    /// Raw bytes for a band's records.
    pub fn band(&self, band: usize) -> &[u8] {
        if band + 1 >= self.band_table.len() {
            return &[];
        }
        let (a, b) = (self.band_table[band], self.band_table[band + 1]);
        let base = self.header.records_offset as usize;
        &self.map[base + a as usize * QUAD_RECORD_BYTES..base + b as usize * QUAD_RECORD_BYTES]
    }

    pub fn quad(&self, band: usize, i: usize) -> Option<QuadRecord> {
        let b = self.band(band);
        let s = i * QUAD_RECORD_BYTES;
        b.get(s..s + QUAD_RECORD_BYTES)
            .and_then(|b| b.try_into().ok())
            .map(|b: &[u8; QUAD_RECORD_BYTES]| QuadRecord::from_bytes(b))
    }

    /// Every quad in `band` whose 4-vector code is within Euclidean `tol`
    /// of `code` -- the blind-solve lookup this whole module exists to
    /// support. Backed by a per-band quantile grid (`blind_grid.rs`; see
    /// its module doc for why a grid rather than a kd-tree, and for both
    /// prototypes' measured numbers), built on this band's first call and
    /// reused after that -- see `grids`' own doc for why that caching is
    /// where it is.
    ///
    /// The returned set is not merely a superset of what a brute-force
    /// Euclidean scan of the band would return: `blind_grid::filter_exact`
    /// re-checks every candidate against its own decoded code, so this is
    /// exactly that brute-force answer, just reached without scanning every
    /// record in the band. A `band` at or past `n_bands`, or an empty band,
    /// yields an empty iterator rather than panicking or building anything
    /// -- the same "out of range is no quads" contract `band_len`/`band`
    /// already use.
    pub fn candidates(&self, code: [f64; 4], tol: f64, band: usize) -> impl Iterator<Item = QuadRecord> {
        let n = self.band_len(band);
        let out = if n == 0 {
            Vec::new()
        } else {
            let grid = match self.grids.get(band) {
                Some(lock) => lock.get_or_init(|| BandGrid::build(self, band, n)),
                None => return Vec::new().into_iter(),
            };
            blind_grid::filter_exact(self, band, grid, code, tol)
        };
        out.into_iter()
    }
}
