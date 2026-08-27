//! HEALPix nested-scheme indexing (Górski et al. 2005).
//!
//! Only what psolve needs: point -> pixel, pixel -> centre, and a disc query.
//! Validated against Gaia DR3's own source_id encoding in tests/healpix_gaia.rs,
//! which is real ground truth rather than a reimplementation of our own belief.

use std::f64::consts::PI;

const TWO_THIRDS: f64 = 2.0 / 3.0;

/// Total pixels at this nside. 12 base faces, each nside x nside.
pub fn npix(nside: u32) -> u64 {
    12 * (nside as u64) * (nside as u64)
}

/// nside must be a power of two so that nesting is a bit shift.
pub fn is_valid_nside(nside: u32) -> bool {
    (1..=4096).contains(&nside) && nside.is_power_of_two()
}

/// Interleave the low bits of x (even positions) and y (odd positions).
/// This is what makes a nested pixel index a quadtree path.
fn interleave(x: u32, y: u32) -> u64 {
    fn spread(v: u32) -> u64 {
        let mut n = v as u64 & 0x0000_ffff;
        n = (n | (n << 8)) & 0x00ff_00ff;
        n = (n | (n << 4)) & 0x0f0f_0f0f;
        n = (n | (n << 2)) & 0x3333_3333;
        n = (n | (n << 1)) & 0x5555_5555;
        n
    }
    spread(x) | (spread(y) << 1)
}

/// RA/Dec in degrees -> nested pixel index at `nside`.
pub fn ang2pix_nest(nside: u32, ra_deg: f64, dec_deg: f64) -> u64 {
    let order = nside.trailing_zeros();
    let ns = nside as i64;

    // colatitude/longitude, normalised
    let dec = dec_deg.clamp(-90.0, 90.0);
    let theta = (90.0 - dec).to_radians();
    let mut phi = ra_deg.to_radians() % (2.0 * PI);
    if phi < 0.0 {
        phi += 2.0 * PI;
    }

    let z = theta.cos();
    let za = z.abs();
    let tt = (phi / (PI / 2.0)) % 4.0; // in [0,4)

    let (face, ix, iy);
    if za <= TWO_THIRDS {
        // equatorial belt
        let temp1 = ns as f64 * (0.5 + tt);
        let temp2 = ns as f64 * z * 0.75;
        let jp = (temp1 - temp2) as i64; // ascending edge
        let jm = (temp1 + temp2) as i64; // descending edge
        let ifp = jp >> order; // 0..4
        let ifm = jm >> order;
        face = if ifp == ifm {
            (ifp & 3) + 4
        } else if ifp < ifm {
            ifp & 3
        } else {
            (ifm & 3) + 8
        };
        ix = jm & (ns - 1);
        iy = ns - (jp & (ns - 1)) - 1;
    } else {
        // polar caps
        let ntt = (tt as i64).min(3);
        let tp = tt - ntt as f64;
        let tmp = ns as f64 * (3.0 * (1.0 - za)).max(0.0).sqrt();
        let jp = ((tp * tmp) as i64).min(ns - 1);
        let jm = (((1.0 - tp) * tmp) as i64).min(ns - 1);
        if z >= 0.0 {
            face = ntt;
            ix = ns - jm - 1;
            iy = ns - jp - 1;
        } else {
            face = ntt + 8;
            ix = jp;
            iy = jm;
        }
    }

    (face as u64) * (ns as u64) * (ns as u64) + interleave(ix as u32, iy as u32)
}

/// Undo the bit interleave: recover (x, y) from a within-face nested index.
fn deinterleave(n: u64) -> (u32, u32) {
    fn compact(v: u64) -> u32 {
        let mut n = v & 0x5555_5555;
        n = (n | (n >> 1)) & 0x3333_3333;
        n = (n | (n >> 2)) & 0x0f0f_0f0f;
        n = (n | (n >> 4)) & 0x00ff_00ff;
        n = (n | (n >> 8)) & 0x0000_ffff;
        n as u32
    }
    (compact(n), compact(n >> 1))
}

/// Face -> ring offset, and face -> phi offset. The canonical HEALPix tables.
const JRLL: [i64; 12] = [2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4];
const JPLL: [i64; 12] = [1, 3, 5, 7, 0, 2, 4, 6, 1, 3, 5, 7];

/// Nested pixel index -> (ra_deg, dec_deg) of the pixel centre.
///
/// Standard ring-based inversion (Górski et al. 2005): recover the global
/// ring number `jr` from the face and face-local (ix, iy), then invert the
/// three regimes (north cap / equatorial belt / south cap) in closed form.
/// This is the exact inverse of `ang2pix_nest`'s face/jp/jm construction, so
/// it round-trips through it pixel-for-pixel (see `pix2ang_round_trips_through_ang2pix`).
pub fn pix2ang_nest(nside: u32, pix: u64) -> (f64, f64) {
    let ns = nside as i64;
    let npface = (ns as u64) * (ns as u64);
    let face = (pix / npface) as usize;
    let (ix, iy) = deinterleave(pix % npface);
    let (ix, iy) = (ix as i64, iy as i64);

    // fact1 = 1 / (3 nside^2), fact2 = 2 / (3 nside)
    let fact1 = 1.0 / (3.0 * ns as f64 * ns as f64);
    let fact2 = 2.0 / (3.0 * ns as f64);
    let nl4 = 4 * ns;

    // Ring number, counted from the north pole: 1 ..= 4*nside-1
    let jr = JRLL[face] * ns - ix - iy - 1;

    let (nr, z, kshift): (i64, f64, i64) = if jr < ns {
        // north polar cap
        let nr = jr;
        (nr, 1.0 - (nr * nr) as f64 * fact1, 0)
    } else if jr > 3 * ns {
        // south polar cap
        let nr = nl4 - jr;
        (nr, (nr * nr) as f64 * fact1 - 1.0, 0)
    } else {
        // equatorial belt
        (ns, (2 * ns - jr) as f64 * fact2, (jr - ns) & 1)
    };

    // Longitude index within the ring.
    let mut jp = (JPLL[face] * nr + ix - iy + 1 + kshift) / 2;
    if jp > nl4 {
        jp -= nl4;
    }
    if jp < 1 {
        jp += nl4;
    }

    let phi = (jp as f64 - (kshift as f64 + 1.0) * 0.5) * (PI / 2.0 / nr as f64);
    let dec = z.clamp(-1.0, 1.0).asin().to_degrees();
    let ra = phi.to_degrees().rem_euclid(360.0);
    (ra, dec)
}

/// An upper bound on the angular distance from a pixel centre to any point
/// inside that pixel, in degrees.
///
/// Used to pad disc queries so a cell that merely *overlaps* the disc is
/// never missed. It must be an OVER-estimate: an under-estimate silently
/// drops catalogue stars near the disc edge, which would look like a sparse
/// field rather than like a bug. The bound is the radius of a spherical cap
/// of four times the pixel area, which is comfortably larger than the true
/// maximum at every nside and is verified by `padding_bound_covers_every_point`.
pub fn max_pixrad_deg(nside: u32) -> f64 {
    let area = 4.0 * PI / npix(nside) as f64;
    let cap = 4.0 * area;
    let cos_r = 1.0 - cap / (2.0 * PI);
    cos_r.clamp(-1.0, 1.0).acos().to_degrees()
}

/// All nested cells at `nside` that could overlap a disc of `radius_deg`.
/// Brute-force scan: correct by construction, ~50k evaluations at nside=64.
pub fn cells_in_disc(nside: u32, ra_deg: f64, dec_deg: f64, radius_deg: f64) -> Vec<u64> {
    let limit = radius_deg + max_pixrad_deg(nside);
    let (r0, d0) = (ra_deg.to_radians(), dec_deg.to_radians());
    let (sin_d0, cos_d0) = (d0.sin(), d0.cos());
    let mut out = Vec::new();
    for pix in 0..npix(nside) {
        let (pra, pdec) = pix2ang_nest(nside, pix);
        let (pr, pd) = (pra.to_radians(), pdec.to_radians());
        let cos_sep = sin_d0 * pd.sin() + cos_d0 * pd.cos() * (pr - r0).cos();
        if cos_sep.clamp(-1.0, 1.0).acos().to_degrees() <= limit {
            out.push(pix);
        }
    }
    out
}
