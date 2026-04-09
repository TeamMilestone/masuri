//! Python bindings via PyO3

use pyo3::prelude::*;
use pyo3::types::PyList;

/// Decoded barcode result for Python
#[pyclass]
#[derive(Clone)]
struct BarcodeResult {
    #[pyo3(get)]
    data: String,
    #[pyo3(get)]
    symbol_type: String,
    #[pyo3(get)]
    quality: i32,
    #[pyo3(get)]
    x: u32,
    #[pyo3(get)]
    y: u32,
}

#[pymethods]
impl BarcodeResult {
    fn __repr__(&self) -> String {
        format!("BarcodeResult(data='{}', type='{}', quality={}, x={}, y={})",
            self.data, self.symbol_type, self.quality, self.x, self.y)
    }
}

/// Decode barcodes from a grayscale image buffer.
///
/// Args:
///     gray: bytes or list of pixel values (grayscale, 8-bit)
///     width: image width in pixels
///     height: image height in pixels
///
/// Returns:
///     list of BarcodeResult
#[pyfunction]
fn decode(gray: &[u8], width: u32, height: u32) -> Vec<BarcodeResult> {
    crate::decode(gray, width, height)
        .into_iter()
        .map(|r| BarcodeResult {
            data: r.data,
            symbol_type: r.sym_type.to_string(),
            quality: r.quality,
            x: r.x,
            y: r.y,
        })
        .collect()
}

/// Decode barcodes from an image file path.
///
/// Args:
///     path: path to image file (JPEG, PNG, etc.)
///
/// Returns:
///     list of BarcodeResult
#[pyfunction]
fn decode_file(path: &str) -> PyResult<Vec<BarcodeResult>> {
    let img = image::open(path)
        .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(format!("{}", e)))?;
    let gray = img.to_luma8();
    let results = crate::decode(gray.as_raw(), gray.width(), gray.height());
    Ok(results
        .into_iter()
        .map(|r| BarcodeResult {
            data: r.data,
            symbol_type: r.sym_type.to_string(),
            quality: r.quality,
            x: r.x,
            y: r.y,
        })
        .collect())
}

/// Masuri - Fast barcode decoder
///
/// Rust port of zbar with NEON SIMD and rayon parallelism.
/// 6x faster than C zbar, supports Code128/EAN-13/EAN-8/UPC-A/UPC-E.
#[pymodule]
pub fn masuri(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(decode, m)?)?;
    m.add_function(wrap_pyfunction!(decode_file, m)?)?;
    m.add_class::<BarcodeResult>()?;
    Ok(())
}
