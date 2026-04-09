//! Image scanner - orchestrates scanning rows/columns of a grayscale image.
//! Port of zbar/img_scanner.c

use crate::{Decoded, SymbolType};
use crate::scanner::Scanner;
use crate::decoder::{Decoder, DecodedSymbol};
use rayon::prelude::*;
use std::collections::HashMap;

/// Scan a single image (single-threaded)
pub fn scan_image(gray: &[u8], width: u32, height: u32) -> Vec<Decoded> {
    let mut scn = Scanner::new();
    let mut dcode = Decoder::new();

    scan_rows(gray, width, height, &mut scn, &mut dcode, 1);
    scan_cols(gray, width, height, &mut scn, &mut dcode, 1);

    dedup_results(&dcode.results)
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

    let all_results: Vec<Vec<DecodedSymbol>> = (0..total)
        .into_par_iter()
        .map(|i| {
            let mut scn = Scanner::new();
            let mut dcode = Decoder::new();
            if i < num_rows {
                let y = row_indices[i];
                scan_single_row(gray, w, y, true, &mut scn, &mut dcode);
                scn.new_scan();
                dcode.new_scan();
                scan_single_row(gray, w, y, false, &mut scn, &mut dcode);
            } else {
                let x = col_indices[i - num_rows];
                scan_single_col(gray, w, h, x, true, &mut scn, &mut dcode);
                scn.new_scan();
                dcode.new_scan();
                scan_single_col(gray, w, h, x, false, &mut scn, &mut dcode);
            }
            dcode.results
        })
        .collect();

    let mut results: Vec<DecodedSymbol> = Vec::new();
    for r in all_results { results.extend(r); }
    dedup_results(&results)
}

fn rescale(gray: &[u8], w: usize, h: usize, pct: usize) -> Vec<u8> {
    use image::{GrayImage, imageops};
    let nw = (w as u32 * pct as u32 / 100).max(1);
    let nh = (h as u32 * pct as u32 / 100).max(1);
    let img = GrayImage::from_raw(w as u32, h as u32, gray.to_vec()).unwrap();
    let resized = imageops::resize(&img, nw, nh, imageops::FilterType::Triangle);
    resized.into_raw()
}

fn scan_single_row(gray: &[u8], w: usize, y: usize, forward: bool, scn: &mut Scanner, dcode: &mut Decoder) {
    scn.new_scan();
    dcode.new_scan();
    dcode.scanline_coord = y as u32;
    dcode.is_row_scan = true;

    if forward {
        for x in 0..w {
            let d = gray[x + y * w] as i32;
            let result = scn.scan_y(d);
            if result.edge != crate::scanner::EdgeType::None {
                dcode.cross_offset = x as u32;
                dcode.decode_width(result.width);
            }
        }
    } else {
        for x in (0..w).rev() {
            let d = gray[x + y * w] as i32;
            let result = scn.scan_y(d);
            if result.edge != crate::scanner::EdgeType::None {
                dcode.cross_offset = x as u32;
                dcode.decode_width(result.width);
            }
        }
    }
    // quiet_border: flush + flush + new_scan (matches C zbar)
    let r = scn.flush();
    if r.edge != crate::scanner::EdgeType::None { dcode.decode_width(r.width); }
    let r = scn.flush();
    if r.edge != crate::scanner::EdgeType::None { dcode.decode_width(r.width); }
    // C zbar sends width=0 on final flush to signal end of scan line
    dcode.decode_width(0);
}

fn scan_single_col(gray: &[u8], w: usize, h: usize, x: usize, forward: bool, scn: &mut Scanner, dcode: &mut Decoder) {
    scn.new_scan();
    dcode.new_scan();
    dcode.scanline_coord = x as u32;
    dcode.is_row_scan = false;

    if forward {
        for y in 0..h {
            let d = gray[x + y * w] as i32;
            let result = scn.scan_y(d);
            if result.edge != crate::scanner::EdgeType::None {
                dcode.cross_offset = y as u32;
                dcode.decode_width(result.width);
            }
        }
    } else {
        for y in (0..h).rev() {
            let d = gray[x + y * w] as i32;
            let result = scn.scan_y(d);
            if result.edge != crate::scanner::EdgeType::None {
                dcode.cross_offset = y as u32;
                dcode.decode_width(result.width);
            }
        }
    }
    let r = scn.flush();
    if r.edge != crate::scanner::EdgeType::None { dcode.decode_width(r.width); }
    let r = scn.flush();
    if r.edge != crate::scanner::EdgeType::None { dcode.decode_width(r.width); }
    dcode.decode_width(0);
}

fn scan_rows(gray: &[u8], width: u32, height: u32, scn: &mut Scanner, dcode: &mut Decoder, density: u32) {
    let w = width as usize;
    let h = height as usize;
    let density = density as usize;

    let border = (((h - 1) % density) + 1) / 2;
    let border = border.min(h / 2);
    let mut y = border;

    scn.new_scan();

    while y < h {
        // Forward scan
        scan_single_row(gray, w, y, true, scn, dcode);

        y += density;
        if y >= h { break; }

        // Reverse scan
        scan_single_row(gray, w, y, false, scn, dcode);

        y += density;
    }
}

fn scan_cols(gray: &[u8], width: u32, height: u32, scn: &mut Scanner, dcode: &mut Decoder, density: u32) {
    let w = width as usize;
    let h = height as usize;
    let density = density as usize;

    let border = (((w - 1) % density) + 1) / 2;
    let border = border.min(w / 2);
    let mut x = border;

    while x < w {
        scan_single_col(gray, w, h, x, true, scn, dcode);

        x += density;
        if x >= w { break; }

        scan_single_col(gray, w, h, x, false, scn, dcode);

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
                    scan_single_row(gray, w, *y, true, &mut scn, &mut dcode);
                    scn.new_scan();
                    dcode.new_scan();
                    scan_single_row(gray, w, *y, false, &mut scn, &mut dcode);
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
                    scan_single_col(gray, w, h, *x, true, &mut scn, &mut dcode);
                    scn.new_scan();
                    dcode.new_scan();
                    scan_single_col(gray, w, h, *x, false, &mut scn, &mut dcode);
                    dcode.results
                }
            }
        })
        .collect();

    let mut results: Vec<DecodedSymbol> = Vec::new();
    for r in all_results { results.extend(r); }
    dedup_results(&results)
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
