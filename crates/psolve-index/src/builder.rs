//! Builds an index in memory, then writes it.
//!
//! The whole catalogue is sorted in RAM: a G<16 build is ~212M records
//! (~3.4 GB) against 128 GB available, so an external merge sort would be
//! complexity bought for no reason. If a future build does not fit, that is a
//! design change, not a tuning knob.

use crate::error::IndexError;
use crate::format::{Header, FORMAT_VERSION};
use crate::healpix::{ang2pix_nest, is_valid_nside, npix};
use crate::record::{pack, StarRecord, RECORD_BYTES};
use crate::sha256::Sha256;
use std::io::{Seek, SeekFrom, Write};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct BuildStats {
    pub written: u64,
    pub clamped: u64,
    pub skipped: u64,
}

pub struct Builder {
    nside: u32,
    mag_limit: f32,
    epoch: f64,
    name: [u8; 32],
    /// (cell, record) pairs, sorted at finish().
    rows: Vec<(u64, StarRecord)>,
    stats: BuildStats,
}

impl Builder {
    pub fn new(nside: u32, mag_limit: f32, epoch: f64, name: &str) -> Result<Builder, IndexError> {
        if !is_valid_nside(nside) {
            return Err(IndexError::BadNside(nside));
        }
        let mut n = [0u8; 32];
        let src = name.as_bytes();
        let take = src.len().min(32);
        n[..take].copy_from_slice(&src[..take]);
        Ok(Builder {
            nside,
            mag_limit,
            epoch,
            name: n,
            rows: Vec::new(),
            stats: BuildStats::default(),
        })
    }

    pub fn push(&mut self, ra_deg: f64, dec_deg: f64, mag: f32, pmra: f32, pmdec: f32) {
        // Position and magnitude are required: a non-finite value there means
        // the row is useless and must be skipped. Proper motion is
        // deliberately NOT part of this guard: Gaia DR3 has ~340M
        // two-parameter sources (a real position, no PM solution, published
        // as NaN). Skipping those would silently discard about a fifth of
        // the catalogue. `pack`/`clamp_pm` already does the right thing with
        // a non-finite PM (NaN -> 0, unflagged; infinite -> clamped and
        // counted), so let it through.
        if !ra_deg.is_finite() || !dec_deg.is_finite() || !mag.is_finite() {
            self.stats.skipped += 1;
            return;
        }
        let cell = ang2pix_nest(self.nside, ra_deg, dec_deg);
        let (rec, clamped) = pack(ra_deg, dec_deg, mag, pmra, pmdec);
        if clamped {
            self.stats.clamped += 1;
        }
        self.rows.push((cell, rec));
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn finish<W: Write + Seek>(mut self, out: &mut W) -> Result<BuildStats, IndexError> {
        // Sort by cell, then by magnitude ascending: brightest first within a cell.
        // This ordering IS the format's contract; every reader depends on it.
        self.rows.sort_unstable_by(|a, b| {
            a.0.cmp(&b.0).then(a.1.mag_milli.cmp(&b.1.mag_milli))
        });

        let n = self.rows.len() as u64;
        let npix = npix(self.nside);
        let records_offset = Header::records_offset_for(self.nside);

        // Cell table: npix+1 offsets, so cell i is [tab[i], tab[i+1]).
        let mut tab = vec![0u64; (npix + 1) as usize];
        for (cell, _) in &self.rows {
            tab[(*cell + 1) as usize] += 1;
        }
        for i in 1..tab.len() {
            tab[i] += tab[i - 1];
        }

        // Records, contiguous and already in final order.
        let mut records = Vec::with_capacity(self.rows.len() * RECORD_BYTES);
        let mut digest = Sha256::new();
        for (_, rec) in &self.rows {
            let b = rec.to_bytes();
            digest.update(&b);
            records.extend_from_slice(&b);
        }

        let header = Header {
            version: FORMAT_VERSION,
            nside: self.nside,
            epoch: self.epoch,
            n_records: n,
            mag_limit: self.mag_limit,
            records_offset,
            records_sha256: digest.finalize(),
            name: self.name,
        };

        out.write_all(&header.to_bytes())?;
        for v in &tab {
            out.write_all(&v.to_le_bytes())?;
        }
        let written_so_far = Header::cell_table_offset() + (tab.len() as u64) * 8;
        let pad = records_offset - written_so_far;
        out.write_all(&vec![0u8; pad as usize])?;
        out.write_all(&records)?;
        out.seek(SeekFrom::Start(0))?;

        self.stats.written = n;
        Ok(self.stats)
    }
}
