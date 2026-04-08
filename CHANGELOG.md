# Changelog

## [0.1.1] - 2026-04-09

### Added
- `--scale N` CLI option for image downscaling before scanning (default: 82%)
- Fast integer bilinear interpolation (8.8 fixed-point) for downscaling
- Parallel image loading with rayon `par_iter()`
- Parallel image-level decoding (multiple images processed concurrently)

### Changed
- Merged row and column scan phases into a single `par_iter` to eliminate synchronization barriers in both `scan_at_scale` (2 barriers -> 1) and `scan_image_neon_parallel` (4 barriers -> 1)
- Default scan mode now downscales to 82% before scanning

### Performance
- Wall time: 16.5s -> ~9.3s (-44%) on 68x 5712x4284 JPEG photos
- CPU utilization: 455% -> 656% on Mac Mini M1
- Accuracy: 65 -> 66 detections (#67 barcode now detected at 82% scale)

### Fixed
- Barcode #67 (`8026040246621`) now detected via 82% downscale, which was previously missed at native resolution by the NEON scanner

## [0.1.0] - 2026-04-08

### Added
- Rust port of ZBar barcode decoder (scanner, decoder, img_scanner)
- Code 128 decoder ported from zbar 0.23 (key accuracy improvements over 0.10)
- EAN-13, EAN-8, UPC-A, UPC-E, ISBN-10, ISBN-13 decoders from zbar 0.10
- Edge detection via EWMA low-pass filter + 2nd derivative zero-crossing
- rayon-based parallel row/column scanning
- NEON SIMD 4-lane scanner for aarch64 (`int32x4_t` packed scanlines)
- Multi-scale scanning fallback (75%, 110%, 50%, 150%)
- CLI with `--parallel`, `--neon`, `--bench N` options
- Python bindings via PyO3 (`--features python`)

### Performance (71 cropped barcode images, Mac Mini M1)
- 7.2x faster than C zbar (4.7 ms vs 33.7 ms per image)
- 96% accuracy (68/71) vs C zbar's 93% (66/71)

### Fixed
- Code 128 checksum digit truncation caused by loop bound captured before `postprocess_c` modifies `character` count (3 barcodes fixed)
- Single-threaded EAN lock interference resolved by defaulting to parallel scanning
