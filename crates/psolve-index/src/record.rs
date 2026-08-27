//! The 16-byte star record. Fixed width and little-endian so the mmap'd
//! record region can be cast to a slice with no parsing at solve time.
//!
//! Layout:
//!   0..4   ra_scaled   u32   ra_deg / 360 * 2^32     (~0.3 mas)
//!   4..8   dec_scaled  i32   dec_deg / 90 * 2^31     (~0.15 mas)
//!   8..10  mag_milli   i16   G magnitude * 1000
//!  10..12  pmra_mas    i16   pmRA*  mas/yr
//!  12..14  pmdec_mas   i16   pmDec  mas/yr
//!  14..16  reserved

pub const RECORD_BYTES: usize = 16;

const RA_SCALE: f64 = 4_294_967_296.0 / 360.0; // 2^32 / 360
const DEC_SCALE: f64 = 2_147_483_647.0 / 90.0; // i32::MAX / 90

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StarRecord {
    pub ra_scaled: u32,
    pub dec_scaled: i32,
    pub mag_milli: i16,
    pub pmra_mas: i16,
    pub pmdec_mas: i16,
}

/// Pack a Gaia row. Returns the record and whether any field was clamped.
/// Clamping is counted rather than rejected: a star with absurd proper motion
/// is still a usable position, and a silent drop would skew the catalogue.
pub fn pack(ra_deg: f64, dec_deg: f64, mag: f32, pmra: f32, pmdec: f32) -> (StarRecord, bool) {
    let mut clamped = false;

    let ra = ra_deg.rem_euclid(360.0);
    let ra_scaled = (ra * RA_SCALE).round() as u64 as u32;

    let dec = dec_deg.clamp(-90.0, 90.0);
    if dec != dec_deg {
        clamped = true;
    }
    let dec_scaled = (dec * DEC_SCALE)
        .round()
        .clamp(i32::MIN as f64, i32::MAX as f64) as i32;

    let m = (mag * 1000.0).round();
    if m < i16::MIN as f32 || m > i16::MAX as f32 {
        clamped = true;
    }
    let mag_milli = m.clamp(i16::MIN as f32, i16::MAX as f32) as i16;

    let clamp_pm = |v: f32, clamped: &mut bool| -> i16 {
        if v.is_nan() {
            return 0;
        }
        let r = v.round();
        if r < i16::MIN as f32 || r > i16::MAX as f32 {
            *clamped = true;
        }
        r.clamp(i16::MIN as f32, i16::MAX as f32) as i16
    };
    let pmra_mas = clamp_pm(pmra, &mut clamped);
    let pmdec_mas = clamp_pm(pmdec, &mut clamped);

    (StarRecord { ra_scaled, dec_scaled, mag_milli, pmra_mas, pmdec_mas }, clamped)
}

impl StarRecord {
    pub fn ra_deg(&self) -> f64 {
        self.ra_scaled as f64 / RA_SCALE
    }
    pub fn dec_deg(&self) -> f64 {
        self.dec_scaled as f64 / DEC_SCALE
    }
    pub fn mag(&self) -> f32 {
        self.mag_milli as f32 / 1000.0
    }
    pub fn pmra_mas_yr(&self) -> f32 {
        self.pmra_mas as f32
    }
    pub fn pmdec_mas_yr(&self) -> f32 {
        self.pmdec_mas as f32
    }

    pub fn to_bytes(&self) -> [u8; RECORD_BYTES] {
        let mut b = [0u8; RECORD_BYTES];
        b[0..4].copy_from_slice(&self.ra_scaled.to_le_bytes());
        b[4..8].copy_from_slice(&self.dec_scaled.to_le_bytes());
        b[8..10].copy_from_slice(&self.mag_milli.to_le_bytes());
        b[10..12].copy_from_slice(&self.pmra_mas.to_le_bytes());
        b[12..14].copy_from_slice(&self.pmdec_mas.to_le_bytes());
        b
    }

    pub fn from_bytes(b: &[u8; RECORD_BYTES]) -> Self {
        StarRecord {
            ra_scaled: u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            dec_scaled: i32::from_le_bytes([b[4], b[5], b[6], b[7]]),
            mag_milli: i16::from_le_bytes([b[8], b[9]]),
            pmra_mas: i16::from_le_bytes([b[10], b[11]]),
            pmdec_mas: i16::from_le_bytes([b[12], b[13]]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_is_exactly_sixteen_bytes() {
        assert_eq!(RECORD_BYTES, 16);
        let (r, _) = pack(0.0, 0.0, 0.0, 0.0, 0.0);
        assert_eq!(r.to_bytes().len(), 16);
    }

    #[test]
    fn position_round_trips_within_a_milliarcsecond() {
        for &(ra, dec) in &[
            (0.0, 0.0), (274.689087, -13.810971), (359.9999999, 89.9999),
            (180.0, -89.9999), (45.0, 45.0),
        ] {
            let (r, _) = pack(ra, dec, 12.0, 0.0, 0.0);
            assert!((r.ra_deg() - ra).abs() * 3_600_000.0 < 1.0, "ra {ra} -> {}", r.ra_deg());
            assert!((r.dec_deg() - dec).abs() * 3_600_000.0 < 1.0, "dec {dec} -> {}", r.dec_deg());
        }
    }

    #[test]
    fn magnitude_round_trips_to_a_millimag() {
        for &m in &[-1.44f32, 0.0, 6.0, 12.3456, 17.641426, 21.0] {
            let (r, clamped) = pack(0.0, 0.0, m, 0.0, 0.0);
            assert!(!clamped, "mag {m} should not clamp");
            assert!((r.mag() - m).abs() < 0.001, "mag {m} -> {}", r.mag());
        }
    }

    #[test]
    fn proper_motion_round_trips_and_covers_barnards_star() {
        // Barnard's Star has the largest known proper motion, ~10.4 arcsec/yr.
        let (r, clamped) = pack(0.0, 0.0, 9.5, -798.58, 10328.12);
        assert!(!clamped, "Barnard's Star must fit without clamping");
        assert_eq!(r.pmra_mas_yr(), -799.0);
        assert_eq!(r.pmdec_mas_yr(), 10328.0);
    }

    #[test]
    fn absurd_proper_motion_clamps_and_reports() {
        let (_, clamped) = pack(0.0, 0.0, 9.5, 99_000.0, 0.0);
        assert!(clamped, "out-of-range pm must set the clamped flag");
    }

    #[test]
    fn missing_proper_motion_becomes_zero_not_nan() {
        let (r, clamped) = pack(10.0, 20.0, 15.0, f32::NAN, f32::NAN);
        assert!(!clamped);
        assert_eq!(r.pmra_mas_yr(), 0.0);
        assert_eq!(r.pmdec_mas_yr(), 0.0);
    }

    #[test]
    fn bytes_round_trip_exactly() {
        let (r, _) = pack(274.689087, -13.810971, 14.25, -12.5, 33.75);
        assert_eq!(StarRecord::from_bytes(&r.to_bytes()), r);
    }

    #[test]
    fn ra_wraps_rather_than_overflowing() {
        let (a, _) = pack(360.0, 0.0, 10.0, 0.0, 0.0);
        let (b, _) = pack(0.0, 0.0, 10.0, 0.0, 0.0);
        assert_eq!(a.ra_scaled, b.ra_scaled);
    }

    #[test]
    fn the_poles_do_not_flip_hemisphere() {
        let (n, _) = pack(0.0, 90.0, 10.0, 0.0, 0.0);
        assert!((n.dec_deg() - 90.0).abs() < 1e-6, "north pole decoded as {}", n.dec_deg());
        let (s, _) = pack(0.0, -90.0, 10.0, 0.0, 0.0);
        assert!((s.dec_deg() + 90.0).abs() < 1e-6, "south pole decoded as {}", s.dec_deg());
    }

    #[test]
    fn out_of_range_declination_clamps_to_the_near_pole_not_the_far_one() {
        let (r, clamped) = pack(0.0, 999.0, 10.0, 0.0, 0.0);
        assert!(clamped);
        assert!(r.dec_deg() > 89.0, "dec 999 should clamp toward +90, got {}", r.dec_deg());
    }

    #[test]
    fn byte_layout_is_little_endian() {
        let r = StarRecord { ra_scaled: 1, dec_scaled: 0, mag_milli: 0, pmra_mas: 0, pmdec_mas: 0 };
        assert_eq!(&r.to_bytes()[0..4], &[1, 0, 0, 0]);
    }
}
