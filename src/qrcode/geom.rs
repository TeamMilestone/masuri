//! QR decoder geometry — points, lines, affine + homography transforms.
//! Rust port of the geometric helpers in zbar/qrcode/qrdec.c (Phase 4-G).
//! Original Copyright (C) 2008-2009 Timothy B. Terriberry.
//! LGPL-2.1-or-later.
//!
//! All quantities are integer; the algorithms run entirely in fixed-point so
//! that intermediate values can be diffed against the zbar C reference build
//! when verifying Phase 4-F/4-D.

use super::util::{qr_divround, qr_extmul, qr_fixmul, qr_flipsigni, qr_ihypot, qr_ilog, qr_maxi};

/// Width of `int` on the targets zbar was tuned for. Used for headroom math.
pub const QR_INT_BITS: i32 = 32;
/// Subpixel precision shared with the image scanner (see `qrcode.h`).
pub const QR_FINDER_SUBPREC: i32 = 2;
/// Subpixel precision used for alignment-pattern sampling.
pub const QR_ALIGN_SUBPREC: i32 = 2;
/// Bit precision used for the homography output square. 14 bits keeps the
/// ideal module size for v40 codes ≥ 2 apart from v39's.
pub const QR_HOM_BITS: i32 = 14;

/// A 2-D integer point. Mirrors `typedef int qr_point[2]` in `qrcode.h`.
pub type QrPoint = [i32; 2];
/// A line `Ax + By + C = 0`. Mirrors `typedef int qr_line[3]` in qrdec.c.
pub type QrLine = [i32; 3];

// ─── Point ────────────────────────────────────────────────────────────────

#[inline]
pub fn qr_point_translate(p: &mut QrPoint, dx: i32, dy: i32) {
    p[0] = p[0].wrapping_add(dx);
    p[1] = p[1].wrapping_add(dy);
}

/// Squared Euclidean distance, returned as u32 (overflow wraps — matches C).
#[inline]
pub fn qr_point_distance2(p1: &QrPoint, p2: &QrPoint) -> u32 {
    let dx = p1[0].wrapping_sub(p2[0]);
    let dy = p1[1].wrapping_sub(p2[1]);
    (dx.wrapping_mul(dx) as u32).wrapping_add(dy.wrapping_mul(dy) as u32)
}

/// Cross-product of three points — positive when CCW in a right-handed
/// coordinate system, 0 when colinear.
#[inline]
pub fn qr_point_ccw(p0: &QrPoint, p1: &QrPoint, p2: &QrPoint) -> i32 {
    p1[0].wrapping_sub(p0[0]).wrapping_mul(p2[1].wrapping_sub(p0[1]))
        .wrapping_sub(p1[1].wrapping_sub(p0[1]).wrapping_mul(p2[0].wrapping_sub(p0[0])))
}

// ─── Line ─────────────────────────────────────────────────────────────────

#[inline]
pub fn qr_line_eval(line: &QrLine, x: i32, y: i32) -> i32 {
    line[0].wrapping_mul(x).wrapping_add(line[1].wrapping_mul(y)).wrapping_add(line[2])
}

/// Least-squares line passing through `(x0, y0)` given second-order moments.
/// `res` bounds the bit width of `l[0]*l[1]` so intersection math is safe.
pub fn qr_line_fit(l: &mut QrLine, x0: i32, y0: i32, sxx: i32, sxy: i32, syy: i32, res: i32) {
    let u = sxx.wrapping_sub(syy).abs();
    let v = (-sxy).wrapping_shl(1);
    let w = qr_ihypot(u, v) as i32;
    // C: dshift = max(0, max(ilog(u), ilog(|v|)) + 1 - ((res+1)>>1))
    let dshift = qr_maxi(0, qr_maxi(qr_ilog(u as u32), qr_ilog(v.unsigned_abs())) + 1 - ((res + 1) >> 1));
    let dround = (1i32 << dshift) >> 1;
    if sxx > syy {
        l[0] = v.wrapping_add(dround) >> dshift;
        l[1] = u.wrapping_add(w).wrapping_add(dround) >> dshift;
    } else {
        l[0] = u.wrapping_add(w).wrapping_add(dround) >> dshift;
        l[1] = v.wrapping_add(dround) >> dshift;
    }
    l[2] = -(x0.wrapping_mul(l[0]).wrapping_add(y0.wrapping_mul(l[1])));
}

/// Least-squares line through a list of points (≥2 points required).
pub fn qr_line_fit_points(l: &mut QrLine, p: &[QrPoint], res: i32) {
    let np = p.len() as i32;
    let mut sx = 0i32;
    let mut sy = 0i32;
    let mut xmin = i32::MAX;
    let mut xmax = i32::MIN;
    let mut ymin = i32::MAX;
    let mut ymax = i32::MIN;
    for pi in p {
        sx = sx.wrapping_add(pi[0]);
        xmin = xmin.min(pi[0]);
        xmax = xmax.max(pi[0]);
        sy = sy.wrapping_add(pi[1]);
        ymin = ymin.min(pi[1]);
        ymax = ymax.max(pi[1]);
    }
    let xbar = (sx + (np >> 1)) / np;
    let ybar = (sy + (np >> 1)) / np;
    let max_d = (xmax - xbar).max(xbar - xmin).max(ymax - ybar).max(ybar - ymin);
    let sshift = qr_maxi(0, qr_ilog((np * max_d) as u32) - ((QR_INT_BITS - 1) >> 1));
    let sround = (1i32 << sshift) >> 1;
    let mut sxx = 0i32;
    let mut sxy = 0i32;
    let mut syy = 0i32;
    for pi in p {
        // C: dx = (p[i][0] - xbar + sround) >> sshift
        let dx = pi[0].wrapping_sub(xbar).wrapping_add(sround) >> sshift;
        let dy = pi[1].wrapping_sub(ybar).wrapping_add(sround) >> sshift;
        sxx = sxx.wrapping_add(dx.wrapping_mul(dx));
        sxy = sxy.wrapping_add(dx.wrapping_mul(dy));
        syy = syy.wrapping_add(dy.wrapping_mul(dy));
    }
    qr_line_fit(l, xbar, ybar, sxx, sxy, syy, res);
}

/// Flip line direction so `(x, y)` lies in the half-plane `Ax+By+C ≥ 0`.
#[inline]
pub fn qr_line_orient(l: &mut QrLine, x: i32, y: i32) {
    if qr_line_eval(l, x, y) < 0 {
        l[0] = -l[0];
        l[1] = -l[1];
        l[2] = -l[2];
    }
}

/// Intersect two lines. Returns `Err(-1)` when parallel. On success writes
/// the (rounded) intersection point to `p`.
pub fn qr_line_isect(p: &mut QrPoint, l0: &QrLine, l1: &QrLine) -> i32 {
    let mut d = l0[0].wrapping_mul(l1[1]).wrapping_sub(l0[1].wrapping_mul(l1[0]));
    if d == 0 {
        return -1;
    }
    let mut x = l0[1].wrapping_mul(l1[2]).wrapping_sub(l1[1].wrapping_mul(l0[2]));
    let mut y = l1[0].wrapping_mul(l0[2]).wrapping_sub(l0[0].wrapping_mul(l1[2]));
    if d < 0 {
        x = -x; y = -y; d = -d;
    }
    p[0] = qr_divround(x, d);
    p[1] = qr_divround(y, d);
    0
}

// ─── Affine ───────────────────────────────────────────────────────────────

/// Affine homography: maps the image (at subpel resolution) to a square
/// domain with power-of-two sides (`1 << res`) and back.
#[derive(Clone, Copy, Debug)]
pub struct QrAff {
    pub fwd: [[i32; 2]; 2],
    pub inv: [[i32; 2]; 2],
    pub x0: i32,
    pub y0: i32,
    pub res: i32,
}

impl QrAff {
    pub fn zero() -> Self {
        QrAff { fwd: [[0; 2]; 2], inv: [[0; 2]; 2], x0: 0, y0: 0, res: 0 }
    }
}

/// Initialize from three corner points + the desired square-domain bit width.
/// Caller must arrange `p0`, `p1`, `p2` so the determinant is positive.
pub fn qr_aff_init(aff: &mut QrAff, p0: &QrPoint, p1: &QrPoint, p2: &QrPoint, res: i32) {
    let det = qr_point_ccw(p0, p1, p2);
    let dx1 = p1[0].wrapping_sub(p0[0]);
    let dx2 = p2[0].wrapping_sub(p0[0]);
    let dy1 = p1[1].wrapping_sub(p0[1]);
    let dy2 = p2[1].wrapping_sub(p0[1]);
    aff.fwd[0][0] = dx1;
    aff.fwd[0][1] = dx2;
    aff.fwd[1][0] = dy1;
    aff.fwd[1][1] = dy2;
    aff.inv[0][0] = qr_divround(dy2.wrapping_shl(res as u32), det);
    aff.inv[0][1] = qr_divround((-dx2).wrapping_shl(res as u32), det);
    aff.inv[1][0] = qr_divround((-dy1).wrapping_shl(res as u32), det);
    aff.inv[1][1] = qr_divround(dx1.wrapping_shl(res as u32), det);
    aff.x0 = p0[0];
    aff.y0 = p0[1];
    aff.res = res;
}

/// Map from image space into the square domain.
#[inline]
pub fn qr_aff_unproject(q: &mut QrPoint, aff: &QrAff, x: i32, y: i32) {
    let dx = x.wrapping_sub(aff.x0);
    let dy = y.wrapping_sub(aff.y0);
    q[0] = aff.inv[0][0].wrapping_mul(dx).wrapping_add(aff.inv[0][1].wrapping_mul(dy));
    q[1] = aff.inv[1][0].wrapping_mul(dx).wrapping_add(aff.inv[1][1].wrapping_mul(dy));
}

/// Map from the square domain into image space (with sub-pixel rounding).
#[inline]
pub fn qr_aff_project(p: &mut QrPoint, aff: &QrAff, u: i32, v: i32) {
    let round = 1i32 << (aff.res - 1);
    let x_num = aff.fwd[0][0].wrapping_mul(u)
        .wrapping_add(aff.fwd[0][1].wrapping_mul(v))
        .wrapping_add(round);
    let y_num = aff.fwd[1][0].wrapping_mul(u)
        .wrapping_add(aff.fwd[1][1].wrapping_mul(v))
        .wrapping_add(round);
    p[0] = (x_num >> aff.res).wrapping_add(aff.x0);
    p[1] = (y_num >> aff.res).wrapping_add(aff.y0);
}

// ─── Homography ───────────────────────────────────────────────────────────

/// Full projective homography from image-space to a square `1<<res` domain.
#[derive(Clone, Copy, Debug)]
pub struct QrHom {
    pub fwd: [[i32; 2]; 3],
    pub inv: [[i32; 2]; 3],
    pub fwd22: i32,
    pub inv22: i32,
    pub x0: i32,
    pub y0: i32,
    pub res: i32,
}

impl QrHom {
    pub fn zero() -> Self {
        QrHom { fwd: [[0; 2]; 3], inv: [[0; 2]; 3], fwd22: 0, inv22: 0, x0: 0, y0: 0, res: 0 }
    }
}

/// Initialize a homography from four corner correspondences.
/// Maps the unit square `(0,0)→(0,0)`, `(1<<res, 0)→(x1,y1)`,
/// `(0, 1<<res)→(x2,y2)`, `(1<<res, 1<<res)→(x3,y3)`.
#[allow(clippy::too_many_arguments)]
pub fn qr_hom_init(
    hom: &mut QrHom,
    x0: i32, y0: i32,
    x1: i32, y1: i32,
    x2: i32, y2: i32,
    x3: i32, y3: i32,
    res: i32,
) {
    let dx10 = x1.wrapping_sub(x0);
    let dx20 = x2.wrapping_sub(x0);
    let dx30 = x3.wrapping_sub(x0);
    let dx31 = x3.wrapping_sub(x1);
    let dx32 = x3.wrapping_sub(x2);
    let dy10 = y1.wrapping_sub(y0);
    let dy20 = y2.wrapping_sub(y0);
    let dy30 = y3.wrapping_sub(y0);
    let dy31 = y3.wrapping_sub(y1);
    let dy32 = y3.wrapping_sub(y2);
    let a20 = dx32.wrapping_mul(dy10).wrapping_sub(dx10.wrapping_mul(dy32));
    let a21 = dx20.wrapping_mul(dy31).wrapping_sub(dx31.wrapping_mul(dy20));
    let a22 = dx32.wrapping_mul(dy31).wrapping_sub(dx31.wrapping_mul(dy32));

    let b0 = qr_ilog(qr_maxi(dx10.abs(), dy10.abs()) as u32)
        + qr_ilog(a20.wrapping_add(a22).unsigned_abs());
    let b1 = qr_ilog(qr_maxi(dx20.abs(), dy20.abs()) as u32)
        + qr_ilog(a21.wrapping_add(a22).unsigned_abs());
    let b2 = qr_ilog(qr_maxi(qr_maxi(a20.abs(), a21.abs()), a22.abs()) as u32);
    let s1 = qr_maxi(0, res + qr_maxi(qr_maxi(b0, b1), b2) - (QR_INT_BITS - 2));
    let r1 = (1i32 << s1) >> 1;

    hom.fwd[0][0] = qr_fixmul(dx10, a20.wrapping_add(a22), r1 as i64, s1 as u32);
    hom.fwd[0][1] = qr_fixmul(dx20, a21.wrapping_add(a22), r1 as i64, s1 as u32);
    hom.x0 = x0;
    hom.fwd[1][0] = qr_fixmul(dy10, a20.wrapping_add(a22), r1 as i64, s1 as u32);
    hom.fwd[1][1] = qr_fixmul(dy20, a21.wrapping_add(a22), r1 as i64, s1 as u32);
    hom.y0 = y0;
    hom.fwd[2][0] = a20.wrapping_add(r1) >> s1;
    hom.fwd[2][1] = a21.wrapping_add(r1) >> s1;
    // C: fwd22 = s1>res ? (a22 + (r1>>res)) >> (s1-res) : a22 << (res-s1)
    hom.fwd22 = if s1 > res {
        (a22.wrapping_add(r1 >> res)) >> (s1 - res)
    } else {
        a22.wrapping_shl((res - s1) as u32)
    };

    let b0 = qr_ilog(qr_maxi(qr_maxi(dx10.abs(), dx20.abs()), dx30.abs()) as u32)
        + qr_ilog(qr_maxi(hom.fwd[0][0].abs(), hom.fwd[1][0].abs()) as u32);
    let b1 = qr_ilog(qr_maxi(qr_maxi(dy10.abs(), dy20.abs()), dy30.abs()) as u32)
        + qr_ilog(qr_maxi(hom.fwd[0][1].abs(), hom.fwd[1][1].abs()) as u32);
    let b2 = qr_ilog(a22.abs() as u32) - s1;
    let s2 = qr_maxi(0, qr_maxi(b0, b1) + b2 - (QR_INT_BITS - 3));
    let r2 = (1i32 << s2) >> 1;
    let s1 = s1 + s2;
    let r1 = (r1 as i64).wrapping_shl(s2 as u32);

    hom.inv[0][0] = qr_fixmul(hom.fwd[1][1], a22, r1, s1 as u32);
    hom.inv[0][1] = qr_fixmul(-hom.fwd[0][1], a22, r1, s1 as u32);
    hom.inv[1][0] = qr_fixmul(-hom.fwd[1][0], a22, r1, s1 as u32);
    hom.inv[1][1] = qr_fixmul(hom.fwd[0][0], a22, r1, s1 as u32);
    hom.inv[2][0] = qr_fixmul(
        hom.fwd[1][0], hom.fwd[2][1],
        -qr_extmul(hom.fwd[1][1], hom.fwd[2][0], r2 as i64), s2 as u32);
    hom.inv[2][1] = qr_fixmul(
        hom.fwd[0][1], hom.fwd[2][0],
        -qr_extmul(hom.fwd[0][0], hom.fwd[2][1], r2 as i64), s2 as u32);
    hom.inv22 = qr_fixmul(
        hom.fwd[0][0], hom.fwd[1][1],
        -qr_extmul(hom.fwd[0][1], hom.fwd[1][0], r2 as i64), s2 as u32);
    hom.res = res;
}

/// Map an image-space point to the square domain. Returns `-1` if the
/// projection sent the point to infinity (`w == 0`), in which case `q` is
/// filled with the appropriate signed infinity.
pub fn qr_hom_unproject(q: &mut QrPoint, hom: &QrHom, x: i32, y: i32) -> i32 {
    let dx = x.wrapping_sub(hom.x0);
    let dy = y.wrapping_sub(hom.y0);
    let xi = hom.inv[0][0].wrapping_mul(dx).wrapping_add(hom.inv[0][1].wrapping_mul(dy));
    let yi = hom.inv[1][0].wrapping_mul(dx).wrapping_add(hom.inv[1][1].wrapping_mul(dy));
    let round = 1i32 << (hom.res - 1);
    let w_num = hom.inv[2][0].wrapping_mul(dx)
        .wrapping_add(hom.inv[2][1].wrapping_mul(dy))
        .wrapping_add(hom.inv22)
        .wrapping_add(round);
    let w = w_num >> hom.res;
    if w == 0 {
        q[0] = if xi < 0 { i32::MIN } else { i32::MAX };
        q[1] = if yi < 0 { i32::MIN } else { i32::MAX };
        -1
    } else {
        let (x_use, y_use, w_use) = if w < 0 { (-xi, -yi, -w) } else { (xi, yi, w) };
        q[0] = qr_divround(x_use, w_use);
        q[1] = qr_divround(y_use, w_use);
        0
    }
}

/// Finish a partial projection (`x`, `y`, `w` already computed in
/// homogeneous coords) — divide and re-add the origin.
pub fn qr_hom_fproject(p: &mut QrPoint, hom: &QrHom, x: i32, y: i32, w: i32) {
    if w == 0 {
        p[0] = if x < 0 { i32::MIN } else { i32::MAX };
        p[1] = if y < 0 { i32::MIN } else { i32::MAX };
    } else {
        let (x_use, y_use, w_use) = if w < 0 { (-x, -y, -w) } else { (x, y, w) };
        p[0] = qr_divround(x_use, w_use).wrapping_add(hom.x0);
        p[1] = qr_divround(y_use, w_use).wrapping_add(hom.y0);
    }
}

/// Map from the square domain back into image space. (Debug-only in zbar.)
pub fn qr_hom_project(p: &mut QrPoint, hom: &QrHom, u: i32, v: i32) {
    qr_hom_fproject(
        p, hom,
        hom.fwd[0][0].wrapping_mul(u).wrapping_add(hom.fwd[0][1].wrapping_mul(v)),
        hom.fwd[1][0].wrapping_mul(u).wrapping_add(hom.fwd[1][1].wrapping_mul(v)),
        hom.fwd[2][0].wrapping_mul(u).wrapping_add(hom.fwd[2][1].wrapping_mul(v)).wrapping_add(hom.fwd22),
    );
}

// ─── Homography cell ──────────────────────────────────────────────────────

/// A homography for one cell of the QR sampling grid. Maps grid coordinates
/// (not the unit square) directly to image space; no inverse stored.
#[derive(Clone, Copy, Debug)]
pub struct QrHomCell {
    pub fwd: [[i32; 3]; 3],
    pub x0: i32,
    pub y0: i32,
    pub u0: i32,
    pub v0: i32,
}

impl QrHomCell {
    pub fn zero() -> Self {
        QrHomCell { fwd: [[0; 3]; 3], x0: 0, y0: 0, u0: 0, v0: 0 }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn qr_hom_cell_init(
    cell: &mut QrHomCell,
    u0: i32, v0: i32, u1: i32, v1: i32, u2: i32, v2: i32, u3: i32, v3: i32,
    x0: i32, y0: i32, x1: i32, y1: i32, x2: i32, y2: i32, x3: i32, y3: i32,
) {
    let du10 = u1.wrapping_sub(u0);
    let du20 = u2.wrapping_sub(u0);
    let du30 = u3.wrapping_sub(u0);
    let du31 = u3.wrapping_sub(u1);
    let du32 = u3.wrapping_sub(u2);
    let dv10 = v1.wrapping_sub(v0);
    let dv20 = v2.wrapping_sub(v0);
    let dv30 = v3.wrapping_sub(v0);
    let dv31 = v3.wrapping_sub(v1);
    let dv32 = v3.wrapping_sub(v2);

    let a20 = du32.wrapping_mul(dv10).wrapping_sub(du10.wrapping_mul(dv32));
    let a21 = du20.wrapping_mul(dv31).wrapping_sub(du31.wrapping_mul(dv20));
    // C: if(a20||a21) a22 = du32*dv31-du31*dv32; else a22 = 1
    let a22 = if a20 != 0 || a21 != 0 {
        du32.wrapping_mul(dv31).wrapping_sub(du31.wrapping_mul(dv32))
    } else {
        1
    };
    let a00 = du10.wrapping_mul(a20.wrapping_add(a22));
    let a01 = du20.wrapping_mul(a21.wrapping_add(a22));
    let a10 = dv10.wrapping_mul(a20.wrapping_add(a22));
    let a11 = dv20.wrapping_mul(a21.wrapping_add(a22));

    let mut i00 = a11.wrapping_mul(a22);
    let mut i01 = (-a01).wrapping_mul(a22);
    let mut i10 = (-a10).wrapping_mul(a22);
    let mut i11 = a00.wrapping_mul(a22);
    let mut i20 = a10.wrapping_mul(a21).wrapping_sub(a11.wrapping_mul(a20));
    let mut i21 = a01.wrapping_mul(a20).wrapping_sub(a00.wrapping_mul(a21));
    let i22 = a00.wrapping_mul(a11).wrapping_sub(a01.wrapping_mul(a10));

    if i00 != 0 { i00 = qr_flipsigni(qr_divround(i22, i00.abs()), i00); }
    if i01 != 0 { i01 = qr_flipsigni(qr_divround(i22, i01.abs()), i01); }
    if i10 != 0 { i10 = qr_flipsigni(qr_divround(i22, i10.abs()), i10); }
    if i11 != 0 { i11 = qr_flipsigni(qr_divround(i22, i11.abs()), i11); }
    if i20 != 0 { i20 = qr_flipsigni(qr_divround(i22, i20.abs()), i20); }
    if i21 != 0 { i21 = qr_flipsigni(qr_divround(i22, i21.abs()), i21); }

    let dx10 = x1.wrapping_sub(x0);
    let dx20 = x2.wrapping_sub(x0);
    let dx30 = x3.wrapping_sub(x0);
    let dx31 = x3.wrapping_sub(x1);
    let dx32 = x3.wrapping_sub(x2);
    let dy10 = y1.wrapping_sub(y0);
    let dy20 = y2.wrapping_sub(y0);
    let dy30 = y3.wrapping_sub(y0);
    let dy31 = y3.wrapping_sub(y1);
    let dy32 = y3.wrapping_sub(y2);
    let a20_x = dx32.wrapping_mul(dy10).wrapping_sub(dx10.wrapping_mul(dy32));
    let a21_x = dx20.wrapping_mul(dy31).wrapping_sub(dx31.wrapping_mul(dy20));
    let a22_x = dx32.wrapping_mul(dy31).wrapping_sub(dx31.wrapping_mul(dy32));

    let b0 = qr_ilog(qr_maxi(dx10.abs(), dy10.abs()) as u32)
        + qr_ilog(a20_x.wrapping_add(a22_x).unsigned_abs());
    let b1 = qr_ilog(qr_maxi(dx20.abs(), dy20.abs()) as u32)
        + qr_ilog(a21_x.wrapping_add(a22_x).unsigned_abs());
    let b2 = qr_ilog(qr_maxi(qr_maxi(a20_x.abs(), a21_x.abs()), a22_x.abs()) as u32);
    let shift = qr_maxi(0, qr_maxi(qr_maxi(b0, b1), b2) - (QR_INT_BITS - 3 - QR_ALIGN_SUBPREC));
    let round = (1i32 << shift) >> 1;

    let a00 = qr_fixmul(dx10, a20_x.wrapping_add(a22_x), round as i64, shift as u32);
    let a01 = qr_fixmul(dx20, a21_x.wrapping_add(a22_x), round as i64, shift as u32);
    let a10 = qr_fixmul(dy10, a20_x.wrapping_add(a22_x), round as i64, shift as u32);
    let a11 = qr_fixmul(dy20, a21_x.wrapping_add(a22_x), round as i64, shift as u32);

    let div_or_zero = |num: i32, den: i32| -> i32 {
        if den != 0 { qr_divround(num, den) } else { 0 }
    };
    cell.fwd[0][0] = div_or_zero(a00, i00).wrapping_add(div_or_zero(a01, i10));
    cell.fwd[0][1] = div_or_zero(a00, i01).wrapping_add(div_or_zero(a01, i11));
    cell.fwd[1][0] = div_or_zero(a10, i00).wrapping_add(div_or_zero(a11, i10));
    cell.fwd[1][1] = div_or_zero(a10, i01).wrapping_add(div_or_zero(a11, i11));
    // C: fwd[2][0] = (div_a20/i00 + div_a21/i10 + div_a22/i20 + round) >> shift
    cell.fwd[2][0] = (div_or_zero(a20_x, i00)
        .wrapping_add(div_or_zero(a21_x, i10))
        .wrapping_add(div_or_zero(a22_x, i20))
        .wrapping_add(round)) >> shift;
    cell.fwd[2][1] = (div_or_zero(a20_x, i01)
        .wrapping_add(div_or_zero(a21_x, i11))
        .wrapping_add(div_or_zero(a22_x, i21))
        .wrapping_add(round)) >> shift;
    cell.fwd[2][2] = a22_x.wrapping_add(round) >> shift;

    // Distribute rounding error across the corner: compute a02, a12 from the
    // three known points' residuals.
    let compute_xyw = |du: i32, dv: i32, cell: &QrHomCell| -> (i32, i32, i32) {
        let x = cell.fwd[0][0].wrapping_mul(du).wrapping_add(cell.fwd[0][1].wrapping_mul(dv));
        let y = cell.fwd[1][0].wrapping_mul(du).wrapping_add(cell.fwd[1][1].wrapping_mul(dv));
        let w = cell.fwd[2][0].wrapping_mul(du)
            .wrapping_add(cell.fwd[2][1].wrapping_mul(dv))
            .wrapping_add(cell.fwd[2][2]);
        (x, y, w)
    };
    let (x_1, y_1, w_1) = compute_xyw(du10, dv10, cell);
    let mut a02 = dx10.wrapping_mul(w_1).wrapping_sub(x_1);
    let mut a12 = dy10.wrapping_mul(w_1).wrapping_sub(y_1);
    let (x_2, y_2, w_2) = compute_xyw(du20, dv20, cell);
    a02 = a02.wrapping_add(dx20.wrapping_mul(w_2).wrapping_sub(x_2));
    a12 = a12.wrapping_add(dy20.wrapping_mul(w_2).wrapping_sub(y_2));
    let (x_3, y_3, w_3) = compute_xyw(du30, dv30, cell);
    a02 = a02.wrapping_add(dx30.wrapping_mul(w_3).wrapping_sub(x_3));
    a12 = a12.wrapping_add(dy30.wrapping_mul(w_3).wrapping_sub(y_3));

    cell.fwd[0][2] = a02.wrapping_add(2) >> 2;
    cell.fwd[1][2] = a12.wrapping_add(2) >> 2;
    cell.x0 = x0;
    cell.y0 = y0;
    cell.u0 = u0;
    cell.v0 = v0;
}

pub fn qr_hom_cell_fproject(p: &mut QrPoint, cell: &QrHomCell, x: i32, y: i32, w: i32) {
    if w == 0 {
        p[0] = if x < 0 { i32::MIN } else { i32::MAX };
        p[1] = if y < 0 { i32::MIN } else { i32::MAX };
    } else {
        let (xu, yu, wu) = if w < 0 { (-x, -y, -w) } else { (x, y, w) };
        p[0] = qr_divround(xu, wu).wrapping_add(cell.x0);
        p[1] = qr_divround(yu, wu).wrapping_add(cell.y0);
    }
}

pub fn qr_hom_cell_project(p: &mut QrPoint, cell: &QrHomCell, mut u: i32, mut v: i32, res: i32) {
    u = u.wrapping_sub(cell.u0.wrapping_shl(res as u32));
    v = v.wrapping_sub(cell.v0.wrapping_shl(res as u32));
    qr_hom_cell_fproject(
        p, cell,
        cell.fwd[0][0].wrapping_mul(u).wrapping_add(cell.fwd[0][1].wrapping_mul(v)).wrapping_add(cell.fwd[0][2].wrapping_shl(res as u32)),
        cell.fwd[1][0].wrapping_mul(u).wrapping_add(cell.fwd[1][1].wrapping_mul(v)).wrapping_add(cell.fwd[1][2].wrapping_shl(res as u32)),
        cell.fwd[2][0].wrapping_mul(u).wrapping_add(cell.fwd[2][1].wrapping_mul(v)).wrapping_add(cell.fwd[2][2].wrapping_shl(res as u32)),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn point_translate_and_distance() {
        let mut p: QrPoint = [10, 20];
        qr_point_translate(&mut p, 3, -5);
        assert_eq!(p, [13, 15]);
        let q: QrPoint = [16, 11];
        assert_eq!(qr_point_distance2(&p, &q), 9 + 16);
    }

    #[test]
    fn point_ccw_sign() {
        let a: QrPoint = [0, 0];
        let b: QrPoint = [1, 0];
        let c: QrPoint = [0, 1];
        assert!(qr_point_ccw(&a, &b, &c) > 0);  // CCW
        assert!(qr_point_ccw(&a, &c, &b) < 0);  // CW
        let d: QrPoint = [2, 0];
        assert_eq!(qr_point_ccw(&a, &b, &d), 0); // colinear
    }

    #[test]
    fn line_eval_basic() {
        // Line x + y - 5 = 0 → coefficients [1, 1, -5]
        let l: QrLine = [1, 1, -5];
        assert_eq!(qr_line_eval(&l, 2, 3), 0);
        assert_eq!(qr_line_eval(&l, 0, 0), -5);
        assert_eq!(qr_line_eval(&l, 5, 5), 5);
    }

    #[test]
    fn line_isect_known() {
        // x = 0  (1*x + 0*y + 0 = 0) and y = 0  (0*x + 1*y + 0 = 0) → (0,0).
        let l0: QrLine = [1, 0, 0];
        let l1: QrLine = [0, 1, 0];
        let mut p: QrPoint = [-1, -1];
        assert_eq!(qr_line_isect(&mut p, &l0, &l1), 0);
        assert_eq!(p, [0, 0]);

        // Parallel lines: y = 0 and y = 5 → l1 = [0,1,0], l2 = [0,1,-5].
        let l2: QrLine = [0, 1, -5];
        let mut p: QrPoint = [99, 99];
        assert_eq!(qr_line_isect(&mut p, &l1, &l2), -1);
    }

    #[test]
    fn line_fit_points_horizontal() {
        // 5 colinear points on y=10. Fitted line should evaluate to ≈0 at each.
        let pts = [
            [0, 10], [1, 10], [2, 10], [3, 10], [4, 10],
        ];
        let mut l: QrLine = [0; 3];
        qr_line_fit_points(&mut l, &pts, 16);
        for p in &pts {
            assert!(qr_line_eval(&l, p[0], p[1]).abs() < l[0].abs().max(l[1].abs()).max(1) * 2,
                "line {:?} not satisfied by {:?}", l, p);
        }
    }

    #[test]
    fn aff_project_unproject_round_trip() {
        // Three points forming a CCW triangle in image space.
        let p0: QrPoint = [100, 200];
        let p1: QrPoint = [200, 200];
        let p2: QrPoint = [100, 300];
        let res = 14;
        let mut aff = QrAff::zero();
        qr_aff_init(&mut aff, &p0, &p1, &p2, res);
        // project(0,0) ≈ p0; project(1<<res, 0) ≈ p1; project(0, 1<<res) ≈ p2.
        let mut q: QrPoint = [0; 2];
        qr_aff_project(&mut q, &aff, 0, 0);
        assert_eq!(q, p0);
        qr_aff_project(&mut q, &aff, 1 << res, 0);
        assert!((q[0] - p1[0]).abs() <= 1 && (q[1] - p1[1]).abs() <= 1, "{:?} vs {:?}", q, p1);
        qr_aff_project(&mut q, &aff, 0, 1 << res);
        assert!((q[0] - p2[0]).abs() <= 1 && (q[1] - p2[1]).abs() <= 1, "{:?} vs {:?}", q, p2);
        // unproject(project(u,v)) ≈ (u,v).
        let mut p: QrPoint = [0; 2];
        let mut u_back: QrPoint = [0; 2];
        for &(u, v) in &[(0i32, 0i32), (1 << (res - 1), 0), (0, 1 << (res - 1)), (1 << res, 1 << res)] {
            qr_aff_project(&mut p, &aff, u, v);
            qr_aff_unproject(&mut u_back, &aff, p[0], p[1]);
            let tol = 1 << 8;
            assert!(
                (u_back[0] - u).abs() <= tol && (u_back[1] - v).abs() <= tol,
                "round trip: (u,v)=({},{}) -> p={:?} -> back={:?}",
                u, v, p, u_back
            );
        }
    }

    #[test]
    fn hom_project_unproject_round_trip() {
        // Slightly perspective-distorted unit square in image space.
        let res = 14;
        let mut hom = QrHom::zero();
        qr_hom_init(&mut hom,
            100, 100,
            500, 110,
            105, 500,
            520, 530,
            res);
        // Corners must round-trip exactly.
        let cases = [
            (0i32, 0i32, [100, 100]),
            (1 << res, 0, [500, 110]),
            (0, 1 << res, [105, 500]),
        ];
        for (u, v, expected) in cases {
            let mut p: QrPoint = [0; 2];
            qr_hom_project(&mut p, &hom, u, v);
            assert!((p[0] - expected[0]).abs() <= 2 && (p[1] - expected[1]).abs() <= 2,
                "project corner ({},{}): got {:?}, expected ≈ {:?}", u, v, p, expected);
        }
        // Image-space point in the middle should round-trip.
        let mut q: QrPoint = [0; 2];
        assert_eq!(qr_hom_unproject(&mut q, &hom, 300, 300), 0);
        let mut p_back: QrPoint = [0; 2];
        qr_hom_project(&mut p_back, &hom, q[0], q[1]);
        assert!((p_back[0] - 300).abs() <= 4 && (p_back[1] - 300).abs() <= 4,
            "hom round-trip: image (300,300) -> q={:?} -> back={:?}", q, p_back);
    }

    #[test]
    fn hom_cell_init_identity_grid() {
        // Map grid points (0..1, 0..1) onto an axis-aligned square in image space.
        let mut cell = QrHomCell::zero();
        qr_hom_cell_init(&mut cell,
            0, 0, 1, 0, 0, 1, 1, 1,
            100, 200, 200, 200, 100, 300, 200, 300);
        // project the four corners back at res=0.
        let cases: [(i32, i32, QrPoint); 4] = [
            (0, 0, [100, 200]),
            (1, 0, [200, 200]),
            (0, 1, [100, 300]),
            (1, 1, [200, 300]),
        ];
        for (u, v, expected) in cases {
            let mut p: QrPoint = [0; 2];
            qr_hom_cell_project(&mut p, &cell, u, v, 0);
            assert!((p[0] - expected[0]).abs() <= 2 && (p[1] - expected[1]).abs() <= 2,
                "cell project ({},{}): got {:?}, expected {:?}", u, v, p, expected);
        }
    }
}
