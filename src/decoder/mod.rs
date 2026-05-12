//! Decoder hub - multiplexes bar width stream to parallel decoders.
//! Rust port of zbar/decoder.c + decoder.h
//! Original Copyright (C) 2007-2009 Jeff Brown <spadix@users.sourceforge.net>
//! LGPL-2.1-or-later

pub mod ean;
pub mod code128;
pub mod i25;

use crate::SymbolType;
use crate::qrcode::finder::QrFinderState;

const DECODE_WINDOW: usize = 16;
const BUFFER_MIN: usize = 0x20;
const BUFFER_MAX: usize = 0x100;
const BUFFER_INCR: usize = 0x10;

#[inline(always)]
fn test_cfg(config: u32, cfg: u32) -> bool {
    (config >> cfg) & 1 != 0
}

/// Decoded symbol from width stream
#[derive(Debug, Clone)]
pub struct DecodedSymbol {
    pub sym_type: SymbolType,
    pub data: String,
    pub x: u32,
    pub y: u32,
}

pub struct Decoder {
    pub idx: u8,
    pub w: [u32; DECODE_WINDOW],
    pub sym_type: SymbolType,
    pub lock: SymbolType,

    pub buf: Vec<u8>,
    pub buflen: usize,

    pub ean: ean::EanDecoder,
    pub code128: code128::Code128Decoder,
    pub i25: i25::I25Decoder,
    pub qr: QrFinderState,

    // Collected results for current scan line
    pub results: Vec<DecodedSymbol>,

    // Position tracking (set by img_scanner before scanning)
    pub scanline_coord: u32,  // row scan: y, col scan: x (exact axis)
    pub cross_offset: u32,    // row scan: x, col scan: y (approximate axis)
    pub is_row_scan: bool,    // true = row scan, false = col scan
}

impl Decoder {
    pub fn new() -> Self {
        let mut d = Decoder {
            idx: 0,
            w: [0; DECODE_WINDOW],
            sym_type: SymbolType::None,
            lock: SymbolType::None,
            buf: vec![0u8; BUFFER_MIN],
            buflen: 0,
            ean: ean::EanDecoder::new(),
            code128: code128::Code128Decoder::new(),
            i25: i25::I25Decoder::new(),
            qr: QrFinderState::new(),
            results: Vec::new(),
            scanline_coord: 0,
            cross_offset: 0,
            is_row_scan: true,
        };
        d.reset();
        d
    }

    pub fn reset(&mut self) {
        self.idx = 0;
        self.w = [0; DECODE_WINDOW];
        self.sym_type = SymbolType::None;
        self.lock = SymbolType::None;
        self.ean.reset();
        self.code128.reset();
        self.i25.reset();
        self.qr.reset();
    }

    pub fn new_scan(&mut self) {
        self.w = [0; DECODE_WINDOW];
        self.lock = SymbolType::None;
        self.idx = 0;
        self.ean.new_scan();
        self.code128.reset();
        self.i25.reset();
        self.qr.reset();
    }

    #[inline(always)]
    pub fn get_color(&self) -> u8 {
        self.idx & 1
    }

    #[inline(always)]
    pub fn get_width(&self, offset: u8) -> u32 {
        self.w[(self.idx.wrapping_sub(offset) as usize) & (DECODE_WINDOW - 1)]
    }

    #[inline(always)]
    pub fn pair_width(&self, offset: u8) -> u32 {
        self.get_width(offset) + self.get_width(offset + 1)
    }

    #[inline(always)]
    pub fn calc_s(&self, offset: u8, n: u8) -> u32 {
        let mut s = 0u32;
        for i in 0..n {
            s += self.get_width(offset + i);
        }
        s
    }

    pub fn get_lock(&mut self, req: SymbolType) -> bool {
        if self.lock != SymbolType::None {
            return true; // locked
        }
        self.lock = req;
        false
    }

    /// Detect a 1:1:3:1:1 QR finder pattern at the current decode window.
    /// Mirrors `_zbar_find_qr` in zbar/decoder/qr_finder.c.
    ///
    /// On detection, fills `self.qr.line` in width-units (pos[0]=pos[1] until the
    /// img_scanner applies subpixel + direction fixup) and returns true.
    #[inline]
    fn find_qr(&mut self) -> bool {
        // sliding sum: drop width at offset 6, add width at offset 1
        self.qr.s5 = self.qr.s5
            .wrapping_sub(self.get_width(6))
            .wrapping_add(self.get_width(1));
        let s = self.qr.s5;

        // current width must be a SPACE (color==0) and total span >= 7 modules
        if self.get_color() != 0 || s < 7 {
            return false;
        }

        // 1:1:3:1:1 ratio check via decode_e on consecutive pairs
        if decode_e(self.pair_width(1), s, 7) != 0 { return false; }
        if decode_e(self.pair_width(2), s, 7) != 2 { return false; }
        if decode_e(self.pair_width(3), s, 7) != 2 { return false; }
        if decode_e(self.pair_width(4), s, 7) != 0 { return false; }

        // valid finder — record line in width-units
        let qz = self.get_width(0);
        let w1 = self.get_width(1);
        self.qr.line.eoffs = (qz + (w1 + 1) / 2) as i32;
        self.qr.line.len = (qz + w1 + self.get_width(2)) as i32;
        let pos0 = self.qr.line.len + self.get_width(3) as i32;
        self.qr.line.pos = [pos0, pos0];
        let w5 = self.get_width(5);
        let boffs = pos0 as u32 + self.get_width(4) + (w5 + 1) / 2;
        self.qr.line.boffs = boffs as i32;
        true
    }

    pub fn size_buf(&mut self, len: usize) -> bool {
        if len <= self.buf.len() {
            return false;
        }
        if len > BUFFER_MAX {
            return true; // overflow
        }
        let new_len = (len.max(self.buf.len() + BUFFER_INCR)).min(BUFFER_MAX);
        self.buf.resize(new_len, 0);
        false
    }

    /// Process one bar/space width through all enabled decoders.
    /// Returns `true` if a QR finder pattern was detected on this width
    /// (caller reads `self.qr.line` for the width-unit coordinates).
    #[inline(always)]
    pub fn decode_width(&mut self, width: u32) -> bool {
        self.w[(self.idx as usize) & (DECODE_WINDOW - 1)] = width;
        self.sym_type = SymbolType::None;

        // 0.23: update shared 6-element character width
        // used by Code128 and others for cross-decoder width estimation

        // EAN decoder
        if self.ean.enable {
            let sym = ean::decode_ean(self);
            if sym != SymbolType::None {
                self.sym_type = sym;
            }
        }

        // Code 128 decoder
        if self.code128.enabled() {
            let sym = code128::decode_code128(self);
            if sym as i32 > SymbolType::Partial as i32 {
                self.sym_type = sym;
            }
        }

        // Interleaved 2 of 5 decoder
        if self.i25.enabled() {
            let sym = i25::decode_i25(self);
            if sym as i32 > SymbolType::Partial as i32 {
                self.sym_type = sym;
            }
        }

        // QR finder line detector (1:1:3:1:1). Always-on for now.
        // Returned to caller; doesn't update sym_type — the 1D collection
        // branch below stays untouched. img_scanner runs the qr_handler
        // (subpixel fixup + direction-aware swap) and pushes into the
        // image-level h/v line vectors.
        let qr_found = self.find_qr();

        self.idx = self.idx.wrapping_add(1);

        if self.sym_type != SymbolType::None && self.sym_type as i32 > SymbolType::Partial as i32 {
            // Collect decoded symbol
            let data = String::from_utf8_lossy(&self.buf[..self.buflen]).to_string();
            let (x, y) = if self.is_row_scan {
                (self.cross_offset, self.scanline_coord)
            } else {
                (self.scanline_coord, self.cross_offset)
            };
            self.results.push(DecodedSymbol {
                sym_type: self.sym_type,
                data,
                x,
                y,
            });
            if self.lock != SymbolType::None && self.sym_type as i32 > SymbolType::Partial as i32 {
                self.lock = SymbolType::None;
            }
        }
        qr_found
    }
}

/// Fixed character width decode assist (decode_e from decoder.h)
/// C original: unsigned char E = ((e * n * 2 + 1) / s - 3) / 2;
/// return (E >= n - 3) ? -1 : E;
#[inline(always)]
pub fn decode_e(e: u32, s: u32, n: u32) -> i32 {
    if s == 0 {
        return -1;
    }
    let raw = (e * n * 2 + 1) / s;
    if raw < 3 {
        return -1;  // would underflow in C unsigned arithmetic
    }
    let e_val = (raw - 3) / 2;
    if e_val >= n - 3 {
        -1
    } else {
        e_val as i32
    }
}
