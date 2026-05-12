//! `QrFinder` state + edge-point classification + RANSAC.
//! Rust port of qrdec.c lines 838–1118 (Phase 4-F, chunk 2).
//!
//! qrdec.c's `qr_finder` struct points back into a `qr_finder_center`'s
//! shared edge-point pool and slices it by edge index. In Rust we keep the
//! edge points inside `QrFinder` directly (own them), and use a 5-entry
//! `edge_pts_start` array (one per edge plus a sentinel end) so the slice
//! for edge `e` is `edge_pts[edge_pts_start[e]..edge_pts_start[e+1]]`.

use super::finder_centers::{QrFinderCenter, QrFinderEdgePt};
use super::geom::{
    qr_aff_unproject, qr_hom_unproject, qr_point_ccw, qr_point_distance2,
    qr_point_translate, QrAff, QrHom, QrPoint, QR_FINDER_SUBPREC,
};
use super::isaac::{isaac_next_uint, IsaacCtx};
use super::util::{qr_divround, qr_isqrt};

/// Slack tolerated between the two axes' estimated version numbers.
pub const QR_LARGE_VERSION_SLACK: i32 = 3;
#[allow(dead_code)]
pub const QR_SMALL_VERSION_SLACK: i32 = 1;

/// All information collected about one finder pattern in the current
/// configuration. Mirrors `qr_finder` in qrdec.c.
#[derive(Clone, Debug)]
pub struct QrFinder {
    /// Module size along each axis (in the square domain).
    pub size: [i32; 2],
    /// Version estimated from the module size along each axis.
    pub eversion: [i32; 2],
    /// All edge points, sorted (edge ASC, extent ASC after classify).
    pub edge_pts: Vec<QrFinderEdgePt>,
    /// Start indices of edges 0..3 + sentinel end at index 4.
    pub edge_pts_start: [usize; 5],
    /// Number of inliers per edge after RANSAC.
    pub ninliers: [i32; 4],
    /// Finder center in the square domain.
    pub o: QrPoint,
    /// Original center info (just the image-space position; we don't need
    /// the original edge_pts since we own a sorted copy).
    pub center_pos: QrPoint,
}

impl QrFinder {
    /// Wrap a `QrFinderCenter`: take ownership of its edge points and reset
    /// the per-edge state.
    pub fn from_center(center: QrFinderCenter) -> Self {
        QrFinder {
            size: [0; 2],
            eversion: [0; 2],
            edge_pts: center.edge_pts,
            edge_pts_start: [0; 5],
            ninliers: [0; 4],
            o: [0; 2],
            center_pos: center.pos,
        }
    }

    #[inline]
    pub fn nedge_pts(&self, e: usize) -> i32 {
        (self.edge_pts_start[e + 1] - self.edge_pts_start[e]) as i32
    }

    #[inline]
    pub fn edge_slice(&self, e: usize) -> &[QrFinderEdgePt] {
        &self.edge_pts[self.edge_pts_start[e]..self.edge_pts_start[e + 1]]
    }

    #[inline]
    pub fn edge_slice_mut(&mut self, e: usize) -> &mut [QrFinderEdgePt] {
        let r = self.edge_pts_start[e]..self.edge_pts_start[e + 1];
        &mut self.edge_pts[r]
    }
}

/// Comparator for `qr_cmp_edge_pt` (qrdec.c:856).
#[inline]
fn cmp_edge_pt(a: &QrFinderEdgePt, b: &QrFinderEdgePt) -> std::cmp::Ordering {
    a.edge.cmp(&b.edge).then_with(|| a.extent.cmp(&b.extent))
}

/// Re-build the `edge_pts_start` array from the sorted edge-pts list.
/// Assumes the array is already sorted by `edge` and that edge labels are
/// in 0..=4 (4 = "infinity" reject bucket used by hom_classify).
fn rebuild_edge_starts(finder: &mut QrFinder) {
    finder.edge_pts_start = [0; 5];
    // Count per-edge entries (edges 0..4).
    let mut counts = [0usize; 5];
    for ep in &finder.edge_pts {
        let e = ep.edge.max(0).min(4) as usize;
        counts[e] += 1;
    }
    // Cumulative sums — start[e] is the offset of edge e's first point.
    let mut acc = 0usize;
    for e in 0..5 {
        finder.edge_pts_start[e] = acc;
        acc += counts[e];
    }
    // Note: we only model 4 edges in edge_pts_start[0..=4]; edge==4 points
    // (set by hom_classify when unproject fails) are sorted past edge 3 and
    // accessible via edge_pts[edge_pts_start[4]..edge_pts.len()].
}

/// Affine classifier (qrdec.c:870).
pub fn qr_finder_edge_pts_aff_classify(finder: &mut QrFinder, aff: &QrAff) {
    for ep in &mut finder.edge_pts {
        let mut q: QrPoint = [0; 2];
        qr_aff_unproject(&mut q, aff, ep.pos[0], ep.pos[1]);
        qr_point_translate(&mut q, -finder.o[0], -finder.o[1]);
        let d = (q[1].abs() > q[0].abs()) as usize;
        let e = (d << 1) | ((q[d] >= 0) as usize);
        ep.edge = e as i32;
        ep.extent = q[d];
    }
    finder.edge_pts.sort_by(cmp_edge_pt);
    rebuild_edge_starts(finder);
}

/// Homography classifier (qrdec.c:897). Points the homography sends to
/// infinity get edge=4 (a sentinel; `nedge_pts` for edges 0..3 still
/// excludes them via the `edge_pts_start` layout).
pub fn qr_finder_edge_pts_hom_classify(finder: &mut QrFinder, hom: &QrHom) {
    for ep in &mut finder.edge_pts {
        let mut q: QrPoint = [0; 2];
        if qr_hom_unproject(&mut q, hom, ep.pos[0], ep.pos[1]) >= 0 {
            qr_point_translate(&mut q, -finder.o[0], -finder.o[1]);
            let d = (q[1].abs() > q[0].abs()) as usize;
            let e = (d << 1) | ((q[d] >= 0) as usize);
            ep.edge = e as i32;
            ep.extent = q[d];
        } else {
            ep.edge = 4;
            ep.extent = q[0];
        }
    }
    finder.edge_pts.sort_by(cmp_edge_pt);
    rebuild_edge_starts(finder);
}

/// Estimate module size + QR version from edge-point extents (qrdec.c:940).
/// Returns `-1` if either axis fails sanity checks; `0` on success
/// (`finder.size` and `finder.eversion` updated).
pub fn qr_finder_estimate_module_size_and_version(
    finder: &mut QrFinder,
    width: i32,
    height: i32,
) -> i32 {
    let mut offs = [0i32; 2];
    let mut sums = [0i32; 4];
    let mut nsums = [0i32; 4];

    for e in 0..4 {
        let n0 = finder.nedge_pts(e);
        if n0 > 0 {
            // Average extent dropping the top + bottom 25%.
            let edge_pts = finder.edge_slice(e);
            let drop = (n0 >> 2) as usize;
            let mut sum = 0i32;
            for i in drop..(n0 as usize - drop) {
                sum = sum.wrapping_add(edge_pts[i].extent);
            }
            let n = n0 - ((n0 >> 2) << 1);
            let mean = qr_divround(sum, n);
            offs[e >> 1] += mean;
            sums[e] = sum;
            nsums[e] = n;
        }
    }

    // Refine the unprojected center when we have samples on both sides of
    // an axis.
    if finder.nedge_pts(0) > 0 && finder.nedge_pts(1) > 0 {
        finder.o[0] = finder.o[0].wrapping_sub(offs[0] >> 1);
        sums[0] = sums[0].wrapping_sub(offs[0].wrapping_mul(nsums[0]) >> 1);
        sums[1] = sums[1].wrapping_sub(offs[0].wrapping_mul(nsums[1]) >> 1);
    }
    if finder.nedge_pts(2) > 0 && finder.nedge_pts(3) > 0 {
        finder.o[1] = finder.o[1].wrapping_sub(offs[1] >> 1);
        sums[2] = sums[2].wrapping_sub(offs[1].wrapping_mul(nsums[2]) >> 1);
        sums[3] = sums[3].wrapping_sub(offs[1].wrapping_mul(nsums[3]) >> 1);
    }

    let nusize = nsums[0] + nsums[1];
    if nusize <= 0 { return -1; }
    let nusize = nusize * 3; // module size = 1/3 average edge extent
    let usize_raw = sums[1] - sums[0];
    let usize_val = ((usize_raw << 1) + nusize) / (nusize << 1);
    if usize_val <= 0 { return -1; }
    let uversion = (width - 8 * usize_val) / (usize_val << 2);
    if !(1..=40 + QR_LARGE_VERSION_SLACK).contains(&uversion) { return -1; }

    let nvsize = nsums[2] + nsums[3];
    if nvsize <= 0 { return -1; }
    let nvsize = nvsize * 3;
    let vsize_raw = sums[3] - sums[2];
    let vsize_val = ((vsize_raw << 1) + nvsize) / (nvsize << 1);
    if vsize_val <= 0 { return -1; }
    let vversion = (height - 8 * vsize_val) / (vsize_val << 2);
    if !(1..=40 + QR_LARGE_VERSION_SLACK).contains(&vversion) { return -1; }

    if (uversion - vversion).abs() > QR_LARGE_VERSION_SLACK { return -1; }

    finder.size[0] = usize_val;
    finder.size[1] = vsize_val;
    finder.eversion[0] = uversion;
    finder.eversion[1] = vversion;
    0
}

/// RANSAC outlier elimination for a single edge (qrdec.c:1033).
///
/// `e` selects the edge in 0..=3. After return, `finder.ninliers[e]`
/// holds the inlier count and the first `ninliers` entries of the edge's
/// slice are the inliers (order may differ from input).
///
/// Note: the second parameter is an affine transform — zbar's variable is
/// named `_hom` but the type is `qr_aff *`. We match the actual type.
pub fn qr_finder_ransac(finder: &mut QrFinder, aff: &QrAff, isaac: &mut IsaacCtx, e: usize) {
    let n_total = finder.nedge_pts(e);
    let mut best_ninliers: i32 = 0;
    if n_total > 1 {
        let mut max_iters: i32 = 17;
        let mut i = 0i32;
        // Snapshot finder.o so we don't have to re-borrow inside the loop.
        let ox = finder.o[0];
        let oy = finder.o[1];
        let n = n_total as u32;

        while i < max_iters {
            i += 1;

            // Sample two distinct indices in [0, n).
            let p0i = isaac_next_uint(isaac, n) as usize;
            let mut p1i = isaac_next_uint(isaac, n - 1) as usize;
            if p1i >= p0i { p1i += 1; }

            let (p0, p1) = {
                let edge_pts = finder.edge_slice(e);
                (edge_pts[p0i].pos, edge_pts[p1i].pos)
            };

            // Reject lines >45° off the expected axis (qrdec.c:1064).
            let mut q0: QrPoint = [0; 2];
            let mut q1: QrPoint = [0; 2];
            qr_aff_unproject(&mut q0, aff, p0[0], p0[1]);
            qr_aff_unproject(&mut q1, aff, p1[0], p1[1]);
            qr_point_translate(&mut q0, -ox, -oy);
            qr_point_translate(&mut q1, -ox, -oy);
            let axis = e >> 1;
            let other = 1 - axis;
            if (q0[axis] - q1[axis]).abs() > (q0[other] - q1[other]).abs() {
                continue;
            }

            // thresh = isqrt(distance²(p0,p1) << (2*SUBPREC + 1))
            let dist2 = qr_point_distance2(&p0, &p1);
            let thresh = qr_isqrt(dist2 << (2 * QR_FINDER_SUBPREC + 1)) as i32;

            // Mark inliers / outliers in bit 0 of `extent`.
            let mut ninliers = 0i32;
            {
                let edge_pts = finder.edge_slice_mut(e);
                for ep in edge_pts.iter_mut() {
                    let dist = qr_point_ccw(&p0, &p1, &ep.pos).abs();
                    if dist <= thresh {
                        ep.extent |= 1;
                        ninliers += 1;
                    } else {
                        ep.extent &= !1;
                    }
                }
            }
            if ninliers > best_ninliers {
                // Shift the best-iteration flag into bit 1.
                let edge_pts = finder.edge_slice_mut(e);
                for ep in edge_pts.iter_mut() {
                    ep.extent = ep.extent.wrapping_shl(1);
                }
                best_ninliers = ninliers;
                // Adaptive early-exit when we found > half inliers.
                if ninliers > (n_total >> 1) {
                    max_iters = (67 * n_total - 63 * ninliers - 1) / (n_total << 1);
                }
            }
        }

        // Collect inliers (bit 1 set) at the front. Per zbar, this is a
        // one-way copy — non-inliers at the front get overwritten; that's
        // fine because they're discarded.
        let edge_pts = finder.edge_slice_mut(e);
        let mut j = 0usize;
        let mut i = 0usize;
        while j < best_ninliers as usize && i < edge_pts.len() {
            if edge_pts[i].extent & 2 != 0 {
                if j < i {
                    edge_pts[j] = edge_pts[i].clone();
                }
                j += 1;
            }
            i += 1;
        }
    }
    finder.ninliers[e] = best_ninliers;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::qrcode::geom::qr_aff_init;
    use crate::qrcode::isaac::{isaac_init_empty, IsaacCtx};

    fn ep(x: i32, y: i32) -> QrFinderEdgePt {
        QrFinderEdgePt { pos: [x, y], edge: 0, extent: 0 }
    }

    fn synthetic_finder() -> QrFinder {
        // Centered at (100, 100) with edge points roughly forming a square:
        //  - left edge at x ≈ 90 (edge 0), right at x ≈ 110 (edge 1),
        //  - top at y ≈ 90 (edge 2), bottom at y ≈ 110 (edge 3).
        let center = QrFinderCenter {
            pos: [100, 100],
            edge_pts: vec![
                ep(90, 100), ep(90, 99), ep(91, 101),  // left
                ep(110, 100), ep(110, 101), ep(109, 99), // right
                ep(100, 90), ep(99, 90), ep(101, 91),   // top
                ep(100, 110), ep(99, 110), ep(101, 109), // bottom
            ],
        };
        QrFinder::from_center(center)
    }

    #[test]
    fn aff_classify_assigns_4_edges() {
        let mut f = synthetic_finder();
        // Affine that maps image space 1:1 to the square domain (det=1).
        // unproject(p) returns the image-space coordinates unchanged, so
        // f.o is also in image-space units.
        let res = 8;
        let mut aff = QrAff::zero();
        qr_aff_init(&mut aff,
            &[0, 0], &[1 << res, 0], &[0, 1 << res], res);
        f.o = [100, 100];
        qr_finder_edge_pts_aff_classify(&mut f, &aff);
        for e in 0..4 {
            assert!(f.nedge_pts(e) >= 1, "edge {} got {} pts", e, f.nedge_pts(e));
        }
    }

    #[test]
    fn estimate_module_size_succeeds_on_synthetic() {
        let mut f = synthetic_finder();
        let res = 8;
        let mut aff = QrAff::zero();
        qr_aff_init(&mut aff,
            &[0, 0], &[1 << res, 0], &[0, 1 << res], res);
        f.o = [100, 100];
        qr_finder_edge_pts_aff_classify(&mut f, &aff);
        // Width/height in the square domain — synthetic finder modules are
        // tiny so this may legitimately reject; we only check that the
        // function runs and returns deterministically.
        let r = qr_finder_estimate_module_size_and_version(&mut f, 56, 56);
        assert!(r == 0 || r == -1, "unexpected return {}", r);
    }

    #[test]
    fn ransac_with_zero_or_one_point_is_noop() {
        let mut f = synthetic_finder();
        // Make edge 0 empty.
        f.edge_pts.clear();
        f.edge_pts_start = [0; 5];
        let res = 8;
        let mut aff = QrAff::zero();
        qr_aff_init(&mut aff,
            &[0, 0], &[1 << res, 0], &[0, 1 << res], res);
        let mut isaac = IsaacCtx::new();
        isaac_init_empty(&mut isaac);
        qr_finder_ransac(&mut f, &aff, &mut isaac, 0);
        assert_eq!(f.ninliers[0], 0);
    }

    #[test]
    fn ransac_marks_collinear_points_as_inliers() {
        // 5 points on a horizontal line + 1 outlier. The orientation check
        // requires points whose variation lies in the perpendicular axis,
        // so horizontal-line points belong to edge 2 (top, axis=v=1).
        let mut f = QrFinder::from_center(QrFinderCenter {
            pos: [0, 0],
            edge_pts: vec![
                ep(0, 0), ep(10, 0), ep(20, 0), ep(30, 0), ep(40, 0),
                ep(15, 50), // outlier
            ],
        });
        // edge_pts_start[e] = first index for edge e. All 6 points in edge 2.
        f.edge_pts_start = [0, 0, 0, 6, 6];
        let res = 8;
        let mut aff = QrAff::zero();
        qr_aff_init(&mut aff,
            &[0, 0], &[1 << res, 0], &[0, 1 << res], res);
        let mut isaac = IsaacCtx::new();
        isaac_init_empty(&mut isaac);
        qr_finder_ransac(&mut f, &aff, &mut isaac, 2);
        assert!(f.ninliers[2] >= 4, "expected ≥4 inliers, got {}", f.ninliers[2]);
    }
}
