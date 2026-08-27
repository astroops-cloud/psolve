//! On-disk layout:
//!   0..128                      Header
//!   128..128+8*(npix+1)         cell offset table (u64 LE, n+1 entries;
//!                               cell i occupies records [tab[i], tab[i+1]) )
//!   records_offset..            records, 16 bytes each, 4096-aligned so the
//!                               mmap'd region is page-aligned and castable

use crate::error::IndexError;

pub const MAGIC: [u8; 8] = *b"PSIDX\0\0\0";
pub const FORMAT_VERSION: u32 = 1;
pub const HEADER_BYTES: usize = 128;
pub const RECORD_ALIGN: u64 = 4096;

#[derive(Debug, Clone, PartialEq)]
pub struct Header {
    pub version: u32,
    pub nside: u32,
    pub epoch: f64,
    pub n_records: u64,
    pub mag_limit: f32,
    pub records_offset: u64,
    pub records_sha256: [u8; 32],
    pub name: [u8; 32],
}

impl Header {
    pub fn cell_table_offset() -> u64 {
        HEADER_BYTES as u64
    }

    /// npix + 1 entries, so every cell has an explicit end.
    pub fn cell_table_bytes(nside: u32) -> u64 {
        (crate::healpix::npix(nside) + 1) * 8
    }

    /// Where records begin: after the cell table, rounded up to RECORD_ALIGN.
    pub fn records_offset_for(nside: u32) -> u64 {
        let end = Self::cell_table_offset() + Self::cell_table_bytes(nside);
        end.div_ceil(RECORD_ALIGN) * RECORD_ALIGN
    }

    pub fn name_str(&self) -> &str {
        let end = self.name.iter().position(|&c| c == 0).unwrap_or(self.name.len());
        std::str::from_utf8(&self.name[..end]).unwrap_or("")
    }

    pub fn to_bytes(&self) -> [u8; HEADER_BYTES] {
        let mut b = [0u8; HEADER_BYTES];
        b[0..8].copy_from_slice(&MAGIC);
        b[8..12].copy_from_slice(&self.version.to_le_bytes());
        b[12..16].copy_from_slice(&self.nside.to_le_bytes());
        b[16..24].copy_from_slice(&self.epoch.to_le_bytes());
        b[24..32].copy_from_slice(&self.n_records.to_le_bytes());
        b[32..36].copy_from_slice(&self.mag_limit.to_le_bytes());
        b[36..44].copy_from_slice(&self.records_offset.to_le_bytes());
        b[44..76].copy_from_slice(&self.records_sha256);
        b[76..108].copy_from_slice(&self.name);
        b
    }

    pub fn from_bytes(b: &[u8]) -> Result<Header, IndexError> {
        if b.len() < HEADER_BYTES {
            return Err(IndexError::Truncated {
                expected: HEADER_BYTES as u64,
                actual: b.len() as u64,
            });
        }
        if b[0..8] != MAGIC {
            return Err(IndexError::BadMagic);
        }
        let version = u32::from_le_bytes([b[8], b[9], b[10], b[11]]);
        if version != FORMAT_VERSION {
            return Err(IndexError::UnsupportedVersion(version));
        }
        let nside = u32::from_le_bytes([b[12], b[13], b[14], b[15]]);
        if !crate::healpix::is_valid_nside(nside) {
            return Err(IndexError::BadNside(nside));
        }
        let mut sha = [0u8; 32];
        sha.copy_from_slice(&b[44..76]);
        let mut name = [0u8; 32];
        name.copy_from_slice(&b[76..108]);
        Ok(Header {
            version,
            nside,
            epoch: f64::from_le_bytes(b[16..24].try_into().unwrap_or([0; 8])),
            n_records: u64::from_le_bytes(b[24..32].try_into().unwrap_or([0; 8])),
            mag_limit: f32::from_le_bytes(b[32..36].try_into().unwrap_or([0; 4])),
            records_offset: u64::from_le_bytes(b[36..44].try_into().unwrap_or([0; 8])),
            records_sha256: sha,
            name,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Header {
        let mut name = [0u8; 32];
        name[..20].copy_from_slice(b"gaia-dr3-g14-nside64");
        Header {
            version: FORMAT_VERSION,
            nside: 64,
            epoch: 2016.0,
            n_records: 75_000_000,
            mag_limit: 14.0,
            records_offset: Header::records_offset_for(64),
            records_sha256: [7u8; 32],
            name,
        }
    }

    #[test]
    fn header_round_trips() {
        let h = sample();
        assert_eq!(Header::from_bytes(&h.to_bytes()).unwrap(), h);
    }

    #[test]
    fn header_is_exactly_128_bytes() {
        assert_eq!(sample().to_bytes().len(), 128);
    }

    #[test]
    fn rejects_bad_magic() {
        let mut b = sample().to_bytes();
        b[0] = b'X';
        assert!(matches!(Header::from_bytes(&b), Err(IndexError::BadMagic)));
    }

    #[test]
    fn rejects_future_version() {
        let mut h = sample();
        h.version = 99;
        let b = h.to_bytes();
        assert!(matches!(Header::from_bytes(&b), Err(IndexError::UnsupportedVersion(99))));
    }

    #[test]
    fn rejects_non_power_of_two_nside() {
        let mut h = sample();
        h.nside = 63;
        let b = h.to_bytes();
        assert!(matches!(Header::from_bytes(&b), Err(IndexError::BadNside(63))));
    }

    #[test]
    fn rejects_truncated_header() {
        let b = sample().to_bytes();
        assert!(matches!(
            Header::from_bytes(&b[..60]),
            Err(IndexError::Truncated { .. })
        ));
    }

    #[test]
    fn records_are_page_aligned() {
        for &nside in &[1u32, 64, 256, 4096] {
            assert_eq!(Header::records_offset_for(nside) % RECORD_ALIGN, 0);
            assert!(
                Header::records_offset_for(nside)
                    >= Header::cell_table_offset() + Header::cell_table_bytes(nside)
            );
        }
    }

    #[test]
    fn name_str_strips_nul_padding() {
        assert_eq!(sample().name_str(), "gaia-dr3-g14-nside64");
    }
}
