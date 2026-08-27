//! Sky geometry: separations, the gnomonic (TAN) projection, and proper motion.
//!
//! Tangent-plane coordinates are in DEGREES, matching CDELT, so a fit done in
//! this space yields CD-matrix coefficients directly rather than through a unit
//! conversion nobody remembers a year later.

/// Great-circle separation in degrees, by haversine.
///
/// Haversine rather than acos of a dot product: for the SMALL separations this
/// pipeline cares about, acos loses precision exactly where the answer matters,
/// because its argument approaches 1 and the derivative blows up.
pub fn angsep_deg(ra1: f64, dec1: f64, ra2: f64, dec2: f64) -> f64 {
    let (r1, d1) = (ra1.to_radians(), dec1.to_radians());
    let (r2, d2) = (ra2.to_radians(), dec2.to_radians());
    let dr = r2 - r1;
    let dd = d2 - d1;
    let a = (dd / 2.0).sin().powi(2) + d1.cos() * d2.cos() * (dr / 2.0).sin().powi(2);
    2.0 * a.sqrt().min(1.0).asin().to_degrees()
}

/// Gnomonic projection about (ra0, dec0). Returns (xi, eta) in degrees, where
/// xi increases towards +RA and eta towards +Dec. `None` when the point lies on
/// or behind the plane through the sphere's centre, which cannot be projected.
pub fn radec_to_tangent(ra: f64, dec: f64, ra0: f64, dec0: f64) -> Option<(f64, f64)> {
    let (r, d) = (ra.to_radians(), dec.to_radians());
    let (r0, d0) = (ra0.to_radians(), dec0.to_radians());
    let cos_c = d0.sin() * d.sin() + d0.cos() * d.cos() * (r - r0).cos();
    if cos_c <= 1e-12 {
        return None;
    }
    let xi = d.cos() * (r - r0).sin() / cos_c;
    let eta = (d0.cos() * d.sin() - d0.sin() * d.cos() * (r - r0).cos()) / cos_c;
    Some((xi.to_degrees(), eta.to_degrees()))
}

/// Inverse gnomonic projection: (xi, eta) in degrees back to (ra, dec).
pub fn tangent_to_radec(xi: f64, eta: f64, ra0: f64, dec0: f64) -> (f64, f64) {
    let x = xi.to_radians();
    let y = eta.to_radians();
    let (r0, d0) = (ra0.to_radians(), dec0.to_radians());
    let rho = (x * x + y * y).sqrt();
    if rho < 1e-15 {
        return (ra0.rem_euclid(360.0), dec0);
    }
    let c = rho.atan();
    let dec = (c.cos() * d0.sin() + y * c.sin() * d0.cos() / rho).clamp(-1.0, 1.0).asin();
    let ra = r0
        + (x * c.sin()).atan2(rho * d0.cos() * c.cos() - y * d0.sin() * c.sin());
    (ra.to_degrees().rem_euclid(360.0), dec.to_degrees())
}

/// Move a star by its proper motion over `years`.
///
/// `pmra_mas_yr` is pmRA* -- Gaia publishes it already multiplied by cos(dec) --
/// so both components are ARCS on the sky, and the true displacement is
/// independent of declination.
///
/// Done in Cartesian rather than by incrementing RA: converting an arc to a
/// coordinate-RA increment means dividing by cos(dec), which diverges at the
/// pole. Clamping that divisor does not tame the divergence, it silently
/// understates the motion -- by a factor of thousands within a thousandth of a
/// degree of the pole. Adding the offset to a unit vector has no such failure.
pub fn apply_proper_motion(
    ra: f64,
    dec: f64,
    pmra_mas_yr: f64,
    pmdec_mas_yr: f64,
    years: f64,
) -> (f64, f64) {
    if years == 0.0 || (pmra_mas_yr == 0.0 && pmdec_mas_yr == 0.0) {
        return (ra, dec);
    }
    const MAS_TO_RAD: f64 = std::f64::consts::PI / (180.0 * 3_600_000.0);
    let d_east = pmra_mas_yr * years * MAS_TO_RAD;
    let d_north = pmdec_mas_yr * years * MAS_TO_RAD;

    let (r, d) = (ra.to_radians(), dec.to_radians());
    let (sr, cr, sd, cd) = (r.sin(), r.cos(), d.sin(), d.cos());
    // Position, plus the local east and north unit vectors. `east` stays a unit
    // vector at every declination including the pole, which is what removes the
    // singularity.
    let p = [cd * cr, cd * sr, sd];
    let e = [-sr, cr, 0.0];
    let n = [-sd * cr, -sd * sr, cd];

    let v = [
        p[0] + d_east * e[0] + d_north * n[0],
        p[1] + d_east * e[1] + d_north * n[1],
        p[2] + d_east * e[2] + d_north * n[2],
    ];
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if len <= 0.0 || !len.is_finite() {
        return (ra, dec);
    }
    let new_dec = (v[2] / len).clamp(-1.0, 1.0).asin().to_degrees();
    let new_ra = v[1].atan2(v[0]).to_degrees().rem_euclid(360.0);
    (new_ra, new_dec)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn angular_separation_matches_known_values() {
        assert!(angsep_deg(0.0, 0.0, 0.0, 0.0).abs() < 1e-12);
        assert!((angsep_deg(0.0, 0.0, 90.0, 0.0) - 90.0).abs() < 1e-9);
        assert!((angsep_deg(0.0, -90.0, 0.0, 90.0) - 180.0).abs() < 1e-9);
        // One degree of declination is one degree of arc, anywhere.
        assert!((angsep_deg(123.0, 45.0, 123.0, 46.0) - 1.0).abs() < 1e-9);
        // One degree of RA at dec 60 subtends half a degree.
        assert!((angsep_deg(0.0, 60.0, 1.0, 60.0) - 0.5).abs() < 1e-3);
    }

    #[test]
    fn angular_separation_is_precise_at_tiny_angles() {
        // The reason for haversine: acos loses precision here.
        let s = angsep_deg(100.0, 20.0, 100.0 + 1.0 / 3600.0 / 20.0, 20.0);
        assert!(s > 0.0, "a sub-arcsecond separation must not collapse to zero");
        assert!(s < 1e-4);
    }

    #[test]
    fn the_tangent_point_projects_to_the_origin() {
        let (xi, eta) = radec_to_tangent(274.689, -13.811, 274.689, -13.811).unwrap();
        assert!(xi.abs() < 1e-12 && eta.abs() < 1e-12);
    }

    #[test]
    fn projection_round_trips_across_a_realistic_field() {
        // The reference rig's field is 2.626 x 1.477 degrees.
        let (ra0, dec0) = (274.689087, -13.810971);
        for dxi in [-1.3, -0.4, 0.0, 0.7, 1.3] {
            for deta in [-0.73, 0.0, 0.51, 0.73] {
                let (ra, dec) = tangent_to_radec(dxi, deta, ra0, dec0);
                let (xi, eta) = radec_to_tangent(ra, dec, ra0, dec0).unwrap();
                assert!((xi - dxi).abs() < 1e-9, "xi {xi} vs {dxi}");
                assert!((eta - deta).abs() < 1e-9, "eta {eta} vs {deta}");
            }
        }
    }

    #[test]
    fn round_trip_holds_near_the_pole_and_across_the_ra_wrap() {
        for (ra0, dec0) in [(0.0, 89.0), (359.9, -88.5), (180.0, 0.0)] {
            for (dxi, deta) in [(0.5, 0.3), (-0.4, -0.6)] {
                let (ra, dec) = tangent_to_radec(dxi, deta, ra0, dec0);
                assert!((0.0..360.0).contains(&ra), "ra {ra} out of range");
                let (xi, eta) = radec_to_tangent(ra, dec, ra0, dec0).unwrap();
                assert!((xi - dxi).abs() < 1e-7 && (eta - deta).abs() < 1e-7);
            }
        }
    }

    #[test]
    fn eta_increases_towards_north() {
        let (ra0, dec0) = (100.0, 20.0);
        let (_, eta) = radec_to_tangent(100.0, 20.5, ra0, dec0).unwrap();
        assert!(eta > 0.0, "a star north of centre must have positive eta");
    }

    #[test]
    fn a_point_behind_the_tangent_plane_is_rejected() {
        // The antipode cannot be projected onto a tangent plane.
        assert!(radec_to_tangent(100.0 + 180.0, -20.0, 100.0, 20.0).is_none());
    }

    #[test]
    fn proper_motion_moves_a_star_the_expected_amount() {
        // 1000 mas/yr for 10 years is 10 arcsec = 1/360 degree.
        let (ra, dec) = apply_proper_motion(100.0, 0.0, 1000.0, 0.0, 10.0);
        let moved = angsep_deg(100.0, 0.0, ra, dec);
        assert!((moved - 10.0 / 3600.0).abs() < 1e-6, "moved {moved} deg");
    }

    #[test]
    fn proper_motion_in_ra_is_already_cos_dec_corrected() {
        // pmRA* is published pre-multiplied by cos(dec), so at dec 60 the same
        // pm value must produce the same ARC as at the equator.
        let a = apply_proper_motion(100.0, 0.0, 3600.0, 0.0, 1.0);
        let b = apply_proper_motion(100.0, 60.0, 3600.0, 0.0, 1.0);
        let arc_a = angsep_deg(100.0, 0.0, a.0, a.1);
        let arc_b = angsep_deg(100.0, 60.0, b.0, b.1);
        assert!((arc_a - arc_b).abs() < 1e-6, "{arc_a} vs {arc_b}");
    }

    #[test]
    fn zero_proper_motion_and_zero_elapsed_time_are_no_ops() {
        assert_eq!(apply_proper_motion(12.0, 34.0, 0.0, 0.0, 10.0), (12.0, 34.0));
        assert_eq!(apply_proper_motion(12.0, 34.0, 500.0, 500.0, 0.0), (12.0, 34.0));
    }

    #[test]
    fn proper_motion_moves_the_same_arc_at_every_declination_including_the_pole() {
        // pmRA* is pre-multiplied by cos(dec), so the displacement is an ARC and
        // must not depend on declination. The old cos(dec) division understated
        // it by ~2900x near the pole while still returning a finite number, so
        // an is_finite() assertion could not see the bug.
        let pm = 10_000.0; // mas/yr
        let years = 10.0;
        let expected_deg = pm * years / 3_600_000.0; // 0.02777... deg
        for dec in [0.0, 45.0, 80.0, 89.9, 89.99999] {
            let (ra2, dec2) = apply_proper_motion(0.0, dec, pm, 0.0, years);
            let moved = angsep_deg(0.0, dec, ra2, dec2);
            assert!(
                (moved - expected_deg).abs() < expected_deg * 0.02,
                "at dec {dec} the star moved {moved} deg, expected {expected_deg}"
            );
        }
    }

    #[test]
    fn proper_motion_stays_in_range_and_finite_everywhere() {
        for dec in [0.0, 89.99999, -89.99999, 90.0, -90.0] {
            let (ra, d) = apply_proper_motion(0.0, dec, 10_000.0, 10_000.0, 10.0);
            assert!(ra.is_finite() && d.is_finite());
            assert!((0.0..360.0).contains(&ra), "ra {ra} out of range at dec {dec}");
            assert!((-90.0..=90.0).contains(&d), "dec {d} out of range at dec {dec}");
        }
    }
}
