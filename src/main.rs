use std::env;
use std::path::Path;
use std::time::Instant;

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

    // Load all images into memory first
    let mut images: Vec<(String, Vec<u8>, u32, u32)> = Vec::new();
    for ip in &image_paths {
        match image::open(ip) {
            Ok(img) => {
                let gray = img.to_luma8();
                let w = gray.width();
                let h = gray.height();
                let pixels = gray.into_raw();
                images.push((ip.clone(), pixels, w, h));
            }
            Err(e) => {
                eprintln!("Error loading {}: {}", ip, e);
            }
        }
    }

    eprintln!("Loaded {} images", images.len());

    // Benchmark
    let mut total_decoded = 0;
    let start = Instant::now();

    for _run in 0..bench_runs {
        for (path, pixels, w, h) in &images {
            let results = if neon {
                #[cfg(target_arch = "aarch64")]
                { masuri::decode_neon_parallel(pixels, *w, *h) }
                #[cfg(not(target_arch = "aarch64"))]
                { masuri::decode_parallel(pixels, *w, *h) }
            } else if parallel {
                masuri::decode_parallel(pixels, *w, *h)
            } else {
                masuri::decode(pixels, *w, *h)
            };

            if bench_runs == 1 {
                if results.is_empty() {
                    println!("{}: (no barcode found)", Path::new(path).file_name().unwrap().to_string_lossy());
                } else {
                    for r in &results {
                        println!("{}: {} [{}] q={}",
                            Path::new(path).file_name().unwrap().to_string_lossy(),
                            r.data, r.sym_type, r.quality);
                    }
                }
            }
            total_decoded += results.len();
        }
    }

    let elapsed = start.elapsed();
    let total_ms = elapsed.as_secs_f64() * 1000.0;

    eprintln!("\n--- Benchmark Results ---");
    eprintln!("Mode: {}", if neon { "NEON+rayon" } else if parallel { "parallel (rayon)" } else { "single-threaded" });
    eprintln!("Images: {}", images.len());
    eprintln!("Runs: {}", bench_runs);
    eprintln!("Total barcodes decoded: {}", total_decoded);
    eprintln!("Total time: {:.2} ms", total_ms);
    eprintln!("Avg per image: {:.3} ms", total_ms / (images.len() as f64 * bench_runs as f64));
    eprintln!("Throughput: {:.1} images/sec", images.len() as f64 * bench_runs as f64 / elapsed.as_secs_f64());
}
