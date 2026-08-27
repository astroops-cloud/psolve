use psolve_index::healpix::{ang2pix_nest, cells_in_disc, max_pixrad_deg, npix, pix2ang_nest};

fn angsep_deg(ra1: f64, dec1: f64, ra2: f64, dec2: f64) -> f64 {
    let (r1, d1, r2, d2) = (
        ra1.to_radians(), dec1.to_radians(), ra2.to_radians(), dec2.to_radians(),
    );
    let (dr, dd) = (r2 - r1, d2 - d1);
    let a = (dd / 2.0).sin().powi(2) + d1.cos() * d2.cos() * (dr / 2.0).sin().powi(2);
    2.0 * a.sqrt().min(1.0).asin().to_degrees()
}

#[test]
fn pix2ang_round_trips_through_ang2pix() {
    for &nside in &[1u32, 4, 64, 256] {
        for pix in 0..npix(nside) {
            let (ra, dec) = pix2ang_nest(nside, pix);
            assert_eq!(
                ang2pix_nest(nside, ra, dec), pix,
                "nside {nside} pix {pix} centre ({ra},{dec}) did not map back"
            );
        }
    }
}

#[test]
fn disc_contains_the_centre_cell() {
    for &(ra, dec) in &[(0.0, 0.0), (274.689, -13.811), (45.0, 89.0), (200.0, -89.5)] {
        let cells = cells_in_disc(64, ra, dec, 1.5);
        assert!(
            cells.contains(&ang2pix_nest(64, ra, dec)),
            "disc at ({ra},{dec}) omitted its own centre cell"
        );
    }
}

#[test]
fn disc_is_a_superset_of_a_brute_force_point_test() {
    // Every cell whose CENTRE lies within the radius must be returned.
    let (ra, dec, r) = (274.689, -13.811, 2.0);
    let got = cells_in_disc(64, ra, dec, r);
    for pix in 0..npix(64) {
        let (pra, pdec) = pix2ang_nest(64, pix);
        if angsep_deg(ra, dec, pra, pdec) <= r {
            assert!(got.contains(&pix), "cell {pix} inside radius but not returned");
        }
    }
}

#[test]
fn disc_returns_sorted_unique_cells() {
    let cells = cells_in_disc(64, 100.0, 20.0, 3.0);
    let mut sorted = cells.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(cells, sorted, "cells must be sorted and unique");
}

#[test]
fn full_sky_radius_returns_every_cell() {
    assert_eq!(cells_in_disc(4, 0.0, 0.0, 180.0).len() as u64, npix(4));
}

#[test]
fn max_pixrad_shrinks_with_nside() {
    assert!(max_pixrad_deg(64) < max_pixrad_deg(32));
    assert!(max_pixrad_deg(64) > 0.0);
}

#[test]
fn the_gaia_level_12_mismatch_is_a_genuine_boundary_case() {
    // Task 2 established that at most one fixture row disagrees with Gaia at
    // level 12. Now that pix2ang_nest exists, prove that disagreement is a
    // star sitting on a pixel edge rather than a wrong pixel choice.
    let raw = include_str!("fixtures/gaia_healpix.csv");
    let pixel_scale_deg =
        (4.0 * std::f64::consts::PI / npix(4096) as f64).sqrt().to_degrees();

    for line in raw.lines().filter(|l| !l.starts_with('#')).skip(1) {
        if line.trim().is_empty() {
            continue;
        }
        let f: Vec<&str> = line.split(',').collect();
        let (ra, dec): (f64, f64) = (f[1].parse().unwrap(), f[2].parse().unwrap());
        let hp12: u64 = f[3].parse().unwrap();
        let got = ang2pix_nest(4096, ra, dec);
        if got == hp12 {
            continue;
        }
        let (ora, odec) = pix2ang_nest(4096, got);
        let (gra, gdec) = pix2ang_nest(4096, hp12);
        let d_ours = angsep_deg(ra, dec, ora, odec);
        let d_gaia = angsep_deg(ra, dec, gra, gdec);
        let centres = angsep_deg(ora, odec, gra, gdec);
        assert!(
            centres <= 1.5 * pixel_scale_deg,
            "source {}: our pixel and Gaia's are {centres} deg apart — not \
             adjacent, so this is a bug rather than a boundary case",
            f[0]
        );
        assert!(
            (d_ours - d_gaia).abs() < 0.25 * pixel_scale_deg,
            "source {}: star is {d_ours} from our centre but {d_gaia} from \
             Gaia's — not equidistant, so our pixel choice is wrong",
            f[0]
        );
    }
}

#[test]
fn padding_bound_covers_every_point() {
    // The property cells_in_disc depends on: any point is within
    // max_pixrad_deg of the centre of the cell that contains it. If this is
    // ever false, disc queries silently drop stars near the edge.
    for &nside in &[4u32, 64, 256] {
        let bound = max_pixrad_deg(nside);
        let mut worst: f64 = 0.0;
        // deterministic pseudo-random sweep, no rand dependency
        for i in 0..20_000u64 {
            let ra = (i as f64 * 0.618_033_988_749_9 * 360.0) % 360.0;
            let z = ((i as f64 * 0.381_966_011_25) % 2.0) - 1.0;
            let dec = z.asin().to_degrees();
            let pix = ang2pix_nest(nside, ra, dec);
            let (cra, cdec) = pix2ang_nest(nside, pix);
            let sep = angsep_deg(ra, dec, cra, cdec);
            worst = worst.max(sep);
            assert!(
                sep <= bound,
                "nside {nside}: point ({ra},{dec}) is {sep} deg from its cell centre, \
                 bound is {bound}"
            );
        }
        assert!(worst > 0.0, "sweep produced no separations at nside {nside}");
    }
}
