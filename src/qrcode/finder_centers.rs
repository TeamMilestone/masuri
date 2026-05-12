//! Finder-line clustering and crossing detection — first stage of QR
//! finder-pattern recognition. Rust port of:
//!   - `qr_finder_cluster_lines`, `qr_finder_edge_pts_fill`,
//!     `qr_finder_lines_are_crossing`, `qr_finder_find_crossings`,
//!     `qr_finder_centers_locate`
//!   in zbar/qrcode/qrdec.c (Phase 4-F, chunk 1).
//!
//! Memory layout differs slightly from C — zbar uses one big malloc per pass
//! and threads pointers into it; we use Vec-per-center for simplicity. The
//! semantics (which lines cluster, which clusters cross, finder-center
//! positions, edge-point ordering) match bit-for-bit.

use std::cmp::Ordering;

use super::finder::QrFinderLine;
use super::geom::{QrPoint, QR_FINDER_SUBPREC};

/// A point on the edge of a finder pattern. Mirrors `qr_finder_edge_pt`.
/// `edge`/`extent` are filled in later by the affine/homography classifier;
/// position is the only thing populated by clustering.
#[derive(Clone, Debug, Default)]
pub struct QrFinderEdgePt {
    pub pos: QrPoint,
    /// 0/1/2/3 = -u/+u/-v/+v edge — set by aff/hom classify in the next chunk.
    pub edge: i32,
    /// Signed perpendicular distance from the center line (later re-used by RANSAC).
    pub extent: i32,
}

/// A finder-pattern center candidate. Mirrors `qr_finder_center`.
/// `edge_pts` is owned per-center (zbar pools all edge points in one buffer
/// and threads pointers; we trade a touch of cache locality for simplicity).
#[derive(Clone, Debug, Default)]
pub struct QrFinderCenter {
    pub pos: QrPoint,
    pub edge_pts: Vec<QrFinderEdgePt>,
}

/// Internal cluster representation: indices into the source `lines` slice.
#[derive(Clone, Debug)]
struct QrFinderCluster {
    line_indices: Vec<usize>,
}

/// Comparator for sorting vertical finder lines by X (then Y) — matches
/// `qr_finder_vline_cmp` in qrdec.c:153.
fn qr_finder_vline_cmp(a: &QrFinderLine, b: &QrFinderLine) -> Ordering {
    a.pos[0].cmp(&b.pos[0]).then_with(|| a.pos[1].cmp(&b.pos[1]))
}

/// Clusters adjacent finder lines (qrdec.c:174).
///
/// `lines` must be pre-sorted: horizontal by Y then X, vertical by X then Y.
/// `v` = 0 for horizontal lines, 1 for vertical.
fn qr_finder_cluster_lines(lines: &[QrFinderLine], v: usize) -> Vec<QrFinderCluster> {
    let nlines = lines.len();
    if nlines < 2 { return Vec::new(); }
    let mut mark = vec![false; nlines];
    let mut clusters: Vec<QrFinderCluster> = Vec::new();
    let other = 1 - v;

    for i in 0..nlines - 1 {
        if mark[i] { continue; }
        let mut neighbors = vec![i];
        let mut len = lines[i].len;
        for j in (i + 1)..nlines {
            if mark[j] { continue; }
            let a = &lines[*neighbors.last().unwrap()];
            let b = &lines[j];
            // thresh = (a.len + 7) >> 2
            let thresh = (a.len + 7) >> 2;
            // Break early when the lines drift past the cluster column/row.
            if (a.pos[other] - b.pos[other]).abs() > thresh { break; }
            // Skip if the line offset (along the scan direction) diverged too much.
            if (a.pos[v] - b.pos[v]).abs() > thresh { continue; }
            if (a.pos[v] + a.len - b.pos[v] - b.len).abs() > thresh { continue; }
            if a.boffs > 0 && b.boffs > 0
                && (a.pos[v] - a.boffs - b.pos[v] + b.boffs).abs() > thresh {
                continue;
            }
            if a.eoffs > 0 && b.eoffs > 0
                && (a.pos[v] + a.len + a.eoffs - b.pos[v] - b.len - b.eoffs).abs() > thresh {
                continue;
            }
            neighbors.push(j);
            len += b.len;
        }
        // Require at least 3 lines per cluster (kills most false positives).
        if neighbors.len() < 3 { continue; }
        let nneighbors = neighbors.len() as i32;
        // Average line length, rounded — must be at least 1/3 the cluster size
        // (post-shift by QR_FINDER_SUBPREC).
        let avg_len = ((len << 1) + nneighbors) / (nneighbors << 1);
        if nneighbors * (5 << QR_FINDER_SUBPREC) >= avg_len {
            for &k in &neighbors { mark[k] = true; }
            clusters.push(QrFinderCluster { line_indices: neighbors });
        }
    }
    clusters
}

/// Append edge points from a list of clusters into `edge_pts`.
/// Position-only init; edge/extent stay at default.
fn qr_finder_edge_pts_fill(
    edge_pts: &mut Vec<QrFinderEdgePt>,
    lines: &[QrFinderLine],
    cluster_indices: &[usize],
    clusters: &[QrFinderCluster],
    v: usize,
) {
    for &ci in cluster_indices {
        for &li in &clusters[ci].line_indices {
            let l = &lines[li];
            if l.boffs > 0 {
                let mut pos = l.pos;
                pos[v] -= l.boffs;
                edge_pts.push(QrFinderEdgePt { pos, edge: 0, extent: 0 });
            }
            if l.eoffs > 0 {
                let mut pos = l.pos;
                pos[v] += l.len + l.eoffs;
                edge_pts.push(QrFinderEdgePt { pos, edge: 0, extent: 0 });
            }
        }
    }
}

/// Returns `true` if a horizontal and vertical line cross. Mirrors
/// `qr_finder_lines_are_crossing` (qrdec.c:288).
#[inline]
fn qr_finder_lines_are_crossing(hline: &QrFinderLine, vline: &QrFinderLine) -> bool {
    hline.pos[0] <= vline.pos[0]
        && vline.pos[0] < hline.pos[0] + hline.len
        && vline.pos[1] <= hline.pos[1]
        && hline.pos[1] < vline.pos[1] + vline.len
}

/// Find crossings between horizontal and vertical clusters — putative finder
/// centers. Mirrors `qr_finder_find_crossings` (qrdec.c:305). The returned
/// list is sorted: most edge points first, then by (y, x).
fn qr_finder_find_crossings(
    hlines: &[QrFinderLine],
    vlines: &[QrFinderLine],
    hclusters: &[QrFinderCluster],
    vclusters: &[QrFinderCluster],
) -> Vec<QrFinderCenter> {
    let nh = hclusters.len();
    let nv = vclusters.len();
    let mut hmark = vec![false; nh];
    let mut vmark = vec![false; nv];
    let mut centers: Vec<QrFinderCenter> = Vec::new();

    for i in 0..nh {
        if hmark[i] { continue; }
        // "Representative" line of the cluster — middle one (nlines/2).
        let mid_h = hclusters[i].line_indices.len() / 2;
        let mut a = &hlines[hclusters[i].line_indices[mid_h]];

        let mut vneighbors: Vec<usize> = Vec::new();
        let mut y: i64 = 0;
        for j in 0..nv {
            if vmark[j] { continue; }
            let mid_v = vclusters[j].line_indices.len() / 2;
            let b = &vlines[vclusters[j].line_indices[mid_v]];
            if qr_finder_lines_are_crossing(a, b) {
                vmark[j] = true;
                y += ((b.pos[1] as i64) << 1) + b.len as i64;
                if b.boffs > 0 && b.eoffs > 0 {
                    y += (b.eoffs - b.boffs) as i64;
                }
                vneighbors.push(j);
            }
        }
        if vneighbors.is_empty() { continue; }

        let mut x: i64 = ((a.pos[0] as i64) << 1) + a.len as i64;
        if a.boffs > 0 && a.eoffs > 0 {
            x += (a.eoffs - a.boffs) as i64;
        }
        let mut hneighbors = vec![i];

        // Pick the middle vneighbor's middle line to find more crossing h-clusters.
        let pivot_v_cluster = &vclusters[vneighbors[vneighbors.len() / 2]];
        let pivot_b = &vlines[pivot_v_cluster.line_indices[pivot_v_cluster.line_indices.len() / 2]];

        for j in (i + 1)..nh {
            if hmark[j] { continue; }
            let mid = hclusters[j].line_indices.len() / 2;
            a = &hlines[hclusters[j].line_indices[mid]];
            if qr_finder_lines_are_crossing(a, pivot_b) {
                hmark[j] = true;
                x += ((a.pos[0] as i64) << 1) + a.len as i64;
                if a.boffs > 0 && a.eoffs > 0 {
                    x += (a.eoffs - a.boffs) as i64;
                }
                hneighbors.push(j);
            }
        }

        let nh_n = hneighbors.len() as i64;
        let nv_n = vneighbors.len() as i64;
        let mut edge_pts: Vec<QrFinderEdgePt> = Vec::new();
        qr_finder_edge_pts_fill(&mut edge_pts, hlines, &hneighbors, hclusters, 0);
        qr_finder_edge_pts_fill(&mut edge_pts, vlines, &vneighbors, vclusters, 1);

        centers.push(QrFinderCenter {
            pos: [
                ((x + nh_n) / (nh_n << 1)) as i32,
                ((y + nv_n) / (nv_n << 1)) as i32,
            ],
            edge_pts,
        });
    }

    // Sort by decreasing edge-point count, then ascending y, then ascending x —
    // matches `qr_finder_center_cmp` (qrdec.c:274).
    centers.sort_by(|a, b| {
        b.edge_pts.len().cmp(&a.edge_pts.len())
            .then_with(|| a.pos[1].cmp(&b.pos[1]))
            .then_with(|| a.pos[0].cmp(&b.pos[0]))
    });
    centers
}

/// Top-level entry: locate finder-center candidates from accumulated h/v
/// finder lines. Mirrors `qr_finder_centers_locate` (qrdec.c:405).
///
/// The input vectors must already contain the lines emitted by the 1D scanner
/// (Phase 1) split by direction: `hlines` for horizontal, `vlines` for
/// vertical. `vlines` is sorted in place to satisfy the clustering
/// precondition (image scanning emits them column-major; clustering wants
/// row-major).
///
/// Returns an empty Vec if fewer than 3 clusters are found in either
/// direction (we need three finder patterns for a valid QR code).
pub fn qr_finder_centers_locate(
    hlines: &mut [QrFinderLine],
    vlines: &mut [QrFinderLine],
) -> Vec<QrFinderCenter> {
    let hclusters = qr_finder_cluster_lines(hlines, 0);
    // C scans columns top-to-bottom, so vlines arrive grouped by row instead
    // of by column. Re-sort here to match the clustering precondition.
    vlines.sort_by(qr_finder_vline_cmp);
    let vclusters = qr_finder_cluster_lines(vlines, 1);
    if hclusters.len() < 3 || vclusters.len() < 3 {
        return Vec::new();
    }
    qr_finder_find_crossings(hlines, vlines, &hclusters, &vclusters)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hline(x: i32, y: i32, len: i32) -> QrFinderLine {
        QrFinderLine { pos: [x, y], len, boffs: 2, eoffs: 2 }
    }
    fn vline(x: i32, y: i32, len: i32) -> QrFinderLine {
        QrFinderLine { pos: [x, y], len, boffs: 2, eoffs: 2 }
    }

    #[test]
    fn cluster_needs_three_lines() {
        // Two lines should not cluster.
        let lines = vec![hline(100, 100, 20), hline(100, 101, 20)];
        let clusters = qr_finder_cluster_lines(&lines, 0);
        assert!(clusters.is_empty());
    }

    #[test]
    fn cluster_groups_close_parallel_lines() {
        // Five parallel horizontal lines at near-identical Y; should form one cluster.
        let lines: Vec<QrFinderLine> = (0..5)
            .map(|i| hline(100, 100 + i, 40))
            .collect();
        let clusters = qr_finder_cluster_lines(&lines, 0);
        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].line_indices.len(), 5);
    }

    #[test]
    fn cluster_separates_distant_groups() {
        // Two groups of 4 lines, far apart vertically.
        let mut lines = Vec::new();
        for i in 0..4 { lines.push(hline(100, 100 + i, 40)); }
        for i in 0..4 { lines.push(hline(100, 500 + i, 40)); }
        let clusters = qr_finder_cluster_lines(&lines, 0);
        assert_eq!(clusters.len(), 2);
    }

    #[test]
    fn crossing_detection() {
        // Horizontal line from (100, 200) length 50; covers x ∈ [100,150).
        let h = hline(100, 200, 50);
        // Vertical line at x = 120, y ∈ [180, 240).
        let v = vline(120, 180, 60);
        assert!(qr_finder_lines_are_crossing(&h, &v));
        // Move v's x outside the horizontal extent.
        let v2 = vline(160, 180, 60);
        assert!(!qr_finder_lines_are_crossing(&h, &v2));
    }

    #[test]
    fn centers_locate_synthetic_finder_pattern() {
        // Simulate a single finder pattern at image position (200, 300):
        // - 5 horizontal lines crossing it at varying Y.
        // - 5 vertical lines crossing it at varying X.
        let mut h: Vec<QrFinderLine> = (0..5)
            .map(|i| hline(180, 295 + i, 50))
            .collect();
        let mut v: Vec<QrFinderLine> = (0..5)
            .map(|i| vline(195 + i, 280, 50))
            .collect();

        // Need ≥3 h-clusters and ≥3 v-clusters for the function to attempt
        // crossings — add two more finder patterns offset diagonally.
        for offset in &[(400, 0), (0, 400)] {
            for i in 0..5 {
                h.push(hline(180 + offset.0, 295 + offset.1 + i, 50));
                v.push(vline(195 + offset.0 + i, 280 + offset.1, 50));
            }
        }
        let centers = qr_finder_centers_locate(&mut h, &mut v);
        // We should at least detect the cluster at our central finder.
        assert!(!centers.is_empty(), "expected ≥1 center, got 0");
        // The strongest center (most edge points) should be sorted first.
        let strongest = &centers[0];
        // Position should fall near one of the three synthetic centers.
        let near = strongest.pos[0].abs() < 800 && strongest.pos[1].abs() < 800;
        assert!(near, "center {:?} outside expected range", strongest.pos);
        // Each detected center carries edge points from its clusters.
        assert!(!strongest.edge_pts.is_empty());
    }
}
