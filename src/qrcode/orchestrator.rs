//! Top-level QR decode orchestration. Ports qrdec.c
//! `qr_reader_try_configuration`, `qr_reader_match_centers`, and the
//! `_zbar_qr_decode` entry point (qrdec.c lines 3589–3956 — Phase 4-D
//! chunk 4, the last of Phase 4).
//!
//! The C side embeds GF tables and the ISAAC PRNG in a `qr_reader` struct.
//! masuri keeps them as separate parameters; a small `QrReader` wrapper
//! can land in Phase 6 if the binding code wants one.

use super::binarize::qr_binarize;
use super::decode::{qr_code_decode, QrCodeData};
use super::finder_centers::QrFinderCenter;
use super::geom::{
    qr_aff_init, qr_aff_unproject, qr_hom_unproject, qr_point_ccw,
    qr_point_distance2, QrAff, QrHom, QR_FINDER_SUBPREC,
};
use super::hom_fit::qr_hom_fit;
use super::isaac::IsaacCtx;
use super::qr_finder::{
    qr_finder_edge_pts_aff_classify, qr_finder_edge_pts_hom_classify,
    qr_finder_estimate_module_size_and_version, QrFinder, QR_LARGE_VERSION_SLACK,
    QR_SMALL_VERSION_SLACK,
};
use super::rs::RsGf256;
use super::util::qr_ilog;
use super::version_info::{qr_finder_fmt_info_decode, qr_finder_version_decode};

const QR_INT_BITS: i32 = 32;

/// Try one of the three possible orderings of finder centers and return the
/// detected version on success, `-1` on failure. Mirrors
/// `qr_reader_try_configuration` (qrdec.c:3594).
pub fn qr_reader_try_configuration(
    gf: &RsGf256,
    isaac: &mut IsaacCtx,
    qrdata: &mut QrCodeData,
    img: &[u8], width: i32, height: i32,
    c: [QrFinderCenter; 3],
) -> i32 {
    let ccw = qr_point_ccw(&c[0].pos, &c[1].pos, &c[2].pos);
    if ccw == 0 { return -1; }

    // Cyclical-order indices, expanded to avoid mods.
    let one = 1 + (ccw < 0) as usize;
    let two = 2 - (ccw < 0) as usize;
    let ci = [0usize, one, two, 0, one, two, 0];

    // Assume the farthest pair is opposite corners; pick i0 = index of UL.
    let mut maxd = qr_point_distance2(&c[1].pos, &c[2].pos);
    let mut i0 = 0;
    for i in 1..3 {
        let d = qr_point_distance2(&c[ci[i + 1]].pos, &c[ci[i + 2]].pos);
        if d > maxd { i0 = i; maxd = d; }
    }

    // Try i0, i0+1, i0+2 orderings.
    for i in i0..i0 + 3 {
        let ul_center = c[ci[i]].clone();
        let ur_center = c[ci[i + 1]].clone();
        let dl_center = c[ci[i + 2]].clone();
        let mut ul = QrFinder::from_center(ul_center);
        let mut ur = QrFinder::from_center(ur_center);
        let mut dl = QrFinder::from_center(dl_center);

        let res = QR_INT_BITS - 2 - QR_FINDER_SUBPREC
            - qr_ilog(width.max(height) as u32 - 1);
        let mut aff = QrAff::zero();
        qr_aff_init(&mut aff, &ul.center_pos, &ur.center_pos, &dl.center_pos, res);

        // UR: classify, estimate size + version.
        qr_aff_unproject(&mut ur.o, &aff, ur.center_pos[0], ur.center_pos[1]);
        qr_finder_edge_pts_aff_classify(&mut ur, &aff);
        if qr_finder_estimate_module_size_and_version(&mut ur, 1 << res, 1 << res) < 0 {
            continue;
        }
        // DL.
        qr_aff_unproject(&mut dl.o, &aff, dl.center_pos[0], dl.center_pos[1]);
        qr_finder_edge_pts_aff_classify(&mut dl, &aff);
        if qr_finder_estimate_module_size_and_version(&mut dl, 1 << res, 1 << res) < 0 {
            continue;
        }
        if (ur.eversion[1] - dl.eversion[0]).abs() > QR_LARGE_VERSION_SLACK {
            continue;
        }
        // UL.
        qr_aff_unproject(&mut ul.o, &aff, ul.center_pos[0], ul.center_pos[1]);
        qr_finder_edge_pts_aff_classify(&mut ul, &aff);
        if qr_finder_estimate_module_size_and_version(&mut ul, 1 << res, 1 << res) < 0
            || (ul.eversion[1] - ur.eversion[1]).abs() > QR_LARGE_VERSION_SLACK
            || (ul.eversion[0] - dl.eversion[0]).abs() > QR_LARGE_VERSION_SLACK
        {
            continue;
        }

        // Promote affine → full homography.
        let mut hom = QrHom::zero();
        if qr_hom_fit(
            &mut hom, &mut ul, &mut ur, &mut dl, &mut qrdata.bbox,
            &aff, isaac, img, width, height,
        ) < 0 {
            continue;
        }
        qr_hom_unproject(&mut ul.o, &hom, ul.center_pos[0], ul.center_pos[1]);
        qr_hom_unproject(&mut ur.o, &hom, ur.center_pos[0], ur.center_pos[1]);
        qr_hom_unproject(&mut dl.o, &hom, dl.center_pos[0], dl.center_pos[1]);
        qr_finder_edge_pts_hom_classify(&mut ur, &hom);
        let dim_w = ur.o[0] - ul.o[0];
        if qr_finder_estimate_module_size_and_version(&mut ur, dim_w, dim_w) < 0 {
            continue;
        }
        qr_finder_edge_pts_hom_classify(&mut dl, &hom);
        let dim_h = dl.o[1] - ul.o[1];
        if qr_finder_estimate_module_size_and_version(&mut dl, dim_h, dim_h) < 0 {
            continue;
        }

        // Decode version (either implicit-from-eversion for v<7, or read bits).
        let ur_version: i32;
        if ur.eversion[1] == dl.eversion[0] && ur.eversion[1] < 7 {
            ur_version = ur.eversion[1];
        } else {
            if (ur.eversion[1] - dl.eversion[0]).abs() > QR_LARGE_VERSION_SLACK { continue; }
            let mut ur_v = -1i32;
            let mut dl_v = -1i32;
            if ur.eversion[1] >= 7 - QR_LARGE_VERSION_SLACK {
                ur_v = qr_finder_version_decode(&ur, &hom, img, width, height, 0);
                if (ur_v - ur.eversion[1]).abs() > QR_LARGE_VERSION_SLACK { ur_v = -1; }
            }
            if dl.eversion[0] >= 7 - QR_LARGE_VERSION_SLACK {
                dl_v = qr_finder_version_decode(&dl, &hom, img, width, height, 1);
                if (dl_v - dl.eversion[0]).abs() > QR_LARGE_VERSION_SLACK { dl_v = -1; }
            }
            if ur_v >= 0 {
                if dl_v >= 0 && dl_v != ur_v { continue; }
                ur_version = ur_v;
            } else if dl_v < 0 {
                continue;
            } else {
                ur_version = dl_v;
            }
        }

        // Re-classify UL under the homography and tighten size checks.
        qr_finder_edge_pts_hom_classify(&mut ul, &hom);
        let w = ur.o[0] - dl.o[0];
        let h = dl.o[1] - ul.o[1];
        if qr_finder_estimate_module_size_and_version(&mut ul, w, h) < 0
            || (ul.eversion[1] - ur.eversion[1]).abs() > QR_SMALL_VERSION_SLACK
            || (ul.eversion[0] - dl.eversion[0]).abs() > QR_SMALL_VERSION_SLACK
        {
            continue;
        }
        let fmt_info = qr_finder_fmt_info_decode(&ul, &ur, &dl, &hom, img, width, height);
        if fmt_info < 0 { continue; }
        if qr_code_decode(
            qrdata, gf,
            &ul.center_pos, &ur.center_pos, &dl.center_pos,
            ur_version, fmt_info, img, width, height,
        ) < 0 {
            continue;
        }
        return ur_version;
    }
    -1
}

/// O(n³) search over candidate finder-center triples. Mirrors
/// `qr_reader_match_centers` (qrdec.c:3806), including the
/// "code-inside-a-code" recursive sub-search.
pub fn qr_reader_match_centers(
    gf: &RsGf256,
    isaac: &mut IsaacCtx,
    qrlist: &mut Vec<QrCodeData>,
    centers: &[QrFinderCenter],
    img: &[u8], width: i32, height: i32,
) {
    let ncenters = centers.len();
    let mut mark = vec![0u8; ncenters];
    for i in 0..ncenters {
        if mark[i] != 0 { continue; }
        for j in (i + 1)..ncenters {
            if mark[i] != 0 { break; }
            if mark[j] != 0 { continue; }
            for k in (j + 1)..ncenters {
                if mark[j] != 0 { break; }
                if mark[k] != 0 { continue; }
                let triple = [centers[i].clone(), centers[j].clone(), centers[k].clone()];
                let mut qrdata = QrCodeData::new();
                let version = qr_reader_try_configuration(
                    gf, isaac, &mut qrdata, img, width, height, triple,
                );
                if version < 0 { continue; }

                // Convert bbox to image-space (drop SUBPREC).
                for p in qrdata.bbox.iter_mut() {
                    p[0] >>= QR_FINDER_SUBPREC;
                    p[1] >>= QR_FINDER_SUBPREC;
                }
                let bbox = qrdata.bbox;
                qrlist.push(qrdata);

                mark[i] = 1; mark[j] = 1; mark[k] = 1;

                // Find any centers inside this code's bbox.
                let mut inside: Vec<QrFinderCenter> = Vec::new();
                for l in 0..ncenters {
                    if mark[l] != 0 { continue; }
                    let p = &centers[l].pos;
                    if qr_point_ccw(&bbox[0], &bbox[1], p) >= 0
                        && qr_point_ccw(&bbox[1], &bbox[3], p) >= 0
                        && qr_point_ccw(&bbox[3], &bbox[2], p) >= 0
                        && qr_point_ccw(&bbox[2], &bbox[0], p) >= 0
                    {
                        mark[l] = 2;
                        inside.push(centers[l].clone());
                    }
                }
                if inside.len() >= 3 {
                    qr_reader_match_centers(gf, isaac, qrlist, &inside, img, width, height);
                }
                // Mark all inside-the-bbox centers as used (codes don't
                // partially overlap).
                for l in 0..ncenters {
                    if mark[l] == 2 { mark[l] = 1; }
                }
            }
        }
    }
}

/// `_zbar_qr_decode` equivalent — top-level entry point. Takes the
/// candidate finder centers + the binarized image and returns the decoded
/// payloads (one entry per successfully decoded QR code).
///
/// The caller is responsible for:
/// - Running the 1D scanner so QrFinderLines accumulate (Phase 1).
/// - Calling `qr_finder_centers_locate` to convert lines into centers
///   (Phase 4-F chunk 1).
/// - Binarizing the image with `qr_binarize` (Phase 3).
///
/// In Phase 6 we wire these up inside the existing img_scanner pipeline.
pub fn decode_image(
    gf: &RsGf256,
    isaac: &mut IsaacCtx,
    centers: &[QrFinderCenter],
    img: &[u8], width: i32, height: i32,
) -> Vec<QrCodeData> {
    let mut qrlist = Vec::new();
    if centers.len() >= 3 {
        qr_reader_match_centers(gf, isaac, &mut qrlist, centers, img, width, height);
    }
    qrlist
}

/// Convenience: from a grayscale image, binarize it and decode.
/// Phase 6 will replace this with a path that re-uses the binarized
/// buffer from img_scanner.
pub fn decode_grayscale(
    gf: &RsGf256,
    isaac: &mut IsaacCtx,
    centers: &[QrFinderCenter],
    gray: &[u8], width: i32, height: i32,
) -> Vec<QrCodeData> {
    let bin = qr_binarize(gray, width, height);
    decode_image(gf, isaac, centers, &bin, width, height)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::qrcode::isaac::isaac_init_empty;

    #[test]
    fn empty_centers_returns_nothing() {
        let gf = RsGf256::new();
        let mut isaac = IsaacCtx::new();
        isaac_init_empty(&mut isaac);
        let out = decode_image(&gf, &mut isaac, &[], &[0u8; 64], 8, 8);
        assert!(out.is_empty());
    }

    #[test]
    fn fewer_than_three_centers_returns_nothing() {
        let gf = RsGf256::new();
        let mut isaac = IsaacCtx::new();
        isaac_init_empty(&mut isaac);
        let centers = vec![
            QrFinderCenter { pos: [10, 10], edge_pts: Vec::new() },
            QrFinderCenter { pos: [50, 10], edge_pts: Vec::new() },
        ];
        let out = decode_image(&gf, &mut isaac, &centers, &[0u8; 4096], 64, 64);
        assert!(out.is_empty());
    }

    #[test]
    fn three_degenerate_centers_dont_panic() {
        // Three centers but with empty edge_pts → classifiers find no
        // points, estimate_module_size returns -1, try_configuration -1,
        // match_centers leaves the list empty.
        let gf = RsGf256::new();
        let mut isaac = IsaacCtx::new();
        isaac_init_empty(&mut isaac);
        let centers = vec![
            QrFinderCenter { pos: [10, 10], edge_pts: Vec::new() },
            QrFinderCenter { pos: [50, 10], edge_pts: Vec::new() },
            QrFinderCenter { pos: [10, 50], edge_pts: Vec::new() },
        ];
        let out = decode_image(&gf, &mut isaac, &centers, &[0u8; 4096], 64, 64);
        assert!(out.is_empty());
    }
}
