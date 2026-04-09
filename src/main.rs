use std::env;
use std::path::Path;
use std::time::Instant;
use rayon::prelude::*;

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() < 2 {
        eprintln!("Usage: masuri <image_or_directory> [--parallel] [--neon] [--bench N]");
        std::process::exit(1);
    }

    let path = &args[1];
    let parallel = args.contains(&"--parallel".to_string());
    let neon = args.contains(&"--neon".to_string());
    let bench_runs = args.iter()
        .position(|a| a == "--bench")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(1);
    let scale_pct = args.iter()
        .position(|a| a == "--scale")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(82);

    // Collect image paths
    let mut image_paths: Vec<String> = Vec::new();
    let p = Path::new(path);
    if p.is_dir() {
        if let Ok(entries) = std::fs::read_dir(p) {
            for entry in entries.flatten() {
                let ep = entry.path();
                if let Some(ext) = ep.extension() {
                    let ext = ext.to_string_lossy().to_lowercase();
                    if ext == "jpg" || ext == "jpeg" || ext == "png" || ext == "bmp" {
                        image_paths.push(ep.to_string_lossy().to_string());
                    }
                }
            }
        }
        image_paths.sort();
    } else {
        image_paths.push(path.clone());
    }

    if image_paths.is_empty() {
        eprintln!("No images found");
        std::process::exit(1);
    }

    // Load all images into memory in parallel (with optional downscale)
    let images: Vec<(String, Vec<u8>, u32, u32)> = image_paths.par_iter()
        .filter_map(|ip| {
            match image::open(ip) {
                Ok(img) => {
                    let gray = img.to_luma8();
                    let (pixels, w, h) = if scale_pct < 100 {
                        let ow = gray.width();
                        let oh = gray.height();
                        let nw = (ow * scale_pct / 100).max(1);
                        let nh = (oh * scale_pct / 100).max(1);
                        let sw = gray.width() as usize;
                        let sh = gray.height() as usize;
                        let src = gray.into_raw();
                        let (dw, dh) = (nw as usize, nh as usize);
                        let mut dst = vec![0u8; dw * dh];
                        // Fixed-point 8.8 — max intermediate = 255*256*256*4 = 67M, fits u32
                        let x_step = ((sw - 1) << 8) / (dw - 1).max(1);
                        let y_step = ((sh - 1) << 8) / (dh - 1).max(1);
                        for dy in 0..dh {
                            let sy_fp = dy * y_step;
                            let sy = sy_fp >> 8;
                            let sy1 = (sy + 1).min(sh - 1);
                            let fy = (sy_fp & 0xFF) as u32;
                            let ify = 256 - fy;
                            let r0 = sy * sw;
                            let r1 = sy1 * sw;
                            for dx in 0..dw {
                                let sx_fp = dx * x_step;
                                let sx = sx_fp >> 8;
                                let sx1 = (sx + 1).min(sw - 1);
                                let fx = (sx_fp & 0xFF) as u32;
                                let ifx = 256 - fx;
                                let v = (src[r0 + sx] as u32 * ify * ifx
                                       + src[r0 + sx1] as u32 * ify * fx
                                       + src[r1 + sx] as u32 * fy * ifx
                                       + src[r1 + sx1] as u32 * fy * fx) >> 16;
                                dst[dy * dw + dx] = v as u8;
                            }
                        }
                        (dst, nw, nh)
                    } else {
                        let w = gray.width();
                        let h = gray.height();
                        (gray.into_raw(), w, h)
                    };
                    Some((ip.clone(), pixels, w, h))
                }
                Err(e) => {
                    eprintln!("Error loading {}: {}", ip, e);
                    None
                }
            }
        })
        .collect();

    eprintln!("Loaded {} images", images.len());

    // Benchmark
    let start = Instant::now();
    let mut total_decoded = 0;

    for _run in 0..bench_runs {
        // Decode all images in parallel
        let all_results: Vec<(&str, Vec<masuri::Decoded>)> = images.par_iter()
            .map(|(path, pixels, w, h)| {
                let mut results = if neon {
                    #[cfg(target_arch = "aarch64")]
                    { masuri::decode_neon_parallel(pixels, *w, *h) }
                    #[cfg(not(target_arch = "aarch64"))]
                    { masuri::decode_parallel(pixels, *w, *h) }
                } else if parallel {
                    masuri::decode_parallel(pixels, *w, *h)
                } else {
                    masuri::decode(pixels, *w, *h)
                };
                // Scale coordinates back to original image size
                if scale_pct < 100 {
                    for r in &mut results {
                        r.x = r.x * 100 / scale_pct;
                        r.y = r.y * 100 / scale_pct;
                    }
                }
                (path.as_str(), results)
            })
            .collect();

        if bench_runs == 1 {
            for (path, results) in &all_results {
                let fname = Path::new(path).file_name().unwrap().to_string_lossy();
                if results.is_empty() {
                    println!("{}: (no barcode found)", fname);
                } else {
                    for r in results {
                        println!("{}: {} [{}] q={} x={} y={}", fname, r.data, r.sym_type, r.quality, r.x, r.y);
                    }
                }
            }
        }
        total_decoded += all_results.iter().map(|(_, r)| r.len()).sum::<usize>();
    }

    let elapsed = start.elapsed();
    let total_ms = elapsed.as_secs_f64() * 1000.0;

    eprintln!("\n--- Benchmark Results ---");
    eprintln!("Mode: {}", if neon { "NEON+rayon" } else if parallel { "parallel (rayon)" } else { "single-threaded" });
    if scale_pct < 100 { eprintln!("Scale: {}%", scale_pct); }
    eprintln!("Images: {}", images.len());
    eprintln!("Runs: {}", bench_runs);
    eprintln!("Total barcodes decoded: {}", total_decoded);
    eprintln!("Total time: {:.2} ms", total_ms);
    eprintln!("Avg per image: {:.3} ms", total_ms / (images.len() as f64 * bench_runs as f64));
    eprintln!("Throughput: {:.1} images/sec", images.len() as f64 * bench_runs as f64 / elapsed.as_secs_f64());
}
