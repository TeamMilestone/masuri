# Masuri

Fast barcode decoder for Code 128, EAN-13, EAN-8, UPC-A, UPC-E.

Rust port of [ZBar](http://zbar.sourceforge.net/) with rayon parallelism and ARM NEON SIMD, optimized for Apple Silicon.

## Performance

### Cropped barcode images (71 images, Mac Mini M1)

| Mode | Per image | vs C zbar |
|------|-----------|-----------|
| C zbarimg 0.23 | 33.7 ms | 1x |
| Single-threaded | 10.3 ms | 3.3x faster |
| rayon parallel | 5.1 ms | 6.6x faster |
| NEON + rayon | 4.7 ms | **7.2x faster** |

### Full-size photos (68 images, 5712x4284 JPEG, Mac Mini M1)

| Version | Wall time | ms/img | CPU usage | Accuracy |
|---------|-----------|--------|-----------|----------|
| v1 sequential | 16.5s | 150 | 455% | 65/65 |
| v4 parallel + 82% scale | **~9.3s** | **105** | **656%** | **66/65** |

Accuracy: **96%** (68/71 cropped images) vs C zbar's 93% (66/71).

## Installation

### CLI

```bash
cargo install masuri
```

### Library

```toml
[dependencies]
masuri = "0.1"
```

## Usage

### CLI

```bash
# Single image (default: 82% downscale + NEON on aarch64)
masuri image.jpg --neon

# Directory batch processing
masuri images/ --neon

# Full resolution (no downscale)
masuri images/ --neon --scale 100

# Custom scale
masuri images/ --neon --scale 75

# Benchmark (3 runs)
masuri images/ --neon --bench 3

# rayon only (no NEON)
masuri images/ --parallel

# Single-threaded
masuri images/
```

Output format:
```
filename.jpg: 301059785856 [CODE-128] q=50 x=3503 y=1202
filename.jpg: 0034776000070 [EAN-13] q=1 x=4146 y=3758
```

Each result includes the barcode's approximate position in the image (x, y in pixels). Useful for selecting the correct barcode when multiple are present (e.g. pick the topmost by smallest y).

### Library

```rust
use masuri::{decode, decode_parallel, Decoded, SymbolType};

let img = image::open("barcode.jpg").unwrap().to_luma8();
let results = decode(img.as_raw(), img.width(), img.height());

for r in &results {
    println!("{} [{}] quality={} at ({}, {})", r.data, r.sym_type, r.quality, r.x, r.y);
}

// NEON + rayon (aarch64)
#[cfg(target_arch = "aarch64")]
let results = masuri::decode_neon_parallel(img.as_raw(), img.width(), img.height());
```

### Python (via PyO3)

```bash
cd rszbar
pip install maturin
maturin develop --release --features python
```

```python
import masuri
results = masuri.decode(pixels, width, height)
for r in results:
    print(f"{r.data} [{r.symbol_type}] q={r.quality} x={r.x} y={r.y}")
```

## Supported symbologies

| Type | Status |
|------|--------|
| Code 128 | Fully supported (ported from zbar 0.23) |
| EAN-13 / EAN-8 | Fully supported |
| UPC-A / UPC-E | Fully supported |
| ISBN-10 / ISBN-13 | Fully supported |

## Architecture

```
src/
  lib.rs              Public API: decode(), decode_parallel(), decode_neon_parallel()
  main.rs             CLI: batch processing, benchmarking, --scale downscale
  scanner.rs          Edge detection (EWMA + 2nd derivative zero-crossing)
  scanner_neon.rs     NEON SIMD 4-lane scanner (aarch64)
  img_scanner.rs      Image scanner: row/col traversal, rayon parallelism, multi-scale
  decoder/
    mod.rs            Decoder hub (bar width -> symbology dispatch)
    ean.rs            EAN-13/EAN-8/UPC-A/UPC-E decoder
    code128.rs        Code 128 decoder (ported from zbar 0.23)
```

### Optimization layers

1. **rayon parallelism** - All image rows and columns scanned in a single barrier-free `par_iter`. Multiple images processed concurrently.
2. **NEON SIMD** (aarch64) - 4 scanlines packed into `int32x4_t` for simultaneous EWMA, derivative, and zero-crossing detection.
3. **82% downscale** - Fast integer bilinear interpolation (8.8 fixed-point) reduces pixel count by 33% with no accuracy loss. Actually improves detection on some edge cases.
4. **Multi-scale fallback** - If native resolution fails, tries 75%, 110%, 50%, 150% scales (parallel/non-NEON path).

## Requirements

- Rust 1.70+
- For NEON SIMD: aarch64 target (Apple Silicon, ARM64 Linux)
- No external C libraries required

## License

LGPL-2.1-or-later

Rust port of ZBar Bar Code Reader.
Original C code Copyright (C) 2007-2011 Jeff Brown.
