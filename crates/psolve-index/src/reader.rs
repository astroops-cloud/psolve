//! Read-only, mmap-backed index access.
//!
//! There is no write path in this module, by design: psolve-core must not be
//! able to modify anything on disk (spec section 4).

use crate::error::IndexError;
use crate::format::Header;
use crate::healpix::{cells_in_disc, npix};
use crate::record::{StarRecord, RECORD_BYTES};
use crate::sha256::sha256;
use memmap2::Mmap;
use std::fs::File;
use std::path::Path;

pub struct Index {
    map: Mmap,
    header: Header,
    /// npix+1 cumulative record offsets, decoded once at open.
    cell_table: Vec<u64>,
}

impl Index {
    pub fn open(path: &Path) -> Result<Index, IndexError> {
        let file = File::open(path)?;
        // SAFETY: the index is a read-only, immutable artifact. A concurrent
        // truncation would be UB; we accept that as we accept it for any mmap.
        let map = unsafe { Mmap::map(&file)? };
        let header = Header::from_bytes(&map)?;

        // `records_offset` and `nside` are two independent header fields; the
        // builder always derives one from the other via `records_offset_for`,
        // but nothing on the read path enforced that until now. Without this
        // check a corrupt or hand-edited header could point `records_offset`
        // partway into the cell table (or anywhere else in the file) and
        // every `cell()`/`star()` slice below would silently read the wrong
        // bytes instead of failing to open.
        let expected_records_offset = Header::records_offset_for(header.nside);
        if header.records_offset != expected_records_offset {
            return Err(IndexError::BadRange {
                what: "records_offset",
                reason: format!(
                    "expected {expected_records_offset} for nside {} (per records_offset_for), \
                     found {}",
                    header.nside, header.records_offset
                ),
            });
        }

        let npix = npix(header.nside);
        let tab_off = Header::cell_table_offset();
        let tab_bytes = Header::cell_table_bytes(header.nside);

        // The header is untrusted: `records_offset` and `nside` are two
        // independent fields and a hostile or corrupt file can set them
        // inconsistently (e.g. a huge nside with a tiny records_offset).
        // Bound the file length against BOTH the full cell table and the
        // full record region before any slicing, using checked arithmetic
        // so a maliciously large n_records can't wrap instead of erroring.
        let record_span = header
            .n_records
            .checked_mul(RECORD_BYTES as u64)
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
        let mut cell_table = Vec::with_capacity((npix + 1) as usize);
        let mut prev = 0u64;
        for i in 0..(npix + 1) as usize {
            let s = tab_off + i * 8;
            // `need` already guarantees this range is in bounds, but we
            // still go through `get` + `try_into` rather than direct
            // indexing: this is untrusted input and must never panic, even
            // if the bound above is ever loosened by a future edit.
            let bytes: [u8; 8] = map
                .get(s..s + 8)
                .and_then(|b| b.try_into().ok())
                .ok_or(IndexError::Truncated { expected: need_table, actual: map.len() as u64 })?;
            let v = u64::from_le_bytes(bytes);
            // The table must be non-decreasing: `cell()`/`cell_len()` slice
            // and subtract on these values without re-checking, so a
            // corrupt (but not merely truncated) file with a decreasing
            // entry would otherwise underflow `cell_len`'s subtraction or
            // hand `cell()` a start past its end.
            if v < prev {
                return Err(IndexError::BadRange {
                    what: "cell table",
                    reason: format!("entry {i} ({v}) is less than entry {} ({prev})", i - 1),
                });
            }
            prev = v;
            cell_table.push(v);
        }
        // The last entry is the running total after every cell, so it must
        // equal the record count. Without this, a cell's end offset could
        // exceed n_records and `cell()` would slice past the record region
        // (still within `need`'s bound in the worst case attackers can
        // construct, but reading records that were never meant to belong to
        // that cell).
        if prev != header.n_records {
            return Err(IndexError::BadRange {
                what: "cell table",
                reason: format!("final offset {prev} does not match n_records {}", header.n_records),
            });
        }

        Ok(Index { map, header, cell_table })
    }

    pub fn header(&self) -> &Header {
        &self.header
    }

    fn record_bytes(&self) -> &[u8] {
        let s = self.header.records_offset as usize;
        let e = s + self.header.n_records as usize * RECORD_BYTES;
        &self.map[s..e]
    }

    /// Recompute the record digest and compare. O(file size) — for `doctor`,
    /// not for the solve path.
    pub fn verify_digest(&self) -> Result<(), IndexError> {
        if sha256(self.record_bytes()) == self.header.records_sha256 {
            Ok(())
        } else {
            Err(IndexError::ChecksumMismatch)
        }
    }

    /// Resolve a global record index (as stored in a paired `.psqidx`
    /// quad's `star_idx`, or returned by `brightest_in_disc_indexed`) back
    /// to the `StarRecord` at that position in the flat record array.
    /// `None` if out of range -- a `.psqidx` reader (Task 3) hands this
    /// untrusted values from a file it did not write itself, so this must
    /// reject rather than slice out of bounds.
    pub fn star_at(&self, global_idx: u32) -> Option<StarRecord> {
        if (global_idx as u64) >= self.header.n_records {
            return None;
        }
        let s = self.header.records_offset as usize + global_idx as usize * RECORD_BYTES;
        self.map.get(s..s + RECORD_BYTES).and_then(|b| b.try_into().ok()).map(StarRecord::from_bytes)
    }

    pub fn cell_len(&self, cell: u64) -> usize {
        if cell + 1 >= self.cell_table.len() as u64 {
            return 0;
        }
        (self.cell_table[cell as usize + 1] - self.cell_table[cell as usize]) as usize
    }

    /// Raw bytes for a cell's records, brightest-first.
    pub fn cell(&self, cell: u64) -> &[u8] {
        if cell + 1 >= self.cell_table.len() as u64 {
            return &[];
        }
        let (a, b) = (self.cell_table[cell as usize], self.cell_table[cell as usize + 1]);
        let base = self.header.records_offset as usize;
        &self.map[base + a as usize * RECORD_BYTES..base + b as usize * RECORD_BYTES]
    }

    pub fn star(&self, cell: u64, i: usize) -> Option<StarRecord> {
        let c = self.cell(cell);
        let s = i * RECORD_BYTES;
        c.get(s..s + RECORD_BYTES)
            .and_then(|b| b.try_into().ok())
            .map(|b: &[u8; RECORD_BYTES]| StarRecord::from_bytes(b))
    }

    /// The `limit` brightest stars within `radius_deg` of the given position.
    ///
    /// Each cell is already magnitude-sorted, so this is a k-way merge over a
    /// handful of sorted runs -- not a sort of the whole neighbourhood. It
    /// reads only as far into each run as it needs to.
    ///
    /// `cells_in_disc` pads its search by `max_pixrad_deg` so a cell that
    /// merely *overlaps* the disc is never missed -- at nside 64 that's
    /// ~1.03 deg. That means the candidate cells can contain stars well
    /// outside `radius_deg`. Those are skipped here by actual angular
    /// separation before being admitted to the merge: without this, an
    /// out-of-field star from a padded cell could be brighter than genuine
    /// in-field stars and, being merged in mag order, displace them once the
    /// result is truncated at `limit`.
    pub fn brightest_in_disc(
        &self,
        ra_deg: f64,
        dec_deg: f64,
        radius_deg: f64,
        limit: usize,
    ) -> Vec<StarRecord> {
        let cells = cells_in_disc(self.header.nside, ra_deg, dec_deg, radius_deg);
        // cursor per cell
        let mut cursors: Vec<(u64, usize)> = cells.iter().map(|&c| (c, 0usize)).collect();
        let mut out = Vec::with_capacity(limit.min(1024));

        while out.len() < limit {
            let mut best: Option<(usize, StarRecord)> = None;
            for (ci, (cell, pos)) in cursors.iter_mut().enumerate() {
                // Permanently skip stars this cursor points at that fall
                // outside the true radius -- they must never be selected, and
                // re-checking them on every outer-loop pass would be wasted
                // work.
                let mut rec = self.star(*cell, *pos);
                while let Some(r) = rec {
                    if angsep_deg(ra_deg, dec_deg, r.ra_deg(), r.dec_deg()) <= radius_deg {
                        break;
                    }
                    *pos += 1;
                    rec = self.star(*cell, *pos);
                }
                if let Some(r) = rec {
                    if best.is_none_or(|(_, b)| r.mag_milli < b.mag_milli) {
                        best = Some((ci, r));
                    }
                }
            }
            match best {
                None => break,
                Some((ci, rec)) => {
                    cursors[ci].1 += 1;
                    out.push(rec);
                }
            }
        }
        out
    }

    /// Every star within `radius_deg` of the given position with magnitude
    /// `<= max_mag` -- no brightest-N truncation.
    ///
    /// `brightest_in_disc` earns its k-way merge because it usually wants far
    /// fewer stars than a disc holds. When the caller wants everything (a
    /// completeness measure, not a solve catalogue), that merge is wasted
    /// work: it re-scans every cursor on every step just to pick the next
    /// brightest, and it has no exit early. This instead walks each
    /// candidate cell once, in cell order, and keeps whatever passes both
    /// filters.
    ///
    /// Reuses `cells_in_disc` for the candidate cell set (padded so a cell
    /// that merely overlaps the disc is never missed) and `angsep_deg` for
    /// the true separation cut, same as `brightest_in_disc` -- see that
    /// method's doc for why the true-separation check matters even though
    /// the candidate cells are already disc-adjacent.
    ///
    /// Emission order is deterministic -- cell order (as returned by
    /// `cells_in_disc`), then within-cell record order -- but it is NOT
    /// magnitude order: cells are magnitude-sorted internally but are not
    /// merged here, so a caller that wants brightest-first must sort the
    /// result itself. A consumer that only needs a stable, repeatable order
    /// (e.g. for diffing two runs) can rely on this ordering as-is.
    pub fn stars_in_disc(
        &self,
        ra_deg: f64,
        dec_deg: f64,
        radius_deg: f64,
        max_mag: f32,
    ) -> Vec<StarRecord> {
        let cells = cells_in_disc(self.header.nside, ra_deg, dec_deg, radius_deg);
        let mut out = Vec::new();
        for cell in cells {
            let mut i = 0;
            while let Some(r) = self.star(cell, i) {
                if r.mag() <= max_mag
                    && angsep_deg(ra_deg, dec_deg, r.ra_deg(), r.dec_deg()) <= radius_deg
                {
                    out.push(r);
                }
                i += 1;
            }
        }
        out
    }

    /// Same contract as `brightest_in_disc`, but each returned star also
    /// carries its own global index into this index's flat record array --
    /// the exact `u32` a `.psqidx` quad record needs to reference a star
    /// without embedding its position (`quad_format.rs`'s module doc,
    /// "Record layout: star-index references, not embedded positions").
    ///
    /// `.psqidx`'s builder (`psolve-cli::cmd_quadindex`, Task 2) is the only
    /// intended caller. The hinted solve path has no use for a raw array
    /// offset and keeps calling `brightest_in_disc` exactly as it always
    /// has -- that method's body is untouched by this addition, so nothing
    /// about the existing hinted path changes.
    ///
    /// The global index is `cell_table[cell] + local_position`: `cell()`'s
    /// own doc establishes that a cell's records occupy the contiguous span
    /// `[cell_table[cell], cell_table[cell+1])` of the single flat record
    /// array, so a cursor's `(cell, pos)` recovers that array offset
    /// directly, with no extra lookup. Converting to `u32` is checked, not
    /// cast: `quad_format.rs`'s own doc establishes this is exact (not
    /// lossy) for any build this project produces, but a checked
    /// conversion costs nothing and turns a hypothetical future
    /// multi-billion-record index into a skipped star instead of silently
    /// wrapped, wrong reference.
    pub fn brightest_in_disc_indexed(
        &self,
        ra_deg: f64,
        dec_deg: f64,
        radius_deg: f64,
        limit: usize,
    ) -> Vec<(u32, StarRecord)> {
        let cells = cells_in_disc(self.header.nside, ra_deg, dec_deg, radius_deg);
        let mut cursors: Vec<(u64, usize)> = cells.iter().map(|&c| (c, 0usize)).collect();
        let mut out = Vec::with_capacity(limit.min(1024));

        while out.len() < limit {
            let mut best: Option<(usize, u32, StarRecord)> = None;
            for (ci, (cell, pos)) in cursors.iter_mut().enumerate() {
                let mut rec = self.star(*cell, *pos);
                while let Some(r) = rec {
                    if angsep_deg(ra_deg, dec_deg, r.ra_deg(), r.dec_deg()) <= radius_deg {
                        break;
                    }
                    *pos += 1;
                    rec = self.star(*cell, *pos);
                }
                if let Some(r) = rec {
                    let Some(&global) = self.cell_table.get(*cell as usize) else {
                        // cell_table always has an entry for every cell in
                        // `cells_in_disc`'s output (both are bounded by the
                        // same validated `npix(nside)`), so this is
                        // unreachable in practice -- but a candidate cell
                        // that somehow falls outside a corrupt cell table is
                        // untrusted-input territory, not a panic.
                        continue;
                    };
                    let Ok(global_idx) = u32::try_from(global + *pos as u64) else {
                        continue;
                    };
                    if best.is_none_or(|(_, _, b)| r.mag_milli < b.mag_milli) {
                        best = Some((ci, global_idx, r));
                    }
                }
            }
            match best {
                None => break,
                Some((ci, global_idx, rec)) => {
                    cursors[ci].1 += 1;
                    out.push((global_idx, rec));
                }
            }
        }
        out
    }

    /// Up to `limit` stars within `radius_deg` of the given position, spread
    /// across the disc's HEALPix cells rather than drawn from the brightest
    /// overall.
    ///
    /// Brightness is spatially correlated -- a globular cluster's core alone
    /// can out-populate `limit` with stars from a few arcminutes, starving
    /// the rest of the field the same way Task 1 found on the image side
    /// (see `stratified_keep` in psolve-core). Round-robining across cells
    /// instead means a dense cell can dominate only up to its own share of
    /// the budget; once every cell has contributed what it has, sparser
    /// cells keep going.
    ///
    /// Reuses `cells_in_disc` for the candidate cell set and `angsep_deg` for
    /// the true separation cut, same as `brightest_in_disc` -- a cell that
    /// merely overlaps the disc still holds stars outside it, and cell
    /// membership alone is not a radius filter. A cursor advances past those
    /// permanently, exactly as `brightest_in_disc` does.
    ///
    /// The round-robin itself does not produce magnitude order -- it takes
    /// one star per cell per pass, so a later pass's star from a sparse cell
    /// can be brighter than an earlier pass's star from a dense one. Callers
    /// assume brightest-first (same contract as `brightest_in_disc`), so the
    /// result is sorted before returning.
    pub fn stratified_in_disc(
        &self,
        ra_deg: f64,
        dec_deg: f64,
        radius_deg: f64,
        limit: usize,
    ) -> Vec<StarRecord> {
        let cells = cells_in_disc(self.header.nside, ra_deg, dec_deg, radius_deg);
        let mut cursors: Vec<(u64, usize)> = cells.iter().map(|&c| (c, 0usize)).collect();
        let mut out = Vec::with_capacity(limit.min(1024));

        'outer: while out.len() < limit {
            let mut progressed = false;
            for (cell, pos) in cursors.iter_mut() {
                // Permanently skip stars this cursor points at that fall
                // outside the true radius, same rationale as
                // `brightest_in_disc`: cell membership is only a candidate
                // filter, and re-checking on every pass would be wasted work.
                let mut rec = self.star(*cell, *pos);
                while let Some(r) = rec {
                    if angsep_deg(ra_deg, dec_deg, r.ra_deg(), r.dec_deg()) <= radius_deg {
                        break;
                    }
                    *pos += 1;
                    rec = self.star(*cell, *pos);
                }
                if let Some(r) = rec {
                    *pos += 1;
                    out.push(r);
                    progressed = true;
                    if out.len() >= limit {
                        break 'outer;
                    }
                }
            }
            if !progressed {
                break;
            }
        }

        out.sort_unstable_by_key(|r| r.mag_milli);
        out
    }
}

/// Great-circle separation between two ra/dec points, in degrees. Haversine
/// form: stable for both very small and near-antipodal separations, unlike
/// the naive spherical law of cosines.
fn angsep_deg(ra1_deg: f64, dec1_deg: f64, ra2_deg: f64, dec2_deg: f64) -> f64 {
    let (r1, d1, r2, d2) =
        (ra1_deg.to_radians(), dec1_deg.to_radians(), ra2_deg.to_radians(), dec2_deg.to_radians());
    let (dr, dd) = (r2 - r1, d2 - d1);
    let a = (dd / 2.0).sin().powi(2) + d1.cos() * d2.cos() * (dr / 2.0).sin().powi(2);
    2.0 * a.sqrt().min(1.0).asin().to_degrees()
}
