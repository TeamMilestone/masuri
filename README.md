# Masuri

Fast barcode decoder for Code128, EAN-13, EAN-8, UPC-A, UPC-E.

Rust port of [ZBar](http://zbar.sourceforge.net/) with NEON SIMD and rayon parallelism, optimized for Apple Silicon.

## Performance

Benchmarked on Mac Mini M1 with 71 real-world barcode images:

| Mode | Per image | vs C zbar |
|------|-----------|-----------|
| C zbarimg (0.23) | 33.7 ms | 1x |
| Rust single-threaded | 10.3 ms | 3.3x faster |
| Rust rayon parallel | 5.1 ms | 6.6x faster |
| Rust NEON + rayon | 4.7 ms | **7.2x faster** |

Accuracy: **96%** (68/71) vs C zbar's 93% (66/71).

## Usage

### As a CLI

```bash
# Single image
masuri image.jpg

# Directory (parallel)
masuri images/ --parallel

# NEON + rayon (aarch64, fastest)
masuri images/ --neon

# Benchmark (10 runs)
masuri images/ --neon --bench 10
```

### As a library

```toml
[dependencies]
masuri = "0.1"
```

```rust
use masuri::{decode, decode_parallel};

let img = image::open("barcode.jpg").unwrap().to_luma8();
let results = decode(img.as_raw(), img.width(), img.height());

for r in &results {
    println!("{} [{}]", r.data, r.sym_type);
}
```

## Supported barcode types

- Code 128
- EAN-13 / EAN-8
- UPC-A / UPC-E
- ISBN-10 / ISBN-13

## License

LGPL-2.1-or-later

Rust port of ZBar Bar Code Reader.
Original C code Copyright (C) 2007-2011 Jeff Brown.
