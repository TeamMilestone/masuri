//! QR sampling grid — divides the code into cells bounded by finder /
//! alignment patterns and stores a `QrHomCell` per cell + a bitmap mask of
//! function-pattern modules. Rust port of qrdec.c lines 2536–2762
//! (Phase 4-D, chunk 1).

use super::alignment::qr_alignment_pattern_search;
use super::geom::{
    qr_hom_cell_init, qr_hom_cell_project, QrHomCell, QrPoint,
    QR_FINDER_SUBPREC, QR_INT_BITS,
};
use super::util::{qr_clampi, qr_sort2i};

/// `QR_ILOG(QR_INT_BITS)` from zbar — for `QR_INT_BITS=32`, this is 5.
/// Used as a shift count: word index = bit_index >> QR_INT_LOGBITS.
pub const QR_INT_LOGBITS: u32 = 5;

/// Alignment-pattern spacing for versions 7..=40 (qrdec.c:2568).
pub const QR_ALIGNMENT_SPACING: [u8; 34] = [
    16, 18, 20, 22, 24, 26, 28,
    20, 22, 24, 24, 26, 28, 28,
    22, 24, 24, 26, 26, 28, 28,
    24, 24, 26, 26, 26, 28, 28,
    24, 26, 26, 26, 28, 28,
];

/// Sampling grid for one QR configuration. The 2-D `cells` array is stored
/// flat as `cells[i * ncells + j]`.
#[derive(Clone, Debug)]
pub struct QrSamplingGrid {
    pub cells: Vec<QrHomCell>,
    pub fpmask: Vec<u32>,
    pub cell_limits: [i32; 6],
    pub ncells: usize,
}

impl QrSamplingGrid {
    pub fn new() -> Self {
        QrSamplingGrid {
            cells: Vec::new(),
            fpmask: Vec::new(),
            cell_limits: [0; 6],
            ncells: 0,
        }
    }

    #[inline]
    pub fn cell(&self, i: usize, j: usize) -> &QrHomCell {
        &self.cells[i * self.ncells + j]
    }

    #[inline]
    pub fn cell_mut(&mut self, i: usize, j: usize) -> &mut QrHomCell {
        let nc = self.ncells;
        &mut self.cells[i * nc + j]
    }
}

#[inline]
fn fpmask_stride(dim: i32) -> usize {
    ((dim + QR_INT_BITS - 1) >> QR_INT_LOGBITS) as usize
}

/// Mark a w×h block of grid cells starting at `(u, v)` as belonging to the
/// function pattern. Bits are stored column-wise (see qrdec.c:2551).
pub fn qr_sampling_grid_fp_mask_rect(
    grid: &mut QrSamplingGrid,
    dim: i32,
    u: i32, v: i32, w: i32, h: i32,
) {
    let stride = fpmask_stride(dim);
    for j in u..u + w {
        for i in v..v + h {
            let word_idx = j as usize * stride + ((i >> QR_INT_LOGBITS) as usize);
            grid.fpmask[word_idx] |= 1u32 << ((i & (QR_INT_BITS - 1)) as u32);
        }
    }
}

/// `true` if cell `(u, v)` lies inside a function-pattern region.
#[inline]
pub fn qr_sampling_grid_is_in_fp(
    grid: &QrSamplingGrid,
    dim: i32,
    u: i32, v: i32,
) -> bool {
    let stride = fpmask_stride(dim);
    let word_idx = u as usize * stride + ((v >> QR_INT_LOGBITS) as usize);
    ((grid.fpmask[word_idx] >> ((v & (QR_INT_BITS - 1)) as u32)) & 1) != 0
}

/// Initialize the sampling grid for the given configuration (qrdec.c:2598).
/// Mutates `p[0..4]` to a bounding quadrilateral; on entry it must hold the
/// estimated corner positions.
#[allow(clippy::too_many_arguments)]
pub fn qr_sampling_grid_init(
    grid: &mut QrSamplingGrid,
    version: i32,
    ul_pos: &QrPoint, ur_pos: &QrPoint, dl_pos: &QrPoint,
    p: &mut [QrPoint; 4],
    img: &[u8], width: i32, height: i32,
) {
    let dim = 17 + (version << 2);
    let nalign = (version / 7) + 2;
    let nalign_us = nalign as usize;

    // Base cell bootstraps the alignment-pattern search.
    let mut base_cell = QrHomCell::zero();
    qr_hom_cell_init(
        &mut base_cell,
        0, 0, dim - 1, 0, 0, dim - 1, dim - 1, dim - 1,
        p[0][0], p[0][1], p[1][0], p[1][1], p[2][0], p[2][1], p[3][0], p[3][1],
    );

    let ncells = (nalign - 1) as usize;
    grid.ncells = ncells;
    grid.cells = vec![QrHomCell::zero(); ncells * ncells];
    grid.fpmask = vec![0u32; dim as usize * fpmask_stride(dim)];

    // Mask out finder + separators + format info bits.
    qr_sampling_grid_fp_mask_rect(grid, dim, 0, 0, 9, 9);
    qr_sampling_grid_fp_mask_rect(grid, dim, 0, dim - 8, 9, 8);
    qr_sampling_grid_fp_mask_rect(grid, dim, dim - 8, 0, 8, 9);
    if version > 6 {
        qr_sampling_grid_fp_mask_rect(grid, dim, 0, dim - 11, 6, 3);
        qr_sampling_grid_fp_mask_rect(grid, dim, dim - 11, 0, 3, 6);
    }
    // Timing patterns.
    qr_sampling_grid_fp_mask_rect(grid, dim, 9, 6, dim - 17, 1);
    qr_sampling_grid_fp_mask_rect(grid, dim, 6, 9, 1, dim - 17);

    let mut align_pos = [0i32; 7];

    if version < 2 {
        // Version 1: no alignment patterns; the base cell is the only cell.
        grid.cells[0] = base_cell;
    } else {
        // Alignment-pattern positions along each axis.
        align_pos[0] = 6;
        align_pos[nalign_us - 1] = dim - 7;
        if version > 6 {
            let d = QR_ALIGNMENT_SPACING[(version - 7) as usize] as i32;
            for i in (1..nalign_us - 1).rev() {
                align_pos[i] = align_pos[i + 1] - d;
            }
        }

        // Grid points: q (square domain) and p (image space).
        let mut q = vec![[0i32; 2]; nalign_us * nalign_us];
        let mut pa = vec![[0i32; 2]; nalign_us * nalign_us];

        // Three corners come from the finder patterns rather than alignment.
        q[0] = [3, 3];
        pa[0] = *ul_pos;
        q[nalign_us - 1] = [dim - 4, 3];
        pa[nalign_us - 1] = *ur_pos;
        q[(nalign_us - 1) * nalign_us] = [3, dim - 4];
        pa[(nalign_us - 1) * nalign_us] = *dl_pos;

        // Diagonal sweep — qrdec.c:2664.
        for k in 1..(2 * nalign_us - 1) {
            let mut jmax = k.min(nalign_us - 1) as i32 - if k == nalign_us - 1 { 1 } else { 0 };
            let jmin = (k as i32 - (nalign_us as i32 - 1)).max(0) + if k == nalign_us - 1 { 1 } else { 0 };
            let _ = &mut jmax;
            for j in jmin..=jmax {
                let i = jmax - (j - jmin);
                let kk = (i as usize) * nalign_us + j as usize;
                let u_idx = align_pos[j as usize];
                let v_idx = align_pos[i as usize];
                q[kk][0] = u_idx;
                q[kk][1] = v_idx;
                // Mask out this alignment pattern's 5×5 footprint.
                qr_sampling_grid_fp_mask_rect(grid, dim, u_idx - 2, v_idx - 2, 5, 5);

                // Pick a cell to govern the alignment-pattern search.
                let search_cell: QrHomCell;
                if i > 1 && j > 1 {
                    // Three predictors from neighboring cells; take their
                    // coordinate-wise median.
                    let cell_a = grid.cell((i - 2) as usize, (j - 1) as usize);
                    let cell_b = grid.cell((i - 2) as usize, (j - 2) as usize);
                    let cell_c = grid.cell((i - 1) as usize, (j - 2) as usize);
                    let mut p0: QrPoint = [0; 2];
                    let mut p1: QrPoint = [0; 2];
                    let mut p2: QrPoint = [0; 2];
                    qr_hom_cell_project(&mut p0, cell_a, u_idx, v_idx, 0);
                    qr_hom_cell_project(&mut p1, cell_b, u_idx, v_idx, 0);
                    qr_hom_cell_project(&mut p2, cell_c, u_idx, v_idx, 0);
                    qr_sort2i(&mut p0[0], &mut p1[0]);
                    qr_sort2i(&mut p0[1], &mut p1[1]);
                    qr_sort2i(&mut p1[0], &mut p2[0]);
                    qr_sort2i(&mut p1[1], &mut p2[1]);
                    qr_sort2i(&mut p0[0], &mut p1[0]);
                    qr_sort2i(&mut p0[1], &mut p1[1]);
                    // Construct a search cell from the three known neighbors
                    // anchored at the predicted center (p1, the median).
                    let mut sc = QrHomCell::zero();
                    let qa = q[kk - nalign_us - 1];
                    let qb = q[kk - nalign_us];
                    let qc = q[kk - 1];
                    let qd = q[kk];
                    let pa_a = pa[kk - nalign_us - 1];
                    let pa_b = pa[kk - nalign_us];
                    let pa_c = pa[kk - 1];
                    qr_hom_cell_init(
                        &mut sc,
                        qa[0], qa[1], qb[0], qb[1], qc[0], qc[1], qd[0], qd[1],
                        pa_a[0], pa_a[1], pa_b[0], pa_b[1], pa_c[0], pa_c[1], p1[0], p1[1],
                    );
                    *grid.cell_mut((i - 1) as usize, (j - 1) as usize) = sc;
                    search_cell = sc;
                } else if i > 1 && j > 0 {
                    search_cell = *grid.cell((i - 2) as usize, (j - 1) as usize);
                } else if i > 0 && j > 1 {
                    search_cell = *grid.cell((i - 1) as usize, (j - 2) as usize);
                } else {
                    search_cell = base_cell;
                }

                // Small search radius — large displacements usually mean a
                // false alignment-pattern match.
                let mut found: QrPoint = [0; 2];
                qr_alignment_pattern_search(&mut found, &search_cell, u_idx, v_idx, 2, img, width, height);
                pa[kk] = found;
                if i > 0 && j > 0 {
                    let qa = q[kk - nalign_us - 1];
                    let qb = q[kk - nalign_us];
                    let qc = q[kk - 1];
                    let qd = q[kk];
                    let pa_a = pa[kk - nalign_us - 1];
                    let pa_b = pa[kk - nalign_us];
                    let pa_c = pa[kk - 1];
                    let pa_d = pa[kk];
                    let mut c = QrHomCell::zero();
                    qr_hom_cell_init(
                        &mut c,
                        qa[0], qa[1], qb[0], qb[1], qc[0], qc[1], qd[0], qd[1],
                        pa_a[0], pa_a[1], pa_b[0], pa_b[1], pa_c[0], pa_c[1], pa_d[0], pa_d[1],
                    );
                    *grid.cell_mut((i - 1) as usize, (j - 1) as usize) = c;
                }
            }
        }
    }

    // Cell limits — boundary u/v values between cells.
    // C: memcpy(cell_limits, align_pos+1, (ncells-1)*sizeof(*cell_limits));
    for k in 0..ncells.saturating_sub(1) {
        grid.cell_limits[k] = align_pos[k + 1];
    }
    grid.cell_limits[ncells - 1] = dim;

    // Bounding quadrilateral of the code in image space.
    let last_cell = grid.ncells - 1;
    {
        let cell = grid.cell(0, 0);
        qr_hom_cell_project(&mut p[0], cell, -1, -1, 1);
    }
    {
        let cell = grid.cell(0, last_cell);
        qr_hom_cell_project(&mut p[1], cell, (dim << 1) - 1, -1, 1);
    }
    {
        let cell = grid.cell(last_cell, 0);
        qr_hom_cell_project(&mut p[2], cell, -1, (dim << 1) - 1, 1);
    }
    {
        let cell = grid.cell(last_cell, last_cell);
        qr_hom_cell_project(&mut p[3], cell, (dim << 1) - 1, (dim << 1) - 1, 1);
    }
    // Clamp to a generous box around the image — handles corners near infinity.
    for i in 0..4 {
        p[i][0] = qr_clampi(
            -width << QR_FINDER_SUBPREC,
            p[i][0],
            width << (QR_FINDER_SUBPREC + 1),
        );
        p[i][1] = qr_clampi(
            -height << QR_FINDER_SUBPREC,
            p[i][1],
            height << (QR_FINDER_SUBPREC + 1),
        );
    }
}

/// Drop the grid's heap allocations. With Rust's Drop, this is implicit —
/// kept as an explicit reset for symmetry with zbar.
pub fn qr_sampling_grid_clear(grid: &mut QrSamplingGrid) {
    grid.cells.clear();
    grid.fpmask.clear();
    grid.ncells = 0;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fpmask_stride_for_small_dims() {
        // dim=21 (v1): 21 bits per column → 1 word. Stride = (21 + 31) >> 5 = 1.
        assert_eq!(fpmask_stride(21), 1);
        // dim=33 (v4): 33 bits → 2 words. Stride = (33 + 31) >> 5 = 2.
        assert_eq!(fpmask_stride(33), 2);
        // dim=177 (v40): 177 → 6 words.
        assert_eq!(fpmask_stride(177), 6);
    }

    #[test]
    fn fp_mask_round_trip() {
        let mut grid = QrSamplingGrid::new();
        let dim = 21;
        let stride = fpmask_stride(dim);
        grid.fpmask = vec![0u32; dim as usize * stride];
        // Mark a small block.
        qr_sampling_grid_fp_mask_rect(&mut grid, dim, 3, 5, 4, 3);
        // Inside the rect → in FP.
        assert!(qr_sampling_grid_is_in_fp(&grid, dim, 3, 5));
        assert!(qr_sampling_grid_is_in_fp(&grid, dim, 6, 7));
        // Edge-just-outside.
        assert!(!qr_sampling_grid_is_in_fp(&grid, dim, 7, 5));
        assert!(!qr_sampling_grid_is_in_fp(&grid, dim, 3, 4));
        assert!(!qr_sampling_grid_is_in_fp(&grid, dim, 3, 8));
    }
}
