use masuri::img_scanner;

fn main() {
    let path = std::env::args().nth(1)
        .unwrap_or("../samples/068_barcode_540773132706.jpg".into());
    let img = image::open(&path).unwrap();
    let gray = img.to_luma8();
    let w = gray.width();
    let h = gray.height();
    eprintln!("Image: {}x{}", w, h);

    let (results, qr_lines) = img_scanner::scan_image_qr_diag(gray.as_raw(), w, h);
    eprintln!("Results: {} (QR finder lines: {})", results.len(), qr_lines);
    for r in &results {
        eprintln!("  {} [{}] q={}", r.data, r.sym_type, r.quality);
    }

    // Try with image crate resize (bilinear)
    for pct in [75u32, 110, 50, 150, 200] {
        let nw = (w * pct / 100).max(1);
        let nh = (h * pct / 100).max(1);
        let resized = image::imageops::resize(&gray, nw, nh, image::imageops::FilterType::Triangle);
        let results = masuri::decode_parallel(resized.as_raw(), nw, nh);
        eprintln!("{}%: {}x{} -> {} results", pct, nw, nh, results.len());
        for r in &results {
            eprintln!("  {} [{}] q={}", r.data, r.sym_type, r.quality);
        }
    }
}
