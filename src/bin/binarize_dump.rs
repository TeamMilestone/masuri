//! Dump the Rust `qr_binarize` result for an image so it can be byte-compared
//! against a C reference build. Used once to verify the Phase 3 port; safe to
//! keep around as a tool but not exercised by tests.

use masuri::qrcode::binarize::qr_binarize;

fn main() {
    let mut args = std::env::args().skip(1);
    let in_path = args.next().expect("usage: binarize_dump <input.{jpg,png,gray}> <out.gray> <out.mask>");
    let gray_path = args.next().expect("need <out.gray>");
    let mask_path = args.next().expect("need <out.mask>");

    let img = image::open(&in_path).unwrap_or_else(|_| panic!("failed to open {}", in_path));
    let gray = img.to_luma8();
    let w = gray.width();
    let h = gray.height();
    eprintln!("{}: {}x{}", in_path, w, h);

    let raw = gray.as_raw();
    std::fs::write(&gray_path, raw).unwrap();
    eprintln!("wrote raw grayscale → {}", gray_path);

    let mask = qr_binarize(raw, w as i32, h as i32);
    std::fs::write(&mask_path, &mask).unwrap();
    eprintln!("wrote Rust binarize mask ({} bytes) → {}", mask.len(), mask_path);
}
