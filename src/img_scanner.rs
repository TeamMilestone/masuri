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
            let extra = scan_at_scale(&scaled, sw, sh);
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

    let row_results: Vec<Vec<DecodedSymbol>> = row_indices
        .par_iter()
        .map(|&y| {
            let mut scn = Scanner::new();
            let mut dcode = Decoder::new();
            scan_single_row(gray, w, y, true, &mut scn, &mut dcode);
            scn.new_scan();
            dcode.new_scan();
            scan_single_row(gray, w, y, false, &mut scn, &mut dcode);
            dcode.results
        })
        .collect();

    let mut col_indices: Vec<usize> = Vec::new();
    let border_x = (((w - 1) % density) + 1) / 2;
    let border_x = border_x.min(w / 2);
    let mut x = border_x;
    while x < w {
        col_indices.push(x);
        x += density;
    }

    let col_results: Vec<Vec<DecodedSymbol>> = col_indices
        .par_iter()
        .map(|&x| {
            let mut scn = Scanner::new();
            let mut dcode = Decoder::new();
            scan_single_col(gray, w, h, x, true, &mut scn, &mut dcode);
            scn.new_scan();
            dcode.new_scan();
            scan_single_col(gray, w, h, x, false, &mut scn, &mut dcode);
            dcode.results
        })
        .collect();

    let mut all_results: Vec<DecodedSymbol> = Vec::new();
    for r in row_results { all_results.extend(r); }
    for r in col_results { all_results.extend(r); }

    dedup_results(&all_results)
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

    if forward {
        for x in 0..w {
            let d = gray[x + y * w] as i32;
            let result = scn.scan_y(d);
            if result.edge != crate::scanner::EdgeType::None {
                dcode.decode_width(result.width);
            }
        }
    } else {
        for x in (0..w).rev() {
            let d = gray[x + y * w] as i32;
            let result = scn.scan_y(d);
            if result.edge != crate::scanner::EdgeType::None {
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

    if forward {
        for y in 0..h {
            let d = gray[x + y * w] as i32;
            let result = scn.scan_y(d);
            if result.edge != crate::scanner::EdgeType::None {
                dcode.decode_width(result.width);
            }
        }
    } else {
        for y in (0..h).rev() {
            let d = gray[x + y * w] as i32;
            let result = scn.scan_y(d);
            if result.edge != crate::scanner::EdgeType::None {
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

    // Group rows into batches of 4 for NEON
    let mut row_batches: Vec<[usize; 4]> = Vec::new();
    let mut y = 0usize;
    while y + 3 < h {
        row_batches.push([y, y + 1, y + 2, y + 3]);
        y += 4;
    }
    // Remaining rows handled by scalar
    let remaining_rows: Vec<usize> = (y..h).collect();

    // NEON batched row scanning with rayon
    let neon_row_results: Vec<Vec<DecodedSymbol>> = row_batches
        .par_iter()
        .map(|batch| {
            let mut all = Vec::new();
            // Forward scan with NEON
            {
                let mut nscn = NeonScanner4::new();
                let mut decoders = [Decoder::new(), Decoder::new(), Decoder::new(), Decoder::new()];
                for x in 0..w {
                    let edges = nscn.scan_y_4(
                        gray[x + batch[0] * w] as i32,
                        gray[x + batch[1] * w] as i32,
                        gray[x + batch[2] * w] as i32,
                        gray[x + batch[3] * w] as i32,
                    );
                    for lane in 0..4 {
                        if edges[lane].has_edge {
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
            // Reverse scan with NEON
            {
                let mut nscn = NeonScanner4::new();
                let mut decoders = [Decoder::new(), Decoder::new(), Decoder::new(), Decoder::new()];
                for x in (0..w).rev() {
                    let edges = nscn.scan_y_4(
                        gray[x + batch[0] * w] as i32,
                        gray[x + batch[1] * w] as i32,
                        gray[x + batch[2] * w] as i32,
                        gray[x + batch[3] * w] as i32,
                    );
                    for lane in 0..4 {
                        if edges[lane].has_edge {
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
        })
        .collect();

    // Scalar scan for remaining rows
    let scalar_row_results: Vec<Vec<DecodedSymbol>> = remaining_rows
        .par_iter()
        .map(|&y| {
            let mut scn = Scanner::new();
            let mut dcode = Decoder::new();
            scan_single_row(gray, w, y, true, &mut scn, &mut dcode);
            scn.new_scan();
            dcode.new_scan();
            scan_single_row(gray, w, y, false, &mut scn, &mut dcode);
            dcode.results
        })
        .collect();

    // Column scanning: group 4 columns with NEON
    let mut col_batches: Vec<[usize; 4]> = Vec::new();
    let mut x = 0usize;
    while x + 3 < w {
        col_batches.push([x, x + 1, x + 2, x + 3]);
        x += 4;
    }
    let remaining_cols: Vec<usize> = (x..w).collect();

    let neon_col_results: Vec<Vec<DecodedSymbol>> = col_batches
        .par_iter()
        .map(|batch| {
            let mut all = Vec::new();
            // Forward
            {
                let mut nscn = NeonScanner4::new();
                let mut decoders = [Decoder::new(), Decoder::new(), Decoder::new(), Decoder::new()];
                for y in 0..h {
                    let edges = nscn.scan_y_4(
                        gray[batch[0] + y * w] as i32,
                        gray[batch[1] + y * w] as i32,
                        gray[batch[2] + y * w] as i32,
                        gray[batch[3] + y * w] as i32,
                    );
                    for lane in 0..4 {
                        if edges[lane].has_edge {
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
                for y in (0..h).rev() {
                    let edges = nscn.scan_y_4(
                        gray[batch[0] + y * w] as i32,
                        gray[batch[1] + y * w] as i32,
                        gray[batch[2] + y * w] as i32,
                        gray[batch[3] + y * w] as i32,
                    );
                    for lane in 0..4 {
                        if edges[lane].has_edge {
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
        })
        .collect();

    let scalar_col_results: Vec<Vec<DecodedSymbol>> = remaining_cols
        .par_iter()
        .map(|&x| {
            let mut scn = Scanner::new();
            let mut dcode = Decoder::new();
            scan_single_col(gray, w, h, x, true, &mut scn, &mut dcode);
            scn.new_scan();
            dcode.new_scan();
            scan_single_col(gray, w, h, x, false, &mut scn, &mut dcode);
            dcode.results
        })
        .collect();

    let mut all_results: Vec<DecodedSymbol> = Vec::new();
    for r in neon_row_results { all_results.extend(r); }
    for r in scalar_row_results { all_results.extend(r); }
    for r in neon_col_results { all_results.extend(r); }
    for r in scalar_col_results { all_results.extend(r); }

    dedup_results(&all_results)
}

fn dedup_results(results: &[DecodedSymbol]) -> Vec<Decoded> {
    let mut seen: HashMap<(String, i32), i32> = HashMap::new();
    for r in results {
        let key = (r.data.clone(), r.sym_type as i32);
        *seen.entry(key).or_insert(0) += 1;
    }

    seen.into_iter()
        .filter(|((_data, _), quality)| *quality > 0)
        .map(|((data, sym_i32), quality)| {
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
            Decoded { data, sym_type, quality }
        })
        .collect()
}
