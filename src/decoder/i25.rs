//! Interleaved 2 of 5 barcode decoder
//! Rust port of zbar 0.23 decoder/i25.c
//! Original Copyright (C) 2008-2010 Jeff Brown <spadix@users.sourceforge.net>
//! LGPL-2.1-or-later

use crate::SymbolType;
use super::{Decoder, decode_e};

const NUM_CFGS: usize = 2;

const CFG_MIN_LEN: usize = 0;
const CFG_MAX_LEN: usize = 1;

const DEFAULT_MIN_LEN: i32 = 6;
const DEFAULT_MAX_LEN: i32 = 0;

pub struct I25Decoder {
    pub direction: u8,
    pub element: u8,
    pub character: i16,
    pub s10: u32,
    pub width: u32,
    pub buf: [u8; 4],
    pub config: u32,
    pub configs: [i32; NUM_CFGS],
}

impl I25Decoder {
    pub fn new() -> Self {
        let mut configs = [0i32; NUM_CFGS];
        configs[CFG_MIN_LEN] = DEFAULT_MIN_LEN;
        configs[CFG_MAX_LEN] = DEFAULT_MAX_LEN;
        I25Decoder {
            direction: 0,
            element: 0,
            character: -1,
            s10: 0,
            width: 0,
            buf: [0; 4],
            config: 1,
            configs,
        }
    }

    pub fn reset(&mut self) {
        self.direction = 0;
        self.element = 0;
        self.character = -1;
        self.s10 = 0;
    }

    pub fn enabled(&self) -> bool {
        self.config & 1 != 0
    }
}

#[inline(always)]
fn i25_decode1(enc: u8, e: u32, s: u32) -> u8 {
    let e_val = decode_e(e, s, 45);
    if e_val < 0 || e_val > 7 {
        return 0xff;
    }
    let mut out = enc << 1;
    if e_val > 2 {
        out |= 1;
    }
    out
}

fn i25_decode10(dcode: &Decoder, offset: u8) -> u8 {
    let s10 = dcode.i25.s10;
    if s10 < 10 {
        return 0xff;
    }

    let mut enc: u8 = 0;
    let mut par: u8 = 0;
    let direction = dcode.i25.direction;

    // i = 8, 6, 4, 2, 0
    let mut i: i32 = 8;
    while i >= 0 {
        let j = if direction != 0 {
            offset.wrapping_add(i as u8)
        } else {
            offset.wrapping_add((8 - i) as u8)
        };
        enc = i25_decode1(enc, dcode.get_width(j), s10);
        if enc == 0xff {
            return 0xff;
        }
        if enc & 1 != 0 {
            par += 1;
        }
        i -= 2;
    }

    // parity check: exactly 2 wide elements out of 5
    if par != 2 {
        return 0xff;
    }

    // decode binary weights — only the low 4 bits matter after parity
    let mut v = enc & 0xf;
    if v & 8 != 0 {
        if v == 12 {
            v = 0;
        } else {
            v = v.wrapping_sub(1);
            if v > 9 {
                return 0xff;
            }
        }
    }
    v
}

fn i25_decode_start(dcode: &mut Decoder) -> SymbolType {
    let s10 = dcode.i25.s10;
    if s10 < 10 {
        return SymbolType::None;
    }

    let mut enc: u8 = 0;
    let mut i: u8 = 10;
    enc = i25_decode1(enc, dcode.get_width(i), s10); i += 1;
    enc = i25_decode1(enc, dcode.get_width(i), s10); i += 1;
    enc = i25_decode1(enc, dcode.get_width(i), s10); i += 1;

    if dcode.get_color() == 1 {
        // ZBAR_BAR (reverse scan): stop pattern reversed is enc==4 (W,N,N)
        if enc != 4 {
            return SymbolType::None;
        }
    } else {
        // ZBAR_SPACE (forward scan): start is 4 narrow elements; consume the 4th
        enc = i25_decode1(enc, dcode.get_width(i), s10);
        i += 1;
        if enc != 0 {
            return SymbolType::None;
        }
    }

    // leading quiet zone: spec is 10n; we require ≥ 3n/8 of s10 width
    let quiet = dcode.get_width(i);
    if quiet != 0 && quiet < s10 * 3 / 8 {
        return SymbolType::None;
    }

    dcode.i25.direction = dcode.get_color();
    dcode.i25.element = 1;
    dcode.i25.character = 0;
    SymbolType::Partial
}

fn i25_acquire_lock(dcode: &mut Decoder) -> bool {
    if dcode.get_lock(SymbolType::I25) {
        dcode.i25.character = -1;
        return true;
    }
    // copy holding buffer into shared buf
    for i in 0..4 {
        if i < dcode.buf.len() {
            dcode.buf[i] = dcode.i25.buf[i];
        }
    }
    false
}

fn i25_decode_end(dcode: &mut Decoder) -> SymbolType {
    let width = dcode.i25.width;

    // trailing quiet zone + 2 narrow elements after the data
    let quiet = dcode.get_width(0);
    let e1 = decode_e(dcode.get_width(1), width, 45);
    let e2 = decode_e(dcode.get_width(2), width, 45);
    if (quiet != 0 && quiet < width * 3 / 8)
        || e1 < 0 || e1 > 2
        || e2 < 0 || e2 > 2
    {
        return SymbolType::None;
    }

    let e3 = decode_e(dcode.get_width(3), width, 45);
    let bad = if dcode.i25.direction == 0 {
        // forward: element 3 must be a wide bar of moderate width (3..=7)
        e3 < 3 || e3 > 7
    } else {
        // reverse: elements 3, 4 both narrow
        if e3 < 0 || e3 > 2 {
            true
        } else {
            let e4 = decode_e(dcode.get_width(4), width, 45);
            e4 < 0 || e4 > 2
        }
    };
    if bad {
        return SymbolType::None;
    }

    // if we never crossed character==4, the lock was never grabbed — try now
    if dcode.i25.character <= 4 && i25_acquire_lock(dcode) {
        return SymbolType::Partial;
    }

    let character = dcode.i25.character as usize;

    if dcode.i25.direction != 0 {
        // reverse: in-place flip of decoded digits
        let half = character / 2;
        for i in 0..half {
            let j = character - 1 - i;
            dcode.buf.swap(i, j);
        }
    }

    let min_len = dcode.i25.configs[CFG_MIN_LEN];
    let max_len = dcode.i25.configs[CFG_MAX_LEN];
    if (dcode.i25.character as i32) < min_len
        || (max_len > 0 && (dcode.i25.character as i32) > max_len)
    {
        // invalid length: release lock & reset
        dcode.lock = SymbolType::None;
        dcode.i25.character = -1;
        return SymbolType::None;
    }

    if character >= dcode.buf.len() {
        dcode.lock = SymbolType::None;
        dcode.i25.character = -1;
        return SymbolType::None;
    }
    dcode.buflen = character;
    dcode.buf[character] = 0;
    dcode.i25.character = -1;
    SymbolType::I25
}

pub fn decode_i25(dcode: &mut Decoder) -> SymbolType {
    // 10-element sliding window: drop oldest, add newest
    let drop = dcode.get_width(10);
    let add = dcode.get_width(0);
    dcode.i25.s10 = dcode.i25.s10.wrapping_sub(drop).wrapping_add(add);

    if dcode.i25.character < 0 {
        let s = i25_decode_start(dcode);
        if s == SymbolType::None {
            return SymbolType::None;
        }
    }

    // 4-bit wrapping decrement of element
    dcode.i25.element = dcode.i25.element.wrapping_sub(1) & 0xf;
    let elem = dcode.i25.element;
    let end_marker = 6u8.wrapping_sub(dcode.i25.direction) & 0xf;
    if elem == end_marker {
        return i25_decode_end(dcode);
    } else if elem != 0 {
        return SymbolType::None;
    }

    // entering a fresh pair: lock in width from accumulated s10
    dcode.i25.width = dcode.i25.s10;

    // acquire lock at character==4 (5th-character boundary)
    if dcode.i25.character == 4 && i25_acquire_lock(dcode) {
        return SymbolType::Partial;
    }

    let c1 = i25_decode10(dcode, 1);
    if c1 > 9 {
        return reset_after_abort(dcode);
    }

    let needed = dcode.i25.character as usize + 3;
    if dcode.size_buf(needed) {
        return reset_after_abort(dcode);
    }

    // choose target buffer: private holding for first 4 chars, then shared buf
    let idx = dcode.i25.character as usize;
    write_char(dcode, idx, c1 + b'0');
    dcode.i25.character += 1;

    let c2 = i25_decode10(dcode, 0);
    if c2 > 9 {
        return reset_after_abort(dcode);
    }

    let idx2 = dcode.i25.character as usize;
    write_char(dcode, idx2, c2 + b'0');
    dcode.i25.character += 1;

    dcode.i25.element = 10;

    if dcode.i25.character == 2 {
        SymbolType::Partial
    } else {
        SymbolType::None
    }
}

#[inline]
fn write_char(dcode: &mut Decoder, idx: usize, byte: u8) {
    if dcode.i25.character >= 4 {
        if idx < dcode.buf.len() {
            dcode.buf[idx] = byte;
        }
    } else {
        if idx < 4 {
            dcode.i25.buf[idx] = byte;
        }
    }
}

fn reset_after_abort(dcode: &mut Decoder) -> SymbolType {
    if dcode.i25.character >= 4 {
        dcode.lock = SymbolType::None;
    }
    dcode.i25.character = -1;
    SymbolType::None
}
