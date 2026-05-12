//! Sampling-grid data reads, mask fill, RS block unpack — and ultimately
//! the bit-stream parse that produces the QR payload. Rust port of qrdec.c
//! lines 2830 onward. Phase 4-D builds this incrementally; each chunk
//! appends to this module.

use super::geom::{qr_hom_cell_fproject, QrPoint, QR_INT_BITS};
use super::qr_finder::qr_img_get_bit;
use super::sampling_grid::{
    qr_sampling_grid_is_in_fp, QrSamplingGrid, QR_INT_LOGBITS,
};

#[inline]
fn stride_for(dim: i32) -> usize {
    ((dim + QR_INT_BITS - 1) >> QR_INT_LOGBITS) as usize
}

/// Fill `mask` with the data mask corresponding to `pattern` (0..=7).
/// Mirrors `qr_data_mask_fill` (qrdec.c:2830). Bits are stored column-wise.
pub fn qr_data_mask_fill(mask: &mut [u32], dim: i32, pattern: i32) {
    let stride = stride_for(dim);
    match pattern & 7 {
        0 => {
            // (i + j + 1) & 1 == 0
            let mut m: u8 = 0x55;
            for j in 0..dim as usize {
                let start = j * stride;
                for w in 0..stride {
                    mask[start + w] = u32::from_ne_bytes([m, m, m, m]);
                }
                m ^= 0xFF;
            }
        }
        1 => {
            // (i + 1) & 1 == 0 — column-invariant, all 0x55 words.
            for w in mask[..dim as usize * stride].iter_mut() {
                *w = u32::from_ne_bytes([0x55, 0x55, 0x55, 0x55]);
            }
        }
        2 => {
            // (j + 1) % 3 == 0 — rotating 8-bit pattern across columns.
            let mut m: u32 = 0xFF;
            for j in 0..dim as usize {
                let byte = (m & 0xFF) as u8;
                let start = j * stride;
                for w in 0..stride {
                    mask[start + w] = u32::from_ne_bytes([byte, byte, byte, byte]);
                }
                m = (m << 8) | (m >> 16);
            }
        }
        3 => {
            // (i + j + 1) % 3 == 0 — 3-bit rotation per word, 1-bit per row.
            let mut mj: u32 = 0;
            // mj seeded with bits at every multiple-of-3 position.
            for i in 0..((QR_INT_BITS + 2) / 3) {
                mj |= 1 << (3 * i);
            }
            for j in 0..dim as usize {
                let mut mi = mj;
                for i in 0..stride {
                    mask[j * stride + i] = mi;
                    // rotate right by QR_INT_BITS % 3 = 32 % 3 = 2
                    mi = (mi >> (QR_INT_BITS as u32 % 3)) | (mi << (3 - (QR_INT_BITS as u32 % 3)));
                }
                // rotate mj right by 1 within 3-bit groups: (mj >> 1) | (mj << 2)
                mj = (mj >> 1) | (mj << 2);
            }
        }
        4 => {
            // ((i >> 1) + (j / 3) + 1) & 1
            let mut m: u32 = 7;
            for j in 0..dim as usize {
                let byte = ((0xCC ^ (m & 1).wrapping_neg()) & 0xFF) as u8;
                let start = j * stride;
                for w in 0..stride {
                    mask[start + w] = u32::from_ne_bytes([byte, byte, byte, byte]);
                }
                m = (m >> 1) | (m << 5);
            }
        }
        5 => {
            // (i * j) % 6 == 0
            for j in 0..dim as usize {
                let mut m: u32 = 0;
                for i in 0..6u32 {
                    if (i * j as u32) % 6 == 0 { m |= 1 << i; }
                }
                let mut k = 6u32;
                while k < QR_INT_BITS as u32 {
                    m |= m << k;
                    k <<= 1;
                }
                let mut mi = m;
                for i in 0..stride {
                    mask[j * stride + i] = mi;
                    mi = (mi >> (QR_INT_BITS as u32 % 6)) | (mi << (6 - (QR_INT_BITS as u32 % 6)));
                }
            }
        }
        6 => {
            // ((i*j) % 3 + i*j + 1) & 1 == 0
            for j in 0..dim as usize {
                let mut m: u32 = 0;
                for i in 0..6u32 {
                    let val = ((i * j as u32) % 3 + i * j as u32 + 1) & 1;
                    m |= val << i;
                }
                let mut k = 6u32;
                while k < QR_INT_BITS as u32 {
                    m |= m << k;
                    k <<= 1;
                }
                let mut mi = m;
                for i in 0..stride {
                    mask[j * stride + i] = mi;
                    mi = (mi >> (QR_INT_BITS as u32 % 6)) | (mi << (6 - (QR_INT_BITS as u32 % 6)));
                }
            }
        }
        _ => {
            // pattern 7 (default): ((i*j) % 3 + i + j + 1) & 1 == 0
            for j in 0..dim as usize {
                let mut m: u32 = 0;
                for i in 0..6u32 {
                    let val = ((i * j as u32) % 3 + i + j as u32 + 1) & 1;
                    m |= val << i;
                }
                let mut k = 6u32;
                while k < QR_INT_BITS as u32 {
                    m |= m << k;
                    k <<= 1;
                }
                let mut mi = m;
                for i in 0..stride {
                    mask[j * stride + i] = mi;
                    mi = (mi >> (QR_INT_BITS as u32 % 6)) | (mi << (6 - (QR_INT_BITS as u32 % 6)));
                }
            }
        }
    }
}

/// Sample every (non-fp) module of the QR code into `data_bits`. The buffer
/// is initialized with the data mask, then XORed with the image bit at each
/// position, so the result is already unmasked (qrdec.c:2948).
pub fn qr_sampling_grid_sample(
    grid: &QrSamplingGrid,
    data_bits: &mut [u32],
    dim: i32,
    fmt_info: i32,
    img: &[u8], width: i32, height: i32,
) {
    qr_data_mask_fill(data_bits, dim, fmt_info & 7);
    let stride = stride_for(dim);
    let mut u0 = 0i32;
    for j in 0..grid.ncells {
        let u1 = grid.cell_limits[j];
        let mut v0 = 0i32;
        for i in 0..grid.ncells {
            let v1 = grid.cell_limits[i];
            let cell = *grid.cell(i, j);
            let du = u0 - cell.u0;
            let dv = v0 - cell.v0;
            let mut x0 = cell.fwd[0][0].wrapping_mul(du)
                .wrapping_add(cell.fwd[0][1].wrapping_mul(dv))
                .wrapping_add(cell.fwd[0][2]);
            let mut y0 = cell.fwd[1][0].wrapping_mul(du)
                .wrapping_add(cell.fwd[1][1].wrapping_mul(dv))
                .wrapping_add(cell.fwd[1][2]);
            let mut w0 = cell.fwd[2][0].wrapping_mul(du)
                .wrapping_add(cell.fwd[2][1].wrapping_mul(dv))
                .wrapping_add(cell.fwd[2][2]);
            for u in u0..u1 {
                let mut x = x0;
                let mut y = y0;
                let mut w = w0;
                for v in v0..v1 {
                    if !qr_sampling_grid_is_in_fp(grid, dim, u, v) {
                        let mut p: QrPoint = [0; 2];
                        qr_hom_cell_fproject(&mut p, &cell, x, y, w);
                        let bit = qr_img_get_bit(img, width, height, p[0], p[1]) as u32;
                        let idx = u as usize * stride + ((v >> QR_INT_LOGBITS) as usize);
                        data_bits[idx] ^= bit << ((v & (QR_INT_BITS - 1)) as u32);
                    }
                    x = x.wrapping_add(cell.fwd[0][1]);
                    y = y.wrapping_add(cell.fwd[1][1]);
                    w = w.wrapping_add(cell.fwd[2][1]);
                }
                x0 = x0.wrapping_add(cell.fwd[0][0]);
                y0 = y0.wrapping_add(cell.fwd[1][0]);
                w0 = w0.wrapping_add(cell.fwd[2][0]);
            }
            v0 = v1;
        }
        u0 = u1;
    }
}

/// Arrange sampled bits into bytes, distribute across Reed-Solomon blocks
/// (qrdec.c:3023). `blocks` is appended to: each block already exists with
/// its target capacity and we `push` bytes into it. After this returns,
/// each block's `len()` is the number of bytes it received (data + parity).
///
/// `nshort_data` = number of data bytes in a short block.
/// `nshort_blocks` = number of short blocks (long ones follow).
pub fn qr_samples_unpack(
    blocks: &mut [Vec<u8>],
    nblocks_in: usize,
    nshort_data: usize,
    nshort_blocks_in: usize,
    data_bits: &[u32],
    fp_mask: &[u32],
    dim: i32,
) {
    let stride = stride_for(dim);
    // If all blocks are short, treat them uniformly.
    let nshort_blocks = if nshort_blocks_in >= nblocks_in { 0 } else { nshort_blocks_in };
    let nblocks = nblocks_in;

    let mut bits: u32 = 0;
    let mut biti: i32 = 0;
    let mut blocki: usize = 0;
    let mut blockj: usize = 0;

    let push_byte = |b: u32, blocks: &mut [Vec<u8>], blocki: &mut usize, blockj: &mut usize| {
        blocks[*blocki].push(b as u8);
        *blocki += 1;
        if *blocki >= nblocks {
            *blockj += 1;
            *blocki = if *blockj == nshort_data { nshort_blocks } else { 0 };
        }
    };

    let mut j: i32 = dim - 1;
    while j > 0 {
        // Scan up a pair of columns.
        let mut nbits: i32 = ((dim - 1) & (QR_INT_BITS - 1)) + 1;
        let mut l = j as usize * stride;
        let mut i = stride;
        while i > 0 {
            i -= 1;
            let data1 = data_bits[l + i];
            let fp1 = fp_mask[l + i];
            let data2 = data_bits[l + i - stride];
            let fp2 = fp_mask[l + i - stride];
            while nbits > 0 {
                nbits -= 1;
                let n = nbits as u32;
                if (fp1 >> n) & 1 == 0 {
                    bits = (bits << 1) | ((data1 >> n) & 1);
                    biti += 1;
                }
                if (fp2 >> n) & 1 == 0 {
                    bits = (bits << 1) | ((data2 >> n) & 1);
                    biti += 1;
                }
                if biti >= 8 {
                    biti -= 8;
                    let byte = bits >> biti;
                    push_byte(byte, blocks, &mut blocki, &mut blockj);
                }
            }
            nbits = QR_INT_BITS;
        }
        j -= 2;
        if j == 6 { j -= 1; }
        // Down scan reads cols `j` and `j-1`; require j ≥ 1.
        if j < 1 { break; }
        // Scan down a pair of columns.
        l = j as usize * stride;
        for i in 0..stride {
            let mut data1 = data_bits[l + i];
            let mut fp1 = fp_mask[l + i];
            let mut data2 = data_bits[l + i - stride];
            let mut fp2 = fp_mask[l + i - stride];
            let mut n = (dim - (i as i32 * (1 << QR_INT_LOGBITS))).min(QR_INT_BITS);
            while n > 0 {
                n -= 1;
                if (fp1 & 1) == 0 {
                    bits = (bits << 1) | (data1 & 1);
                    biti += 1;
                }
                data1 >>= 1;
                fp1 >>= 1;
                if (fp2 & 1) == 0 {
                    bits = (bits << 1) | (data2 & 1);
                    biti += 1;
                }
                data2 >>= 1;
                fp2 >>= 1;
                if biti >= 8 {
                    biti -= 8;
                    let byte = bits >> biti;
                    push_byte(byte, blocks, &mut blocki, &mut blockj);
                }
            }
        }
        // for-loop iter clause: j -= 2.
        j -= 2;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_mask_fill_pattern_1_uniform() {
        // Pattern 1: rows with (i + 1) & 1 == 0 → every other row dark.
        // All words 0x55555555.
        let dim = 21;
        let stride = stride_for(dim);
        let mut m = vec![0u32; dim as usize * stride];
        qr_data_mask_fill(&mut m, dim, 1);
        for &w in &m {
            assert_eq!(w, u32::from_ne_bytes([0x55, 0x55, 0x55, 0x55]));
        }
    }

    #[test]
    fn data_mask_fill_pattern_0_alternates_columns() {
        let dim = 21;
        let stride = stride_for(dim);
        let mut m = vec![0u32; dim as usize * stride];
        qr_data_mask_fill(&mut m, dim, 0);
        // Even columns: 0x55... XOR 0xFF... alternates.
        // First column word == 0x55..., second == 0xAA...
        let col0 = m[0];
        let col1 = m[stride];
        assert_ne!(col0, col1);
        assert_eq!(col0 ^ col1, u32::from_ne_bytes([0xFF, 0xFF, 0xFF, 0xFF]));
    }

    #[test]
    fn samples_unpack_v1_no_panic() {
        // QR version 1 (dim=21). All-zero data + empty fp_mask. We don't
        // verify byte content — only that the column-zigzag walk completes
        // without index underflow.
        let dim = 21i32;
        let stride = stride_for(dim);
        let data_bits = vec![0u32; dim as usize * stride];
        let fp_mask = vec![0u32; dim as usize * stride];
        let mut blocks = vec![Vec::<u8>::with_capacity(64); 1];
        qr_samples_unpack(&mut blocks, 1, 0, 0, &data_bits, &fp_mask, dim);
        // V1 has 26 codewords; with no fp mask all bits land in block 0.
        // Don't assert exact length — different fp_mask population would
        // shift it. The non-panic completion is the contract being tested.
        assert!(!blocks[0].is_empty());
    }
}
