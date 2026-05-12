//! Alignment-pattern fetch + search (qrdec.c:1679–1907, Phase 4-F chunk 4).
//!
//! The alignment pattern is a 5×5 module square embedded in QR codes from
//! version 2 onward. Its on/off cells form the 25-bit constant `0x1F8D63F`:
//!
//! ```text
//! █ █ █ █ █
//! █ . . . █
//! █ . █ . █
//! █ . . . █
//! █ █ █ █ █
//! ```
//!
//! `fetch` reads a 5×5 sample around a candidate center and returns the bits
//! as a u32. `search` projects an initial template through a `QrHomCell`,
//! then walks concentric square rings around the projected center, picking
//! the location with the smallest Hamming distance to `0x1F8D63F`.

use super::geom::{qr_hom_cell_fproject, QrHomCell, QrPoint, QR_ALIGN_SUBPREC, QR_FINDER_SUBPREC};
use super::qr_finder::{qr_finder_locate_crossing, qr_hamming_dist, qr_img_get_bit};
use super::util::{qr_divround, qr_maxi};

/// The expected 25-bit value of a clean alignment pattern.
pub const QR_ALIGNMENT_PATTERN: u32 = 0x1F8D63F;

/// Fetch the 25 bits of the alignment pattern template centered at
/// `(x0, y0)` in the image. The template positions are taken from `p`;
/// `(x0, y0)` shifts them so the template center lands at `(x0, y0)`.
pub fn qr_alignment_pattern_fetch(
    p: &[[QrPoint; 5]; 5],
    x0: i32, y0: i32,
    img: &[u8], width: i32, height: i32,
) -> u32 {
    let dx = x0 - p[2][2][0];
    let dy = y0 - p[2][2][1];
    let mut v = 0u32;
    let mut k = 0u32;
    for i in 0..5 {
        for j in 0..5 {
            v |= (qr_img_get_bit(img, width, height, p[i][j][0] + dx, p[i][j][1] + dy) as u32) << k;
            k += 1;
        }
    }
    v
}

/// Search for an alignment pattern within `r` modules of `(u, v)`.
/// Writes the (subpixel) center to `pcenter` and returns `0` on success.
/// Returns `-1` when the best Hamming distance is still too large; in that
/// case `pcenter` holds the initial template-center estimate.
#[allow(clippy::too_many_arguments, clippy::needless_range_loop)]
pub fn qr_alignment_pattern_search(
    pcenter: &mut QrPoint,
    cell: &QrHomCell,
    u_in: i32, v_in: i32,
    r: i32,
    img: &[u8], width: i32, height: i32,
) -> i32 {
    // Project the 5×5 template through the homography cell *once* — we slide
    // it around afterwards (per zbar's rationale: no good reason to think
    // re-projecting at each candidate center would be more accurate).
    let mut p: [[QrPoint; 5]; 5] = [[[0; 2]; 5]; 5];
    let mut u = (u_in - 2) - cell.u0;
    let mut v = (v_in - 2) - cell.v0;
    let mut x0 = cell.fwd[0][0].wrapping_mul(u)
        .wrapping_add(cell.fwd[0][1].wrapping_mul(v))
        .wrapping_add(cell.fwd[0][2]);
    let mut y0 = cell.fwd[1][0].wrapping_mul(u)
        .wrapping_add(cell.fwd[1][1].wrapping_mul(v))
        .wrapping_add(cell.fwd[1][2]);
    let mut w0 = cell.fwd[2][0].wrapping_mul(u)
        .wrapping_add(cell.fwd[2][1].wrapping_mul(v))
        .wrapping_add(cell.fwd[2][2]);
    let dxdu = cell.fwd[0][0]; let dydu = cell.fwd[1][0]; let dwdu = cell.fwd[2][0];
    let dxdv = cell.fwd[0][1]; let dydv = cell.fwd[1][1]; let dwdv = cell.fwd[2][1];

    for i in 0..5 {
        let mut x = x0; let mut y = y0; let mut w = w0;
        for j in 0..5 {
            qr_hom_cell_fproject(&mut p[i][j], cell, x, y, w);
            x = x.wrapping_add(dxdu);
            y = y.wrapping_add(dydu);
            w = w.wrapping_add(dwdu);
        }
        x0 = x0.wrapping_add(dxdv);
        y0 = y0.wrapping_add(dydv);
        w0 = w0.wrapping_add(dwdv);
    }

    let mut bestx = p[2][2][0];
    let mut besty = p[2][2][1];
    let mut best_match = qr_alignment_pattern_fetch(&p, bestx, besty, img, width, height);
    let mut best_dist = qr_hamming_dist(best_match, QR_ALIGNMENT_PATTERN, 25);

    if best_dist > 0 {
        // Search concentric square rings up to `r` modules out.
        u = u_in - cell.u0;
        v = v_in - cell.v0;
        // x,y,w in QR_ALIGN_SUBPREC resolution.
        let mut x = cell.fwd[0][0].wrapping_mul(u)
            .wrapping_add(cell.fwd[0][1].wrapping_mul(v))
            .wrapping_add(cell.fwd[0][2])
            .wrapping_shl(QR_ALIGN_SUBPREC as u32);
        let mut y = cell.fwd[1][0].wrapping_mul(u)
            .wrapping_add(cell.fwd[1][1].wrapping_mul(v))
            .wrapping_add(cell.fwd[1][2])
            .wrapping_shl(QR_ALIGN_SUBPREC as u32);
        let mut w = cell.fwd[2][0].wrapping_mul(u)
            .wrapping_add(cell.fwd[2][1].wrapping_mul(v))
            .wrapping_add(cell.fwd[2][2])
            .wrapping_shl(QR_ALIGN_SUBPREC as u32);

        let r_search = r << QR_ALIGN_SUBPREC;
        'outer: for i in 1..r_search {
            let side_len = (i << 1) - 1;
            // Move to the top-left corner of the new ring.
            x = x.wrapping_sub(dxdu).wrapping_sub(dxdv);
            y = y.wrapping_sub(dydu).wrapping_sub(dydv);
            w = w.wrapping_sub(dwdu).wrapping_sub(dwdv);

            for j in 0..(4 * side_len) {
                let mut pc: QrPoint = [0; 2];
                qr_hom_cell_fproject(&mut pc, cell, x, y, w);
                let m = qr_alignment_pattern_fetch(&p, pc[0], pc[1], img, width, height);
                let d = qr_hamming_dist(m, QR_ALIGNMENT_PATTERN, best_dist + 1);
                if d < best_dist {
                    best_match = m;
                    best_dist = d;
                    bestx = pc[0];
                    besty = pc[1];
                }
                if j < 2 * side_len {
                    let dir = (j >= side_len) as usize;
                    x = x.wrapping_add(cell.fwd[0][dir]);
                    y = y.wrapping_add(cell.fwd[1][dir]);
                    w = w.wrapping_add(cell.fwd[2][dir]);
                } else {
                    let dir = (j >= 3 * side_len) as usize;
                    x = x.wrapping_sub(cell.fwd[0][dir]);
                    y = y.wrapping_sub(cell.fwd[1][dir]);
                    w = w.wrapping_sub(cell.fwd[2][dir]);
                }
                if best_dist == 0 { break 'outer; }
            }
        }
    }

    if best_dist > 6 {
        pcenter[0] = p[2][2][0];
        pcenter[1] = p[2][2][1];
        return -1;
    }

    // Fine-tune center by averaging crossings along 8 symmetric lines.
    let dx = bestx - p[2][2][0];
    let dy = besty - p[2][2][1];
    let mut nc = [0i32; 4];
    let mut c = [[0i32; 2]; 4];

    const MASK_TESTS: [[u32; 2]; 8] = [
        [0x1040041, 0x1000001], [0x0041040, 0x0001000],
        [0x0110110, 0x0100010], [0x0011100, 0x0001000],
        [0x0420084, 0x0400004], [0x0021080, 0x0001000],
        [0x0006C00, 0x0004400], [0x0003800, 0x0001000],
    ];
    // [col, row] indexing — zbar uses {0,0} as (col=0, row=0).
    const MASK_COORDS: [[u8; 2]; 8] = [
        [0, 0], [1, 1], [4, 0], [3, 1], [2, 0], [2, 1], [0, 2], [1, 2],
    ];

    for i in 0..8 {
        if best_match & MASK_TESTS[i][0] == MASK_TESTS[i][1] {
            let col = MASK_COORDS[i][0] as usize;
            let row = MASK_COORDS[i][1] as usize;
            let x0 = (p[row][col][0] + dx) >> QR_FINDER_SUBPREC;
            if x0 < 0 || x0 >= width { continue; }
            let y0 = (p[row][col][1] + dy) >> QR_FINDER_SUBPREC;
            if y0 < 0 || y0 >= height { continue; }
            let x1 = (p[4 - row][4 - col][0] + dx) >> QR_FINDER_SUBPREC;
            if x1 < 0 || x1 >= width { continue; }
            let y1 = (p[4 - row][4 - col][1] + dy) >> QR_FINDER_SUBPREC;
            if y1 < 0 || y1 >= height { continue; }
            let mut pc: QrPoint = [0; 2];
            if qr_finder_locate_crossing(img, width, height, x0, y0, x1, y1, (i & 1) as i32, &mut pc) == 0 {
                let mut cx = pc[0] - bestx;
                let mut cy = pc[1] - besty;
                let w;
                if i & 1 != 0 {
                    // Crossings around the center dot get 3× weight.
                    w = 3;
                    cx = cx.wrapping_add(cx << 1);
                    cy = cy.wrapping_add(cy << 1);
                } else {
                    w = 1;
                }
                nc[i >> 1] += w;
                c[i >> 1][0] += cx;
                c[i >> 1][1] += cy;
            }
        }
    }

    // Combine orthogonal pairs.
    for i in 0..2 {
        let a = nc[i << 1];
        let b = nc[(i << 1) | 1];
        if a != 0 && b != 0 {
            let w = qr_maxi(a, b);
            c[i << 1][0] = qr_divround(w * (b * c[i << 1][0] + a * c[(i << 1) | 1][0]), a * b);
            c[i << 1][1] = qr_divround(w * (b * c[i << 1][1] + a * c[(i << 1) | 1][1]), a * b);
            nc[i << 1] = w << 1;
        } else {
            c[i << 1][0] += c[(i << 1) | 1][0];
            c[i << 1][1] += c[(i << 1) | 1][1];
            nc[i << 1] += b;
        }
    }
    c[0][0] += c[2][0];
    c[0][1] += c[2][1];
    nc[0] += nc[2];

    if nc[0] != 0 {
        let adj_dx = qr_divround(c[0][0], nc[0]);
        let adj_dy = qr_divround(c[0][1], nc[0]);
        let m = qr_alignment_pattern_fetch(&p, bestx + adj_dx, besty + adj_dy, img, width, height);
        let d = qr_hamming_dist(m, QR_ALIGNMENT_PATTERN, best_dist + 1);
        if d <= best_dist {
            bestx += adj_dx;
            besty += adj_dy;
        }
    }
    pcenter[0] = bestx;
    pcenter[1] = besty;
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alignment_constant_has_expected_bits() {
        // The pattern is a hollow 5×5 ring with a single center dot.
        // Counting set bits: 16 ring + 1 center = 17.
        assert_eq!(QR_ALIGNMENT_PATTERN.count_ones(), 17);
        // Bit 0 = row 0 col 0 = ring corner = set.
        assert_eq!(QR_ALIGNMENT_PATTERN & 1, 1);
        // Center bit = row 2 col 2 = bit 12.
        assert_eq!((QR_ALIGNMENT_PATTERN >> 12) & 1, 1);
        // An interior cell (row 1 col 1 = bit 6) should be 0.
        assert_eq!((QR_ALIGNMENT_PATTERN >> 6) & 1, 0);
    }

    #[test]
    fn fetch_on_synthetic_perfect_pattern() {
        // 9×9 image with the 5×5 alignment pattern centered at (4, 4),
        // surrounded by white margin. We construct `p` so all 25 template
        // coords point at the pattern's pixels.
        let mut img = vec![0u8; 81];
        // Light cells = 0; dark cells = 0xFF; pattern starts at (2, 2).
        for r in 0..5 {
            for cc in 0..5 {
                let bit = (QR_ALIGNMENT_PATTERN >> (r * 5 + cc)) & 1;
                if bit == 1 {
                    img[(2 + r) * 9 + (2 + cc)] = 0xFF;
                }
            }
        }
        // Template `p` in subpel resolution (shifted by QR_FINDER_SUBPREC).
        let shift = QR_FINDER_SUBPREC as u32;
        let mut p: [[QrPoint; 5]; 5] = [[[0; 2]; 5]; 5];
        for r in 0..5 {
            for cc in 0..5 {
                p[r][cc] = [(2 + cc as i32) << shift, (2 + r as i32) << shift];
            }
        }
        let v = qr_alignment_pattern_fetch(&p, p[2][2][0], p[2][2][1], &img, 9, 9);
        assert_eq!(v, QR_ALIGNMENT_PATTERN, "fetched 0x{:X}, expected 0x{:X}", v, QR_ALIGNMENT_PATTERN);
    }
}
