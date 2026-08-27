//! A tiled background surface.
//!
//! A single global level works on a flat field and fails on a gradient -- moon,
//! light pollution, amp glow. It is also why a prototype's brightest "star" was
//! 63,104 pixels of Eagle Nebula: with one global threshold, extended emission
//! is a source. Tiles much larger than a star but much smaller than a gradient
//! make nebulosity part of the background, which is right, because nebulosity
//! is not a star.

use crate::fits::Image;

/// Per-tile sky level and noise, bilinearly interpolated between tile centres.
#[derive(Debug, Clone)]
pub struct Background {
    pub tile: usize,
    pub tx: usize,
    pub ty: usize,
    pub level: Vec<f32>,
    pub noise: Vec<f32>,
}

/// Sort a tile's pixels once; both statistics below read from the result.
///
/// Making the sort explicit removes a hidden side effect: `median_of_sorted` used to sort
/// in place and `robust_sigma` silently depended on that, so reordering the two
/// calls would have produced a wrong sigma with nothing failing.
fn sort_pixels(v: &mut [f32]) {
    v.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
}

/// Median of an ALREADY SORTED slice. A median, not a mean, so a bright star
/// inside a tile cannot drag the level upward.
fn median_of_sorted(v: &[f32]) -> f32 {
    if v.is_empty() {
        0.0
    } else {
        v[v.len() / 2]
    }
}

/// Robust sigma from an ALREADY SORTED slice: the gap between the median and
/// the 15.87th percentile. The faint half of a sky distribution is clean; the
/// bright half is contaminated by stars, so only the lower half is trusted.
fn robust_sigma(sorted: &[f32]) -> f32 {
    if sorted.len() < 8 {
        return 1.0;
    }
    let med = sorted[sorted.len() / 2];
    let lo = sorted[sorted.len() * 1587 / 10000];
    (med - lo).max(1e-3)
}

pub fn estimate(img: &Image, tile: usize) -> Background {
    let tile = tile.max(8);
    let tx = img.nx.div_ceil(tile).max(1);
    let ty = img.ny.div_ceil(tile).max(1);
    let mut level = vec![0f32; tx * ty];
    let mut noise = vec![1f32; tx * ty];
    let mut buf: Vec<f32> = Vec::with_capacity(tile * tile);

    for gy in 0..ty {
        for gx in 0..tx {
            buf.clear();
            let x0 = gx * tile;
            let y0 = gy * tile;
            let x1 = (x0 + tile).min(img.nx);
            let y1 = (y0 + tile).min(img.ny);
            for y in y0..y1 {
                buf.extend_from_slice(&img.px[y * img.nx + x0..y * img.nx + x1]);
            }
            sort_pixels(&mut buf);
            level[gy * tx + gx] = median_of_sorted(&buf);
            noise[gy * tx + gx] = robust_sigma(&buf);
        }
    }
    Background { tile, tx, ty, level, noise }
}

impl Background {
    fn sample(&self, grid: &[f32], x: usize, y: usize) -> f32 {
        if self.tx == 0 || self.ty == 0 {
            return 0.0;
        }
        // Continuous position in tile-centre space, then bilinear.
        let fx = (x as f32 + 0.5) / self.tile as f32 - 0.5;
        let fy = (y as f32 + 0.5) / self.tile as f32 - 0.5;
        let cx = fx.clamp(0.0, (self.tx - 1) as f32);
        let cy = fy.clamp(0.0, (self.ty - 1) as f32);
        let x0 = cx.floor() as usize;
        let y0 = cy.floor() as usize;
        let x1 = (x0 + 1).min(self.tx - 1);
        let y1 = (y0 + 1).min(self.ty - 1);
        let dx = cx - x0 as f32;
        let dy = cy - y0 as f32;
        let a = grid[y0 * self.tx + x0];
        let b = grid[y0 * self.tx + x1];
        let c = grid[y1 * self.tx + x0];
        let d = grid[y1 * self.tx + x1];
        a * (1.0 - dx) * (1.0 - dy) + b * dx * (1.0 - dy) + c * (1.0 - dx) * dy + d * dx * dy
    }

    pub fn level_at(&self, x: usize, y: usize) -> f32 {
        self.sample(&self.level, x, y)
    }

    pub fn noise_at(&self, x: usize, y: usize) -> f32 {
        self.sample(&self.noise, x, y)
    }

    pub fn median_noise(&self) -> f32 {
        let mut v = self.noise.clone();
        sort_pixels(&mut v);
        median_of_sorted(&v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fits::Image;

    fn flat(nx: usize, ny: usize, v: f32) -> Image {
        Image { nx, ny, px: vec![v; nx * ny], binned: 1 }
    }

    #[test]
    fn a_flat_field_has_a_flat_background_and_near_zero_noise() {
        let img = flat(256, 256, 500.0);
        let bg = estimate(&img, 64);
        for y in (0..256).step_by(37) {
            for x in (0..256).step_by(41) {
                assert!((bg.level_at(x, y) - 500.0).abs() < 1.0, "level at {x},{y}");
            }
        }
        assert!(bg.median_noise() < 1.0, "noise was {}", bg.median_noise());
    }

    #[test]
    fn a_linear_gradient_is_tracked_not_averaged_away() {
        // 0 on the left, 1000 on the right. A global level would be 500
        // everywhere and would mis-threshold both halves.
        let (nx, ny) = (256usize, 256usize);
        let mut img = flat(nx, ny, 0.0);
        for y in 0..ny {
            for x in 0..nx {
                img.px[y * nx + x] = 1000.0 * x as f32 / (nx - 1) as f32;
            }
        }
        let bg = estimate(&img, 32);
        for &(x, y) in &[(20usize, 128usize), (128, 128), (230, 128)] {
            let want = 1000.0 * x as f32 / (nx - 1) as f32;
            assert!(
                (bg.level_at(x, y) - want).abs() < 60.0,
                "at x={x} level {} should track {want}",
                bg.level_at(x, y)
            );
        }
    }

    #[test]
    fn noise_reflects_the_real_scatter() {
        // Deterministic pseudo-noise around 100 with a known spread.
        let (nx, ny) = (128usize, 128usize);
        let mut img = flat(nx, ny, 0.0);
        for i in 0..nx * ny {
            let t = ((i * 2654435761usize) % 1000) as f32 / 1000.0;
            img.px[i] = 100.0 + (t - 0.5) * 40.0;
        }
        let bg = estimate(&img, 32);
        let n = bg.median_noise();
        assert!(n > 2.0 && n < 25.0, "noise estimate {n} is implausible");
    }

    #[test]
    fn a_bright_star_does_not_drag_its_tile_upward() {
        // The reason the level is a median rather than a mean.
        let (nx, ny) = (64usize, 64usize);
        let mut img = flat(nx, ny, 200.0);
        for y in 30..34 {
            for x in 30..34 {
                img.px[y * nx + x] = 60000.0;
            }
        }
        let bg = estimate(&img, 32);
        assert!(
            (bg.level_at(32, 32) - 200.0).abs() < 30.0,
            "a 16-pixel star moved the background to {}",
            bg.level_at(32, 32)
        );
    }

    #[test]
    fn extended_nebulosity_becomes_background() {
        // The failure this whole task exists for: a large smooth bright region
        // must be absorbed, so it is never detected as a source.
        let (nx, ny) = (256usize, 256usize);
        let mut img = flat(nx, ny, 100.0);
        for y in 64..192 {
            for x in 64..192 {
                img.px[y * nx + x] = 4000.0;
            }
        }
        let bg = estimate(&img, 32);
        assert!(
            bg.level_at(128, 128) > 3000.0,
            "nebulosity should be absorbed into the background, got {}",
            bg.level_at(128, 128)
        );
    }

    #[test]
    fn an_image_smaller_than_one_tile_still_works() {
        let img = flat(10, 10, 42.0);
        let bg = estimate(&img, 64);
        assert!((bg.level_at(5, 5) - 42.0).abs() < 1.0);
    }

    #[test]
    fn out_of_range_coordinates_clamp_to_the_nearest_tile() {
        // Previously this only checked "does not panic", which would also pass
        // if the function returned an arbitrary non-panicking value.
        let img = flat(64, 64, 7.0);
        let bg = estimate(&img, 32);
        let inside = bg.level_at(63, 63);
        assert!((bg.level_at(9999, 9999) - inside).abs() < 1e-6, "must clamp, not extrapolate");
        assert!(bg.noise_at(9999, 9999).is_finite());
    }

    #[test]
    fn the_two_statistics_do_not_depend_on_call_order() {
        // Pins the contract that used to be an undocumented side effect.
        let mut a: Vec<f32> = (0..100).map(|i| ((i * 37) % 100) as f32).collect();
        let mut b = a.clone();
        sort_pixels(&mut a);
        let (m1, s1) = (median_of_sorted(&a), robust_sigma(&a));
        sort_pixels(&mut b);
        let (s2, m2) = (robust_sigma(&b), median_of_sorted(&b)); // reversed order
        assert_eq!(m1, m2);
        assert_eq!(s1, s2);
    }
}
