//! Sampling-grid data reads, mask fill, RS block unpack — and ultimately
//! the bit-stream parse that produces the QR payload. Rust port of qrdec.c
//! lines 2830 onward. Phase 4-D builds this incrementally; each chunk
//! appends to this module.

use super::geom::{qr_hom_cell_fproject, QrPoint, QR_INT_BITS};
use super::qr_finder::qr_img_get_bit;
use super::rs::{rs_correct, RsGf256, QR_M0};
use super::sampling_grid::{
    qr_sampling_grid_init, qr_sampling_grid_clear, qr_sampling_grid_is_in_fp,
    QrSamplingGrid, QR_INT_LOGBITS,
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

// ─────────────────────────────────────────────────────────────────────────
//  QR code data types + qr_code_data_parse + qr_code_decode
//  (qrdec.c lines 3120–3588, chunk 3 of Phase 4-D)
// ─────────────────────────────────────────────────────────────────────────

/// QR mode constants (qrdec.h::qr_mode).
pub const QR_MODE_NUM: i32 = 1;
pub const QR_MODE_ALNUM: i32 = 2;
pub const QR_MODE_STRUCT: i32 = 3;
pub const QR_MODE_BYTE: i32 = 4;
pub const QR_MODE_FNC1_1ST: i32 = 5;
pub const QR_MODE_ECI: i32 = 7;
pub const QR_MODE_KANJI: i32 = 8;
pub const QR_MODE_FNC1_2ND: i32 = 9;

/// Per-mode payload of a parsed entry. Mirrors the C union of
/// (data buf, eci u32, structured-append header).
#[derive(Clone, Debug)]
pub enum QrPayload {
    None,
    Data(Vec<u8>),
    Eci(u32),
    Sa { sa_index: u8, sa_size: u8, sa_parity: u8 },
}

/// One parsed entry from the QR bit stream.
#[derive(Clone, Debug)]
pub struct QrCodeDataEntry {
    pub mode: i32,
    pub payload: QrPayload,
}

/// Decoded QR code data.
#[derive(Clone, Debug)]
pub struct QrCodeData {
    pub entries: Vec<QrCodeDataEntry>,
    pub version: u8,
    pub ecc_level: u8,
    pub sa_index: u8,
    pub sa_size: u8,
    pub sa_parity: u8,
    pub self_parity: u8,
    pub bbox: [QrPoint; 4],
}

impl QrCodeData {
    pub fn new() -> Self {
        QrCodeData {
            entries: Vec::new(),
            version: 0,
            ecc_level: 0,
            sa_index: 0,
            sa_size: 0,
            sa_parity: 0,
            self_parity: 0,
            bbox: [[0; 2]; 4],
        }
    }
}

/// The alphanumeric character table (qrdec.c:3176).
const QR_ALNUM_TABLE: [u8; 45] = [
    b'0', b'1', b'2', b'3', b'4', b'5', b'6', b'7', b'8', b'9',
    b'A', b'B', b'C', b'D', b'E', b'F', b'G', b'H', b'I', b'J',
    b'K', b'L', b'M', b'N', b'O', b'P', b'Q', b'R', b'S', b'T',
    b'U', b'V', b'W', b'X', b'Y', b'Z', b' ', b'$', b'%', b'*',
    b'+', b'-', b'.', b'/', b':',
];

/// QR version → codeword count (qrdec.c:3441).
pub fn qr_code_ncodewords(version: u32) -> i32 {
    if version == 1 { return 26; }
    let nalign = (version / 7) + 2;
    let v = version as i32;
    // (v<<4)*(v+8) - (5*nalign)*(5*nalign-2) + 36*(v<7) + 83 >> 3
    let na = nalign as i32;
    let term = (v << 4) * (v + 8) - (5 * na) * (5 * na - 2) + 36 * ((v < 7) as i32) + 83;
    term >> 3
}

/// Number of Reed-Solomon blocks per (version-1, ecc_level).
pub const QR_RS_NBLOCKS: [[u8; 4]; 40] = [
    [ 1,  1,  1,  1], [ 1,  1,  1,  1], [ 1,  1,  2,  2], [ 1,  2,  2,  4],
    [ 1,  2,  4,  4], [ 2,  4,  4,  4], [ 2,  4,  6,  5], [ 2,  4,  6,  6],
    [ 2,  5,  8,  8], [ 4,  5,  8,  8], [ 4,  5,  8, 11], [ 4,  8, 10, 11],
    [ 4,  9, 12, 16], [ 4,  9, 16, 16], [ 6, 10, 12, 18], [ 6, 10, 17, 16],
    [ 6, 11, 16, 19], [ 6, 13, 18, 21], [ 7, 14, 21, 25], [ 8, 16, 20, 25],
    [ 8, 17, 23, 25], [ 9, 17, 23, 34], [ 9, 18, 25, 30], [10, 20, 27, 32],
    [12, 21, 29, 35], [12, 23, 34, 37], [12, 25, 34, 40], [13, 26, 35, 42],
    [14, 28, 38, 45], [15, 29, 40, 48], [16, 31, 43, 51], [17, 33, 45, 54],
    [18, 35, 48, 57], [19, 37, 51, 60], [19, 38, 53, 63], [20, 40, 56, 66],
    [21, 43, 59, 70], [22, 45, 62, 74], [24, 47, 65, 77], [25, 49, 68, 81],
];

/// Bulk parity-bytes table — indexed via QR_RS_NPAR_OFFS[version-1] + ecc_level.
pub const QR_RS_NPAR_VALS: [u8; 71] = [
    /*[ 0]*/  7, 10, 13, 17,
    /*[ 4]*/ 10, 16, 22, 28, 26, 26, 26, 22, 24, 22, 22, 26, 24, 18, 22,
    /*[19]*/ 15, 26, 18, 22, 24, 30, 24, 20, 24,
    /*[28]*/ 18, 16, 24, 28, 28, 28, 28, 30, 24,
    /*[37]*/ 20, 18, 18, 26, 24, 28, 24, 30, 26, 28, 28, 26, 28, 30, 30, 22, 20, 24,
    /*[55]*/ 20, 18, 26, 16,
    /*[59]*/ 20, 30, 28, 24, 22, 26, 28, 26, 30, 28, 30, 30,
];

/// Offset into QR_RS_NPAR_VALS for each version (qrdec.c:3482).
pub const QR_RS_NPAR_OFFS: [u8; 40] = [
     0,  4, 19, 55, 15, 28, 37, 12, 51, 39,
    59, 62, 10, 24, 22, 41, 31, 44,  7, 65,
    47, 33, 67, 67, 48, 32, 67, 67, 67, 67,
    67, 67, 67, 67, 67, 67, 67, 67, 67, 67,
];

/// Stream bit reader (libogg-style). Mirrors zbar `qr_pack_buf`.
pub struct QrPackBuf<'a> {
    pub buf: &'a [u8],
    pub endbyte: i32,
    pub endbit: i32,
    pub storage: i32,
}

impl<'a> QrPackBuf<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        QrPackBuf {
            buf: data,
            storage: data.len() as i32,
            endbyte: 0,
            endbit: 0,
        }
    }
}

/// Read up to 16 bits. Returns the value or -1 if not enough bits remain.
pub fn qr_pack_buf_read(b: &mut QrPackBuf, bits_in: i32) -> i32 {
    let m = 16 - bits_in;
    let bits = bits_in + b.endbit;
    let d = b.storage - b.endbyte;
    if d <= 2 {
        if d * 8 < bits {
            b.endbyte += bits >> 3;
            b.endbit = bits & 7;
            return -1;
        }
        if bits_in == 0 { return 0; }
    }
    let p = b.endbyte as usize;
    let p0 = *b.buf.get(p).unwrap_or(&0) as u32;
    let mut ret = p0 << (8 + b.endbit as u32);
    if bits > 8 {
        let p1 = *b.buf.get(p + 1).unwrap_or(&0) as u32;
        ret |= p1 << b.endbit as u32;
        if bits > 16 {
            let p2 = *b.buf.get(p + 2).unwrap_or(&0) as u32;
            ret |= p2 >> (8 - b.endbit as u32);
        }
    }
    b.endbyte += bits >> 3;
    b.endbit = bits & 7;
    ((ret & 0xFFFF) >> m) as i32
}

/// Bits remaining in the buffer.
pub fn qr_pack_buf_avail(b: &QrPackBuf) -> i32 {
    ((b.storage - b.endbyte) << 3) - b.endbit
}

/// Per-version length-bit counts (qrdec.c:3218). Indexed by len_bits_idx
/// (= (v>9) + (v>26)) and mode (0=NUM, 1=ALNUM, 2=BYTE, 3=KANJI).
const LEN_BITS: [[u8; 4]; 3] = [
    [10,  9,  8,  8],
    [12, 11, 16, 10],
    [14, 13, 16, 12],
];

/// Parse the corrected bit stream into `qrdata.entries`. Returns 0 on
/// success, -1 on any malformed mode / underflow.
pub fn qr_code_data_parse(qrdata: &mut QrCodeData, version: i32, data: &[u8]) -> i32 {
    qrdata.entries.clear();
    qrdata.sa_size = 0;
    let len_bits_idx = ((version > 9) as usize) + ((version > 26) as usize);
    let mut qpb = QrPackBuf::new(data);

    while qr_pack_buf_avail(&qpb) >= 4 {
        let mode = qr_pack_buf_read(&mut qpb, 4);
        if mode == 0 { break; } // terminator

        match mode {
            QR_MODE_NUM => {
                let len = qr_pack_buf_read(&mut qpb, LEN_BITS[len_bits_idx][0] as i32);
                if len < 0 { return -1; }
                let count = len / 3;
                let rem = len % 3;
                let need = 10 * count + 7 * ((rem >> 1) & 1) + 4 * (rem & 1);
                if qr_pack_buf_avail(&qpb) < need { return -1; }
                let mut buf = Vec::with_capacity(len as usize);
                for _ in 0..count {
                    let bits = qr_pack_buf_read(&mut qpb, 10) as u32;
                    if bits >= 1000 { return -1; }
                    buf.push(b'0' + (bits / 100) as u8);
                    let r = bits % 100;
                    buf.push(b'0' + (r / 10) as u8);
                    buf.push(b'0' + (r % 10) as u8);
                }
                if rem > 1 {
                    let bits = qr_pack_buf_read(&mut qpb, 7) as u32;
                    if bits >= 100 { return -1; }
                    buf.push(b'0' + (bits / 10) as u8);
                    buf.push(b'0' + (bits % 10) as u8);
                } else if rem == 1 {
                    let bits = qr_pack_buf_read(&mut qpb, 4) as u32;
                    if bits >= 10 { return -1; }
                    buf.push(b'0' + bits as u8);
                }
                qrdata.entries.push(QrCodeDataEntry { mode, payload: QrPayload::Data(buf) });
            }
            QR_MODE_ALNUM => {
                let len = qr_pack_buf_read(&mut qpb, LEN_BITS[len_bits_idx][1] as i32);
                if len < 0 { return -1; }
                let count = len >> 1;
                let rem = len & 1;
                if qr_pack_buf_avail(&qpb) < 11 * count + 6 * rem { return -1; }
                let mut buf = Vec::with_capacity(len as usize);
                for _ in 0..count {
                    let bits = qr_pack_buf_read(&mut qpb, 11) as u32;
                    if bits >= 2025 { return -1; }
                    buf.push(QR_ALNUM_TABLE[(bits / 45) as usize]);
                    buf.push(QR_ALNUM_TABLE[(bits % 45) as usize]);
                }
                if rem != 0 {
                    let bits = qr_pack_buf_read(&mut qpb, 6) as u32;
                    if bits >= 45 { return -1; }
                    buf.push(QR_ALNUM_TABLE[bits as usize]);
                }
                qrdata.entries.push(QrCodeDataEntry { mode, payload: QrPayload::Data(buf) });
            }
            QR_MODE_STRUCT => {
                let bits = qr_pack_buf_read(&mut qpb, 16);
                if bits < 0 { return -1; }
                let bits = bits as u32;
                let sa_index = ((bits >> 12) & 0xF) as u8;
                let sa_size = (((bits >> 8) & 0xF) + 1) as u8;
                let sa_parity = (bits & 0xFF) as u8;
                // Multiple S-A headers: last one wins (qrdec.c:3301 TODO).
                qrdata.sa_index = sa_index;
                qrdata.sa_size = sa_size;
                qrdata.sa_parity = sa_parity;
                qrdata.entries.push(QrCodeDataEntry {
                    mode,
                    payload: QrPayload::Sa { sa_index, sa_size, sa_parity },
                });
            }
            QR_MODE_BYTE => {
                let len = qr_pack_buf_read(&mut qpb, LEN_BITS[len_bits_idx][2] as i32);
                if len < 0 { return -1; }
                if qr_pack_buf_avail(&qpb) < len << 3 { return -1; }
                let mut buf = Vec::with_capacity(len as usize);
                for _ in 0..len {
                    buf.push(qr_pack_buf_read(&mut qpb, 8) as u8);
                }
                qrdata.entries.push(QrCodeDataEntry { mode, payload: QrPayload::Data(buf) });
            }
            QR_MODE_FNC1_1ST | QR_MODE_FNC1_2ND => {
                qrdata.entries.push(QrCodeDataEntry { mode, payload: QrPayload::None });
            }
            QR_MODE_ECI => {
                // ECI: variable-width UTF-8-like encoding.
                let lead = qr_pack_buf_read(&mut qpb, 8);
                if lead < 0 { return -1; }
                let lead = lead as u32;
                let val: u32;
                if lead & 0x80 == 0 {
                    val = lead;
                } else if lead & 0x40 == 0 {
                    // NOTE: zbar 0.10 has an operator-precedence bug here:
                    // `val = bits & 0x3F << 8` parses as `bits & (0x3F<<8)` =
                    // `bits & 0x3F00`, which is 0 for an 8-bit `bits`. We
                    // preserve the bug for bit-equivalence; the practical
                    // effect is that 2-byte ECI codes collapse to 1 byte.
                    // Flagged for review when we add ECI handling for real.
                    let masked = lead & (0x3Fu32 << 8); // = 0 always
                    let next = qr_pack_buf_read(&mut qpb, 8);
                    if next < 0 { return -1; }
                    val = masked | (next as u32);
                } else if lead & 0x20 == 0 {
                    // Same zbar precedence bug as above.
                    let masked = lead & (0x1Fu32 << 16); // = 0 always
                    let next16 = qr_pack_buf_read(&mut qpb, 16);
                    if next16 < 0 { return -1; }
                    val = masked | (next16 as u32);
                    if val >= 1_000_000 { return -1; }
                } else {
                    return -1; // invalid lead byte
                }
                qrdata.entries.push(QrCodeDataEntry { mode, payload: QrPayload::Eci(val) });
            }
            QR_MODE_KANJI => {
                let len = qr_pack_buf_read(&mut qpb, LEN_BITS[len_bits_idx][3] as i32);
                if len < 0 { return -1; }
                if qr_pack_buf_avail(&qpb) < 13 * len { return -1; }
                let mut buf = Vec::with_capacity((2 * len) as usize);
                for _ in 0..len {
                    let bits_in = qr_pack_buf_read(&mut qpb, 13) as u32;
                    let mut bits = ((bits_in / 0xC0) << 8) | (bits_in % 0xC0);
                    bits += 0x8140;
                    if bits >= 0xA000 { bits += 0x4000; }
                    buf.push((bits >> 8) as u8);
                    buf.push((bits & 0xFF) as u8);
                }
                qrdata.entries.push(QrCodeDataEntry { mode, payload: QrPayload::Data(buf) });
            }
            _ => return -1, // unknown mode — can't skip without knowing format
        }
    }
    qrdata.self_parity = 0;
    0
}

/// Full QR code decode pipeline: sample → unpack → RS correct → parse.
/// Returns 0 on success, -1 on failure (RS error or malformed payload).
#[allow(clippy::too_many_arguments)]
pub fn qr_code_decode(
    qrdata: &mut QrCodeData,
    gf: &RsGf256,
    ul_pos: &QrPoint, ur_pos: &QrPoint, dl_pos: &QrPoint,
    version: i32, fmt_info: i32,
    img: &[u8], width: i32, height: i32,
) -> i32 {
    let mut grid = QrSamplingGrid::new();
    qr_sampling_grid_init(
        &mut grid, version, ul_pos, ur_pos, dl_pos, &mut qrdata.bbox, img, width, height,
    );
    let dim = 17 + (version << 2);
    let stride = stride_for(dim);
    let mut data_bits = vec![0u32; dim as usize * stride];
    qr_sampling_grid_sample(&grid, &mut data_bits, dim, fmt_info, img, width, height);

    // ecc_level = (fmt_info >> 3) ^ 1
    let ecc_level = ((fmt_info >> 3) ^ 1) as usize;
    let v_idx = (version - 1) as usize;
    let nblocks = QR_RS_NBLOCKS[v_idx][ecc_level] as usize;
    let npar = QR_RS_NPAR_VALS[QR_RS_NPAR_OFFS[v_idx] as usize + ecc_level] as usize;
    let ncodewords = qr_code_ncodewords(version as u32) as usize;
    let block_sz = ncodewords / nblocks;
    let nshort_blocks = nblocks - (ncodewords % nblocks);

    // Per-block target capacity = block_sz + (i >= nshort_blocks).
    let mut blocks: Vec<Vec<u8>> = (0..nblocks)
        .map(|i| {
            let cap = block_sz + if i >= nshort_blocks { 1 } else { 0 };
            Vec::with_capacity(cap)
        })
        .collect();

    qr_samples_unpack(
        &mut blocks,
        nblocks,
        block_sz - npar,
        nshort_blocks,
        &data_bits,
        &grid.fpmask,
        dim,
    );
    qr_sampling_grid_clear(&mut grid);

    // RS-correct each block, concatenate the data portions.
    let mut corrected_data: Vec<u8> = Vec::with_capacity(ncodewords);
    for (i, block) in blocks.iter_mut().enumerate() {
        let block_szi = block_sz + if i >= nshort_blocks { 1 } else { 0 };
        if block.len() != block_szi {
            // Unpack didn't produce the expected count — bail (treat as
            // unrecoverable). Could indicate a bug above.
            return -1;
        }
        if rs_correct(gf, QR_M0, block.as_mut_slice(), block_szi, npar, &[], 0) < 0 {
            return -1;
        }
        corrected_data.extend_from_slice(&block[..block_szi - npar]);
    }

    let ret = qr_code_data_parse(qrdata, version, &corrected_data);
    if ret < 0 {
        qrdata.entries.clear();
    }
    qrdata.version = version as u8;
    qrdata.ecc_level = ecc_level as u8;
    ret
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
    fn pack_buf_round_trip() {
        let data = [0b1100_1010_u8, 0b1010_0101, 0b1111_0000];
        let mut b = QrPackBuf::new(&data);
        // Read 4 bits → top nibble of byte 0 = 0xC.
        assert_eq!(qr_pack_buf_read(&mut b, 4), 0xC);
        // Next 4 → 0xA.
        assert_eq!(qr_pack_buf_read(&mut b, 4), 0xA);
        // 8 more → 0xA5.
        assert_eq!(qr_pack_buf_read(&mut b, 8), 0xA5);
        // avail = 8 bits remaining.
        assert_eq!(qr_pack_buf_avail(&b), 8);
        // 16 bits requested — only 8 left → -1.
        assert_eq!(qr_pack_buf_read(&mut b, 16), -1);
    }

    #[test]
    fn ncodewords_known_versions() {
        // v1 = 26, v40 = 3706 (canonical values).
        assert_eq!(qr_code_ncodewords(1), 26);
        assert_eq!(qr_code_ncodewords(40), 3706);
        // v7 (alignment patterns kick in) = 196.
        assert_eq!(qr_code_ncodewords(7), 196);
    }

    #[test]
    fn rs_tables_self_consistent() {
        // For v1 ECC-L (idx 0,0): 1 block, npar=7, ncodewords=26.
        let v = 0usize;
        let ecc = 0usize;
        let nblocks = QR_RS_NBLOCKS[v][ecc] as usize;
        let npar = QR_RS_NPAR_VALS[QR_RS_NPAR_OFFS[v] as usize + ecc] as usize;
        assert_eq!(nblocks, 1);
        assert_eq!(npar, 7);
        // For v40 ECC-H (idx 39, 3): 81 blocks, 30 parity.
        let nblocks = QR_RS_NBLOCKS[39][3] as usize;
        let npar = QR_RS_NPAR_VALS[QR_RS_NPAR_OFFS[39] as usize + 3] as usize;
        assert_eq!(nblocks, 81);
        assert_eq!(npar, 30);
    }

    #[test]
    fn parse_numeric_simple() {
        // Hand-built bitstream (MSB-first):
        //   mode=1 (4 bits): 0001
        //   len=3  (10 bits): 0000000011
        //   val=123 (10 bits): 0001111011
        // Total 24 bits → 0x10, 0x0C, 0x7B.  4th byte = 0x00 = terminator.
        let bits: [u8; 4] = [0x10, 0x0C, 0x7B, 0x00];
        let mut qd = QrCodeData::new();
        let ret = qr_code_data_parse(&mut qd, 1, &bits);
        assert_eq!(ret, 0);
        assert_eq!(qd.entries.len(), 1);
        if let QrPayload::Data(ref s) = qd.entries[0].payload {
            assert_eq!(s.as_slice(), b"123");
        } else {
            panic!("expected Data payload");
        }
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
