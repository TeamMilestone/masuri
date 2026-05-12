//! `qr_hom_fit` — promote three finder centers (+ image-walking edge
//! samples) into a full projective homography. Rust port of qrdec.c
//! lines 1909–2256 (Phase 4-F, chunk 5).
//!
//! Pipeline:
//!   1. RANSAC + line fit for the two finder *pairs* (edges 0 / 2).
//!   2. RANSAC + single-finder line fit (edges 1 / 3) with axis-aligned
//!      fallback; derive a stepping direction along each.
//!   3. Walk along edges 1 / 3 collecting `!v:v:!v` crossings in the image
//!      and re-fit the line periodically.
//!   4. Intersect the four lines to get the QR bounding-box corners.
//!   5. If the average estimated version ≥ 2, search for an alignment
//!      pattern to refine the bottom-right corner.
//!   6. Initialize the homography from the four corners.

use super::alignment::qr_alignment_pattern_search;
use super::geom::{
    qr_aff_project, qr_aff_unproject, qr_hom_cell_init, qr_hom_init, qr_line_eval,
    qr_line_fit_points, qr_line_isect, QrAff, QrHom, QrHomCell, QrLine, QrPoint,
    QR_FINDER_SUBPREC, QR_HOM_BITS,
};
use super::isaac::IsaacCtx;
use super::qr_finder::{
    qr_aff_line_step, qr_finder_locate_crossing, qr_finder_quick_crossing_check,
    qr_finder_ransac, qr_line_fit_finder_edge, qr_line_fit_finder_pair, QrFinder,
};
use super::util::{qr_ilog, qr_maxi, qr_signmask};

/// `qr_hom_fit` (qrdec.c:1909). On success, fills `hom` and the four
/// `p[0..4]` corners (UL, UR, DL, DR in that order — zbar's convention)
/// and returns 0. Returns -1 on any geometric sanity-check failure.
#[allow(clippy::too_many_arguments)]
pub fn qr_hom_fit(
    hom: &mut QrHom,
    ul: &mut QrFinder, ur: &mut QrFinder, dl: &mut QrFinder,
    p: &mut [QrPoint; 4],
    aff: &QrAff,
    isaac: &mut IsaacCtx,
    img: &[u8], width: i32, height: i32,
) -> i32 {
    let mut l: [QrLine; 4] = [[0; 3]; 4];

    // ---- 1. UL↔DL (edge 0) and UL↔UR (edge 2) pair lines ----
    qr_finder_ransac(ul, aff, isaac, 0);
    qr_finder_ransac(dl, aff, isaac, 0);
    qr_line_fit_finder_pair(&mut l[0], aff, ul, dl, 0);
    if qr_line_eval(&l[0], dl.center_pos[0], dl.center_pos[1]) < 0
        || qr_line_eval(&l[0], ur.center_pos[0], ur.center_pos[1]) < 0
    {
        return -1;
    }
    qr_finder_ransac(ul, aff, isaac, 2);
    qr_finder_ransac(ur, aff, isaac, 2);
    qr_line_fit_finder_pair(&mut l[2], aff, ul, ur, 2);
    if qr_line_eval(&l[2], dl.center_pos[0], dl.center_pos[1]) < 0
        || qr_line_eval(&l[2], ur.center_pos[0], ur.center_pos[1]) < 0
    {
        return -1;
    }

    // ---- 2. Single-finder edges (UR edge 1 / DL edge 3) ----
    let drv = ur.size[1] >> 1;
    qr_finder_ransac(ur, aff, isaac, 1);
    let dru;
    if qr_line_fit_finder_edge(&mut l[1], ur, 1, aff.res) >= 0 {
        if qr_line_eval(&l[1], ul.center_pos[0], ul.center_pos[1]) < 0
            || qr_line_eval(&l[1], dl.center_pos[0], dl.center_pos[1]) < 0
        {
            return -1;
        }
        let mut d = 0;
        if qr_aff_line_step(aff, &l[1], 1, drv, &mut d) < 0 { return -1; }
        dru = d;
    } else {
        dru = 0;
    }
    let mut ru = ur.o[0] + 3 * ur.size[0] - 2 * dru;
    let mut rv = ur.o[1] - 2 * drv;

    let dbu = dl.size[0] >> 1;
    qr_finder_ransac(dl, aff, isaac, 3);
    let dbv;
    if qr_line_fit_finder_edge(&mut l[3], dl, 3, aff.res) >= 0 {
        if qr_line_eval(&l[3], ul.center_pos[0], ul.center_pos[1]) < 0
            || qr_line_eval(&l[3], ur.center_pos[0], ur.center_pos[1]) < 0
        {
            return -1;
        }
        let mut d = 0;
        if qr_aff_line_step(aff, &l[3], 0, dbu, &mut d) < 0 { return -1; }
        dbv = d;
    } else {
        dbv = 0;
    }
    let mut bu = dl.o[0] - 2 * dbu;
    let mut bv = dl.o[1] + 3 * dl.size[1] - 2 * dbv;

    // ---- Initial point arrays seeded with RANSAC inliers ----
    let mut r: Vec<QrPoint> = Vec::new();
    let mut rlastfit = ur.ninliers[1] as i32;
    {
        let edge_pts = ur.edge_slice(1);
        for i in 0..ur.ninliers[1] as usize {
            r.push(edge_pts[i].pos);
        }
    }
    let mut b: Vec<QrPoint> = Vec::new();
    let mut blastfit = dl.ninliers[3] as i32;
    {
        let edge_pts = dl.edge_slice(3);
        for i in 0..dl.ninliers[3] as usize {
            b.push(edge_pts[i].pos);
        }
    }

    // ---- Step parameters for the affine projection ----
    let ox = (aff.x0 << aff.res) + (1 << (aff.res - 1));
    let oy = (aff.y0 << aff.res) + (1 << (aff.res - 1));
    let mut rx = aff.fwd[0][0] * ru + aff.fwd[0][1] * rv + ox;
    let mut ry = aff.fwd[1][0] * ru + aff.fwd[1][1] * rv + oy;
    let mut drxi = aff.fwd[0][0] * dru + aff.fwd[0][1] * drv;
    let mut dryi = aff.fwd[1][0] * dru + aff.fwd[1][1] * drv;
    let drxj = aff.fwd[0][0] * ur.size[0];
    let dryj = aff.fwd[1][0] * ur.size[0];
    let mut bx = aff.fwd[0][0] * bu + aff.fwd[0][1] * bv + ox;
    let mut by = aff.fwd[1][0] * bu + aff.fwd[1][1] * bv + oy;
    let mut dbxi = aff.fwd[0][0] * dbu + aff.fwd[0][1] * dbv;
    let mut dbyi = aff.fwd[1][0] * dbu + aff.fwd[1][1] * dbv;
    let dbxj = aff.fwd[0][1] * dl.size[1];
    let dbyj = aff.fwd[1][1] * dl.size[1];

    // ---- 3. Walk and collect crossings ----
    let mut nrempty = 0i32;
    let mut nbempty = 0i32;
    loop {
        let res_shift = (aff.res + QR_FINDER_SUBPREC) as u32;
        // Termination guards from qrdec.c:2063
        let bv_dl_mid = (dl.o[1] + bv) >> 1;
        let rdone = rv >= bv.min(bv_dl_mid) || nrempty > 14;
        let ru_ur_mid = (ur.o[0] + ru) >> 1;
        let bdone = bu >= ru.min(ru_ur_mid) || nbempty > 14;

        if !rdone && (bdone || rv < bu) {
            let x0 = (rx + drxj) >> res_shift;
            let y0 = (ry + dryj) >> res_shift;
            let x1 = (rx - drxj) >> res_shift;
            let y1 = (ry - dryj) >> res_shift;
            let nr = r.len();
            r.push([0; 2]);
            let mut new_pt = r[nr];
            let mut ret = qr_finder_quick_crossing_check(img, width, height, x0, y0, x1, y1, 1);
            if ret == 0 {
                ret = qr_finder_locate_crossing(img, width, height, x0, y0, x1, y1, 1, &mut new_pt);
                r[nr] = new_pt;
            }
            if ret >= 0 {
                if ret == 0 {
                    let mut q: QrPoint = [0; 2];
                    qr_aff_unproject(&mut q, aff, r[nr][0], r[nr][1]);
                    ru = (ru + q[0]) >> 1;
                    if q[1] + drv > rv { rv = (rv + q[1]) >> 1; }
                    rx = aff.fwd[0][0] * ru + aff.fwd[0][1] * rv + ox;
                    ry = aff.fwd[1][0] * ru + aff.fwd[1][1] * rv + oy;
                    let nr_new = (nr + 1) as i32;
                    if nr_new > qr_maxi(1, rlastfit + (rlastfit >> 2)) {
                        qr_line_fit_points(&mut l[1], &r[..nr_new as usize], aff.res);
                        let mut d = 0;
                        if qr_aff_line_step(aff, &l[1], 1, drv, &mut d) >= 0 {
                            drxi = aff.fwd[0][0] * d + aff.fwd[0][1] * drv;
                            dryi = aff.fwd[1][0] * d + aff.fwd[1][1] * drv;
                        }
                        rlastfit = nr_new;
                    }
                } else {
                    // ret > 0: endpoints didn't match — discard this attempt
                    r.pop();
                    nrempty = 0;
                }
            } else {
                r.pop();
                nrempty += 1;
            }
            ru += dru;
            if rv + drv > rv { rv += drv; } else { nrempty = i32::MAX; }
            rx += drxi;
            ry += dryi;
        } else if !bdone {
            let x0 = (bx + dbxj) >> res_shift;
            let y0 = (by + dbyj) >> res_shift;
            let x1 = (bx - dbxj) >> res_shift;
            let y1 = (by - dbyj) >> res_shift;
            let nb = b.len();
            b.push([0; 2]);
            let mut new_pt = b[nb];
            let mut ret = qr_finder_quick_crossing_check(img, width, height, x0, y0, x1, y1, 1);
            if ret == 0 {
                ret = qr_finder_locate_crossing(img, width, height, x0, y0, x1, y1, 1, &mut new_pt);
                b[nb] = new_pt;
            }
            if ret >= 0 {
                if ret == 0 {
                    let mut q: QrPoint = [0; 2];
                    qr_aff_unproject(&mut q, aff, b[nb][0], b[nb][1]);
                    if q[0] + dbu > bu { bu = (bu + q[0]) >> 1; }
                    bv = (bv + q[1]) >> 1;
                    bx = aff.fwd[0][0] * bu + aff.fwd[0][1] * bv + ox;
                    by = aff.fwd[1][0] * bu + aff.fwd[1][1] * bv + oy;
                    let nb_new = (nb + 1) as i32;
                    if nb_new > qr_maxi(1, blastfit + (blastfit >> 2)) {
                        qr_line_fit_points(&mut l[3], &b[..nb_new as usize], aff.res);
                        let mut d = 0;
                        if qr_aff_line_step(aff, &l[3], 0, dbu, &mut d) >= 0 {
                            dbxi = aff.fwd[0][0] * dbu + aff.fwd[0][1] * d;
                            dbyi = aff.fwd[1][0] * dbu + aff.fwd[1][1] * d;
                        }
                        blastfit = nb_new;
                    }
                } else {
                    b.pop();
                }
                nbempty = 0;
            } else {
                b.pop();
                nbempty += 1;
            }
            if bu + dbu > bu { bu += dbu; } else { nbempty = i32::MAX; }
            bv += dbv;
            bx += dbxi;
            by += dbyi;
        } else {
            break;
        }
    }

    // ---- 4. Final line fits (or axis-aligned fallback) ----
    if r.len() > 1 {
        qr_line_fit_points(&mut l[1], &r, aff.res);
    } else {
        let mut p_tmp: QrPoint = [0; 2];
        qr_aff_project(&mut p_tmp, aff, ur.o[0] + 3 * ur.size[0], ur.o[1]);
        let shift = qr_maxi(
            0,
            qr_ilog(qr_maxi(aff.fwd[0][1].abs(), aff.fwd[1][1].abs()) as u32) - ((aff.res + 1) >> 1),
        );
        let round = (1i32 << shift) >> 1;
        l[1][0] = (aff.fwd[1][1] + round) >> shift;
        l[1][1] = (-aff.fwd[0][1] + round) >> shift;
        l[1][2] = -(l[1][0] * p_tmp[0] + l[1][1] * p_tmp[1]);
    }
    if b.len() > 1 {
        qr_line_fit_points(&mut l[3], &b, aff.res);
    } else {
        let mut p_tmp: QrPoint = [0; 2];
        qr_aff_project(&mut p_tmp, aff, dl.o[0], dl.o[1] + 3 * dl.size[1]);
        let shift = qr_maxi(
            0,
            qr_ilog(qr_maxi(aff.fwd[0][1].abs(), aff.fwd[1][1].abs()) as u32) - ((aff.res + 1) >> 1),
        );
        let round = (1i32 << shift) >> 1;
        // Note: zbar uses fwd[1][0] and -fwd[0][0] here, plus what looks like
        // a typo: `l[1][0]*p[0]+l[1][1]*p[1]` in the C constant term where
        // intuition says l[3]. We mirror the C verbatim — any change here
        // breaks bit-equivalence.
        l[3][0] = (aff.fwd[1][0] + round) >> shift;
        l[3][1] = (-aff.fwd[0][0] + round) >> shift;
        l[3][2] = -(l[1][0] * p_tmp[0] + l[1][1] * p_tmp[1]);
    }

    // ---- 5. Intersect corners ----
    for i in 0..4 {
        if qr_line_isect(&mut p[i], &l[i & 1], &l[2 + (i >> 1)]) < 0 {
            return -1;
        }
        // Allow corners slightly outside the image but not unboundedly so.
        let lo_x = -width << QR_FINDER_SUBPREC;
        let hi_x = width << (QR_FINDER_SUBPREC + 1);
        let lo_y = -height << QR_FINDER_SUBPREC;
        let hi_y = height << (QR_FINDER_SUBPREC + 1);
        if p[i][0] < lo_x || p[i][0] >= hi_x || p[i][1] < lo_y || p[i][1] >= hi_y {
            return -1;
        }
    }

    // ---- 6. Alignment-pattern refinement of bottom-right (versions ≥ 2) ----
    let mut brx = p[3][0];
    let mut bry = p[3][1];
    let version4 = ul.eversion[0] + ul.eversion[1] + ur.eversion[0] + dl.eversion[1];
    if version4 > 4 {
        let mut cell = QrHomCell::zero();
        let dim = 17 + version4;
        qr_hom_cell_init(
            &mut cell,
            0, 0, dim - 1, 0, 0, dim - 1, dim - 1, dim - 1,
            p[0][0], p[0][1], p[1][0], p[1][1], p[2][0], p[2][1], p[3][0], p[3][1],
        );
        let mut p3: QrPoint = [0; 2];
        if qr_alignment_pattern_search(&mut p3, &cell, dim - 7, dim - 7, 4, img, width, height) >= 0 {
            // Project the alignment center back to the corner of the code area.
            let c21 = (p[2][0] as i64).wrapping_mul(p[1][1] as i64)
                .wrapping_sub((p[2][1] as i64).wrapping_mul(p[1][0] as i64));
            let dx21 = (p[2][0] - p[1][0]) as i64;
            let dy21 = (p[2][1] - p[1][1]) as i64;
            let w = (dim - 7) as i64 * c21
                + (dim - 13) as i64 * ((p[0][0] as i64) * dy21 - (p[0][1] as i64) * dx21)
                + 6 * ((p3[0] as i64) * dy21 - (p3[1] as i64) * dx21);
            let mask: i64 = qr_signmask(w as i32) as i64; // -1 if w<0 else 0
            let w_abs = w.unsigned_abs() as i64;

            // brx = DIVROUND((( EXTMUL_nested ) + mask) ^ mask, w_abs)
            let extmul_x = (dim - 7) as i64 * (p[0][0] as i64) * (p3[0] as i64) * dy21
                + (dim - 13) as i64 * (p3[0] as i64) * (c21 - (p[0][1] as i64) * dx21)
                + 6 * (p[0][0] as i64) * (c21 - (p3[1] as i64) * dx21);
            let extmul_y = (dim - 7) as i64 * (p[0][1] as i64) * (-(p3[1] as i64)) * dx21
                + (dim - 13) as i64 * (p3[1] as i64) * (c21 + (p[0][0] as i64) * dy21)
                + 6 * (p[0][1] as i64) * (c21 + (p3[0] as i64) * dy21);

            let flipped_x = (extmul_x.wrapping_add(mask)) ^ mask;
            let flipped_y = (extmul_y.wrapping_add(mask)) ^ mask;

            // Rounded i64/i64 divide. w_abs > 0 by construction (we only entered
            // when the alignment pattern was found).
            let half = w_abs >> 1;
            brx = ((flipped_x + half) / w_abs) as i32;
            bry = ((flipped_y + half) / w_abs) as i32;
        }
    }

    // ---- Initialize the homography ----
    qr_hom_init(hom, p[0][0], p[0][1], p[1][0], p[1][1], p[2][0], p[2][1], brx, bry, QR_HOM_BITS);
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signatures_compile() {
        // Smoke test: a fully-zeroed call. We don't expect a useful return
        // value, only that the function doesn't panic and produces a
        // deterministic result.
        let mut hom = QrHom::zero();
        let aff = QrAff::zero();
        let mut isaac = IsaacCtx::new();
        super::super::isaac::isaac_init_empty(&mut isaac);
        let img = vec![0u8; 64 * 64];
        // Three finders with empty edge_pts — qr_finder_ransac early-returns
        // with ninliers=0, then qr_line_fit_finder_pair synthesizes points.
        let mut ul = QrFinder {
            size: [4, 4], eversion: [1, 1],
            edge_pts: Vec::new(), edge_pts_start: [0; 5],
            ninliers: [0; 4], o: [10, 10], center_pos: [10, 10],
        };
        let mut ur = QrFinder {
            size: [4, 4], eversion: [1, 1],
            edge_pts: Vec::new(), edge_pts_start: [0; 5],
            ninliers: [0; 4], o: [50, 10], center_pos: [50, 10],
        };
        let mut dl = QrFinder {
            size: [4, 4], eversion: [1, 1],
            edge_pts: Vec::new(), edge_pts_start: [0; 5],
            ninliers: [0; 4], o: [10, 50], center_pos: [10, 50],
        };
        let mut p: [QrPoint; 4] = [[0; 2]; 4];
        // Just check that the function runs without panicking.
        let _r = qr_hom_fit(&mut hom, &mut ul, &mut ur, &mut dl, &mut p, &aff, &mut isaac, &img, 64, 64);
        // Result is deterministic (likely -1 due to degenerate aff), but
        // shouldn't crash.
    }
}
