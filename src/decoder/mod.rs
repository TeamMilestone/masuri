//! Decoder hub - multiplexes bar width stream to parallel decoders.
//! Rust port of zbar/decoder.c + decoder.h
//! Original Copyright (C) 2007-2009 Jeff Brown <spadix@users.sourceforge.net>
//! LGPL-2.1-or-later

pub mod ean;
pub mod code128;

use crate::SymbolType;

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

    // Collected results for current scan line
    pub results: Vec<DecodedSymbol>,
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
            results: Vec::new(),
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
    }

    pub fn new_scan(&mut self) {
        self.w = [0; DECODE_WINDOW];
        self.lock = SymbolType::None;
        self.idx = 0;
        self.ean.new_scan();
        self.code128.reset();
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

    /// Process one bar/space width through all enabled decoders
    #[inline(always)]
    pub fn decode_width(&mut self, width: u32) {
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

        self.idx = self.idx.wrapping_add(1);

        if self.sym_type != SymbolType::None && self.sym_type as i32 > SymbolType::Partial as i32 {
            // Collect decoded symbol
            let data = String::from_utf8_lossy(&self.buf[..self.buflen]).to_string();
            self.results.push(DecodedSymbol {
                sym_type: self.sym_type,
                data,
            });
            if self.lock != SymbolType::None && self.sym_type as i32 > SymbolType::Partial as i32 {
                self.lock = SymbolType::None;
            }
        }
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
