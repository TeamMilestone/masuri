use masuri::scanner::{Scanner, EdgeType};
use masuri::decoder::Decoder;

fn main() {
    let path = std::env::args().nth(1)
        .unwrap_or("../samples/008_barcode_301059785856.jpg".into());
    let img = image::open(&path).unwrap();
    let gray = img.to_luma8();
    let w = gray.width() as usize;
    let h = gray.height() as usize;
    let pixels = gray.as_raw();
    eprintln!("Image: {}x{}", w, h);

    let mut max_chars = 0i16;
    let mut max_chars_row = 0;
    let mut total_results = 0;
    let mut abort_decode = 0u32;
    let mut abort_width = 0u32;

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
                    let old_char = dcode.code128.character;
                    let old_s6 = dcode.code128.s6;
                    let old_width = dcode.code128.width;
                    dcode.decode_width(r.width);
                    let new_char = dcode.code128.character;

                    if new_char > max_chars {
                        max_chars = new_char;
                        max_chars_row = y;
                    }

                    // Track abort reasons
                    if old_char > 0 && new_char < 0 {
                        // Was in progress, now aborted
                        if old_width > 0 {
                            let dw = if old_width > old_s6 { old_width - old_s6 } else { old_s6 - old_width };
                            if dw * 4 > old_width {
                                abort_width += 1;
                            } else {
                                abort_decode += 1;
                            }
                        } else {
                            abort_decode += 1;
                        }
                    }
                }
            }
            let r = scn.flush();
            if r.edge != EdgeType::None { dcode.decode_width(r.width); }
            let r = scn.flush();
            if r.edge != EdgeType::None { dcode.decode_width(r.width); }
            total_results += dcode.results.len();
            for res in &dcode.results {
                eprintln!("  row {} {}: {} [{}]", y, if forward {"fwd"} else {"rev"}, res.data, res.sym_type);
            }
        }
    }
    eprintln!("\nMax chars reached: {} (row {})", max_chars, max_chars_row);
    eprintln!("Aborts: decode_fail={} width_var={}", abort_decode, abort_width);
    eprintln!("Total results: {}", total_results);
}
