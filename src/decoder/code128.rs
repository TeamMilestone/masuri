//! Code 128 barcode decoder
//! Rust port of zbar 0.23 decoder/code128.c
//! Original Copyright (C) 2007-2010 Jeff Brown <spadix@users.sourceforge.net>
//! LGPL-2.1-or-later

use crate::SymbolType;
use super::{Decoder, decode_e};

#[allow(dead_code)]
const NUM_CFGS: usize = 2;
const SHIFT: u8 = 0x62;
const CODE_C: u8 = 0x63;
#[allow(dead_code)]
const CODE_B: u8 = 0x64;
const CODE_A: u8 = 0x65;
const FNC1: u8 = 0x66;
const START_A: u8 = 0x67;
#[allow(dead_code)]
const START_B: u8 = 0x68;
const START_C: u8 = 0x69;
const STOP_FWD: u8 = 0x6a;
const STOP_REV: u8 = 0x6b;

static CHARACTERS: [u8; 108] = [
    0x5c, 0xbf, 0xa1,
    0x2a, 0xc5, 0x0c, 0xa4,
    0x2d, 0xe3, 0x0f,
    0x5f, 0xe4,
    0x6b, 0xe8, 0x69, 0xa7, 0xe7,
    0xc1, 0x51, 0x1e, 0x83, 0xd9, 0x00, 0x84, 0x1f,
    0xc7, 0x0d, 0x33, 0x86, 0xb5, 0x0e, 0x15, 0x87,
    0x10, 0xda, 0x11,
    0x36, 0xe5, 0x18, 0x37,
    0xcc, 0x13, 0x39, 0x89, 0x97, 0x14, 0x1b, 0x8a, 0x3a, 0xbd,
    0xa2, 0x5e, 0x01, 0x85, 0xb0, 0x02, 0xa3,
    0xa5, 0x2c, 0x16, 0x88, 0xbc, 0x12, 0xa6,
    0x61, 0xe6, 0x56, 0x62,
    0x19, 0xdb, 0x1a,
    0xa8, 0x32, 0x1c, 0x8b, 0xcd, 0x1d, 0xa9,
    0xc3, 0x20, 0xc4,
    0x50, 0x5d, 0xc0,
    0x2b, 0xc6,
    0x2e,
    0x53, 0x60,
    0x31,
    0x52, 0xc2,
    0x34, 0xc8,
    0x55,
    0x57, 0x3e, 0xce,
    0x3b, 0xc9,
    0x6a,
    0x54, 0x4f,
    0x38,
    0x58, 0xcb,
    0x2f, 0xca,
];

static LO_BASE: [u8; 8] = [0x00, 0x07, 0x0c, 0x19, 0x24, 0x32, 0x40, 0x47];

static LO_OFFSET: [u8; 128] = [
    0xff, 0xf0, 0xff, 0x1f, 0xff, 0xf2, 0xff, 0xff,
    0xff, 0xff, 0xff, 0x3f, 0xf4, 0xf5, 0xff, 0x6f,
    0xff, 0xff, 0xff, 0xff, 0xf0, 0xf1, 0xff, 0x2f,
    0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x3f, 0x4f,
    0xff, 0x0f, 0xf1, 0xf2, 0xff, 0x3f, 0xff, 0xf4,
    0xf5, 0xf6, 0xf7, 0x89, 0xff, 0xab, 0xff, 0xfc,
    0xff, 0xff, 0x0f, 0x1f, 0x23, 0x45, 0xf6, 0x7f,
    0xff, 0xff, 0xff, 0xff, 0xf8, 0xff, 0xf9, 0xaf,
    0xf0, 0xf1, 0xff, 0x2f, 0xff, 0xf3, 0xff, 0xff,
    0x4f, 0x5f, 0x67, 0x89, 0xfa, 0xbf, 0xff, 0xcd,
    0xf0, 0xf1, 0xf2, 0x3f, 0xf4, 0x56, 0xff, 0xff,
    0xff, 0xff, 0x7f, 0x8f, 0x9a, 0xff, 0xbc, 0xdf,
    0x0f, 0x1f, 0xf2, 0xff, 0xff, 0x3f, 0xff, 0xff,
    0xf4, 0xff, 0xf5, 0x6f, 0xff, 0xff, 0xff, 0xff,
    0x0f, 0x1f, 0x23, 0xff, 0x45, 0x6f, 0xff, 0xff,
    0xf7, 0xff, 0xf8, 0x9f, 0xff, 0xff, 0xff, 0xff,
];

pub struct Code128Decoder {
    pub direction: u8,
    pub element: u8,
    pub character: i16,
    pub start: u8,      // 0.23: stores start character
    pub s6: u32,
    pub width: u32,     // 0.23: last character width for variance check
    pub config: u32,
    pub configs: [i32; NUM_CFGS],
}

impl Code128Decoder {
    pub fn new() -> Self {
        Code128Decoder {
            direction: 0,
            element: 0,
            character: -1,
            start: 0,
            s6: 0,
            width: 0,
            config: 1, // enabled
            configs: [0; NUM_CFGS],
        }
    }

    pub fn reset(&mut self) {
        self.direction = 0;
        self.element = 0;
        self.character = -1;
        self.s6 = 0;
    }

    pub fn enabled(&self) -> bool {
        self.config & 1 != 0
    }
}

// Returns the raw character code (NOT masked with 0x7f) as i16.
// Matches C behavior where signed char c = characters[idx] is returned.
// Caller uses c == -1 check and later c & 0x7f for the actual value.
fn decode_lo(sig: i32) -> i16 {
    let offset = (((sig >> 1) & 0x01)
        | ((sig >> 3) & 0x06)
        | ((sig >> 5) & 0x18)
        | ((sig >> 7) & 0x60)) as usize;
    if offset >= LO_OFFSET.len() { return -1; }
    let mut idx = LO_OFFSET[offset];
    if sig & 1 != 0 {
        idx &= 0xf;
    } else {
        idx >>= 4;
    }
    if idx == 0xf { return -1; }

    let base = ((sig >> 11) | ((sig >> 9) & 1)) as usize;
    if base >= 8 { return -1; }
    let idx = idx as usize + LO_BASE[base] as usize;
    if idx > 0x50 || idx >= CHARACTERS.len() { return -1; }
    // Return raw character value (like C signed char)
    CHARACTERS[idx] as i16
}

fn decode_hi(sig: i32) -> i16 {
    let rev = (sig & 0x4400) != 0;
    let sig = if rev {
        ((sig >> 12) & 0x000f)
            | ((sig >> 4) & 0x00f0)
            | ((sig << 4) & 0x0f00)
            | ((sig << 12) & 0xf000)
    } else {
        sig
    };

    let idx: u8 = match sig {
        0x0014 => 0x0, 0x0025 => 0x1, 0x0034 => 0x2,
        0x0134 => 0x3, 0x0143 => 0x4, 0x0243 => 0x5,
        0x0341 => 0x6, 0x0352 => 0x7, 0x1024 => 0x8,
        0x1114 => 0x9, 0x1134 => 0xa, 0x1242 => 0xb,
        0x1243 => 0xc, 0x1441 => { return CHARACTERS[0x51 + 0xd] as i16; }
        _ => return -1,
    };
    let final_idx = if rev { idx + 0xe } else { idx };
    let ci = 0x51 + final_idx as usize;
    if ci >= CHARACTERS.len() { return -1; }
    CHARACTERS[ci] as i16
}

fn calc_check(c: u8) -> u8 {
    if c & 0x80 == 0 { return 0x18; }
    let c = c & 0x7f;
    if c < 0x3d {
        return if c < 0x30 && c != 0x17 { 0x10 } else { 0x20 };
    }
    if c < 0x50 {
        return if c == 0x4d { 0x20 } else { 0x10 };
    }
    if c < 0x67 { 0x20 } else { 0x10 }
}

// Returns raw character code (with high bit) or -1 on failure.
// decode6 in C returns signed char, but values can be 0x00-0x6b (0-107).
// We use i16 to avoid truncation issues. Caller uses & 0x7f for final value.
fn decode6(dcode: &Decoder) -> i16 {
    let s = dcode.code128.s6;
    if s < 5 { return -1; }

    let color = dcode.get_color();
    let sig = if color == 1 {
        let d0 = decode_e(dcode.get_width(0) + dcode.get_width(1), s, 11);
        let d1 = decode_e(dcode.get_width(1) + dcode.get_width(2), s, 11);
        let d2 = decode_e(dcode.get_width(2) + dcode.get_width(3), s, 11);
        let d3 = decode_e(dcode.get_width(3) + dcode.get_width(4), s, 11);
        if d0 < 0 || d1 < 0 || d2 < 0 || d3 < 0 { return -1; }
        (d0 << 12) | (d1 << 8) | (d2 << 4) | d3
    } else {
        let d0 = decode_e(dcode.get_width(5) + dcode.get_width(4), s, 11);
        let d1 = decode_e(dcode.get_width(4) + dcode.get_width(3), s, 11);
        let d2 = decode_e(dcode.get_width(3) + dcode.get_width(2), s, 11);
        let d3 = decode_e(dcode.get_width(2) + dcode.get_width(1), s, 11);
        if d0 < 0 || d1 < 0 || d2 < 0 || d3 < 0 { return -1; }
        (d0 << 12) | (d1 << 8) | (d2 << 4) | d3
    };

    // c is the raw character value from CHARACTERS table (may have 0x80 bit set)
    let c = if sig & 0x4444 != 0 { decode_hi(sig) } else { decode_lo(sig) };
    if c == -1 { return -1; }

    // character validation via bar widths
    let bars = if color == 1 {
        dcode.get_width(0) + dcode.get_width(2) + dcode.get_width(4)
    } else {
        dcode.get_width(1) + dcode.get_width(3) + dcode.get_width(5)
    };
    let bars = bars * 11 * 4 / s;
    // calc_check uses the raw character value (with 0x80 bit)
    let chk = calc_check(c as u8);
    if bars + 7 < chk as u32 || bars > chk as u32 + 7 { return -1; }

    // Return masked value (like C: return c & 0x7f)
    c & 0x7f
}

fn validate_checksum(dcode: &Decoder) -> bool {
    let c128 = &dcode.code128;
    if c128.character < 3 { return true; }
    let len = c128.character as usize;

    let start_idx = if c128.direction != 0 { len - 1 } else { 0 };
    if start_idx >= dcode.buf.len() { return true; }
    let mut sum = dcode.buf[start_idx] as u32;
    if sum >= 103 { sum -= 103; }

    let mut acc: u32 = 0;
    for i in (1..len.saturating_sub(2)).rev() {
        if sum >= 103 { return true; }
        let idx = if c128.direction != 0 { len - 1 - i } else { i };
        if idx >= dcode.buf.len() { return true; }
        acc += dcode.buf[idx] as u32;
        if acc >= 103 { acc -= 103; }
        if acc >= 103 { return true; }
        sum += acc;
        if sum >= 103 { sum -= 103; }
    }

    let check_idx = if c128.direction != 0 { 1 } else { len - 2 };
    if check_idx >= dcode.buf.len() { return true; }
    let check = dcode.buf[check_idx] as u32;
    sum != check
}

fn postprocess_code128(dcode: &mut Decoder) -> bool {
    if (dcode.code128.character as usize) < 3 { return true; }

    if dcode.code128.direction != 0 {
        let len = dcode.code128.character as usize;
        let half = len / 2;
        for i in 0..half {
            let j = len - 1 - i;
            dcode.buf.swap(i, j);
        }
    }

    let code = dcode.buf[0];
    if code < START_A || code > START_C { return true; }

    let mut charset = code - START_A;
    let mut j = 0usize;
    let mut cexp = if code == START_C { 1usize } else { 0 };

    let mut i = 1usize;
    // C: for(i = 1, j = 0; i < dcode128->character - 2; i++)
    // character may change during postprocess_c, so re-read each iteration
    while i < (dcode.code128.character as usize).saturating_sub(2) {
        let code = dcode.buf[i];
        if code & 0x80 != 0 { return true; }

        if (charset & 0x2) != 0 && code < 100 {
            // defer character set C for expansion
            i += 1;
            continue;
        } else if code < 0x60 {
            // convert character set B to ASCII
            let mut ascii = code + 0x20;
            if (charset == 0 || charset == 0x81) && ascii >= 0x60 {
                // convert character set A to ASCII
                ascii -= 0x60;
            }
            if j < dcode.buf.len() {
                dcode.buf[j] = ascii;
            }
            j += 1;
            if charset & 0x80 != 0 {
                charset &= 0x7f;
            }
        } else {
            if charset & 0x2 != 0 && cexp > 0 {
                // expand character set C to ASCII
                let delta = postprocess_c(dcode, cexp, i, j);
                i += delta;
                j += delta * 2;
                cexp = 0;
            }
            if code < CODE_C {
                if code == SHIFT {
                    charset |= 0x80;
                }
            } else if code == FNC1 {
                if i == 1 {
                    // GS1 modifier
                } else if i < (dcode.code128.character as usize).saturating_sub(3) {
                    if j < dcode.buf.len() {
                        dcode.buf[j] = 0x1d;
                    }
                    j += 1;
                }
            } else if code >= START_A {
                return true;
            } else {
                let newset = CODE_A - code;
                if newset != charset {
                    charset = newset;
                }
            }
            if charset & 0x2 != 0 {
                cexp = i + 1;
            }
        }
        i += 1;
    }

    if charset & 0x2 != 0 && cexp > 0 {
        let delta = postprocess_c(dcode, cexp, i, j);
        j += delta * 2;
    }

    dcode.buflen = j;
    if j < dcode.buf.len() {
        dcode.buf[j] = 0;
    }
    dcode.code128.character = j as i16;
    false
}

fn postprocess_c(dcode: &mut Decoder, start: usize, end: usize, dst: usize) -> usize {
    if end <= start { return 0; }
    let delta = end - start;
    let newlen = dcode.code128.character as usize + delta;
    if dcode.buf.len() < newlen {
        dcode.buf.resize(newlen, 0);
    }

    let src_end = dcode.code128.character as usize;
    if start + delta <= dcode.buf.len() && src_end <= dcode.buf.len() && src_end > start {
        dcode.buf.copy_within(start..src_end, start + delta);
    }
    dcode.code128.character = newlen as i16;

    for (i, ji) in (0..delta).zip((dst..).step_by(2)) {
        if ji + 1 >= dcode.buf.len() { break; }
        let code_idx = start + delta + i;
        if code_idx >= dcode.buf.len() { break; }
        let mut code = dcode.buf[code_idx];
        dcode.buf[ji] = b'0';
        if code >= 50 { code -= 50; dcode.buf[ji] += 5; }
        if code >= 30 { code -= 30; dcode.buf[ji] += 3; }
        if code >= 20 { code -= 20; dcode.buf[ji] += 2; }
        if code >= 10 { code -= 10; dcode.buf[ji] += 1; }
        dcode.buf[ji + 1] = b'0' + code;
    }
    delta
}

/// Main Code128 decode entry point - ported from zbar 0.23
pub fn decode_code128(dcode: &mut Decoder) -> SymbolType {
    // Update latest character width
    dcode.code128.s6 = dcode.code128.s6.wrapping_sub(dcode.get_width(6));
    dcode.code128.s6 = dcode.code128.s6.wrapping_add(dcode.get_width(0));

    // 0.23 entry condition: when character < 0, only enter on SPACE color
    // when character >= 0, process every 6th element in correct direction
    if dcode.code128.character < 0 {
        if dcode.get_color() != 0 {  // != ZBAR_SPACE
            return SymbolType::None;
        }
    } else {
        dcode.code128.element += 1;
        if dcode.code128.element != 6 || dcode.get_color() != dcode.code128.direction {
            return SymbolType::None;
        }
    }
    dcode.code128.element = 0;

    let c = decode6(dcode);

    if dcode.code128.character < 0 {
        // Looking for start character
        if c < 0 || (c as u8) < START_A || (c as u8) > STOP_REV || c as u8 == STOP_FWD {
            return SymbolType::None;
        }
        let qz = dcode.get_width(6);
        if qz != 0 && qz < (dcode.code128.s6 * 3) / 4 {
            return SymbolType::None;
        }
        // 0.23: Don't acquire lock yet, just initialize state
        dcode.code128.character = 1;  // Start at 1, not 0
        if c as u8 == STOP_REV {
            dcode.code128.direction = 1; // ZBAR_BAR
            dcode.code128.element = 7;
        } else {
            dcode.code128.direction = 0; // ZBAR_SPACE
        }
        dcode.code128.start = c as u8;  // Save start character
        dcode.code128.width = dcode.code128.s6;  // Save initial width
        return SymbolType::None;  // 0.23: return 0 after start, don't store yet
    } else if c < 0 || dcode.size_buf(dcode.code128.character as usize + 1) {
        // Abort
        if dcode.code128.character > 1 {
            dcode.lock = SymbolType::None;  // release_lock
        }
        dcode.code128.character = -1;
        return SymbolType::None;
    } else {
        // 0.23: Width variance check
        let dw = if dcode.code128.width > dcode.code128.s6 {
            dcode.code128.width - dcode.code128.s6
        } else {
            dcode.code128.s6 - dcode.code128.width
        };
        if dw * 4 > dcode.code128.width {
            // Width variance too high - reject
            if dcode.code128.character > 1 {
                dcode.lock = SymbolType::None;  // release_lock
            }
            dcode.code128.character = -1;
            return SymbolType::None;
        }
    }
    dcode.code128.width = dcode.code128.s6;

    // 0.23: Acquire lock on first data character (character == 1)
    if dcode.code128.character == 1 {
        if dcode.get_lock(SymbolType::Code128) {
            dcode.code128.character = -1;
            return SymbolType::None;
        }
        dcode.buf[0] = dcode.code128.start;  // Place saved start char
    }

    let idx = dcode.code128.character as usize;
    if idx < dcode.buf.len() {
        dcode.buf[idx] = c as u8;
    }
    dcode.code128.character += 1;

    if dcode.code128.character > 2 {
        let is_end = if dcode.code128.direction != 0 {
            c >= 0 && (c as u8) >= START_A && (c as u8) <= START_C
        } else {
            c >= 0 && c as u8 == STOP_FWD
        };

        if is_end {
            let mut sym = SymbolType::Code128;
            if validate_checksum(dcode) || postprocess_code128(dcode) {
                sym = SymbolType::None;
            }
            dcode.code128.character = -1;
            if sym == SymbolType::None {
                dcode.lock = SymbolType::None;  // release_lock
            }
            return sym;
        }
    }

    SymbolType::None
}
