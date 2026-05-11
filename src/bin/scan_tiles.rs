// Brute-force tile scanner: slides a window across an image at multiple scales
// and reports every successful decode, plus the (x, y, w, h, scale) that found it.
// Used to empirically locate barcodes that the full-frame decoder misses.
use image::imageops::FilterType;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: scan_tiles <image>");
        std::process::exit(2);
    }
    let path = &args[1];
    let img = image::open(path).expect("open").to_luma8();
    let (w, h) = (img.width(), img.height());
    eprintln!("image {}x{}", w, h);

    // Scales relative to original. For phone-camera 3024x4032 originals, the
    // long invoice barcode occupies maybe 800-1500 px wide, so test broad
    // scale set.
    let scales: &[f32] = &[0.25, 0.33, 0.5, 0.66, 0.75, 1.0];
    let tile_fracs: &[(f32, f32, f32)] = &[
        // (tile_w_frac, tile_h_frac, step_frac) — step is fraction of tile dim
        (0.40, 0.20, 0.5),
        (0.30, 0.15, 0.5),
        (0.55, 0.30, 0.5),
        (0.20, 0.40, 0.5),  // tall narrow — for vertical barcodes
        (0.30, 0.55, 0.5),
    ];

    let mut hits: Vec<String> = Vec::new();
    for &s in scales {
        let nw = ((w as f32) * s) as u32;
        let nh = ((h as f32) * s) as u32;
        if nw < 200 || nh < 200 { continue; }
        let resized = image::imageops::resize(&img, nw, nh, FilterType::Triangle);
        for &(tw_f, th_f, step_f) in tile_fracs {
            let tw = ((nw as f32) * tw_f) as u32;
            let th = ((nh as f32) * th_f) as u32;
            if tw < 100 || th < 50 { continue; }
            let step_x = ((tw as f32) * step_f).max(1.0) as u32;
            let step_y = ((th as f32) * step_f).max(1.0) as u32;
            let mut y = 0u32;
            while y + th <= nh {
                let mut x = 0u32;
                while x + tw <= nw {
                    let tile = image::imageops::crop_imm(&resized, x, y, tw, th).to_image();
                    let res = masuri::decode_parallel(tile.as_raw(), tw, th);
                    for r in &res {
                        if matches!(r.sym_type, masuri::SymbolType::Code128) {
                            let line = format!(
                                "scale={:.2} tile={}x{} at=({},{}) -> {} q={} (orig roi=({},{} +{}x{}))",
                                s, tw, th, x, y, r.data, r.quality,
                                ((x as f32)/s) as u32, ((y as f32)/s) as u32,
                                ((tw as f32)/s) as u32, ((th as f32)/s) as u32,
                            );
                            eprintln!("{}", line);
                            hits.push(format!("{}\t{}", r.data, r.quality));
                        }
                    }
                    x += step_x;
                }
                y += step_y;
            }
        }
    }
    eprintln!("---\nhits: {}", hits.len());
    for h in &hits { eprintln!("  {}", h); }
}
