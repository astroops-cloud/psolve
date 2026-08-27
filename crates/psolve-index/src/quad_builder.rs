//! Builds a `.psqidx` quad index in memory, then writes it.
//!
//! Mirrors `builder.rs`'s shape for the star index -- an in-memory
//! accumulator is pushed to per record, then written in a single pass -- but
//! deliberately knows NOTHING about sky tiling, star selection, or the
//! geometric quad-forming machinery (`psolve_core::quad`) that produces the
//! codes it is handed. Those all live in `psolve-cli::cmd_quadindex`, the
//! only crate this milestone allows to depend on both `psolve-index` and
//! `psolve-core` (task brief: "psolve-index may use memmap2; psolve-cli may
//! use psolve-index, psolve-core, rayon. No new dependency."). This module's
//! job is purely the on-disk shape: banding, ordering, page alignment, and
//! the digest -- exactly the same division of labour `builder.rs` (star
//! rows in, cells out) already has with `cmd_index.rs` (CSV parsing) and
//! `gaia.rs` (column mapping).

use crate::error::IndexError;
use crate::quad_format::{
    QuadHeader, QuadRecord, FORMAT_VERSION, MAX_BANDS, NAME_BYTES, QUAD_RECORD_BYTES,
};
use crate::sha256::Sha256;
use std::io::{Seek, SeekFrom, Write};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct QuadBuildStats {
    pub written: u64,
    /// Count of `pack_code` clamps across every pushed quad -- see
    /// `quad_format.rs`'s doc for why this should be 0 for a genuine
    /// `quad_code` output and must be surfaced, not silently absorbed, if
    /// it ever isn't.
    pub clamped: u64,
}

pub struct QuadIndexBuilder {
    nside: u32,
    epoch: f64,
    mag_limit: f32,
    name: [u8; NAME_BYTES],
    star_index_fingerprint: [u8; 8],
    band_scales_millideg: [u16; MAX_BANDS],
    n_bands: u32,
    /// One `Vec` per configured band, in caller push order. Grouped by band
    /// at push time rather than sorted at `finish()`: unlike the star
    /// index's `Builder`, which cannot know a row's cell until the whole
    /// catalogue has been seen, the caller here already knows a quad's band
    /// before pushing it (its own diagonal decided that) -- see `push`'s
    /// doc for the determinism contract this places on the caller.
    bands: Vec<Vec<QuadRecord>>,
    stats: QuadBuildStats,
}

impl QuadIndexBuilder {
    /// `band_scales_deg` is the ordered list of band scales (degrees) this
    /// build covers -- e.g. the spec's six doubling bands, 0.25..8. Its
    /// length becomes `n_bands`; `push`'s `band` argument indexes into it.
    pub fn new(
        nside: u32,
        epoch: f64,
        mag_limit: f32,
        name: &str,
        star_index_fingerprint: [u8; 8],
        band_scales_deg: &[f32],
    ) -> Result<QuadIndexBuilder, IndexError> {
        if !crate::healpix::is_valid_nside(nside) {
            return Err(IndexError::BadNside(nside));
        }
        if band_scales_deg.is_empty() || band_scales_deg.len() > MAX_BANDS {
            return Err(IndexError::BadRange {
                what: "band_scales_deg",
                reason: format!(
                    "{} band(s) given; must be 1..={MAX_BANDS}",
                    band_scales_deg.len()
                ),
            });
        }

        let mut n = [0u8; NAME_BYTES];
        let src = name.as_bytes();
        let take = src.len().min(NAME_BYTES);
        n[..take].copy_from_slice(&src[..take]);

        // Milli-degree resolution mirrors `QuadHeader::band_scales_millideg`'s
        // own doc: exact for the spec's doubling bands, no rounding either
        // way. `u16::MAX` millideg is 65.535 deg, comfortably above the
        // spec's widest (8 deg) band.
        let mut band_scales_millideg = [0u16; MAX_BANDS];
        for (i, &deg) in band_scales_deg.iter().enumerate() {
            if !deg.is_finite() || deg <= 0.0 || deg > 65.535 {
                return Err(IndexError::BadRange {
                    what: "band scale",
                    reason: format!(
                        "{deg} deg must be finite, positive and <= 65.535 deg (u16 millideg range)"
                    ),
                });
            }
            band_scales_millideg[i] = (deg * 1000.0).round() as u16;
        }

        Ok(QuadIndexBuilder {
            nside,
            epoch,
            mag_limit,
            name: n,
            star_index_fingerprint,
            band_scales_millideg,
            n_bands: band_scales_deg.len() as u32,
            bands: vec![Vec::new(); band_scales_deg.len()],
            stats: QuadBuildStats::default(),
        })
    }

    /// Push one quad into `band` (an index into the `band_scales_deg` slice
    /// given to `new`). `code` is the raw `quad_code` output
    /// (pre-quantization); this calls `QuadRecord::new` (hence
    /// `quad_format::pack_code`) itself and counts a clamp, the same
    /// contract `record.rs::pack` established for the star index. Returns
    /// whether this particular quad clamped.
    ///
    /// **Determinism contract**: records accumulate in push order within
    /// their band, with no sort at `finish()` -- so push order literally IS
    /// output order. For two builds of the same input to be byte-identical
    /// regardless of thread count (the milestone's non-negotiable), the
    /// CALLER must push in a fixed, thread-count-independent sequence (e.g.
    /// tiles swept in a fixed order, parallel per-tile work collected back
    /// into that order before any push happens). This builder enforces
    /// nothing about that itself -- the same way `Vec::push` enforces
    /// nothing about the order its caller calls it in -- determinism is a
    /// property of the call sequence, not of this type.
    pub fn push(
        &mut self,
        band: usize,
        code: [f64; 4],
        star_idx: [u32; 4],
    ) -> Result<bool, IndexError> {
        let n_bands = self.bands.len();
        let slot = self.bands.get_mut(band).ok_or_else(|| IndexError::BadRange {
            what: "band",
            reason: format!("band {band} is out of range for {n_bands} configured band(s)"),
        })?;
        let (rec, clamped) = QuadRecord::new(code, star_idx);
        if clamped {
            self.stats.clamped += 1;
        }
        slot.push(rec);
        self.stats.written += 1;
        Ok(clamped)
    }

    pub fn stats(&self) -> QuadBuildStats {
        self.stats
    }

    /// Quads pushed to `band` so far. Lets a caller enforce a per-tile cap
    /// (e.g. "stop once this tile's band already holds 25") by asking the
    /// builder rather than keeping a duplicate running count of its own that
    /// could drift out of sync with what was actually pushed.
    pub fn band_len(&self, band: usize) -> usize {
        self.bands.get(band).map_or(0, Vec::len)
    }

    /// Write the header, band offset table, padding, and records, in that
    /// order -- the exact layout `quad_format.rs`'s module doc specifies.
    /// Bands are concatenated in band-index order, each band's own records
    /// in push order; nothing here reorders or sorts a caller's pushes, so
    /// this function's own determinism is exact given a deterministic
    /// caller (see `push`'s doc).
    pub fn finish<W: Write + Seek>(self, out: &mut W) -> Result<QuadBuildStats, IndexError> {
        let n_bands = self.n_bands;
        let records_offset = QuadHeader::records_offset_for(n_bands);

        // Band offset table: n_bands+1 cumulative offsets, band i occupies
        // [tab[i], tab[i+1]) -- the same shape the star index's cell table
        // uses, so every band (even an empty one) has an explicit end.
        let mut tab = vec![0u64; self.bands.len() + 1];
        for (i, band) in self.bands.iter().enumerate() {
            tab[i + 1] = tab[i] + band.len() as u64;
        }
        let n_quads = *tab.last().unwrap_or(&0);

        let mut records = Vec::with_capacity(n_quads as usize * QUAD_RECORD_BYTES);
        let mut digest = Sha256::new();
        for band in &self.bands {
            for rec in band {
                let b = rec.to_bytes();
                digest.update(&b);
                records.extend_from_slice(&b);
            }
        }

        let header = QuadHeader {
            version: FORMAT_VERSION,
            nside: self.nside,
            epoch: self.epoch,
            n_quads,
            n_bands,
            band_scales_millideg: self.band_scales_millideg,
            mag_limit: self.mag_limit,
            records_offset,
            records_sha256: digest.finalize(),
            star_index_fingerprint: self.star_index_fingerprint,
            name: self.name,
        };

        out.write_all(&header.to_bytes())?;
        for v in &tab {
            out.write_all(&v.to_le_bytes())?;
        }
        let written_so_far = QuadHeader::band_table_offset() + (tab.len() as u64) * 8;
        let pad = records_offset - written_so_far;
        out.write_all(&vec![0u8; pad as usize])?;
        out.write_all(&records)?;
        out.seek(SeekFrom::Start(0))?;

        Ok(self.stats)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fp() -> [u8; 8] {
        [1, 2, 3, 4, 5, 6, 7, 8]
    }

    #[test]
    fn new_rejects_bad_nside() {
        assert!(matches!(
            QuadIndexBuilder::new(63, 2016.0, 16.0, "x", fp(), &[0.25]),
            Err(IndexError::BadNside(63))
        ));
    }

    #[test]
    fn new_rejects_empty_or_over_capacity_bands() {
        assert!(QuadIndexBuilder::new(64, 2016.0, 16.0, "x", fp(), &[]).is_err());
        let too_many = vec![0.25f32; MAX_BANDS + 1];
        assert!(QuadIndexBuilder::new(64, 2016.0, 16.0, "x", fp(), &too_many).is_err());
    }

    #[test]
    fn new_rejects_a_band_scale_outside_u16_millideg_range() {
        assert!(QuadIndexBuilder::new(64, 2016.0, 16.0, "x", fp(), &[0.0]).is_err());
        assert!(QuadIndexBuilder::new(64, 2016.0, 16.0, "x", fp(), &[-1.0]).is_err());
        assert!(QuadIndexBuilder::new(64, 2016.0, 16.0, "x", fp(), &[100.0]).is_err());
        assert!(QuadIndexBuilder::new(64, 2016.0, 16.0, "x", fp(), &[f32::NAN]).is_err());
    }

    #[test]
    fn push_rejects_an_out_of_range_band() {
        let mut b = QuadIndexBuilder::new(64, 2016.0, 16.0, "x", fp(), &[0.25, 0.5]).unwrap();
        assert!(b.push(2, [0.0, 0.5, 0.5, 1.0], [0, 1, 2, 3]).is_err());
        assert!(b.push(0, [0.0, 0.5, 0.5, 1.0], [0, 1, 2, 3]).is_ok());
    }

    #[test]
    fn push_counts_clamps_without_dropping_the_record() {
        let mut b = QuadIndexBuilder::new(64, 2016.0, 16.0, "x", fp(), &[0.25]).unwrap();
        let clamped = b.push(0, [-5.0, 0.0, 0.0, 0.0], [0, 1, 2, 3]).unwrap();
        assert!(clamped);
        assert_eq!(b.stats().clamped, 1);
        assert_eq!(b.stats().written, 1);
        assert_eq!(b.band_len(0), 1, "a clamped quad is still stored, not discarded");
    }

    #[test]
    fn band_len_tracks_pushes_per_band_independently() {
        let mut b = QuadIndexBuilder::new(64, 2016.0, 16.0, "x", fp(), &[0.25, 0.5]).unwrap();
        b.push(0, [0.0, 0.5, 0.5, 1.0], [0, 1, 2, 3]).unwrap();
        b.push(0, [0.1, 0.4, 0.4, 0.9], [4, 5, 6, 7]).unwrap();
        b.push(1, [0.2, 0.3, 0.3, 0.8], [8, 9, 10, 11]).unwrap();
        assert_eq!(b.band_len(0), 2);
        assert_eq!(b.band_len(1), 1);
    }

    fn write_and_reopen(b: QuadIndexBuilder) -> (QuadBuildStats, Vec<u8>) {
        let mut buf = std::io::Cursor::new(Vec::new());
        let stats = b.finish(&mut buf).unwrap();
        (stats, buf.into_inner())
    }

    #[test]
    fn finish_writes_a_header_that_round_trips() {
        let mut b = QuadIndexBuilder::new(64, 2016.0, 16.0, "gaia-g16-b", fp(), &[0.25, 0.5, 1.0])
            .unwrap();
        b.push(0, [0.0, 0.5, 0.5, 1.0], [0, 1, 2, 3]).unwrap();
        b.push(2, [0.1, 0.4, 0.4, 0.9], [4, 5, 6, 7]).unwrap();
        let (stats, bytes) = write_and_reopen(b);
        assert_eq!(stats.written, 2);

        let h = QuadHeader::from_bytes(&bytes).unwrap();
        assert_eq!(h.nside, 64);
        assert_eq!(h.epoch, 2016.0);
        assert_eq!(h.n_quads, 2);
        assert_eq!(h.n_bands, 3);
        assert_eq!(h.band_scales_deg(), vec![0.25f32, 0.5, 1.0]);
        assert_eq!(h.star_index_fingerprint, fp());
        assert_eq!(h.name_str(), "gaia-g16-b");
    }

    #[test]
    fn finish_orders_records_by_band_then_push_order() {
        let mut b = QuadIndexBuilder::new(64, 2016.0, 16.0, "x", fp(), &[0.25, 0.5]).unwrap();
        // Push band 1 first, then band 0 -- output must still be band-order,
        // not push-order, across bands (push order only governs WITHIN a
        // band).
        b.push(1, [0.9, 0.9, 0.1, 0.1], [100, 101, 102, 103]).unwrap();
        b.push(0, [0.0, 0.5, 0.5, 1.0], [0, 1, 2, 3]).unwrap();
        b.push(0, [0.1, 0.4, 0.4, 0.9], [4, 5, 6, 7]).unwrap();
        let (_, bytes) = write_and_reopen(b);
        let h = QuadHeader::from_bytes(&bytes).unwrap();

        let base = h.records_offset as usize;
        let rec_at = |i: usize| {
            let s = base + i * QUAD_RECORD_BYTES;
            QuadRecord::from_bytes(bytes[s..s + QUAD_RECORD_BYTES].try_into().unwrap())
        };
        assert_eq!(rec_at(0).star_idx, [0, 1, 2, 3], "band 0's first push comes first");
        assert_eq!(rec_at(1).star_idx, [4, 5, 6, 7], "band 0's second push comes second");
        assert_eq!(rec_at(2).star_idx, [100, 101, 102, 103], "band 1 follows band 0 entirely");
    }

    #[test]
    fn finish_produces_a_verifiable_digest() {
        let mut b = QuadIndexBuilder::new(64, 2016.0, 16.0, "x", fp(), &[0.25]).unwrap();
        for i in 0..5u32 {
            b.push(0, [0.0, 0.1 * i as f64, 0.2, 0.9], [i, i + 1, i + 2, i + 3]).unwrap();
        }
        let (_, bytes) = write_and_reopen(b);
        let h = QuadHeader::from_bytes(&bytes).unwrap();
        let base = h.records_offset as usize;
        let region = &bytes[base..base + h.n_quads as usize * QUAD_RECORD_BYTES];
        assert_eq!(crate::sha256::sha256(region), h.records_sha256);
    }

    #[test]
    fn finish_on_an_empty_builder_produces_a_zero_quad_but_valid_file() {
        let b = QuadIndexBuilder::new(64, 2016.0, 16.0, "empty", fp(), &[0.25, 0.5]).unwrap();
        let (stats, bytes) = write_and_reopen(b);
        assert_eq!(stats.written, 0);
        let h = QuadHeader::from_bytes(&bytes).unwrap();
        assert_eq!(h.n_quads, 0);
        assert_eq!(h.records_sha256, crate::sha256::sha256(&[]));
    }

    #[test]
    fn identical_pushes_produce_byte_identical_output() {
        let build = || {
            let mut b =
                QuadIndexBuilder::new(64, 2016.0, 16.0, "repeat", fp(), &[0.25, 0.5, 1.0]).unwrap();
            for i in 0..30u32 {
                let band = (i % 3) as usize;
                let code = [
                    (i as f64 * 0.013) % 1.0,
                    (i as f64 * 0.027) % 1.0,
                    (i as f64 * 0.041) % 1.0,
                    (i as f64 * 0.059) % 1.0,
                ];
                b.push(band, code, [i, i + 1, i + 2, i + 3]).unwrap();
            }
            let (_, bytes) = write_and_reopen(b);
            bytes
        };
        assert_eq!(build(), build());
    }

    #[test]
    fn records_offset_is_page_aligned() {
        let b = QuadIndexBuilder::new(64, 2016.0, 16.0, "x", fp(), &[0.25, 0.5, 1.0, 2.0, 4.0, 8.0])
            .unwrap();
        let (_, bytes) = write_and_reopen(b);
        let h = QuadHeader::from_bytes(&bytes).unwrap();
        assert_eq!(h.records_offset % crate::quad_format::RECORD_ALIGN, 0);
    }
}
