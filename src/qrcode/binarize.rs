//! Adaptive binarization for QR decoding.
//! Rust port of zbar/qrcode/binarize.c.
//! Original Copyright (C) 2008-2009 Timothy B. Terriberry (tterribe@xiph.org)
//! LGPL-2.1-or-later
//!
//! Only the simple adaptive thresholder (the outer `#else` branch — Sauvola
//! and Gatos in the same file are gated out with `#if 0`) is reachable from
//! `qrdec.c::_zbar_qr_decode`, so it is the only function ported here. The
//! wiener_filter and sauvola/gatos variants the header still declares are
//! dead code in the zbar 0.10 build.
//!
//! Algorithm: integral-column sliding window. For each pixel `g` at (x,y),
//! compute the sum `m` of all grayscale values inside a `windw × windh`
//! window centered there (clamped to the image), and mark the output mask
//! 0xFF when `(g + 3) * windw * windh < m` — i.e. when the pixel is darker
//! than the local mean minus 3. Otherwise mark 0.

use super::util::{qr_maxi, qr_mini};

/// Produces a binarization mask (0 = light, 0xFF = dark) the same size as
/// the input image. Returns an empty Vec on degenerate (0×N or N×0) inputs.
pub fn qr_binarize(img: &[u8], width: i32, height: i32) -> Vec<u8> {
    if width <= 0 || height <= 0 {
        return Vec::new();
    }
    let w = width as usize;
    let h = height as usize;
    assert_eq!(img.len(), w * h, "image buffer size mismatch");

    let mut mask = vec![0u8; w * h];

    // Window size: ≥16, doubling until 1<<log ≥ ceil(dim/8); capped at 1<<7 = 128.
    // C: `for(logwindw=4; logwindw<8 && (1<<logwindw) < (_width+7>>3); logwindw++);`
    let mut logwindw: u32 = 4;
    while logwindw < 8 && (1u32 << logwindw) < (((width + 7) >> 3) as u32) {
        logwindw += 1;
    }
    let mut logwindh: u32 = 4;
    while logwindh < 8 && (1u32 << logwindh) < (((height + 7) >> 3) as u32) {
        logwindh += 1;
    }
    let windw = 1i32 << logwindw;
    let windh = 1i32 << logwindh;

    let mut col_sums = vec![0u32; w];

    // Initial column sums: top half of vertical window (replicate top row by
    // weight `windh/2`, then walk down to fill the rest).
    // C: col_sums[x] = (g << (logwindh-1)) + g  ⇒ g * (1 + 1<<(logwindh-1))
    for x in 0..w {
        let g = img[x] as u32;
        col_sums[x] = (g << (logwindh - 1)) + g;
    }
    for y in 1..(windh >> 1) {
        let y1offs = qr_mini(y, height - 1) as usize * w;
        for x in 0..w {
            col_sums[x] += img[y1offs + x] as u32;
        }
    }

    for y in 0..height {
        // Initialize window sum from col_sums[0] (replicated leftmost column).
        let mut m: u32 = (col_sums[0] << (logwindw - 1)) + col_sums[0];
        for x in 1..(windw >> 1) {
            let x1 = qr_mini(x, width - 1) as usize;
            m += col_sums[x1];
        }
        for x in 0..width {
            let row_base = y as usize * w;
            let g = img[row_base + x as usize] as u32;
            // Threshold test: (g + 3) << (logwindw + logwindh) < m
            //   m is the unscaled window sum (windw*windh times the mean).
            //   So the inequality is "pixel + 3 < mean".
            let lhs = (g + 3) << (logwindw + logwindh);
            mask[row_base + x as usize] = if lhs < m { 0xFF } else { 0 };

            // Slide the window one column to the right.
            if x + 1 < width {
                let x0 = qr_maxi(0, x - (windw >> 1)) as usize;
                let x1 = qr_mini(x + (windw >> 1), width - 1) as usize;
                m = m.wrapping_add(col_sums[x1]).wrapping_sub(col_sums[x0]);
            }
        }
        // Slide the column sums one row down.
        if y + 1 < height {
            let y0offs = qr_maxi(0, y - (windh >> 1)) as usize * w;
            let y1offs = qr_mini(y + (windh >> 1), height - 1) as usize * w;
            for x in 0..w {
                col_sums[x] = col_sums[x]
                    .wrapping_sub(img[y0offs + x] as u32)
                    .wrapping_add(img[y1offs + x] as u32);
            }
        }
    }

    mask
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_inputs_return_empty() {
        assert!(qr_binarize(&[], 0, 0).is_empty());
        assert!(qr_binarize(&[], 10, 0).is_empty());
        assert!(qr_binarize(&[], 0, 10).is_empty());
    }

    #[test]
    fn uniform_image_threshold_3() {
        // For a uniform image at value g, mean m = g * windw * windh, so the
        // test "(g+3) * N < m" reduces to "(g+3)*N < g*N" → 3*N < 0, never true.
        // So a flat image should produce an all-zero mask.
        for &val in &[0u8, 1, 127, 200, 255] {
            let img = vec![val; 32 * 32];
            let mask = qr_binarize(&img, 32, 32);
            assert!(mask.iter().all(|&m| m == 0),
                "uniform value {} produced non-zero mask", val);
        }
    }

    #[test]
    fn dark_spot_on_bright_background() {
        // 32×32 image, background 240, single 8×8 dark spot at center (val 0).
        // The dark pixels should become 0xFF (foreground); bright pixels stay 0.
        let mut img = vec![240u8; 32 * 32];
        for y in 12..20 {
            for x in 12..20 {
                img[y * 32 + x] = 0;
            }
        }
        let mask = qr_binarize(&img, 32, 32);
        // Center of the dark spot must be foreground.
        assert_eq!(mask[16 * 32 + 16], 0xFF);
        // Far corner must be background.
        assert_eq!(mask[0], 0);
    }

    #[test]
    fn dimensions_below_window_clamp() {
        // 8×8 image — smaller than the minimum window (16×16). Algorithm
        // should still complete without panicking and produce a valid mask.
        let mut img = vec![0u8; 8 * 8];
        for i in 0..32 { img[i] = 200; }   // top half bright
        for i in 32..64 { img[i] = 50; }   // bottom half dark
        let mask = qr_binarize(&img, 8, 8);
        assert_eq!(mask.len(), 64);
    }
}
