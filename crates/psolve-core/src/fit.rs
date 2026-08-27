//! The TAN WCS and its least-squares fit.
//!
//! In the tangent plane about (ra0, dec0):
//!     xi  = a0 + a1*x + a2*y
//!     eta = b0 + b1*x + b2*y
//! Those are TWO INDEPENDENT 3-parameter least squares, not one coupled
//! 6-parameter problem -- a 3x3 normal-equation solve done twice, plus a 2x2
//! inverse to recover CRPIX. That is why this needs no linear-algebra crate.

use crate::project::{angsep_deg, radec_to_tangent, tangent_to_radec};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Parity {
    Normal,
    Mirrored,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Wcs {
    pub crval: [f64; 2],
    pub crpix: [f64; 2],
    /// Degrees per pixel. cd[0] maps (dx,dy) to xi, cd[1] to eta.
    pub cd: [[f64; 2]; 2],
}

impl Wcs {
    pub fn pix_to_radec(&self, x: f64, y: f64) -> (f64, f64) {
        let dx = x - self.crpix[0];
        let dy = y - self.crpix[1];
        let xi = self.cd[0][0] * dx + self.cd[0][1] * dy;
        let eta = self.cd[1][0] * dx + self.cd[1][1] * dy;
        tangent_to_radec(xi, eta, self.crval[0], self.crval[1])
    }

    pub fn radec_to_pix(&self, ra: f64, dec: f64) -> Option<(f64, f64)> {
        let (xi, eta) = radec_to_tangent(ra, dec, self.crval[0], self.crval[1])?;
        let det = self.cd[0][0] * self.cd[1][1] - self.cd[0][1] * self.cd[1][0];
        if det.abs() < 1e-30 {
            return None;
        }
        let dx = (self.cd[1][1] * xi - self.cd[0][1] * eta) / det;
        let dy = (-self.cd[1][0] * xi + self.cd[0][0] * eta) / det;
        Some((dx + self.crpix[0], dy + self.crpix[1]))
    }

    /// Arcseconds per pixel, from the geometric mean of the CD matrix scales --
    /// correct under rotation and shear alike.
    pub fn scale_arcsec(&self) -> f64 {
        let det = self.cd[0][0] * self.cd[1][1] - self.cd[0][1] * self.cd[1][0];
        det.abs().sqrt() * 3600.0
    }

    /// Position angle, degrees east of north, of the frame's +y pixel axis.
    ///
    /// This MUST be +y: every other tool (and spec section 7.2's own
    /// illustrative solve) reports PA of +y, not -y. A prior version of this
    /// function used -y and was off by a constant 180 degrees, which no
    /// existing test could see -- comparing a fitted WCS to a truth WCS
    /// through this same function cancels a constant offset exactly, which
    /// is why the regression test below pins an ABSOLUTE reference instead.
    pub fn orientation_deg(&self) -> f64 {
        self.cd[0][1].atan2(self.cd[1][1]).to_degrees().rem_euclid(360.0)
    }

    /// A non-mirrored (E-left, N-up) sky image has a NEGATIVE CD determinant --
    /// that is the standard WCS handedness, because RA increases in the
    /// opposite sense to pixel x for an unflipped frame. A POSITIVE determinant
    /// means an odd number of reflections in the optical train: real equipment
    /// does this, and assuming one handedness fails half of it.
    pub fn parity(&self) -> Parity {
        let det = self.cd[0][0] * self.cd[1][1] - self.cd[0][1] * self.cd[1][0];
        if det < 0.0 { Parity::Normal } else { Parity::Mirrored }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FitResult {
    pub wcs: Wcs,
    pub used: usize,
    pub rms_deg: f64,
    pub max_residual_deg: f64,
    /// Correlation between residual size and radius from the field centre.
    /// Near zero means TAN is sufficient; a systematic positive value is the
    /// data asking for a distortion model.
    pub radial_trend: f64,
}

/// Solve a symmetric 3x3 system by Gaussian elimination with partial pivoting.
/// Returns None when the matrix is singular -- collinear points, for instance,
/// which must be detected rather than inverted anyway.
fn solve3(mut m: [[f64; 4]; 3]) -> Option<[f64; 3]> {
    for col in 0..3 {
        let mut piv = col;
        for r in (col + 1)..3 {
            if m[r][col].abs() > m[piv][col].abs() {
                piv = r;
            }
        }
        if m[piv][col].abs() < 1e-12 {
            return None;
        }
        m.swap(col, piv);
        let d = m[col][col];
        for v in m[col].iter_mut().skip(col) {
            *v /= d;
        }
        let pivot_row = m[col];
        for (r, row) in m.iter_mut().enumerate() {
            if r == col {
                continue;
            }
            let f = row[col];
            if f == 0.0 {
                continue;
            }
            for (v, p) in row.iter_mut().skip(col).zip(pivot_row.iter().skip(col)) {
                *v -= f * p;
            }
        }
    }
    let out = [m[0][3], m[1][3], m[2][3]];
    if out.iter().all(|v| v.is_finite()) { Some(out) } else { None }
}

/// One unweighted linear least squares of `t = p0 + p1*x + p2*y`.
fn lsq_plane(rows: &[(f64, f64, f64)]) -> Option<[f64; 3]> {
    let (mut s1, mut sx, mut sy) = (0.0, 0.0, 0.0);
    let (mut sxx, mut syy, mut sxy) = (0.0, 0.0, 0.0);
    let (mut st, mut sxt, mut syt) = (0.0, 0.0, 0.0);
    for &(x, y, t) in rows {
        s1 += 1.0;
        sx += x;
        sy += y;
        sxx += x * x;
        syy += y * y;
        sxy += x * y;
        st += t;
        sxt += x * t;
        syt += y * t;
    }
    solve3([
        [s1, sx, sy, st],
        [sx, sxx, sxy, sxt],
        [sy, sxy, syy, syt],
    ])
}

fn build(a: [f64; 3], b: [f64; 3], ra0: f64, dec0: f64) -> Option<Wcs> {
    let cd = [[a[1], a[2]], [b[1], b[2]]];
    let det = cd[0][0] * cd[1][1] - cd[0][1] * cd[1][0];
    if det.abs() < 1e-30 {
        return None;
    }
    // CRPIX is where xi = eta = 0, i.e. cd * crpix = -[a0, b0].
    let crpix0 = (cd[1][1] * -a[0] - cd[0][1] * -b[0]) / det;
    let crpix1 = (-cd[1][0] * -a[0] + cd[0][0] * -b[0]) / det;
    if !crpix0.is_finite() || !crpix1.is_finite() {
        return None;
    }
    Some(Wcs { crval: [ra0, dec0], crpix: [crpix0, crpix1], cd })
}

/// A pixel/sky correspondence: ((pixel_x, pixel_y), (ra_deg, dec_deg)).
pub type Correspondence = ((f64, f64), (f64, f64));

/// Fit a TAN WCS to pixel/sky correspondences, sigma-clipping twice.
///
/// `pairs` are ((pixel_x, pixel_y), (ra_deg, dec_deg)). `ra0`/`dec0` set the
/// tangent point -- pass the hint centre; CRVAL comes out equal to it and CRPIX
/// absorbs the offset.
pub fn fit_tan(
    pairs: &[Correspondence],
    ra0: f64,
    dec0: f64,
    clip_sigma: f64,
) -> Option<FitResult> {
    // Three points determine the plane; below four there is nothing to check it
    // against, so a "fit" would be a restatement of the input.
    if pairs.len() < 4 {
        return None;
    }

    let mut rows: Vec<(f64, f64, f64, f64)> = Vec::with_capacity(pairs.len());
    for &((x, y), (ra, dec)) in pairs {
        if let Some((xi, eta)) = radec_to_tangent(ra, dec, ra0, dec0) {
            rows.push((x, y, xi, eta));
        }
    }
    if rows.len() < 4 {
        return None;
    }

    let mut wcs;
    let mut kept = rows.clone();
    for round in 0..3 {
        let xi_rows: Vec<(f64, f64, f64)> = kept.iter().map(|r| (r.0, r.1, r.2)).collect();
        let eta_rows: Vec<(f64, f64, f64)> = kept.iter().map(|r| (r.0, r.1, r.3)).collect();
        let a = lsq_plane(&xi_rows)?;
        let b = lsq_plane(&eta_rows)?;
        wcs = build(a, b, ra0, dec0)?;

        // Residuals as true angular separations, not tangent-plane differences.
        let mut res: Vec<f64> = Vec::with_capacity(kept.len());
        for r in &kept {
            let (pra, pdec) = wcs.pix_to_radec(r.0, r.1);
            let (tra, tdec) = tangent_to_radec(r.2, r.3, ra0, dec0);
            res.push(angsep_deg(pra, pdec, tra, tdec));
        }
        let n = res.len() as f64;
        let rms = (res.iter().map(|v| v * v).sum::<f64>() / n).sqrt();

        if round == 2 || kept.len() <= 6 || rms <= 0.0 {
            let max_residual_deg = res.iter().cloned().fold(0.0f64, f64::max);
            // Correlation of residual with radius from the field centre.
            let cx = kept.iter().map(|r| r.0).sum::<f64>() / n;
            let cy = kept.iter().map(|r| r.1).sum::<f64>() / n;
            let radii: Vec<f64> = kept
                .iter()
                .map(|r| ((r.0 - cx).powi(2) + (r.1 - cy).powi(2)).sqrt())
                .collect();
            let radial_trend = correlation(&radii, &res);
            return Some(FitResult {
                wcs,
                used: kept.len(),
                rms_deg: rms,
                max_residual_deg,
                radial_trend,
            });
        }

        // Residuals below a thousandth of a pixel are floating-point round-off
        // from the trig round trip, not real astrometric error -- no real
        // centroid is ever known that precisely, so a threshold built purely
        // from `rms` (which itself collapses towards that same noise floor as
        // clipping proceeds) would keep discarding points forever. Flooring the
        // limit at a sub-milli-pixel angle breaks that spiral without weakening
        // rejection of any genuine outlier, which sits many orders of magnitude
        // above it.
        let floor_deg = 1e-3 * wcs.scale_arcsec() / 3600.0;
        let limit = (rms * clip_sigma).max(floor_deg);
        let next: Vec<(f64, f64, f64, f64)> = kept
            .iter()
            .zip(res.iter())
            .filter(|(_, r)| **r <= limit)
            .map(|(k, _)| *k)
            .collect();
        if next.len() < 4 || next.len() == kept.len() {
            let max_residual_deg = res.iter().cloned().fold(0.0f64, f64::max);
            let cx = kept.iter().map(|r| r.0).sum::<f64>() / n;
            let cy = kept.iter().map(|r| r.1).sum::<f64>() / n;
            let radii: Vec<f64> = kept
                .iter()
                .map(|r| ((r.0 - cx).powi(2) + (r.1 - cy).powi(2)).sqrt())
                .collect();
            let radial_trend = correlation(&radii, &res);
            return Some(FitResult {
                wcs,
                used: kept.len(),
                rms_deg: rms,
                max_residual_deg,
                radial_trend,
            });
        }
        kept = next;
    }
    None
}

fn correlation(a: &[f64], b: &[f64]) -> f64 {
    let n = a.len() as f64;
    if n < 3.0 {
        return 0.0;
    }
    let ma = a.iter().sum::<f64>() / n;
    let mb = b.iter().sum::<f64>() / n;
    let mut num = 0.0;
    let mut da = 0.0;
    let mut db = 0.0;
    for (x, y) in a.iter().zip(b.iter()) {
        num += (x - ma) * (y - mb);
        da += (x - ma).powi(2);
        db += (y - mb).powi(2);
    }
    if da <= 0.0 || db <= 0.0 {
        return 0.0;
    }
    num / (da.sqrt() * db.sqrt())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{angsep_deg, tangent_to_radec};

    /// Build a synthetic truth WCS resembling the reference rig:
    /// 2.461 arcsec/px, 3840x2160, rotated, optionally mirrored.
    fn truth(rot_deg: f64, mirrored: bool) -> Wcs {
        let s = 2.4614 / 3600.0;
        let r = rot_deg.to_radians();
        let (c, si) = (r.cos(), r.sin());
        let m = if mirrored { -1.0 } else { 1.0 };
        Wcs {
            crval: [274.689087, -13.810971],
            crpix: [1920.5, 1080.5],
            cd: [[-s * c * m, s * si], [s * si * m, s * c]],
        }
    }

    /// Sample the truth WCS on a grid to make perfect correspondences.
    fn pairs_from(w: &Wcs, n: usize) -> Vec<((f64, f64), (f64, f64))> {
        let mut out = Vec::new();
        for i in 0..n {
            let t = i as f64;
            let x = (t * 173.0) % 3800.0 + 20.0;
            let y = (t * 97.0) % 2140.0 + 10.0;
            out.push(((x, y), w.pix_to_radec(x, y)));
        }
        out
    }

    #[test]
    fn recovers_a_known_wcs_from_perfect_correspondences() {
        let w = truth(31.0, false);
        let pairs = pairs_from(&w, 40);
        let r = fit_tan(&pairs, w.crval[0], w.crval[1], 3.0).expect("should fit");
        assert_eq!(r.used, 40);
        assert!(r.rms_deg * 3600.0 < 0.01, "rms {} arcsec", r.rms_deg * 3600.0);
        // Centre agreement is the number that matters downstream.
        let c_truth = w.pix_to_radec(1920.5, 1080.5);
        let c_fit = r.wcs.pix_to_radec(1920.5, 1080.5);
        let sep = angsep_deg(c_truth.0, c_truth.1, c_fit.0, c_fit.1) * 3600.0;
        assert!(sep < 0.05, "centre off by {sep} arcsec");
    }

    #[test]
    fn recovers_the_pixel_scale_and_orientation() {
        for rot in [0.0, 45.0, 122.6, 300.0] {
            let w = truth(rot, false);
            let r = fit_tan(&pairs_from(&w, 30), w.crval[0], w.crval[1], 3.0).unwrap();
            assert!(
                (r.wcs.scale_arcsec() - 2.4614).abs() < 0.005,
                "rot {rot}: scale {}",
                r.wcs.scale_arcsec()
            );
            let d = (r.wcs.orientation_deg() - w.orientation_deg()).abs() % 360.0;
            assert!(d < 0.05 || (360.0 - d) < 0.05, "rot {rot}: orientation drift {d}");
        }
    }

    #[test]
    fn parity_is_detected_in_both_directions() {
        let n = truth(20.0, false);
        let m = truth(20.0, true);
        assert_eq!(n.parity(), Parity::Normal);
        assert_eq!(m.parity(), Parity::Mirrored);
        let rn = fit_tan(&pairs_from(&n, 30), n.crval[0], n.crval[1], 3.0).unwrap();
        let rm = fit_tan(&pairs_from(&m, 30), m.crval[0], m.crval[1], 3.0).unwrap();
        assert_eq!(rn.wcs.parity(), Parity::Normal);
        assert_eq!(rm.wcs.parity(), Parity::Mirrored, "a mirrored frame must be recognised");
    }

    #[test]
    fn pixel_and_sky_round_trip_through_the_fitted_wcs() {
        let w = truth(77.0, false);
        let r = fit_tan(&pairs_from(&w, 30), w.crval[0], w.crval[1], 3.0).unwrap();
        for (x, y) in [(100.0, 100.0), (1920.5, 1080.5), (3700.0, 2000.0)] {
            let (ra, dec) = r.wcs.pix_to_radec(x, y);
            let (bx, by) = r.wcs.radec_to_pix(ra, dec).expect("in front of the plane");
            assert!((bx - x).abs() < 1e-6 && (by - y).abs() < 1e-6, "({x},{y}) -> ({bx},{by})");
        }
    }

    #[test]
    fn a_single_outlier_is_clipped_and_does_not_drag_the_fit() {
        let w = truth(15.0, false);
        let mut pairs = pairs_from(&w, 40);
        // Move one correspondence half a degree off.
        let (px, _) = pairs[7];
        let bad = tangent_to_radec(0.5, 0.5, w.crval[0], w.crval[1]);
        pairs[7] = (px, bad);
        let r = fit_tan(&pairs, w.crval[0], w.crval[1], 3.0).unwrap();
        assert!(r.used < 40, "the outlier should have been clipped");
        assert!(r.rms_deg * 3600.0 < 0.1, "rms {} arcsec after clipping", r.rms_deg * 3600.0);
    }

    #[test]
    fn too_few_pairs_returns_none_rather_than_a_meaningless_fit() {
        let w = truth(0.0, false);
        assert!(fit_tan(&pairs_from(&w, 2), w.crval[0], w.crval[1], 3.0).is_none());
        assert!(fit_tan(&[], w.crval[0], w.crval[1], 3.0).is_none());
    }

    #[test]
    fn collinear_points_do_not_produce_a_bogus_solution() {
        // A degenerate normal matrix must be detected, not inverted anyway.
        let w = truth(0.0, false);
        let mut pairs = Vec::new();
        for i in 0..10 {
            let x = 100.0 + i as f64 * 50.0;
            pairs.push(((x, 500.0), w.pix_to_radec(x, 500.0)));
        }
        assert!(fit_tan(&pairs, w.crval[0], w.crval[1], 3.0).is_none());
    }

    #[test]
    fn radial_trend_is_near_zero_for_a_pure_tan_field() {
        // If TAN is right, residuals do not grow with radius. This statistic is
        // how the data decides whether SIP is ever needed.
        let w = truth(10.0, false);
        let r = fit_tan(&pairs_from(&w, 60), w.crval[0], w.crval[1], 3.0).unwrap();
        assert!(r.radial_trend.abs() < 0.5, "radial trend {} on a clean field", r.radial_trend);
    }

    #[test]
    fn orientation_deg_reports_position_angle_of_plus_y_not_minus_y() {
        // Comparing a fitted WCS to a truth WCS through orientation_deg()
        // itself cancels a constant 180-degree offset exactly, so this pins
        // an ABSOLUTE reference instead: for an unrotated, unmirrored WCS
        // the +y pixel axis points due north, so its position angle (east of
        // north) must be ~0 degrees, not 180.
        let w = truth(0.0, false);
        let pa = w.orientation_deg();
        assert!(!(0.5..=359.5).contains(&pa), "PA(+y) of an unrotated frame should be ~0, got {pa}");
    }

    #[test]
    fn a_point_behind_the_plane_has_no_pixel_position() {
        let w = truth(0.0, false);
        let anti = (w.crval[0] + 180.0) % 360.0;
        assert!(w.radec_to_pix(anti, -w.crval[1]).is_none());
    }
}
