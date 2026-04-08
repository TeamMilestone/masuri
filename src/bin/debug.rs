use masuri::scanner::{Scanner, EdgeType};
use masuri::decoder::Decoder;

fn main() {
    let path = std::env::args().nth(1)
        .unwrap_or("../samples/026_barcode_302444947853.jpg".into());
    let img = image::open(&path).unwrap();
    let gray = img.to_luma8();
    let w = gray.width() as usize;
    let h = gray.height() as usize;
    let pixels = gray.as_raw();
    eprintln!("Image: {}x{}", w, h);

    let mut max_chars = 0i16;
    let mut total_starts = 0u32;
    let mut abort_decode = 0u32;
    let mut abort_width = 0u32;
    let mut total_results = 0u32;

    // Scan all rows + cols (like img_scanner), EAN off
    // Rows
    for y in 0..h {
        for forward in [true, false] {
            let mut scn = Scanner::new();
            let mut dcode = Decoder::new();
            dcode.ean.enable = false;

            let iter: Box<dyn Iterator<Item = usize>> = if forward {
                Box::new(0..w)
            } else {
                Box::new((0..w).rev())
            };
            for x in iter {
                let d = pixels[x + y * w] as i32;
                let r = scn.scan_y(d);
                if r.edge != EdgeType::None {
                    let old = dcode.code128.character;
                    dcode.decode_width(r.width);
                    let new = dcode.code128.character;
                    if old < 0 && new > 0 { total_starts += 1; }
                    if new > max_chars { max_chars = new; }
                    if old > 0 && new < 0 {
                        if dcode.code128.width > 0 {
                            let s6 = dcode.code128.s6;
                            let w = dcode.code128.width;
                            let dw = if w > s6 { w - s6 } else { s6 - w };
                            if dw * 4 > w { abort_width += 1; } else { abort_decode += 1; }
                        } else { abort_decode += 1; }
                    }
                }
            }
            let r = scn.flush();
            if r.edge != EdgeType::None { dcode.decode_width(r.width); }
            let r = scn.flush();
            if r.edge != EdgeType::None { dcode.decode_width(r.width); }
            total_results += dcode.results.len() as u32;
            for res in &dcode.results {
                eprintln!("  ROW {} {}: {} [{}]", y, if forward {"fwd"} else {"rev"}, res.data, res.sym_type);
            }
        }
    }
    // Cols
    for x in 0..w {
        for forward in [true, false] {
            let mut scn = Scanner::new();
            let mut dcode = Decoder::new();
            dcode.ean.enable = false;

            let iter: Box<dyn Iterator<Item = usize>> = if forward {
                Box::new(0..h)
            } else {
                Box::new((0..h).rev())
            };
            for y in iter {
                let d = pixels[x + y * w] as i32;
                let r = scn.scan_y(d);
                if r.edge != EdgeType::None {
                    let old = dcode.code128.character;
                    dcode.decode_width(r.width);
                    let new = dcode.code128.character;
                    if old < 0 && new > 0 { total_starts += 1; }
                    if new > max_chars { max_chars = new; }
                    if old > 0 && new < 0 {
                        abort_decode += 1;
                    }
                }
            }
            let r = scn.flush();
            if r.edge != EdgeType::None { dcode.decode_width(r.width); }
            let r = scn.flush();
            if r.edge != EdgeType::None { dcode.decode_width(r.width); }
            total_results += dcode.results.len() as u32;
            for res in &dcode.results {
                eprintln!("  COL {} {}: {} [{}]", x, if forward {"fwd"} else {"rev"}, res.data, res.sym_type);
            }
        }
    }
    eprintln!("\nStarts: {}, Max chars: {}, Aborts: decode={} width={}", total_starts, max_chars, abort_decode, abort_width);
    eprintln!("Total results: {}", total_results);
}
