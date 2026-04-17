# Changelog

## [0.1.2] - 2026-04-17

### Added
- **Android / Kotlin bindings via UniFFI** — new `android` feature gates `uniffi` (0.28) and exposes `decode_bytes(Vec<u8>, u32, u32) -> Vec<Decoded>` for JNI consumption
- `uniffi-bindgen` helper binary (`required-features = ["android"]`) for generating Kotlin bindings from the compiled `.so`
- `#[derive(uniffi::Record)]` on `Decoded` and `#[derive(uniffi::Enum)]` on `SymbolType` (feature-gated), enabling proc-macro FFI scaffolding without a separate UDL file

### Build
- Cross-compilation to `aarch64-linux-android` verified via `cargo ndk` (NDK r28) — `libmasuri.so` ~900KB release
- Default and `python` feature builds unchanged; `android` feature is opt-in and does not affect desktop/PyPI artifacts

## [0.1.1] - 2026-04-09

### Added
- **Barcode position coordinates (x, y)** in all decoded results — enables position-based barcode selection (e.g. topmost barcode in multi-barcode images)
  - Row scans: exact y, approximate x (pixel position at decode time)
  - Column scans: exact x, approximate y
  - Dedup merges coordinates via median across all detections
  - Coordinates are scaled back to original image size when `--scale` is used
- `--scale N` CLI option for image downscaling before scanning (default: 82%)
- Fast integer bilinear interpolation (8.8 fixed-point) for downscaling
- Parallel image loading with rayon `par_iter()`
- Parallel image-level decoding (multiple images processed concurrently)

### Changed
- CLI output format now includes coordinates: `filename: data [TYPE] q=N x=X y=Y`
- `Decoded` struct now has `x: u32` and `y: u32` fields
- Python `BarcodeResult` now exposes `x` and `y` attributes
- Merged row and column scan phases into a single `par_iter` to eliminate synchronization barriers in both `scan_at_scale` (2 barriers -> 1) and `scan_image_neon_parallel` (4 barriers -> 1)
- Default scan mode now downscales to 82% before scanning

### Performance
- Wall time: 16.5s -> ~9.3s (-44%) on 68x 5712x4284 JPEG photos
- CPU utilization: 455% -> 656% on Mac Mini M1
- Accuracy: 65 -> 66 detections (#67 barcode now detected at 82% scale)

### Fixed
- Barcode #67 (`8026040246621`) now detected via 82% downscale, which was previously missed at native resolution by the NEON scanner
- Multi-barcode selection (#44, #45, #46) can now be resolved using y-coordinate (select topmost barcode)

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
