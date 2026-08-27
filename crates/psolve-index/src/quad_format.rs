//! The `.psqidx` blind-solve quad index: precomputed geometric quad codes
//! for the whole sky, banded by angular scale, looked up by 4-vector code
//! rather than by sky position.
//!
//! This is a SEPARATE file format from `.psidx` (see `format.rs`), not an
//! extension of it -- the star index is load-bearing for the working hinted
//! solve path and must not churn (`2026-08-15-blind-solve-design.md` section
//! 3, item 3: "Do not extend `.psidx`'s format"). The two formats share only
//! a common *shape* -- fixed header, a trailing offset table, page-aligned
//! records, a SHA-256 digest over the record region -- and this crate's
//! `sha256.rs`. They do not share a magic, a version counter, or any code
//! that would couple their evolution: a `.psqidx` reader must reject a real
//! `.psidx` file (and vice versa) outright, not misparse it.
//!
//! On-disk layout:
//!   0..128                        Header
//!   128..128+8*(n_bands+1)        band offset table (u64 LE, n_bands+1
//!                                 entries; band i occupies records
//!                                 [tab[i], tab[i+1]) )
//!   records_offset..              records, QUAD_RECORD_BYTES each,
//!                                 RECORD_ALIGN-aligned so the mmap'd region
//!                                 is page-aligned and castable
//!
//! ## Record layout: star-index references, not embedded positions
//!
//! A `QuadRecord` must carry the 4-vector code and let a reader recover the
//! four stars' sky positions. Two designs were on the table:
//!
//! - **Embed all four stars' ra/dec directly.** Self-contained -- a
//!   `.psqidx` would need no companion file to be meaningful -- but even
//!   `.psidx`'s own compact position encoding (`record.rs`'s
//!   `ra_scaled`+`dec_scaled`, 8 bytes/star) costs 32 bytes/quad for
//!   positions alone, before the code. At the spike's measured ~18.67M
//!   quads that is >600 MB for positions alone, blowing the spike's ~448 MB
//!   budget by itself.
//! - **Store indices into the paired `.psidx`'s flat record array.** A
//!   `.psidx`'s records are one contiguous, magnitude-sorted array --
//!   `builder.rs` sorts once and writes once; `reader.rs`'s cell table is
//!   only a set of offsets into that same array -- so a single `u32` global
//!   index is enough to locate a star's full `StarRecord` (position,
//!   magnitude, proper motion) in the companion file. `.psidx` builds top
//!   out in the low hundreds of millions of records (the G<=16 build this
//!   milestone targets is ~212M, per the spec), comfortably inside
//!   `u32::MAX` (~4.29B) -- so a `u32` reference is exact, not a lossy
//!   approximation.
//!
//! **Chosen: indices into the paired `.psidx`.** It is what makes the
//! spike's 24-bytes/quad estimate achievable at all: 4 stars x 4 bytes
//! (`u32` index) + 4 code components x 2 bytes (`u16`, see below) = 24
//! bytes exactly (`QUAD_RECORD_BYTES`). The embedded-position alternative
//! is not a smaller variant of the same idea -- it is >2x the size before
//! the code is even counted -- so it was not a close call.
//!
//! The coupling this creates is real and is not left implicit: a `.psqidx`
//! built against one `.psidx` and then paired at read time with a
//! *different* `.psidx` (rebuilt, re-sorted, or a different depth) would
//! resolve its indices into the wrong stars and produce confident garbage,
//! not an error. `QuadHeader::star_index_fingerprint` carries the first 8
//! bytes of the paired `.psidx`'s own `records_sha256` (`format::Header`).
//! A reader (Task 3) MUST compare this against the `.psidx` it actually
//! opened and refuse the pairing on mismatch. Eight bytes is deliberately
//! not the full 32-byte digest: this field is a tripwire against
//! *accidental* mispairing -- the failure mode that actually occurs (an
//! operator swaps in a rebuilt `.psidx` and forgets the paired `.psqidx`)
//! -- not a cryptographic proof of content, which is already `.psidx`'s own
//! header's job for its own file. The birthday bound on 64 bits (~2^32
//! distinct builds before an accidental collision becomes likely) is
//! already far more than this milestone will ever build.
//!
//! ## Code quantization
//!
//! `quad_code` (`psolve_core::quad`) returns `[x_C, y_C, x_D, y_D]`. By
//! construction -- A and B are the most widely separated pair among the
//! four points, so C and D lie inside the circle on the A-B diameter -- its
//! own test suite confirms every component lands in `[-0.5, 1.5]`
//! (`quad.rs::inner_points_land_inside_the_unit_frame`). Each component is
//! stored here as a `u16` over `[CODE_MIN, CODE_MAX]` = `[-1.0, 2.0]`, a
//! margin outside that proven range so the canonicalization's own `1e-15`
//! comparison tolerances can never push a legitimate value into a clamp.
//! Quantization step is `(2.0 - (-1.0)) / 65535 ~= 4.58e-5` -- about three
//! orders of magnitude finer than any code-space match tolerance this
//! milestone is expected to use (Task 4). The code only has to be good
//! enough to find the right bucket: once a candidate is found, verification
//! (Task 5/6) checks the *actual* star positions (resolved via the indices
//! above), not the quantized code.

use crate::error::IndexError;

pub const MAGIC: [u8; 8] = *b"PSQIDX\0\0";
pub const FORMAT_VERSION: u32 = 1;
pub const HEADER_BYTES: usize = 128;
pub const RECORD_ALIGN: u64 = 4096;

/// Fixed capacity for the header's inline band-scale table. The spec's six
/// doubling bands (0.25 deg .. 8 deg) use 6 of these; the rest is headroom
/// for a shallower or deeper sweep without a format version bump. `n_bands`
/// says how many of the `MAX_BANDS` slots are meaningful.
pub const MAX_BANDS: usize = 8;

/// Public (not just crate-internal) because `quad_builder.rs` -- a sibling
/// module, not a descendant, so a private `const` here would not be visible
/// to it -- needs the exact width to size the `name` array it hands to
/// `QuadHeader` without duplicating the literal `24` and risking the two
/// silently drifting apart.
pub const NAME_BYTES: usize = 24;

/// Quad code component range as stored on disk. See the module doc's "Code
/// quantization" section.
pub const CODE_MIN: f64 = -1.0;
pub const CODE_MAX: f64 = 2.0;
const CODE_SCALE: f64 = u16::MAX as f64 / (CODE_MAX - CODE_MIN);

/// 4 code components x 2 bytes (`u16`) + 4 star references x 4 bytes
/// (`u32`) = 24 bytes -- exactly the spike's assumed per-quad size. See the
/// module doc's record-layout section for why this is achievable only with
/// index references, not embedded positions.
pub const QUAD_RECORD_BYTES: usize = 24;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QuadHeader {
    pub version: u32,
    /// HEALPix nside used to lay out per-band sky tiles at build time.
    /// Provenance only: unlike `.psidx`'s `nside`, this does not size
    /// anything in *this* file's layout -- `n_bands` sizes the band table
    /// -- so a reader never needs it to slice safely. Still validated as a
    /// legal HEALPix nside on read, on the general principle that a
    /// malformed file is untrusted input even where the field isn't
    /// safety-load-bearing.
    pub nside: u32,
    pub epoch: f64,
    pub n_quads: u64,
    pub n_bands: u32,
    /// Band scale (roughly, the tile diagonal quads in that band are drawn
    /// from) in milli-degrees. Only the first `n_bands` entries are
    /// meaningful. Milli-degree (0.001 deg = 3.6") resolution represents
    /// the spec's doubling bands (250, 500, 1000, 2000, 4000, 8000)
    /// exactly, with no rounding on either write or read.
    pub band_scales_millideg: [u16; MAX_BANDS],
    /// The source catalogue's magnitude limit, e.g. 16.0 for the G<=16
    /// build the spike recommends.
    pub mag_limit: f32,
    pub records_offset: u64,
    /// SHA-256 over the record region only (not the header, not the band
    /// table).
    pub records_sha256: [u8; 32],
    /// First 8 bytes of the paired `.psidx`'s `records_sha256`. See the
    /// module doc's record-layout section for why this exists and why 8
    /// bytes is enough for its purpose.
    pub star_index_fingerprint: [u8; 8],
    pub name: [u8; NAME_BYTES],
}

impl QuadHeader {
    pub fn band_table_offset() -> u64 {
        HEADER_BYTES as u64
    }

    /// n_bands + 1 entries, so every band has an explicit end -- the same
    /// shape as `.psidx`'s cell table.
    pub fn band_table_bytes(n_bands: u32) -> u64 {
        (n_bands as u64 + 1) * 8
    }

    /// Where records begin: after the band table, rounded up to
    /// RECORD_ALIGN. Mirrors `format::Header::records_offset_for`, keyed on
    /// `n_bands` here rather than `nside` since the band table (not a cell
    /// table) is what precedes the records in this format.
    pub fn records_offset_for(n_bands: u32) -> u64 {
        let end = Self::band_table_offset() + Self::band_table_bytes(n_bands);
        end.div_ceil(RECORD_ALIGN) * RECORD_ALIGN
    }

    pub fn name_str(&self) -> &str {
        let end = self.name.iter().position(|&c| c == 0).unwrap_or(self.name.len());
        std::str::from_utf8(&self.name[..end]).unwrap_or("")
    }

    /// Band scales in degrees, `n_bands` entries. Clamps the count to
    /// `MAX_BANDS` defensively -- `from_bytes` already rejects a
    /// stored `n_bands` over capacity, but this keeps the accessor itself
    /// panic-free even if a caller builds a `QuadHeader` by hand with a
    /// bad `n_bands`.
    pub fn band_scales_deg(&self) -> Vec<f32> {
        self.band_scales_millideg[..(self.n_bands as usize).min(MAX_BANDS)]
            .iter()
            .map(|&m| m as f32 / 1000.0)
            .collect()
    }

    pub fn to_bytes(&self) -> [u8; HEADER_BYTES] {
        let mut b = [0u8; HEADER_BYTES];
        b[0..8].copy_from_slice(&MAGIC);
        b[8..12].copy_from_slice(&self.version.to_le_bytes());
        b[12..16].copy_from_slice(&self.nside.to_le_bytes());
        b[16..24].copy_from_slice(&self.epoch.to_le_bytes());
        b[24..32].copy_from_slice(&self.n_quads.to_le_bytes());
        b[32..36].copy_from_slice(&self.n_bands.to_le_bytes());
        b[36..40].copy_from_slice(&self.mag_limit.to_le_bytes());
        for i in 0..MAX_BANDS {
            let s = 40 + i * 2;
            b[s..s + 2].copy_from_slice(&self.band_scales_millideg[i].to_le_bytes());
        }
        b[56..64].copy_from_slice(&self.records_offset.to_le_bytes());
        b[64..96].copy_from_slice(&self.records_sha256);
        b[96..104].copy_from_slice(&self.star_index_fingerprint);
        b[104..104 + NAME_BYTES].copy_from_slice(&self.name);
        b
    }

    pub fn from_bytes(b: &[u8]) -> Result<QuadHeader, IndexError> {
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
        let n_bands = u32::from_le_bytes([b[32], b[33], b[34], b[35]]);
        if n_bands as usize > MAX_BANDS {
            return Err(IndexError::BadRange {
                what: "n_bands",
                reason: format!("{n_bands} exceeds MAX_BANDS ({MAX_BANDS})"),
            });
        }
        let mut band_scales_millideg = [0u16; MAX_BANDS];
        for (i, slot) in band_scales_millideg.iter_mut().enumerate() {
            let s = 40 + i * 2;
            *slot = u16::from_le_bytes([b[s], b[s + 1]]);
        }
        let mut records_sha256 = [0u8; 32];
        records_sha256.copy_from_slice(&b[64..96]);
        let mut star_index_fingerprint = [0u8; 8];
        star_index_fingerprint.copy_from_slice(&b[96..104]);
        let mut name = [0u8; NAME_BYTES];
        name.copy_from_slice(&b[104..104 + NAME_BYTES]);

        Ok(QuadHeader {
            version,
            nside,
            epoch: f64::from_le_bytes(b[16..24].try_into().unwrap_or([0; 8])),
            n_quads: u64::from_le_bytes(b[24..32].try_into().unwrap_or([0; 8])),
            n_bands,
            band_scales_millideg,
            mag_limit: f32::from_le_bytes(b[36..40].try_into().unwrap_or([0; 4])),
            records_offset: u64::from_le_bytes(b[56..64].try_into().unwrap_or([0; 8])),
            records_sha256,
            star_index_fingerprint,
            name,
        })
    }
}

/// Quantize a raw `quad_code` output to the on-disk `u16` representation.
/// Returns the packed code and whether any component fell outside
/// `[CODE_MIN, CODE_MAX]` and had to be clamped -- see the module doc for
/// why that should not happen for a genuine `quad_code` output, but callers
/// (Task 2's builder) are expected to check and count it, the same
/// contract `record.rs::pack` uses for star fields.
pub fn pack_code(code: [f64; 4]) -> ([u16; 4], bool) {
    let mut clamped = false;
    let mut out = [0u16; 4];
    for (i, &v) in code.iter().enumerate() {
        let c = v.clamp(CODE_MIN, CODE_MAX);
        if c != v {
            clamped = true;
        }
        out[i] = ((c - CODE_MIN) * CODE_SCALE).round().clamp(0.0, u16::MAX as f64) as u16;
    }
    (out, clamped)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuadRecord {
    /// Quantized `[x_C, y_C, x_D, y_D]`. Use `code_f64()` to decode.
    pub code: [u16; 4],
    /// Global record indices into the paired `.psidx`'s flat record array,
    /// `[A, B, C, D]` in `quad_code`'s canonical order.
    pub star_idx: [u32; 4],
}

impl QuadRecord {
    /// Build a record from a raw geometric code and star references.
    /// Returns whether the code had to be clamped (see `pack_code`).
    pub fn new(code: [f64; 4], star_idx: [u32; 4]) -> (QuadRecord, bool) {
        let (packed, clamped) = pack_code(code);
        (QuadRecord { code: packed, star_idx }, clamped)
    }

    pub fn code_f64(&self) -> [f64; 4] {
        let mut out = [0.0; 4];
        for (i, &c) in self.code.iter().enumerate() {
            out[i] = c as f64 / CODE_SCALE + CODE_MIN;
        }
        out
    }

    pub fn to_bytes(&self) -> [u8; QUAD_RECORD_BYTES] {
        let mut b = [0u8; QUAD_RECORD_BYTES];
        for i in 0..4 {
            b[i * 2..i * 2 + 2].copy_from_slice(&self.code[i].to_le_bytes());
        }
        for i in 0..4 {
            let s = 8 + i * 4;
            b[s..s + 4].copy_from_slice(&self.star_idx[i].to_le_bytes());
        }
        b
    }

    pub fn from_bytes(b: &[u8; QUAD_RECORD_BYTES]) -> Self {
        let mut code = [0u16; 4];
        for (i, slot) in code.iter_mut().enumerate() {
            *slot = u16::from_le_bytes([b[i * 2], b[i * 2 + 1]]);
        }
        let mut star_idx = [0u32; 4];
        for (i, slot) in star_idx.iter_mut().enumerate() {
            let s = 8 + i * 4;
            *slot = u32::from_le_bytes([b[s], b[s + 1], b[s + 2], b[s + 3]]);
        }
        QuadRecord { code, star_idx }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> QuadHeader {
        let mut name = [0u8; NAME_BYTES];
        name[..16].copy_from_slice(b"gaia-g16-blind-b");
        let mut band_scales_millideg = [0u16; MAX_BANDS];
        for (i, &deg) in [250u16, 500, 1000, 2000, 4000, 8000].iter().enumerate() {
            band_scales_millideg[i] = deg;
        }
        QuadHeader {
            version: FORMAT_VERSION,
            nside: 64,
            epoch: 2016.0,
            n_quads: 18_674_481,
            n_bands: 6,
            band_scales_millideg,
            mag_limit: 16.0,
            records_offset: QuadHeader::records_offset_for(6),
            records_sha256: [9u8; 32],
            star_index_fingerprint: [3u8; 8],
            name,
        }
    }

    #[test]
    fn header_round_trips() {
        let h = sample();
        assert_eq!(QuadHeader::from_bytes(&h.to_bytes()).unwrap(), h);
    }

    #[test]
    fn header_is_exactly_128_bytes() {
        assert_eq!(sample().to_bytes().len(), HEADER_BYTES);
    }

    #[test]
    fn magic_differs_from_star_index_magic() {
        assert_ne!(MAGIC, crate::format::MAGIC, "the two formats must not share a magic");
    }

    #[test]
    fn rejects_bad_magic() {
        let mut b = sample().to_bytes();
        b[0] = b'X';
        assert!(matches!(QuadHeader::from_bytes(&b), Err(IndexError::BadMagic)));
    }

    /// The whole reason `.psqidx` has its own magic: a real `.psidx` header
    /// handed to this parser must fail cleanly, not be silently misread as
    /// a (badly corrupt) quad header.
    #[test]
    fn rejects_a_real_psidx_header_rather_than_misparsing_it() {
        let mut name = [0u8; 32];
        name[..9].copy_from_slice(b"gaia-dr3-");
        let star_header = crate::format::Header {
            version: crate::format::FORMAT_VERSION,
            nside: 64,
            epoch: 2016.0,
            n_records: 212_000_000,
            mag_limit: 16.0,
            records_offset: crate::format::Header::records_offset_for(64),
            records_sha256: [5u8; 32],
            name,
        };
        let b = star_header.to_bytes();
        assert!(
            matches!(QuadHeader::from_bytes(&b), Err(IndexError::BadMagic)),
            "a real .psidx header must be rejected by BadMagic, not misparsed into a QuadHeader"
        );
    }

    #[test]
    fn rejects_future_version() {
        let mut h = sample();
        h.version = 99;
        let b = h.to_bytes();
        assert!(matches!(QuadHeader::from_bytes(&b), Err(IndexError::UnsupportedVersion(99))));
    }

    #[test]
    fn rejects_truncated_header() {
        let b = sample().to_bytes();
        assert!(matches!(
            QuadHeader::from_bytes(&b[..60]),
            Err(IndexError::Truncated { .. })
        ));
    }

    #[test]
    fn rejects_n_bands_over_capacity() {
        let mut h = sample();
        h.n_bands = MAX_BANDS as u32 + 1;
        let b = h.to_bytes();
        assert!(matches!(QuadHeader::from_bytes(&b), Err(IndexError::BadRange { .. })));
    }

    #[test]
    fn rejects_non_power_of_two_nside() {
        let mut h = sample();
        h.nside = 63;
        let b = h.to_bytes();
        assert!(matches!(QuadHeader::from_bytes(&b), Err(IndexError::BadNside(63))));
    }

    #[test]
    fn name_str_strips_nul_padding() {
        assert_eq!(sample().name_str(), "gaia-g16-blind-b");
    }

    #[test]
    fn band_scales_decode_to_degrees_exactly_for_the_spec_bands() {
        let h = sample();
        assert_eq!(h.band_scales_deg(), vec![0.25f32, 0.5, 1.0, 2.0, 4.0, 8.0]);
    }

    #[test]
    fn records_are_page_aligned_for_every_band_count() {
        for n in [0u32, 1, 6, MAX_BANDS as u32] {
            assert_eq!(QuadHeader::records_offset_for(n) % RECORD_ALIGN, 0);
            assert!(
                QuadHeader::records_offset_for(n)
                    >= QuadHeader::band_table_offset() + QuadHeader::band_table_bytes(n)
            );
        }
    }

    // -- QuadRecord --

    #[test]
    fn quad_record_is_exactly_24_bytes() {
        assert_eq!(QUAD_RECORD_BYTES, 24);
        let (r, _) = QuadRecord::new([0.0, 0.5, 0.5, 1.0], [1, 2, 3, 4]);
        assert_eq!(r.to_bytes().len(), 24);
    }

    #[test]
    fn quad_record_round_trips() {
        let (r, clamped) = QuadRecord::new([-0.4, 0.1, 0.6, 1.3], [10, 4_000_000_000, 0, 1]);
        assert!(!clamped);
        assert_eq!(QuadRecord::from_bytes(&r.to_bytes()), r);
    }

    #[test]
    fn quad_code_round_trips_to_documented_precision() {
        let code = [-0.4123, 0.0987, 0.6543, 1.2001];
        let (r, clamped) = QuadRecord::new(code, [0, 0, 0, 0]);
        assert!(!clamped);
        for (a, b) in r.code_f64().iter().zip(code.iter()) {
            assert!((a - b).abs() < 1e-4, "{a} vs {b}");
        }
    }

    #[test]
    fn out_of_proven_range_code_clamps_and_reports() {
        let (_, clamped) = QuadRecord::new([-5.0, 0.0, 0.0, 0.0], [0, 0, 0, 0]);
        assert!(clamped, "a component outside [CODE_MIN, CODE_MAX] must set the clamped flag");
    }

    #[test]
    fn star_idx_is_exact_not_lossy() {
        let (r, _) = QuadRecord::new([0.0, 0.0, 0.0, 0.0], [0, u32::MAX, 1, 211_999_999]);
        assert_eq!(r.star_idx, [0, u32::MAX, 1, 211_999_999]);
    }

    #[test]
    fn byte_layout_is_little_endian() {
        let r = QuadRecord { code: [1, 0, 0, 0], star_idx: [0, 0, 0, 0] };
        assert_eq!(&r.to_bytes()[0..2], &[1, 0]);
    }

    // -- digest --

    /// The digest mechanism itself: covers the record region and detects a
    /// single flipped byte anywhere in it. This exercises `sha256.rs`
    /// directly over a `QuadRecord`-shaped byte region -- the same
    /// mechanism `QuadHeader::records_sha256` is defined to hold -- rather
    /// than a full builder+reader round trip, which is Task 2/3's job.
    #[test]
    fn digest_covers_record_region_and_detects_a_flipped_byte() {
        let (r1, _) = QuadRecord::new([0.0, 0.1, 0.2, 0.3], [1, 2, 3, 4]);
        let (r2, _) = QuadRecord::new([0.4, 0.5, 0.6, 0.7], [5, 6, 7, 8]);
        let mut region = Vec::new();
        region.extend_from_slice(&r1.to_bytes());
        region.extend_from_slice(&r2.to_bytes());
        let digest = crate::sha256::sha256(&region);

        // Recomputing over the unmodified bytes reproduces the same digest.
        assert_eq!(crate::sha256::sha256(&region), digest);

        for i in 0..region.len() {
            let mut flipped = region.clone();
            flipped[i] ^= 0x01;
            assert_ne!(
                crate::sha256::sha256(&flipped),
                digest,
                "flipping byte {i} of the record region must change the digest"
            );
        }
    }
}
