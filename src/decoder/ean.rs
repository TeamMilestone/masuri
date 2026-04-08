//! EAN/UPC barcode decoder
//! Rust port of zbar/decoder/ean.c
//! Original Copyright (C) 2007-2009 Jeff Brown <spadix@users.sourceforge.net>
//! LGPL-2.1-or-later

use crate::{SymbolType, test_cfg_inline};
use super::{Decoder, decode_e};

const STATE_ADDON: i8 = 0x40;
const STATE_IDX: i8 = 0x1f;
const EAN_RIGHT: i32 = 0x1000;

#[derive(Debug, Clone)]
pub struct EanPass {
    pub state: i8,
    pub raw: [u8; 7],
}

impl EanPass {
    fn new() -> Self {
        EanPass { state: -1, raw: [0; 7] }
    }
}

pub struct EanDecoder {
    pub pass: [EanPass; 4],
    pub left: SymbolType,
    pub right: SymbolType,
    pub addon: SymbolType,
    pub s4: u32,
    pub buf: [i8; 18],
    pub enable: bool,
    pub ean13_config: u32,
    pub ean8_config: u32,
    pub upca_config: u32,
    pub upce_config: u32,
    pub isbn10_config: u32,
    pub isbn13_config: u32,
}

// Convert compact encoded D2E1E2 to character (bit4 is parity)
static DIGITS: [u8; 20] = [
    0x06, 0x10, 0x04, 0x13,
    0x19, 0x08, 0x11, 0x05,
    0x09, 0x12, 0x07, 0x15,
    0x16, 0x00, 0x14, 0x03,
    0x18, 0x01, 0x02, 0x17,
];

static PARITY_DECODE: [u8; 32] = [
    0xf0,
    0xff, 0xff, 0x0f, 0xff,
    0x1f, 0x2f, 0xf3, 0xff,
    0x4f, 0x7f, 0xf8, 0x5f,
    0xf9, 0xf6, 0xff,
    0xff, 0x6f, 0x9f, 0xf5,
    0x8f, 0xf7, 0xf4, 0xff,
    0x3f, 0xf2, 0xf1, 0xff,
    0xff, 0xff, 0xff, 0x0f,
];

impl EanDecoder {
    pub fn new() -> Self {
        EanDecoder {
            pass: [EanPass::new(), EanPass::new(), EanPass::new(), EanPass::new()],
            left: SymbolType::None,
            right: SymbolType::None,
            addon: SymbolType::None,
            s4: 0,
            buf: [-1; 18],
            enable: true,
            ean13_config: (1 << 0) | (1 << 2), // ENABLE | EMIT_CHECK
            ean8_config: (1 << 0) | (1 << 2),
            upca_config: 1 << 2,
            upce_config: 1 << 2,
            isbn10_config: 1 << 2,
            isbn13_config: 1 << 2,
        }
    }

    pub fn new_scan(&mut self) {
        for p in &mut self.pass { p.state = -1; }
        self.s4 = 0;
    }

    pub fn reset(&mut self) {
        self.new_scan();
        self.left = SymbolType::None;
        self.right = SymbolType::None;
        self.addon = SymbolType::None;
    }

    fn get_config(&self, sym: SymbolType) -> u32 {
        match sym {
            SymbolType::Ean13 => self.ean13_config,
            SymbolType::Ean8 => self.ean8_config,
            SymbolType::Upca => self.upca_config,
            SymbolType::Upce => self.upce_config,
            SymbolType::Isbn10 => self.isbn10_config,
            SymbolType::Isbn13 => self.isbn13_config,
            _ => 0,
        }
    }
}

fn aux_end(dcode: &Decoder, fwd: bool) -> i8 {
    let s = dcode.calc_s(if fwd { 4 } else { 4 }, 4);

    if !fwd {
        let qz = dcode.get_width(0);
        if qz != 0 && qz < s * 3 / 4 {
            return -1;
        }
    }

    let mut code: i8 = 0;
    let start = if fwd { 0 } else { 1 };
    let end = if fwd { 4 } else { 3 };
    for i in start..=end {
        let e = dcode.get_width(i) + dcode.get_width(i + 1);
        let d = decode_e(e, s, 7);
        if d < 0 { return -1; }
        code = (code << 2) | d as i8;
        if code < 0 { return -1; }
    }
    code
}

fn aux_start(dcode: &Decoder) -> i8 {
    let s4 = dcode.ean.s4;
    let e2 = dcode.get_width(5) + dcode.get_width(6);
    if decode_e(e2, s4, 7) != 0 {
        return -1;
    }

    let e1 = dcode.get_width(4) + dcode.get_width(5);
    let e1_val = decode_e(e1, s4, 7);

    if dcode.get_color() == 1 { // BAR
        let qz = dcode.get_width(7);
        if qz == 0 || qz >= s4 * 3 / 4 {
            if e1_val == 0 { return 0; }
            if e1_val == 1 { return STATE_ADDON; }
        }
        return -1;
    }

    if e1_val == 0 {
        let e3 = dcode.get_width(6) + dcode.get_width(7);
        if decode_e(e3, s4, 7) == 0 {
            return 0;
        }
    }
    -1
}

fn decode4(dcode: &Decoder) -> i8 {
    let e1 = if dcode.get_color() == 1 {
        dcode.get_width(0) + dcode.get_width(1)
    } else {
        dcode.get_width(2) + dcode.get_width(3)
    };
    let e2 = dcode.get_width(1) + dcode.get_width(2);

    let s4 = dcode.ean.s4;
    let d1 = decode_e(e1, s4, 7);
    let d2 = decode_e(e2, s4, 7);
    if d1 < 0 || d2 < 0 { return -1; }

    let mut code = (d1 << 2) | d2;
    if code < 0 { return -1; }

    // 4 combinations require additional determinant
    if (1 << code) & 0x0660 != 0 {
        let d2_sum = if dcode.get_color() == 1 {
            dcode.get_width(0) + dcode.get_width(2)
        } else {
            dcode.get_width(1) + dcode.get_width(3)
        };
        let d2_val = d2_sum * 7;
        let mid = if (1 << code) & 0x0420 != 0 { 3 } else { 4 };
        let alt = d2_val > mid * s4;
        if alt {
            code = ((code >> 1) & 3) | 0x10;
        }
    }

    if code as usize >= DIGITS.len() { return -1; }
    code as i8
}

fn ean_part_end4(pass: &mut EanPass, fwd: bool) -> i32 {
    let par = ((pass.raw[1] & 0x10) >> 1)
        | ((pass.raw[2] & 0x10) >> 2)
        | ((pass.raw[3] & 0x10) >> 3)
        | ((pass.raw[4] & 0x10) >> 4);

    if par != 0 && par != 0xf { return SymbolType::None as i32; }

    if (par == 0) != fwd {
        pass.raw.swap(1, 4);
        pass.raw.swap(2, 3);
    }

    if par == 0 {
        SymbolType::Ean8 as i32 | EAN_RIGHT
    } else {
        SymbolType::Ean8 as i32
    }
}

fn ean_part_end7(ean: &EanDecoder, pass: &mut EanPass, fwd: bool) -> i32 {
    let par: u8 = if fwd {
        ((pass.raw[1] & 0x10) << 1)
            | (pass.raw[2] & 0x10)
            | ((pass.raw[3] & 0x10) >> 1)
            | ((pass.raw[4] & 0x10) >> 2)
            | ((pass.raw[5] & 0x10) >> 3)
            | ((pass.raw[6] & 0x10) >> 4)
    } else {
        ((pass.raw[1] & 0x10) >> 4)
            | ((pass.raw[2] & 0x10) >> 3)
            | ((pass.raw[3] & 0x10) >> 2)
            | ((pass.raw[4] & 0x10) >> 1)
            | (pass.raw[5] & 0x10)
            | ((pass.raw[6] & 0x10) << 1)
    };

    let idx = (par >> 1) as usize;
    if idx >= PARITY_DECODE.len() { return SymbolType::None as i32; }
    let mut raw0 = PARITY_DECODE[idx];
    if par & 1 != 0 {
        raw0 &= 0xf;
    } else {
        raw0 >>= 4;
    }
    raw0 &= 0xf;
    pass.raw[0] = raw0;

    if raw0 == 0xf { return SymbolType::None as i32; }

    if (par == 0) != fwd {
        for i in 1..4 {
            let j = 7 - i;
            pass.raw.swap(i, j);
        }
    }

    if test_cfg_inline(ean.ean13_config, 0) {
        if par == 0 {
            return SymbolType::Ean13 as i32 | EAN_RIGHT;
        }
        if par & 0x20 != 0 {
            return SymbolType::Ean13 as i32;
        }
    }
    if par != 0 && par & 0x20 == 0 {
        return SymbolType::Upce as i32;
    }

    SymbolType::None as i32
}

fn decode_pass(dcode: &Decoder, pass: &mut EanPass) -> i32 {
    pass.state += 1;
    let idx = pass.state & STATE_IDX;
    let fwd = pass.state & 1 != 0;

    // EAN-8 end check
    if dcode.get_color() == 0
        && (idx == 0x10 || idx == 0x11)
        && test_cfg_inline(dcode.ean.ean8_config, 0)
        && aux_end(dcode, fwd) == 0
    {
        let part = ean_part_end4(pass, fwd);
        pass.state = -1;
        return part;
    }

    if (idx & 0x03) == 0 && idx <= 0x14 {
        if dcode.ean.s4 == 0 { return 0; }
        if pass.state == 0 || (pass.state > 0 && (pass.state & STATE_IDX) == 0) {
            let start_result = aux_start(dcode);
            if start_result < 0 {
                pass.state = -1;
                return 0;
            }
            pass.state = start_result;
            // idx is now potentially updated
        }
        let code = decode4(dcode);
        if code < 0 {
            pass.state = -1;
        } else {
            let slot = ((pass.state & STATE_IDX) >> 2) as usize + 1;
            if slot < 7 {
                pass.raw[slot] = DIGITS[code as usize];
            }
        }
    }

    let idx = pass.state & STATE_IDX;
    if dcode.get_color() == 0 && (idx == 0x18 || idx == 0x19) {
        let fwd = pass.state & 1 != 0;
        let mut part = SymbolType::None as i32;
        if aux_end(dcode, fwd) == 0 {
            part = ean_part_end7(&dcode.ean, pass, fwd);
        }
        pass.state = -1;
        return part;
    }
    0
}

fn ean_verify_checksum(ean: &EanDecoder, n: usize) -> bool {
    let mut chk: u32 = 0;
    for i in 0..n {
        let d = ean.buf[i];
        if d < 0 || d >= 10 { return true; } // invalid
        let d = d as u32;
        chk += d;
        if (i ^ n) & 1 != 0 {
            chk += d << 1;
            if chk >= 20 { chk -= 20; }
        }
        if chk >= 10 { chk -= 10; }
    }
    if chk >= 10 { return true; }
    if chk != 0 { chk = 10 - chk; }
    let d = ean.buf[n];
    if d < 0 || d >= 10 { return true; }
    chk != d as u32
}

fn integrate_partial(ean: &mut EanDecoder, pass: &EanPass, part: i32) -> i32 {
    let sym_part = part & 0xfff; // ZBAR_SYMBOL mask
    let is_right = part & EAN_RIGHT != 0;

    if (ean.left != SymbolType::None && sym_to_i32(ean.left) != sym_part)
        || (ean.right != SymbolType::None && sym_to_i32(ean.right) != sym_part)
    {
        ean.left = SymbolType::None;
        ean.right = SymbolType::None;
        ean.addon = SymbolType::None;
    }

    if is_right {
        let j_start = if sym_part == SymbolType::Ean13 as i32 { 12 } else { 7 };
        let i_start = if sym_part == SymbolType::Ean13 as i32 { 6 } else { 4 };
        let mut j = j_start as usize;
        for i in (1..=i_start).rev() {
            let digit = (pass.raw[i] & 0xf) as i8;
            if ean.right != SymbolType::None && ean.buf[j] != digit {
                ean.left = SymbolType::None;
                ean.right = SymbolType::None;
                ean.addon = SymbolType::None;
            }
            ean.buf[j] = digit;
            j = j.wrapping_sub(1);
        }
        ean.right = i32_to_sym(sym_part);
    } else if sym_part != SymbolType::Upce as i32 {
        // EAN_LEFT
        let j_start = if sym_part == SymbolType::Ean13 as i32 { 6 } else { 3 };
        let i_start = if sym_part == SymbolType::Ean13 as i32 { 6 } else { 4 };
        let mut j = j_start as usize;
        let mut i = i_start as usize;
        loop {
            let digit = (pass.raw[i] & 0xf) as i8;
            if ean.left != SymbolType::None && ean.buf[j] != digit {
                ean.left = SymbolType::None;
                ean.right = SymbolType::None;
                ean.addon = SymbolType::None;
            }
            ean.buf[j] = digit;
            if j == 0 || i == 0 { break; }
            i -= 1;
            j -= 1;
        }
        ean.left = i32_to_sym(sym_part);
    } else {
        // UPC-E expand
        ean_expand_upce(ean, pass);
    }

    let mut result;
    if sym_part != SymbolType::Upce as i32 {
        result = sym_to_i32(ean.left) & sym_to_i32(ean.right);
        if result == 0 {
            return SymbolType::Partial as i32;
        }
    } else {
        result = sym_part;
    }

    // Checksum verification
    if (result == SymbolType::Ean13 as i32 || result == SymbolType::Upce as i32)
        && ean_verify_checksum(ean, 12)
    {
        return SymbolType::None as i32;
    }
    if result == SymbolType::Ean8 as i32 && ean_verify_checksum(ean, 7) {
        return SymbolType::None as i32;
    }

    // Special case EAN-13 subsets
    if result == SymbolType::Ean13 as i32 {
        if ean.buf[0] == 0 && ean.upca_config & 1 != 0 {
            result = SymbolType::Upca as i32;
        } else if ean.buf[0] == 9 && ean.buf[1] == 7 {
            if ean.buf[2] == 8 && ean.isbn10_config & 1 != 0 {
                result = SymbolType::Isbn10 as i32;
            } else if (ean.buf[2] == 8 || ean.buf[2] == 9) && ean.isbn13_config & 1 != 0 {
                result = SymbolType::Isbn13 as i32;
            }
        }
    } else if result == SymbolType::Upce as i32 {
        if ean.upce_config & 1 != 0 {
            ean.buf[0] = 0;
            ean.buf[1] = 0;
            for i in 2..8usize {
                ean.buf[i] = (pass.raw[i - 1] & 0xf) as i8;
            }
            ean.buf[8] = (pass.raw[0] & 0xf) as i8;
        } else if ean.upca_config & 1 != 0 {
            result = SymbolType::Upca as i32;
        } else if ean.ean13_config & 1 != 0 {
            result = SymbolType::Ean13 as i32;
        } else {
            result = SymbolType::None as i32;
        }
    }

    result
}

fn ean_expand_upce(ean: &mut EanDecoder, pass: &EanPass) {
    let mut i = 0usize;
    ean.buf[12] = (pass.raw[i] & 0xf) as i8;
    i += 1;

    let decode = (pass.raw[6] & 0xf) as usize;
    ean.buf[0] = 0;
    ean.buf[1] = 0;
    ean.buf[2] = (pass.raw[i] & 0xf) as i8; i += 1;
    ean.buf[3] = (pass.raw[i] & 0xf) as i8; i += 1;
    ean.buf[4] = if decode < 3 { decode as i8 } else { let v = (pass.raw[i] & 0xf) as i8; i += 1; v };
    ean.buf[5] = if decode < 4 { 0 } else { let v = (pass.raw[i] & 0xf) as i8; i += 1; v };
    ean.buf[6] = if decode < 5 { 0 } else { let v = (pass.raw[i] & 0xf) as i8; i += 1; v };
    ean.buf[7] = 0;
    ean.buf[8] = 0;
    ean.buf[9] = if decode < 3 { let v = (pass.raw[i] & 0xf) as i8; i += 1; v } else { 0 };
    ean.buf[10] = if decode < 4 { let v = (pass.raw[i] & 0xf) as i8; i += 1; v } else { 0 };
    ean.buf[11] = if decode < 5 { let v = (pass.raw[i] & 0xf) as i8; v } else { decode as i8 };
}

fn postprocess_ean(dcode: &mut Decoder, sym: i32) {
    let base = sym & 0xfff;
    let mut i: usize;
    let mut j: usize = 0;

    if base <= SymbolType::Partial as i32 {
        dcode.buflen = 0;
        return;
    }

    let ean = &dcode.ean;
    i = match base {
        x if x == SymbolType::Upca as i32 => 1,
        x if x == SymbolType::Upce as i32 => 1,
        x if x == SymbolType::Isbn10 as i32 => 3,
        _ => 0,
    };

    let mut limit = base;
    if base == SymbolType::Isbn10 as i32
        || !test_cfg_inline(ean.get_config(i32_to_sym(sym & 0xfff)), 2)
    {
        limit -= 1;
    }
    // Isbn13 -> Ean13 base
    let limit = if base == SymbolType::Isbn13 as i32 { SymbolType::Ean13 as i32 - 1 } else { limit } as usize;

    while j < limit && i < 18 {
        let d = ean.buf[i];
        if d < 0 { break; }
        dcode.buf[j] = d as u8 + b'0';
        i += 1;
        j += 1;
    }

    if (sym & 0xfff) == SymbolType::Isbn10 as i32 && j == 9
        && test_cfg_inline(ean.isbn10_config, 2)
    {
        dcode.buf[j] = isbn10_calc_checksum(ean);
        j += 1;
    }

    dcode.buflen = j;
    if j < dcode.buf.len() {
        dcode.buf[j] = 0;
    }
}

fn isbn10_calc_checksum(ean: &EanDecoder) -> u8 {
    let mut chk: u32 = 0;
    let mut w: u32 = 10;
    while w > 1 {
        let idx = (13 - w) as usize;
        let d = ean.buf[idx];
        if d < 0 || d >= 10 { return b'?'; }
        chk += d as u32 * w;
        w -= 1;
    }
    chk %= 11;
    if chk == 0 { return b'0'; }
    chk = 11 - chk;
    if chk < 10 { chk as u8 + b'0' } else { b'X' }
}

/// Main EAN decode entry point
pub fn decode_ean(dcode: &mut Decoder) -> SymbolType {
    let pass_idx = dcode.idx & 3;

    // Update latest character width
    dcode.ean.s4 = dcode.ean.s4.wrapping_sub(dcode.get_width(4));
    dcode.ean.s4 = dcode.ean.s4.wrapping_add(dcode.get_width(0));

    let mut sym = SymbolType::None;

    for i in 0..4u8 {
        let pass_state = dcode.ean.pass[i as usize].state;
        if pass_state >= 0 || i == pass_idx {
            // Extract pass to avoid borrow conflict
            let mut pass = dcode.ean.pass[i as usize].clone();
            let part = decode_pass(dcode as &Decoder, &mut pass);
            dcode.ean.pass[i as usize] = pass;

            if part != 0 {
                let pass = dcode.ean.pass[i as usize].clone();
                let int_sym = integrate_partial(&mut dcode.ean, &pass, part);
                if int_sym != 0 {
                    for p in &mut dcode.ean.pass { p.state = -1; }
                    if int_sym > SymbolType::Partial as i32 {
                        if !dcode.get_lock(SymbolType::Ean13) {
                            postprocess_ean(dcode, int_sym);
                            sym = i32_to_sym(int_sym & 0xfff);
                        } else {
                            sym = SymbolType::Partial;
                        }
                    }
                }
            }
        }
    }
    sym
}

fn sym_to_i32(sym: SymbolType) -> i32 {
    sym as i32
}

fn i32_to_sym(val: i32) -> SymbolType {
    match val {
        0 => SymbolType::None,
        1 => SymbolType::Partial,
        2 => SymbolType::Ean2,
        5 => SymbolType::Ean5,
        8 => SymbolType::Ean8,
        9 => SymbolType::Upce,
        10 => SymbolType::Isbn10,
        12 => SymbolType::Upca,
        13 => SymbolType::Ean13,
        14 => SymbolType::Isbn13,
        25 => SymbolType::I25,
        39 => SymbolType::Code39,
        128 => SymbolType::Code128,
        _ => SymbolType::None,
    }
}
