//! Masuri - Fast barcode decoder
//!
//! Rust port of the ZBar Bar Code Reader (http://zbar.sourceforge.net)
//! Original C code Copyright (C) 2007-2011 Jeff Brown <spadix@users.sourceforge.net>
//! Rust port Copyright (C) 2026 wonsup
//!
//! Licensed under the GNU Lesser General Public License v2.1 or later.
//! See LICENSE file for details.

pub mod scanner;
#[cfg(target_arch = "aarch64")]
pub mod scanner_neon;
pub mod decoder;
pub mod img_scanner;

/// Decoded barcode result
#[derive(Debug, Clone)]
pub struct Decoded {
    pub data: String,
    pub sym_type: SymbolType,
    pub quality: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum SymbolType {
    None = 0,
    Partial = 1,
    Ean2 = 2,
    Ean5 = 5,
    Ean8 = 8,
    Upce = 9,
    Isbn10 = 10,
    Upca = 12,
    Ean13 = 13,
    Isbn13 = 14,
    I25 = 25,
    Code39 = 39,
    Code128 = 128,
}

impl std::fmt::Display for SymbolType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SymbolType::None => write!(f, "NONE"),
            SymbolType::Partial => write!(f, "PARTIAL"),
            SymbolType::Ean2 => write!(f, "EAN-2"),
            SymbolType::Ean5 => write!(f, "EAN-5"),
            SymbolType::Ean8 => write!(f, "EAN-8"),
            SymbolType::Upce => write!(f, "UPC-E"),
            SymbolType::Isbn10 => write!(f, "ISBN-10"),
            SymbolType::Upca => write!(f, "UPC-A"),
            SymbolType::Ean13 => write!(f, "EAN-13"),
            SymbolType::Isbn13 => write!(f, "ISBN-13"),
            SymbolType::I25 => write!(f, "I2/5"),
            SymbolType::Code39 => write!(f, "CODE-39"),
            SymbolType::Code128 => write!(f, "CODE-128"),
        }
    }
}

/// Decode barcodes from a grayscale image buffer (uses parallel scanning by default)
pub fn decode(gray: &[u8], width: u32, height: u32) -> Vec<Decoded> {
    img_scanner::scan_image_parallel(gray, width, height)
}

/// Decode barcodes from a grayscale image buffer using parallel scanning
pub fn decode_parallel(gray: &[u8], width: u32, height: u32) -> Vec<Decoded> {
    img_scanner::scan_image_parallel(gray, width, height)
}

/// Decode with NEON SIMD + rayon (aarch64 only)
#[cfg(target_arch = "aarch64")]
pub fn decode_neon_parallel(gray: &[u8], width: u32, height: u32) -> Vec<Decoded> {
    img_scanner::scan_image_neon_parallel(gray, width, height)
}

#[inline(always)]
pub fn test_cfg_inline(config: u32, cfg: u32) -> bool {
    (config >> cfg) & 1 != 0
}
