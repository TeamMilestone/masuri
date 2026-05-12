//! Image scanner - orchestrates scanning rows/columns of a grayscale image.
//! Port of zbar/img_scanner.c

use crate::{Decoded, SymbolType};
use crate::scanner::Scanner;
use crate::decoder::{Decoder, DecodedSymbol};
use crate::qrcode::finder::QrFinderLine;
use crate::qrcode::scanner_fixup::qr_handler_fixup;
use rayon::prelude::*;
use std::collections::HashMap;

/// Full QR pipeline: cluster finder lines into centers, binarize the image,
/// run the orchestrator + text extractor, return one `Decoded` per QR.
/// Empty Vec when fewer than 9 finder lines have accumulated in either
/// direction (mirrors zbar's early-out in `_zbar_qr_decode`).
fn decode_qr_codes(
    gray: &[u8], width: i32, height: i32,
    hlines: &mut Vec<QrFinderLine>, vlines: &mut Vec<QrFinderLine>,
) -> Vec<Decoded> {
    if hlines.len() < 9 || vlines.len() < 9 { return Vec::new(); }
    use crate::qrcode::binarize::qr_binarize;
    use crate::qrcode::finder_centers::qr_finder_centers_locate;
    use crate::qrcode::isaac::{isaac_init_empty, IsaacCtx};
    use crate::qrcode::orchestrator::decode_image;
    use crate::qrcode::rs::RsGf256;
    use crate::qrcode::text::qr_code_data_list_extract_text;

    let centers = qr_finder_centers_locate(hlines, vlines);
    if centers.len() < 3 { return Vec::new(); }
    let bin = qr_binarize(gray, width, height);
    let gf = RsGf256::new();
    let mut isaac = IsaacCtx::new();
    isaac_init_empty(&mut isaac);
    let qrlist = decode_image(&gf, &mut isaac, &centers, &bin, width, height);
    let payloads = qr_code_data_list_extract_text(&qrlist);

    let mut out = Vec::with_capacity(payloads.len());
    for (text, indices) in payloads {
        // Use bounding-box centroid for x/y (first QR if SA group).
        let primary = indices[0];
        let bbox = qrlist[primary].bbox;
        let cx = (bbox[0][0] + bbox[1][0] + bbox[2][0] + bbox[3][0]) / 4;
        let cy = (bbox[0][1] + bbox[1][1] + bbox[2][1] + bbox[3][1]) / 4;
        out.push(Decoded {
            data: text,
            sym_type: SymbolType::QrCode,
            quality: indices.len() as i32,
            x: cx.max(0) as u32,
            y: cy.max(0) as u32,
        });
    }
    out
}

/// Run one decode_width call, then if a QR finder was detected, apply the
/// subpixel fixup and push into the appropriate (h or v) line vector.
#[inline(always)]
fn decode_width_and_capture(
    dcode: &mut Decoder,
    scn: &Scanner,
    width: u32,
    forward: bool,
    scanline_pixel: i32,
    umin: i32,
    is_row: bool,
    qr_lines: &mut Vec<QrFinderLine>,
) {
    if dcode.decode_width(width) {
        let fixed = qr_handler_fixup(
            &dcode.qr.line, scn, forward, scanline_pixel, umin, is_row,
        );
        qr_lines.push(fixed);
    }
}

/// Scan a single image (single-threaded)
pub fn scan_image(gray: &[u8], width: u32, height: u32) -> Vec<Decoded> {
    let mut scn = Scanner::new();
    let mut dcode = Decoder::new();
    let mut hlines: Vec<QrFinderLine> = Vec::new();
    let mut vlines: Vec<QrFinderLine> = Vec::new();

    scan_rows(gray, width, height, &mut scn, &mut dcode, 1, &mut hlines);
    scan_cols(gray, width, height, &mut scn, &mut dcode, 1, &mut vlines);

    let mut out = dedup_results(&dcode.results);
    let qr_results = decode_qr_codes(gray, width as i32, height as i32, &mut hlines, &mut vlines);
    out.extend(qr_results);
    out
}

/// Scan a single image with parallel row scanning
pub fn scan_image_parallel(gray: &[u8], width: u32, height: u32) -> Vec<Decoded> {
    let mut all_results = scan_at_scale(gray, width as usize, height as usize);

    // Multi-scale: try additional scales to catch barcodes missed at native resolution
    for pct in [75, 110, 50, 150] {
        if all_results.is_empty() || all_results.iter().all(|r| r.quality < 3) {
            let sw = (width as usize * pct / 100).max(1);
            let sh = (height as usize * pct / 100).max(1);
            let scaled = rescale(gray, width as usize, height as usize, pct);
            let mut extra = scan_at_scale(&scaled, sw, sh);
            // Map coordinates back to original image size
            for r in &mut extra {
                r.x = r.x * 100 / pct as u32;
                r.y = r.y * 100 / pct as u32;
            }
            all_results.extend(extra);
        } else {
            break;
        }
    }

    // Dedup across scales
    let mut seen: HashMap<String, Decoded> = HashMap::new();
    for r in all_results {
        let entry = seen.entry(r.data.clone()).or_insert(r.clone());
        if r.quality > entry.quality {
            *entry = r;
        }
    }
    seen.into_values().collect()
}

fn scan_at_scale(gray: &[u8], w: usize, h: usize) -> Vec<Decoded> {
    let density = 1usize;

    let mut row_indices: Vec<usize> = Vec::new();
    let border = (((h - 1) % density) + 1) / 2;
    let border = border.min(h / 2);
    let mut y = border;
    while y < h {
        row_indices.push(y);
        y += density;
    }

    let mut col_indices: Vec<usize> = Vec::new();
    let border_x = (((w - 1) % density) + 1) / 2;
    let border_x = border_x.min(w / 2);
    let mut x = border_x;
    while x < w {
        col_indices.push(x);
        x += density;
    }

    // Single par_iter over all row + col tasks — no barrier between phases
    let num_rows = row_indices.len();
    let total = num_rows + col_indices.len();

    let all_results: Vec<(Vec<DecodedSymbol>, Vec<QrFinderLine>, Vec<QrFinderLine>)> =
        (0..total)
        .into_par_iter()
        .map(|i| {
            let mut scn = Scanner::new();
            let mut dcode = Decoder::new();
            let mut hl: Vec<QrFinderLine> = Vec::new();
            let mut vl: Vec<QrFinderLine> = Vec::new();
            if i < num_rows {
                let y = row_indices[i];
                scan_single_row(gray, w, y, true, &mut scn, &mut dcode, &mut hl);
                scn.new_scan();
                dcode.new_scan();
                scan_single_row(gray, w, y, false, &mut scn, &mut dcode, &mut hl);
            } else {
                let x = col_indices[i - num_rows];
                scan_single_col(gray, w, h, x, true, &mut scn, &mut dcode, &mut vl);
                scn.new_scan();
                dcode.new_scan();
                scan_single_col(gray, w, h, x, false, &mut scn, &mut dcode, &mut vl);
            }
            (dcode.results, hl, vl)
        })
        .collect();

    let mut results: Vec<DecodedSymbol> = Vec::new();
    let mut hlines: Vec<QrFinderLine> = Vec::new();
    let mut vlines: Vec<QrFinderLine> = Vec::new();
    for (r, hl, vl) in all_results {
        results.extend(r);
        hlines.extend(hl);
        vlines.extend(vl);
    }
    let mut out = dedup_results(&results);
    let qr_results = decode_qr_codes(gray, w as i32, h as i32, &mut hlines, &mut vlines);
    out.extend(qr_results);
    out
}

fn rescale(gray: &[u8], w: usize, h: usize, pct: usize) -> Vec<u8> {
    use image::{GrayImage, imageops};
    let nw = (w as u32 * pct as u32 / 100).max(1);
    let nh = (h as u32 * pct as u32 / 100).max(1);
    let img = GrayImage::from_raw(w as u32, h as u32, gray.to_vec()).unwrap();
    let resized = imageops::resize(&img, nw, nh, imageops::FilterType::Triangle);
    resized.into_raw()
}

fn scan_single_row(
    gray: &[u8], w: usize, y: usize, forward: bool,
    scn: &mut Scanner, dcode: &mut Decoder,
    hlines: &mut Vec<QrFinderLine>,
) {
    scn.new_scan();
    dcode.new_scan();
    dcode.scanline_coord = y as u32;
    dcode.is_row_scan = true;
    let umin = if forward { 0i32 } else { (w as i32) - 1 };
    let v = y as i32;

    if forward {
        for x in 0..w {
            let d = gray[x + y * w] as i32;
            let result = scn.scan_y(d);
            if result.edge != crate::scanner::EdgeType::None {
                dcode.cross_offset = x as u32;
                decode_width_and_capture(dcode, scn, result.width, forward, v, umin, true, hlines);
            }
        }
    } else {
        for x in (0..w).rev() {
            let d = gray[x + y * w] as i32;
            let result = scn.scan_y(d);
            if result.edge != crate::scanner::EdgeType::None {
                dcode.cross_offset = x as u32;
                decode_width_and_capture(dcode, scn, result.width, forward, v, umin, true, hlines);
            }
        }
    }
    // quiet_border: flush + flush + new_scan (matches C zbar)
    let r = scn.flush();
    if r.edge != crate::scanner::EdgeType::None {
        decode_width_and_capture(dcode, scn, r.width, forward, v, umin, true, hlines);
    }
    let r = scn.flush();
    if r.edge != crate::scanner::EdgeType::None {
        decode_width_and_capture(dcode, scn, r.width, forward, v, umin, true, hlines);
    }
    decode_width_and_capture(dcode, scn, 0, forward, v, umin, true, hlines);
}

fn scan_single_col(
    gray: &[u8], w: usize, h: usize, x: usize, forward: bool,
    scn: &mut Scanner, dcode: &mut Decoder,
    vlines: &mut Vec<QrFinderLine>,
) {
    scn.new_scan();
    dcode.new_scan();
    dcode.scanline_coord = x as u32;
    dcode.is_row_scan = false;
    let umin = if forward { 0i32 } else { (h as i32) - 1 };
    let v = x as i32;

    if forward {
        for y in 0..h {
            let d = gray[x + y * w] as i32;
            let result = scn.scan_y(d);
            if result.edge != crate::scanner::EdgeType::None {
                dcode.cross_offset = y as u32;
                decode_width_and_capture(dcode, scn, result.width, forward, v, umin, false, vlines);
            }
        }
    } else {
        for y in (0..h).rev() {
            let d = gray[x + y * w] as i32;
            let result = scn.scan_y(d);
            if result.edge != crate::scanner::EdgeType::None {
                dcode.cross_offset = y as u32;
                decode_width_and_capture(dcode, scn, result.width, forward, v, umin, false, vlines);
            }
        }
    }
    let r = scn.flush();
    if r.edge != crate::scanner::EdgeType::None {
        decode_width_and_capture(dcode, scn, r.width, forward, v, umin, false, vlines);
    }
    let r = scn.flush();
    if r.edge != crate::scanner::EdgeType::None {
        decode_width_and_capture(dcode, scn, r.width, forward, v, umin, false, vlines);
    }
    decode_width_and_capture(dcode, scn, 0, forward, v, umin, false, vlines);
}

fn scan_rows(
    gray: &[u8], width: u32, height: u32,
    scn: &mut Scanner, dcode: &mut Decoder, density: u32,
    hlines: &mut Vec<QrFinderLine>,
) {
    let w = width as usize;
    let h = height as usize;
    let density = density as usize;

    let border = (((h - 1) % density) + 1) / 2;
    let border = border.min(h / 2);
    let mut y = border;

    scn.new_scan();

    while y < h {
        scan_single_row(gray, w, y, true, scn, dcode, hlines);

        y += density;
        if y >= h { break; }

        scan_single_row(gray, w, y, false, scn, dcode, hlines);

        y += density;
    }
}

fn scan_cols(
    gray: &[u8], width: u32, height: u32,
    scn: &mut Scanner, dcode: &mut Decoder, density: u32,
    vlines: &mut Vec<QrFinderLine>,
) {
    let w = width as usize;
    let h = height as usize;
    let density = density as usize;

    let border = (((w - 1) % density) + 1) / 2;
    let border = border.min(w / 2);
    let mut x = border;

    while x < w {
        scan_single_col(gray, w, h, x, true, scn, dcode, vlines);

        x += density;
        if x >= w { break; }

        scan_single_col(gray, w, h, x, false, scn, dcode, vlines);

        x += density;
    }
}

/// NEON-optimized parallel scan: 4 rows at a time with NEON SIMD + rayon
#[cfg(target_arch = "aarch64")]
pub fn scan_image_neon_parallel(gray: &[u8], width: u32, height: u32) -> Vec<Decoded> {
    use crate::scanner_neon::NeonScanner4;

    let w = width as usize;
    let h = height as usize;

    // Collect all scan tasks into a single list — no barriers between phases
    enum ScanTask {
        NeonRows([usize; 4]),
        ScalarRow(usize),
        NeonCols([usize; 4]),
        ScalarCol(usize),
    }

    let mut tasks: Vec<ScanTask> = Vec::new();

    // Row tasks
    let mut y = 0usize;
    while y + 3 < h {
        tasks.push(ScanTask::NeonRows([y, y + 1, y + 2, y + 3]));
        y += 4;
    }
    while y < h {
        tasks.push(ScanTask::ScalarRow(y));
        y += 1;
    }

    // Column tasks
    let mut x = 0usize;
    while x + 3 < w {
        tasks.push(ScanTask::NeonCols([x, x + 1, x + 2, x + 3]));
        x += 4;
    }
    while x < w {
        tasks.push(ScanTask::ScalarCol(x));
        x += 1;
    }

    // Single par_iter over all tasks — 4 barriers → 1
    let all_results: Vec<Vec<DecodedSymbol>> = tasks
        .par_iter()
        .map(|task| {
            match task {
                ScanTask::NeonRows(batch) => {
                    let mut all = Vec::new();
                    // Forward
                    {
                        let mut nscn = NeonScanner4::new();
                        let mut decoders = [Decoder::new(), Decoder::new(), Decoder::new(), Decoder::new()];
                        for lane in 0..4 {
                            decoders[lane].scanline_coord = batch[lane] as u32;
                            decoders[lane].is_row_scan = true;
                        }
                        for x in 0..w {
                            let edges = nscn.scan_y_4(
                                gray[x + batch[0] * w] as i32,
                                gray[x + batch[1] * w] as i32,
                                gray[x + batch[2] * w] as i32,
                                gray[x + batch[3] * w] as i32,
                            );
                            for lane in 0..4 {
                                if edges[lane].has_edge {
                                    decoders[lane].cross_offset = x as u32;
                                    decoders[lane].decode_width(edges[lane].width);
                                }
                            }
                        }
                        for lane in 0..4 {
                            let r = nscn.flush_lane(lane);
                            if r.has_edge { decoders[lane].decode_width(r.width); }
                            let r = nscn.flush_lane(lane);
                            if r.has_edge { decoders[lane].decode_width(r.width); }
                            decoders[lane].decode_width(0);
                        }
                        for d in &decoders { all.extend(d.results.iter().cloned()); }
                    }
                    // Reverse
                    {
                        let mut nscn = NeonScanner4::new();
                        let mut decoders = [Decoder::new(), Decoder::new(), Decoder::new(), Decoder::new()];
                        for lane in 0..4 {
                            decoders[lane].scanline_coord = batch[lane] as u32;
                            decoders[lane].is_row_scan = true;
                        }
                        for x in (0..w).rev() {
                            let edges = nscn.scan_y_4(
                                gray[x + batch[0] * w] as i32,
                                gray[x + batch[1] * w] as i32,
                                gray[x + batch[2] * w] as i32,
                                gray[x + batch[3] * w] as i32,
                            );
                            for lane in 0..4 {
                                if edges[lane].has_edge {
                                    decoders[lane].cross_offset = x as u32;
                                    decoders[lane].decode_width(edges[lane].width);
                                }
                            }
                        }
                        for lane in 0..4 {
                            let r = nscn.flush_lane(lane);
                            if r.has_edge { decoders[lane].decode_width(r.width); }
                            let r = nscn.flush_lane(lane);
                            if r.has_edge { decoders[lane].decode_width(r.width); }
                            decoders[lane].decode_width(0);
                        }
                        for d in &decoders { all.extend(d.results.iter().cloned()); }
                    }
                    all
                }
                ScanTask::ScalarRow(y) => {
                    let mut scn = Scanner::new();
                    let mut dcode = Decoder::new();
                    let mut hl: Vec<QrFinderLine> = Vec::new();
                    scan_single_row(gray, w, *y, true, &mut scn, &mut dcode, &mut hl);
                    scn.new_scan();
                    dcode.new_scan();
                    scan_single_row(gray, w, *y, false, &mut scn, &mut dcode, &mut hl);
                    // NEON path: QR finder lines emitted only by the scalar
                    // fallback rows/cols. Phase 6 native-aarch64 QR is gated
                    // behind `decode()` / `scan_image_parallel` which has the
                    // full pipeline; the NEON entry stays 1D-only.
                    let _ = hl;
                    dcode.results
                }
                ScanTask::NeonCols(batch) => {
                    let mut all = Vec::new();
                    // Forward
                    {
                        let mut nscn = NeonScanner4::new();
                        let mut decoders = [Decoder::new(), Decoder::new(), Decoder::new(), Decoder::new()];
                        for lane in 0..4 {
                            decoders[lane].scanline_coord = batch[lane] as u32;
                            decoders[lane].is_row_scan = false;
                        }
                        for y in 0..h {
                            let edges = nscn.scan_y_4(
                                gray[batch[0] + y * w] as i32,
                                gray[batch[1] + y * w] as i32,
                                gray[batch[2] + y * w] as i32,
                                gray[batch[3] + y * w] as i32,
                            );
                            for lane in 0..4 {
                                if edges[lane].has_edge {
                                    decoders[lane].cross_offset = y as u32;
                                    decoders[lane].decode_width(edges[lane].width);
                                }
                            }
                        }
                        for lane in 0..4 {
                            let r = nscn.flush_lane(lane);
                            if r.has_edge { decoders[lane].decode_width(r.width); }
                            let r = nscn.flush_lane(lane);
                            if r.has_edge { decoders[lane].decode_width(r.width); }
                            decoders[lane].decode_width(0);
                        }
                        for d in &decoders { all.extend(d.results.iter().cloned()); }
                    }
                    // Reverse
                    {
                        let mut nscn = NeonScanner4::new();
                        let mut decoders = [Decoder::new(), Decoder::new(), Decoder::new(), Decoder::new()];
                        for lane in 0..4 {
                            decoders[lane].scanline_coord = batch[lane] as u32;
                            decoders[lane].is_row_scan = false;
                        }
                        for y in (0..h).rev() {
                            let edges = nscn.scan_y_4(
                                gray[batch[0] + y * w] as i32,
                                gray[batch[1] + y * w] as i32,
                                gray[batch[2] + y * w] as i32,
                                gray[batch[3] + y * w] as i32,
                            );
                            for lane in 0..4 {
                                if edges[lane].has_edge {
                                    decoders[lane].cross_offset = y as u32;
                                    decoders[lane].decode_width(edges[lane].width);
                                }
                            }
                        }
                        for lane in 0..4 {
                            let r = nscn.flush_lane(lane);
                            if r.has_edge { decoders[lane].decode_width(r.width); }
                            let r = nscn.flush_lane(lane);
                            if r.has_edge { decoders[lane].decode_width(r.width); }
                            decoders[lane].decode_width(0);
                        }
                        for d in &decoders { all.extend(d.results.iter().cloned()); }
                    }
                    all
                }
                ScanTask::ScalarCol(x) => {
                    let mut scn = Scanner::new();
                    let mut dcode = Decoder::new();
                    let mut vl: Vec<QrFinderLine> = Vec::new();
                    scan_single_col(gray, w, h, *x, true, &mut scn, &mut dcode, &mut vl);
                    scn.new_scan();
                    dcode.new_scan();
                    scan_single_col(gray, w, h, *x, false, &mut scn, &mut dcode, &mut vl);
                    let _ = vl;
                    dcode.results
                }
            }
        })
        .collect();

    let mut results: Vec<DecodedSymbol> = Vec::new();
    for r in all_results { results.extend(r); }
    dedup_results(&results)
}

/// Diagnostic: single-scale parallel scan that also returns the count of
/// QR finder lines detected (sum across all rows/cols, before clustering).
pub fn scan_image_qr_diag(gray: &[u8], width: u32, height: u32) -> (Vec<Decoded>, usize) {
    let w = width as usize;
    let h = height as usize;

    let row_outputs: Vec<(Vec<DecodedSymbol>, Vec<QrFinderLine>)> = (0..h)
        .into_par_iter()
        .map(|y| {
            let mut scn = Scanner::new();
            let mut dcode = Decoder::new();
            let mut hl: Vec<QrFinderLine> = Vec::new();
            scan_single_row(gray, w, y, true, &mut scn, &mut dcode, &mut hl);
            scn.new_scan();
            dcode.new_scan();
            scan_single_row(gray, w, y, false, &mut scn, &mut dcode, &mut hl);
            (dcode.results, hl)
        })
        .collect();

    let col_outputs: Vec<(Vec<DecodedSymbol>, Vec<QrFinderLine>)> = (0..w)
        .into_par_iter()
        .map(|x| {
            let mut scn = Scanner::new();
            let mut dcode = Decoder::new();
            let mut vl: Vec<QrFinderLine> = Vec::new();
            scan_single_col(gray, w, h, x, true, &mut scn, &mut dcode, &mut vl);
            scn.new_scan();
            dcode.new_scan();
            scan_single_col(gray, w, h, x, false, &mut scn, &mut dcode, &mut vl);
            (dcode.results, vl)
        })
        .collect();

    let mut results: Vec<DecodedSymbol> = Vec::new();
    let mut hlines: Vec<QrFinderLine> = Vec::new();
    let mut vlines: Vec<QrFinderLine> = Vec::new();
    for (r, hl) in row_outputs { results.extend(r); hlines.extend(hl); }
    for (r, vl) in col_outputs { results.extend(r); vlines.extend(vl); }
    let qr_total = hlines.len() + vlines.len();
    let mut out = dedup_results(&results);
    let qr_results = decode_qr_codes(gray, width as i32, height as i32, &mut hlines, &mut vlines);
    out.extend(qr_results);
    (out, qr_total)
}

fn dedup_results(results: &[DecodedSymbol]) -> Vec<Decoded> {
    let mut groups: HashMap<(String, i32), Vec<(u32, u32)>> = HashMap::new();
    for r in results {
        let key = (r.data.clone(), r.sym_type as i32);
        groups.entry(key).or_default().push((r.x, r.y));
    }

    groups.into_iter()
        .map(|((data, sym_i32), positions)| {
            let quality = positions.len() as i32;
            let sym_type = match sym_i32 {
                8 => SymbolType::Ean8,
                9 => SymbolType::Upce,
                10 => SymbolType::Isbn10,
                12 => SymbolType::Upca,
                13 => SymbolType::Ean13,
                14 => SymbolType::Isbn13,
                25 => SymbolType::I25,
                64 => SymbolType::QrCode,
                128 => SymbolType::Code128,
                _ => SymbolType::None,
            };
            let mut xs: Vec<u32> = positions.iter().map(|p| p.0).collect();
            let mut ys: Vec<u32> = positions.iter().map(|p| p.1).collect();
            xs.sort_unstable();
            ys.sort_unstable();
            Decoded { data, sym_type, quality, x: xs[xs.len() / 2], y: ys[ys.len() / 2] }
        })
        .collect()
}
