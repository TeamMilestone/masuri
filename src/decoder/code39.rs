//! Code 39 barcode decoder
//! Rust port of zbar 0.23 decoder/code39.c
//! Original Copyright (C) 2008-2010 Jeff Brown <spadix@users.sourceforge.net>
//! LGPL-2.1-or-later

use crate::SymbolType;
use super::{Decoder, decode_e};

const NUM_CFGS: usize = 2;

const CFG_MIN_LEN: usize = 0;
const CFG_MAX_LEN: usize = 1;

// zbar default for Code 39 is MIN_LEN=1; we raise it to suppress false
// positives (Code 39 is 3-of-9 parity, weak against noise). EMS S10 is
// 13 chars, well above this floor. Mirrors the i25 port's reasoning.
const DEFAULT_MIN_LEN: i32 = 6;
const DEFAULT_MAX_LEN: i32 = 0;

const NUM_CHARS: usize = 0x2c; // 44

/// Coarse lookup: first 5 encoded element widths (5-bit enc) -> partial index.
/// High 2 bits select how the remaining 4 widths refine the index:
///   0x00 = direct, 0x40 = "4" (2 bits), 0x80 = "2 next", 0xc0 = "2 skip".
/// 0xff = invalid.
const CODE39_HI: [u8; 32] = [
    0x80, 0x42, 0x86, 0xc8, 0x4a, 0x8e, 0xd0, 0x12,
    0x93, 0xd5, 0x97, 0xff, 0xd9, 0x1b, 0xff, 0xff,
    0x5c, 0xa0, 0xe2, 0x24, 0xa5, 0xff, 0x27, 0xff,
    0xe8, 0x2a, 0xff, 0xff, 0x2b, 0xff, 0xff, 0xff,
];

struct Char39 {
    chk: u8,
    rev: u8,
    fwd: u8,
}

const CODE39_ENCODINGS: [Char39; NUM_CHARS] = [
    Char39 { chk: 0x07, rev: 0x1a, fwd: 0x20 }, // 00
    Char39 { chk: 0x0d, rev: 0x10, fwd: 0x03 }, // 01
    Char39 { chk: 0x13, rev: 0x17, fwd: 0x22 }, // 02
    Char39 { chk: 0x16, rev: 0x1d, fwd: 0x23 }, // 03
    Char39 { chk: 0x19, rev: 0x0d, fwd: 0x05 }, // 04
    Char39 { chk: 0x1c, rev: 0x13, fwd: 0x06 }, // 05
    Char39 { chk: 0x25, rev: 0x07, fwd: 0x0c }, // 06
    Char39 { chk: 0x2a, rev: 0x2a, fwd: 0x27 }, // 07
    Char39 { chk: 0x31, rev: 0x04, fwd: 0x0e }, // 08
    Char39 { chk: 0x34, rev: 0x00, fwd: 0x0f }, // 09
    Char39 { chk: 0x43, rev: 0x15, fwd: 0x25 }, // 0a
    Char39 { chk: 0x46, rev: 0x1c, fwd: 0x26 }, // 0b
    Char39 { chk: 0x49, rev: 0x0b, fwd: 0x08 }, // 0c
    Char39 { chk: 0x4c, rev: 0x12, fwd: 0x09 }, // 0d
    Char39 { chk: 0x52, rev: 0x19, fwd: 0x2b }, // 0e
    Char39 { chk: 0x58, rev: 0x0f, fwd: 0x00 }, // 0f
    Char39 { chk: 0x61, rev: 0x02, fwd: 0x11 }, // 10
    Char39 { chk: 0x64, rev: 0x09, fwd: 0x12 }, // 11
    Char39 { chk: 0x70, rev: 0x06, fwd: 0x13 }, // 12
    Char39 { chk: 0x85, rev: 0x24, fwd: 0x16 }, // 13
    Char39 { chk: 0x8a, rev: 0x29, fwd: 0x28 }, // 14
    Char39 { chk: 0x91, rev: 0x21, fwd: 0x18 }, // 15
    Char39 { chk: 0x94, rev: 0x2b, fwd: 0x19 }, // 16
    Char39 { chk: 0xa2, rev: 0x28, fwd: 0x29 }, // 17
    Char39 { chk: 0xa8, rev: 0x27, fwd: 0x2a }, // 18
    Char39 { chk: 0xc1, rev: 0x1f, fwd: 0x1b }, // 19
    Char39 { chk: 0xc4, rev: 0x26, fwd: 0x1c }, // 1a
    Char39 { chk: 0xd0, rev: 0x23, fwd: 0x1d }, // 1b
    Char39 { chk: 0x03, rev: 0x14, fwd: 0x1e }, // 1c
    Char39 { chk: 0x06, rev: 0x1b, fwd: 0x1f }, // 1d
    Char39 { chk: 0x09, rev: 0x0a, fwd: 0x01 }, // 1e
    Char39 { chk: 0x0c, rev: 0x11, fwd: 0x02 }, // 1f
    Char39 { chk: 0x12, rev: 0x18, fwd: 0x21 }, // 20
    Char39 { chk: 0x18, rev: 0x0e, fwd: 0x04 }, // 21
    Char39 { chk: 0x21, rev: 0x01, fwd: 0x0a }, // 22
    Char39 { chk: 0x24, rev: 0x08, fwd: 0x0b }, // 23
    Char39 { chk: 0x30, rev: 0x05, fwd: 0x0d }, // 24
    Char39 { chk: 0x42, rev: 0x16, fwd: 0x24 }, // 25
    Char39 { chk: 0x48, rev: 0x0c, fwd: 0x07 }, // 26
    Char39 { chk: 0x60, rev: 0x03, fwd: 0x10 }, // 27
    Char39 { chk: 0x81, rev: 0x1e, fwd: 0x14 }, // 28
    Char39 { chk: 0x84, rev: 0x25, fwd: 0x15 }, // 29
    Char39 { chk: 0x90, rev: 0x22, fwd: 0x17 }, // 2a
    Char39 { chk: 0xc0, rev: 0x20, fwd: 0x1a }, // 2b
];

/// charset: 0-9 A-Z - . SPACE $ / + % * (43 data + start/stop)
const CODE39_CHARACTERS: &[u8; NUM_CHARS] =
    b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ-. $/+%*";

pub struct Code39Decoder {
    pub direction: u8,   // scan direction: 0=fwd, 1=rev
    pub element: u8,     // element offset within character cycle (0-10)
    pub character: i16,  // character position in symbol (-1 = idle)
    pub s9: u32,         // current 9-element character width
    pub width: u32,      // last accepted character width
    pub config: u32,
    pub configs: [i32; NUM_CFGS],
}

impl Code39Decoder {
    pub fn new() -> Self {
        let mut configs = [0i32; NUM_CFGS];
        configs[CFG_MIN_LEN] = DEFAULT_MIN_LEN;
        configs[CFG_MAX_LEN] = DEFAULT_MAX_LEN;
        Code39Decoder {
            direction: 0,
            element: 0,
            character: -1,
            s9: 0,
            width: 0,
            config: 1,
            configs,
        }
    }

    pub fn reset(&mut self) {
        self.direction = 0;
        self.element = 0;
        self.character = -1;
        self.s9 = 0;
    }

    pub fn enabled(&self) -> bool {
        self.config & 1 != 0
    }
}

/// Threshold one element width into a 0/1 bit appended to `enc`.
/// Returns 0xff on out-of-range width. C: code39_decode1, decode_e(e, s, 72).
#[inline(always)]
fn code39_decode1(enc: u8, e: u32, s: u32) -> u8 {
    let e_val = decode_e(e, s, 72);
    if e_val < 0 || e_val > 18 {
        return 0xff;
    }
    let mut out = enc << 1;
    if e_val > 6 {
        out |= 1;
    }
    out
}

/// Decode a 9-element character. Returns the character code (0..43) or -1.
/// On success updates `width` to the character's width (s9).
fn code39_decode9(dcode: &mut Decoder) -> i8 {
    let s9 = dcode.code39.s9;
    if s9 < 9 {
        return -1;
    }

    // zbar의 고정 임계(decode_e, 2:1 비율 가정)는 3:1로 인쇄된 라벨에서 굵어진
    // narrow 스페이스를 wide로 오분류한다. Code 39는 문자당 정확히 3개가 wide
    // 라는 불변식이 있으므로, wide/narrow 분리가 뚜렷하면 상위 3개 폭을 wide로
    // 분류한다. 분리가 모호하면(동률 또는 비율 < 1.2) 기존 임계로 폴백.
    // 단 시작(`*`) 탐지에는 쓰지 않는다 — 관대한 분류는 노이즈에서 가짜 스타트를
    // 양산해 진짜 스타트를 가리고, 고정 임계의 노이즈 기각이 거기서는 필수다.
    let mut ws = [0u32; 9];
    for i in 0..9u8 {
        ws[i as usize] = dcode.get_width(i);
    }
    let mut sorted = ws;
    sorted.sort_unstable();
    let min_wide = sorted[6];
    let max_narrow = sorted[5];
    let split_ok = dcode.code39.character >= 0
        && min_wide > max_narrow && min_wide * 5 >= max_narrow * 6;

    let mut enc: u8 = 0;
    for i in 0..5u8 {
        enc = if split_ok {
            // 모아레로 1px까지 깎인 narrow도 sort 분류로는 유효하다 —
            // 0폭(NEON 레인 flush 잔여물)과 과대폭(> s9/4)만 기각한다.
            if ws[i as usize] == 0 || ws[i as usize] * 4 > s9 {
                return -1;
            }
            (enc << 1) | (ws[i as usize] >= min_wide) as u8
        } else {
            code39_decode1(enc, ws[i as usize], s9)
        };
        if enc == 0xff {
            return -1;
        }
    }
    // enc is now a 5-bit value (< 0x20)

    // coarse lookup of first 5 widths
    let mut idx = CODE39_HI[enc as usize];
    if idx == 0xff {
        return -1;
    }

    // encode remaining 4 widths (NB the first encoded bit is shifted out of u8)
    for i in 5..9u8 {
        enc = if split_ok {
            // 모아레로 1px까지 깎인 narrow도 sort 분류로는 유효하다 —
            // 0폭(NEON 레인 flush 잔여물)과 과대폭(> s9/4)만 기각한다.
            if ws[i as usize] == 0 || ws[i as usize] * 4 > s9 {
                return -1;
            }
            (enc << 1) | (ws[i as usize] >= min_wide) as u8
        } else {
            code39_decode1(enc, ws[i as usize], s9)
        };
        if enc == 0xff {
            return -1;
        }
    }

    // refine index using the high bits of the full encoding
    if idx & 0xc0 == 0x80 {
        idx = (idx & 0x3f) + ((enc >> 3) & 1);
    } else if idx & 0xc0 == 0xc0 {
        idx = (idx & 0x3f) + ((enc >> 2) & 1);
    } else if idx & 0xc0 != 0 {
        idx = (idx & 0x3f) + ((enc >> 2) & 3);
    }
    if idx as usize >= NUM_CHARS {
        return -1;
    }

    let c = &CODE39_ENCODINGS[idx as usize];
    if enc != c.chk {
        return -1;
    }

    dcode.code39.width = s9;
    if dcode.code39.direction != 0 {
        c.rev as i8
    } else {
        c.fwd as i8
    }
}

/// Detect the start `*` character + leading quiet zone.
fn code39_decode_start(dcode: &mut Decoder) -> SymbolType {
    let c = code39_decode9(dcode);
    // 0x2b = '*' read forward, 0x19 = '*' read reversed
    if c != 0x19 && c != 0x2b {
        return SymbolType::None;
    }
    if c == 0x19 {
        dcode.code39.direction ^= 1;
    }

    // leading quiet zone: spec is 10x; require >= s9/2
    let quiet = dcode.get_width(9);
    if quiet != 0 && quiet < dcode.code39.s9 / 2 {
        return SymbolType::None;
    }

    dcode.code39.element = 9;
    dcode.code39.character = 0;
    SymbolType::Partial
}

/// Reverse (if needed) and translate raw character codes into ASCII.
/// Always succeeds (returns true). C: code39_postprocess returns 0 on success.
fn code39_postprocess(dcode: &mut Decoder) -> bool {
    let character = dcode.code39.character as usize;
    if dcode.code39.direction != 0 {
        // reverse buffer in place
        for i in 0..(character / 2) {
            let j = character - 1 - i;
            dcode.buf.swap(i, j);
        }
    }
    for i in 0..character {
        let v = dcode.buf[i] as usize;
        dcode.buf[i] = if v < 0x2b {
            CODE39_CHARACTERS[v]
        } else {
            b'?'
        };
    }
    dcode.buflen = character;
    if character < dcode.buf.len() {
        dcode.buf[character] = 0;
    }
    true
}

/// Inter-character width consistency: w within +/-25% of ref.
/// C: check_width — 3*ref <= 4*w <= 5*ref.
#[inline(always)]
fn check_width(ref_w: u32, w: u32) -> bool {
    let dref = ref_w;
    let ref4 = ref_w * 4;
    let w4 = w * 4;
    ref4 - dref <= w4 && w4 <= ref4 + dref
}

pub fn decode_code39(dcode: &mut Decoder) -> SymbolType {
    // update latest 9-element character width (sliding window)
    let drop = dcode.get_width(9);
    let add = dcode.get_width(0);
    dcode.code39.s9 = dcode.code39.s9.wrapping_sub(drop).wrapping_add(add);

    if dcode.code39.character < 0 {
        // only attempt a start at a bar boundary
        if dcode.get_color() != 1 {
            return SymbolType::None;
        }
        return code39_decode_start(dcode);
    }

    // accumulate elements; act on the 9th (decode) and 10th (gap) only
    dcode.code39.element = (dcode.code39.element + 1) & 0x0f;
    if dcode.code39.element < 9 {
        return SymbolType::None;
    }

    if dcode.code39.element == 10 {
        let space = dcode.get_width(0);
        let character = dcode.code39.character;
        if character != 0 && dcode.buf[(character - 1) as usize] == 0x2b {
            // last decoded character was STOP `*` — trim and finalize
            dcode.code39.character -= 1;
            let character = dcode.code39.character;
            let width = dcode.code39.width;
            let min_len = dcode.code39.configs[CFG_MIN_LEN];
            let max_len = dcode.code39.configs[CFG_MAX_LEN];
            let mut sym = SymbolType::None;
            if space != 0 && space * 24 < width {
                // 정지 문자 뒤 여백. 규격은 narrow 10배지만 현장 라벨(AMT 운송장)은
                // 심볼 바로 옆 한 모듈 간격에 동반 Code-39 문자를 찍어 여백이 없다.
                // 여기까지 온 심볼은 시작/정지 문자 + MIN_LEN개 이상 + 문자 간 폭
                // 일관성을 이미 통과했으므로, 남은 확인은 "정지 바가 옆 잉크에
                // 붙지 않았다" 뿐 — narrow 모듈(≈ width/12)의 절반으로 충분하다.
            } else if (character as i32) < min_len
                || (max_len > 0 && (character as i32) > max_len)
            {
                // invalid length
            } else if code39_postprocess(dcode) {
                // FIXME checksum (mod 43) intentionally not enforced
                sym = SymbolType::Code39;
            }
            dcode.code39.character = -1;
            if sym == SymbolType::None && dcode.lock == SymbolType::Code39 {
                dcode.lock = SymbolType::None;
            }
            return sym;
        }
        if space > dcode.code39.width / 2 {
            // inter-character gap too wide — abort
            if character != 0 && dcode.lock == SymbolType::Code39 {
                dcode.lock = SymbolType::None;
            }
            dcode.code39.character = -1;
        }
        dcode.code39.element = 0;
        return SymbolType::None;
    }

    // element == 9: decode a fresh character
    if !check_width(dcode.code39.width, dcode.code39.s9) {
        if dcode.code39.character != 0 && dcode.lock == SymbolType::Code39 {
            dcode.lock = SymbolType::None;
        }
        dcode.code39.character = -1;
        return SymbolType::None;
    }

    let c = code39_decode9(dcode);

    // lock shared resources at the first character
    if dcode.code39.character == 0 && dcode.get_lock(SymbolType::Code39) {
        dcode.code39.character = -1;
        return SymbolType::Partial;
    }

    if c < 0 || dcode.size_buf((dcode.code39.character + 1) as usize) {
        if dcode.lock == SymbolType::Code39 {
            dcode.lock = SymbolType::None;
        }
        dcode.code39.character = -1;
        return SymbolType::None;
    }

    let idx = dcode.code39.character as usize;
    dcode.buf[idx] = c as u8;
    dcode.code39.character += 1;
    SymbolType::None
}
