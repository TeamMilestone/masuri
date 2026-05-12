//! Convert a width-unit `QrFinderLine` from the 1D scanner into subpixel
//! image coordinates. Rust port of `qr_handler` from zbar's img_scanner.c
//! (Phase 6 integration glue).

use crate::scanner::Scanner;
use super::finder::QrFinderLine;
use super::geom::QR_FINDER_SUBPREC;

/// Fix up a finder line detected during a scan. Mirrors zbar's `qr_handler`.
///
/// - `raw` is the width-units line as filled in by `Decoder::find_qr`.
/// - `scn` is the scanner at the moment of detection (its `last_edge`
///   anchors the subpixel positions).
/// - `forward` is the scan direction (true = forward scan; false = reverse).
/// - `scanline_pixel` is the cross-axis pixel coordinate (y for row scan,
///   x for col scan).
/// - `umin` is the starting pixel of the scan (0 for forward, width-1 or
///   height-1 for reverse).
/// - `is_row_scan` controls which of `pos[0]` / `pos[1]` carries the
///   scan-axis vs cross-axis subpixel position.
pub fn qr_handler_fixup(
    raw: &QrFinderLine,
    scn: &Scanner,
    forward: bool,
    scanline_pixel: i32,
    umin: i32,
    is_row_scan: bool,
) -> QrFinderLine {
    // Subpixel positions of the four reference points (current edge minus
    // width-unit offset, in QR_FINDER_SUBPREC fixed point).
    let u = scn.get_edge(raw.pos[0] as u32, QR_FINDER_SUBPREC) as i32;
    let b_pos = scn.get_edge(raw.boffs as u32, QR_FINDER_SUBPREC) as i32;
    let mut len_pos = scn.get_edge(raw.len as u32, QR_FINDER_SUBPREC) as i32;
    let e_pos = scn.get_edge(raw.eoffs as u32, QR_FINDER_SUBPREC) as i32;

    let mut boffs = u.wrapping_sub(b_pos);
    let mut eoffs = e_pos.wrapping_sub(len_pos);
    len_pos = len_pos.wrapping_sub(u);          // middle bar length
    let len_val = len_pos;                       // alias for readability

    // zbar: u = QR_FIXED(umin, 0) + du * u_raw.  QR_FIXED(v, 0) = v << SUBPREC.
    let du: i32 = if forward { 1 } else { -1 };
    let mut u_abs = (umin << QR_FINDER_SUBPREC).wrapping_add(du.wrapping_mul(u));
    if du < 0 {
        u_abs = u_abs.wrapping_sub(len_val);
        std::mem::swap(&mut boffs, &mut eoffs);
    }

    // Cross-axis: QR_FIXED(v, 1) = (v << SUBPREC) + (1 << (SUBPREC-1)).
    let v_fixed = (scanline_pixel << QR_FINDER_SUBPREC)
        + (1 << (QR_FINDER_SUBPREC - 1));

    // Row scan (is_row_scan=true) ⇒ vert=0: pos[0] = u_abs (x), pos[1] = v_fixed (y).
    // Col scan (is_row_scan=false) ⇒ vert=1: pos[0] = v_fixed (x), pos[1] = u_abs (y).
    let pos = if is_row_scan {
        [u_abs, v_fixed]
    } else {
        [v_fixed, u_abs]
    };

    QrFinderLine {
        pos,
        len: len_val,
        boffs,
        eoffs,
    }
}
