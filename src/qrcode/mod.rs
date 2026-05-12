//! QR Code decoder modules.
//! Rust port of zbar/qrcode/ + zbar/decoder/qr_finder.{c,h}.
//! Original Copyright (C) 2008-2009 Timothy B. Terriberry (tterribe@xiph.org)
//! LGPL-2.1-or-later

pub mod bch15_5;
pub mod binarize;
pub mod finder;
pub mod finder_centers;
pub mod geom;
pub mod isaac;
pub mod qr_finder;
pub mod rs;
pub mod util;
