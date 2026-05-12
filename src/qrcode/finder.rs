//! QR finder line — 1:1:3:1:1 pattern detection.
//! Rust port of zbar/decoder/qr_finder.{c,h} + qrcode.h::qr_finder_line.
//! Original Copyright (C) 2008-2009 Timothy B. Terriberry (tterribe@xiph.org)
//! LGPL-2.1-or-later

/// A line crossing a finder pattern (horizontal or vertical — direction by context).
/// Mirrors `qr_finder_line` in zbar/qrcode.h.
///
/// In Phase 1 the fields are filled in *width-units* by the 1D scanner. Phase 4-F
/// will fix them up to subpixel coordinates against the scanner state.
#[derive(Debug, Clone, Default)]
pub struct QrFinderLine {
    pub pos: [i32; 2],
    pub len: i32,
    pub boffs: i32,
    pub eoffs: i32,
}

/// Per-decoder finder state. Mirrors `qr_finder_t` in zbar/decoder/qr_finder.h.
#[derive(Debug, Clone, Default)]
pub struct QrFinderState {
    /// Sliding sum of the last 5 widths (positions 1..5 in zbar offset semantics).
    pub s5: u32,
    pub line: QrFinderLine,
}

impl QrFinderState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset(&mut self) {
        self.s5 = 0;
    }
}
