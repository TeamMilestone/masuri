//! Reed-Solomon encoder/decoder over GF(2⁸) for QR data correction.
//! Rust port of zbar/qrcode/rs.{c,h}.
//! Original Copyright (C) 1991-1995 Henry Minsky (Universal Access).
//! Updates Copyright (C) 2008-2009 Timothy B. Terriberry.
//! LGPL-2.1-or-later.
//!
//! Faithful 1:1 port — same memory layout, same algorithm
//! (Berlekamp-Massey + Chien search with explicit quartic/cubic/quadratic
//! root finders for low-degree polynomials).

use super::util::qr_signmask;

/// QR's irreducible primitive polynomial: x⁸ + x⁴ + x³ + x² + 1.
pub const QR_PPOLY: u32 = 0x1D;
/// Index to start the generator polynomial from (0..=254).
pub const QR_M0: i32 = 0;

/// Log/exp tables for GF(2⁸). `exp` is extended by 256 entries so callers can
/// add two logs (each in 0..255) and index without taking a mod.
pub struct RsGf256 {
    pub log: [u8; 256],
    pub exp: [u8; 511],
}

impl RsGf256 {
    pub fn new() -> Self {
        let mut gf = RsGf256 { log: [0; 256], exp: [0; 511] };
        rs_gf256_init(&mut gf, QR_PPOLY);
        gf
    }
}

/// Build the discrete-log tables for GF(2⁸) from a primitive irreducible
/// polynomial. For QR, pass `QR_PPOLY` (0x1D).
pub fn rs_gf256_init(gf: &mut RsGf256, ppoly: u32) {
    let mut p: u32 = 1;
    for i in 0..256 {
        gf.exp[i] = p as u8;
        gf.exp[i + 255] = p as u8;
        // C: p = ((p<<1) ^ (-(p>>7) & ppoly)) & 0xFF
        let top = (p >> 7) & 1;
        let mask = top.wrapping_neg() & ppoly;
        p = ((p << 1) ^ mask) & 0xFF;
    }
    for i in 0..255 {
        gf.log[gf.exp[i] as usize] = i as u8;
    }
    gf.log[0] = 0; // sentinel — callers guard against a == 0
}

#[inline]
fn rs_gmul(gf: &RsGf256, a: u32, b: u32) -> u32 {
    if a == 0 || b == 0 { 0 }
    else { gf.exp[gf.log[a as usize] as usize + gf.log[b as usize] as usize] as u32 }
}

#[inline]
fn rs_gdiv(gf: &RsGf256, a: u32, b: u32) -> u32 {
    if a == 0 { 0 }
    else { gf.exp[gf.log[a as usize] as usize + 255 - gf.log[b as usize] as usize] as u32 }
}

#[inline]
fn rs_hgmul(gf: &RsGf256, a: u32, logb: u32) -> u32 {
    if a == 0 { 0 }
    else { gf.exp[gf.log[a as usize] as usize + logb as usize] as u32 }
}

#[inline]
fn rs_gsqrt(gf: &RsGf256, a: u32) -> u32 {
    if a == 0 { return 0; }
    let loga = gf.log[a as usize] as u32;
    // C: exp[(loga + (255 & -(loga & 1))) >> 1]
    let _ = qr_signmask; // not strictly needed here
    let adj = 255 & (loga & 1).wrapping_neg();
    gf.exp[((loga + adj) >> 1) as usize] as u32
}

/// Solve `x² + b x + c = 0` in GF(2⁸); writes distinct roots to `x` and
/// returns the root count (0, 1, or 2).
fn rs_quadratic_solve(gf: &RsGf256, b_in: u32, c_in: u32, x: &mut [u8]) -> i32 {
    let _b = b_in;
    let mut _c = c_in;
    if _b == 0 {
        x[0] = rs_gsqrt(gf, _c) as u8;
        return 1;
    }
    if _c == 0 {
        x[0] = 0;
        x[1] = _b as u8;
        return 2;
    }
    let mut logb = gf.log[_b as usize] as u32;
    let mut logc = gf.log[_c as usize] as u32;
    // Scale if b lies in GF(2⁴): logb % (255/15) == 0
    let inc = logb % (255 / 15) == 0;
    let b;
    if inc {
        b = gf.exp[(logb + 254) as usize] as u32;
        logb = gf.log[b as usize] as u32;
        _c = gf.exp[(logc + 253) as usize] as u32;
        logc = gf.log[_c as usize] as u32;
    } else {
        b = _b;
    }
    let logb2 = gf.log[gf.exp[(logb << 1) as usize] as usize] as u32;
    let logb4 = gf.log[gf.exp[(logb2 << 1) as usize] as usize] as u32;
    let logb8 = gf.log[gf.exp[(logb4 << 1) as usize] as usize] as u32;
    let logb12 = gf.log[gf.exp[(logb4 + logb8) as usize] as usize] as u32;
    let logb14 = gf.log[gf.exp[(logb2 + logb12) as usize] as usize] as u32;
    let logc2 = gf.log[gf.exp[(logc << 1) as usize] as usize] as u32;
    let logc4 = gf.log[gf.exp[(logc2 << 1) as usize] as usize] as u32;
    let c8 = gf.exp[(logc4 << 1) as usize] as u32;
    let arg = gf.exp[(logb14 + logc) as usize] as u32
        ^ gf.exp[(logb12 + logc2) as usize] as u32
        ^ gf.exp[(logb8 + logc4) as usize] as u32
        ^ c8;
    let g3 = rs_hgmul(gf, arg, logb);
    if gf.log[g3 as usize] as u32 % (255 / 15) != 0 {
        return 0;
    }
    let z3 = rs_gdiv(gf, g3, gf.exp[(logb8 << 1) as usize] as u32 ^ b);
    let l3 = rs_hgmul(
        gf,
        rs_gmul(gf, z3, z3) ^ rs_hgmul(gf, z3, logb) ^ _c,
        255 - logb2,
    );
    let c0 = rs_hgmul(gf, l3, 255 - 2 * (255 / 15));
    let g2 = rs_hgmul(
        gf,
        rs_hgmul(gf, c0, 255 - 2 * (255 / 15)) ^ rs_gmul(gf, c0, c0),
        255 - 255 / 15,
    );
    let z2 = rs_gdiv(
        gf,
        g2,
        gf.exp[(255 - (255 / 15) * 4) as usize] as u32 ^ gf.exp[(255 - 255 / 15) as usize] as u32,
    );
    let l2 = rs_hgmul(
        gf,
        rs_gmul(gf, z2, z2) ^ rs_hgmul(gf, z2, 255 - (255 / 15)) ^ c0,
        2 * (255 / 15),
    );
    let inc_offset: u32 = if inc { 1 } else { 0 };
    let sum = z3
        ^ rs_hgmul(gf, rs_hgmul(gf, l2, 255 / 3) ^ rs_hgmul(gf, z2, 255 / 15), logb);
    x[0] = gf.exp[(gf.log[sum as usize] as u32 + inc_offset) as usize];
    x[1] = x[0] ^ _b as u8;
    2
}

/// Solve `x³ + a x² + b x + c = 0` in GF(2⁸). Returns root count.
fn rs_cubic_solve(gf: &RsGf256, _a: u32, _b: u32, _c: u32, x: &mut [u8]) -> i32 {
    if _c == 0 {
        let mut nroots = rs_quadratic_solve(gf, _a, _b, x);
        if _b != 0 {
            x[nroots as usize] = 0;
            nroots += 1;
        }
        return nroots;
    }
    let mut k = rs_gmul(gf, _a, _b) ^ _c;
    let d2 = rs_gmul(gf, _a, _a) ^ _b;
    if d2 == 0 {
        if k == 0 {
            x[0] = _a as u8;
            return 1;
        }
        let mut logx = gf.log[k as usize] as u32;
        if logx % 3 != 0 { return 0; }
        logx /= 3;
        x[0] = _a as u8 ^ gf.exp[logx as usize];
        x[1] = _a as u8 ^ gf.exp[(logx + 255 / 3) as usize];
        x[2] = _a as u8 ^ x[0] ^ x[1];
        return 3;
    }
    let logd2 = gf.log[d2 as usize] as u32;
    // logd = (logd2 + (255 & -(logd2 & 1))) >> 1
    let adj = 255 & (logd2 & 1).wrapping_neg();
    let logd = (logd2 + adj) >> 1;
    k = rs_gdiv(gf, k, gf.exp[(logd + logd2) as usize] as u32);
    // Substitute y = w + 1/w, z = w³ ⇒ z² + k*z + 1 = 0
    let nroots = rs_quadratic_solve(gf, k, 1, x);
    if nroots < 1 {
        return 0;
    }
    let mut logw = gf.log[x[0] as usize] as u32;
    if logw != 0 {
        if logw % 3 != 0 { return 0; }
        logw /= 3;
        x[0] = gf.exp[(gf.log[(gf.exp[logw as usize] ^ gf.exp[(255 - logw) as usize]) as usize] as u32 + logd) as usize] ^ _a as u8;
        logw += 255 / 3;
        x[1] = gf.exp[(gf.log[(gf.exp[logw as usize] ^ gf.exp[(255 - logw) as usize]) as usize] as u32 + logd) as usize] ^ _a as u8;
        x[2] = x[0] ^ x[1] ^ _a as u8;
        3
    } else {
        x[0] = _a as u8;
        1
    }
}

/// Solve `x⁴ + a x³ + b x² + c x + d = 0` in GF(2⁸). Returns root count.
fn rs_quartic_solve(gf: &RsGf256, _a: u32, _b: u32, _c: u32, _d: u32, x: &mut [u8]) -> i32 {
    if _d == 0 {
        let mut nroots = rs_cubic_solve(gf, _a, _b, _c, x);
        if _c != 0 {
            x[nroots as usize] = 0;
            nroots += 1;
        }
        return nroots;
    }
    if _a != 0 {
        let loga = gf.log[_a as usize] as u32;
        let r = rs_hgmul(gf, _c, 255 - loga);
        let s = rs_gsqrt(gf, r);
        let t = _d ^ rs_gmul(gf, _b, r) ^ rs_gmul(gf, r, r);
        let nroots;
        if t != 0 {
            let logti = 255 - gf.log[t as usize] as u32;
            let arg2 = rs_hgmul(gf, _b ^ rs_hgmul(gf, s, loga), logti);
            let arg3 = gf.exp[(loga + logti) as usize] as u32;
            let arg4 = gf.exp[logti as usize] as u32;
            nroots = rs_quartic_solve(gf, 0, arg2, arg3, arg4, x);
            for i in 0..nroots as usize {
                x[i] = gf.exp[(255 - gf.log[x[i] as usize] as u32) as usize] ^ s as u8;
            }
        } else {
            // s is a double root; only the quadratic remains.
            let mut n = rs_quadratic_solve(gf, _a, _b ^ r, x);
            // s may be a triple root if s = b/a but not quadruple (a != 0).
            if !(n == 2 && (x[0] as u32 == s || x[1] as u32 == s)) {
                x[n as usize] = s as u8;
                n += 1;
            }
            nroots = n;
        }
        return nroots;
    }
    // a == 0 path
    if _c == 0 {
        return rs_quadratic_solve(gf, rs_gsqrt(gf, _b), rs_gsqrt(gf, _d), x);
    }
    // Factor into (x² + r x + s)(x² + r x + t) via r³ + b r + c = 0.
    let nroots = rs_cubic_solve(gf, 0, _b, _c, x);
    if nroots < 1 { return 0; }
    let r = x[0] as u32;
    let b = rs_gdiv(gf, _c, r);
    let nr2 = rs_quadratic_solve(gf, b, _d, x);
    if nr2 < 2 { return 0; }
    let s = x[0] as u32;
    let t = x[1] as u32;
    let n1 = rs_quadratic_solve(gf, r, s, x);
    let (head, tail) = x.split_at_mut(n1 as usize);
    let n2 = rs_quadratic_solve(gf, r, t, tail);
    let _ = head;
    n1 + n2
}

#[inline]
fn rs_poly_zero(p: &mut [u8], dp1: usize) {
    for v in &mut p[..dp1] { *v = 0; }
}

#[inline]
fn rs_poly_copy(p: &mut [u8], q: &[u8], dp1: usize) {
    p[..dp1].copy_from_slice(&q[..dp1]);
}

/// p = q * x — coefficient shift up by one. `dp1 > 0`.
fn rs_poly_mul_x(p: &mut [u8], dp1: usize) {
    // memmove(p+1, q, dp1-1); p[0] = 0
    // Since zbar always uses p == q at call sites, we accept aliased operation.
    p.copy_within(0..dp1 - 1, 1);
    p[0] = 0;
}

#[allow(dead_code)]
fn rs_poly_div_x(p: &mut [u8], dp1: usize) {
    // memmove(p, q+1, dp1-1); p[dp1-1] = 0
    p.copy_within(1..dp1, 0);
    p[dp1 - 1] = 0;
}

/// Compute the first dp1 coefficients of q*r.
fn rs_poly_mult(
    gf: &RsGf256,
    p: &mut [u8],
    dp1: usize,
    q: &[u8],
    ep1: usize,
    r: &[u8],
    fp1: usize,
) {
    rs_poly_zero(p, dp1);
    let m = ep1.min(dp1);
    for i in 0..m {
        if q[i] != 0 {
            let n = (dp1 - i).min(fp1);
            let logqi = gf.log[q[i] as usize] as u32;
            for j in 0..n {
                p[i + j] ^= rs_hgmul(gf, r[j] as u32, logqi) as u8;
            }
        }
    }
}

/// Syndrome of a codeword.
fn rs_calc_syndrome(gf: &RsGf256, m0: i32, s: &mut [u8], npar: usize, data: &[u8], ndata: usize) {
    for j in 0..npar {
        let alphaj = gf.log[gf.exp[j + m0 as usize] as usize] as u32;
        let mut sj: u32 = 0;
        for i in 0..ndata {
            sj = data[i] as u32 ^ rs_hgmul(gf, sj, alphaj);
        }
        s[j] = sj as u8;
    }
}

/// λ initialization from known erasures.
fn rs_init_lambda(
    gf: &RsGf256,
    lambda: &mut [u8],
    npar: usize,
    erasures: &[u8],
    nerasures: usize,
    ndata: usize,
) {
    let span = if npar < 4 { 4 } else { npar };
    rs_poly_zero(lambda, span + 1);
    lambda[0] = 1;
    for i in 0..nerasures {
        let mut j = i + 1;
        while j > 0 {
            lambda[j] ^= rs_hgmul(gf, lambda[j - 1] as u32, (ndata - 1 - erasures[i] as usize) as u32) as u8;
            j -= 1;
        }
    }
}

/// Modified Berlekamp-Massey — returns the degree of λ (number of errors).
fn rs_modified_berlekamp_massey(
    gf: &RsGf256,
    lambda: &mut [u8],
    s: &[u8],
    omega: &mut [u8],
    npar: usize,
    erasures: &[u8],
    nerasures: usize,
    ndata: usize,
) -> i32 {
    let mut tt = [0u8; 256];
    rs_init_lambda(gf, lambda, npar, erasures, nerasures, ndata);
    rs_poly_copy(&mut tt, lambda, npar + 1);
    let mut l: i32 = nerasures as i32;
    let mut k: i32 = 0;
    for n in (nerasures + 1)..=npar {
        // tt = tt * x truncated to length n-k+1
        rs_poly_mul_x(&mut tt, n - k as usize + 1);
        let mut d: u32 = 0;
        for i in 0..=l as usize {
            d ^= rs_gmul(gf, lambda[i] as u32, s[n - 1 - i] as u32);
        }
        if d != 0 {
            let logd = gf.log[d as usize] as u32;
            if l < n as i32 - k {
                let t = n as i32 - k;
                for i in 0..=(n - k as usize) {
                    let tti = tt[i];
                    tt[i] = rs_hgmul(gf, lambda[i] as u32, 255 - logd) as u8;
                    lambda[i] ^= rs_hgmul(gf, tti as u32, logd) as u8;
                }
                k = n as i32 - l;
                l = t;
            } else {
                for i in 0..=l as usize {
                    lambda[i] ^= rs_hgmul(gf, tt[i] as u32, logd) as u8;
                }
            }
        }
    }
    rs_poly_mult(gf, omega, npar, lambda, (l + 1) as usize, s, npar);
    l
}

/// Find roots of the error-locator polynomial. Returns root count.
fn rs_find_roots(
    gf: &RsGf256,
    epos: &mut [u8],
    lambda: &[u8],
    nerrors_in: i32,
    ndata: usize,
) -> i32 {
    let mut nroots = 0usize;
    let nerrors = nerrors_in;
    if nerrors <= 4 {
        // λ[0] is always 1; pass λ[1..=4].
        let mut roots = [0u8; 4];
        let n = rs_quartic_solve(gf, lambda[1] as u32, lambda[2] as u32, lambda[3] as u32, lambda[4] as u32, &mut roots) as usize;
        for i in 0..n {
            if roots[i] != 0 {
                let alpha = gf.log[roots[i] as usize] as i32;
                if alpha < ndata as i32 {
                    epos[nroots] = alpha as u8;
                    nroots += 1;
                }
            }
        }
        return nroots as i32;
    }
    // Fallback Chien search for degree > 4 (not exercised in QR-only use).
    for alpha in 0..(ndata as u32) {
        let mut sum: u32 = 0;
        let mut alphai: u32 = 0;
        for i in 0..=nerrors as usize {
            sum ^= rs_hgmul(gf, lambda[nerrors as usize - i] as u32, alphai);
            alphai = gf.log[gf.exp[(alphai + alpha) as usize] as usize] as u32;
        }
        if sum == 0 {
            epos[nroots] = alpha as u8;
            nroots += 1;
        }
    }
    nroots as i32
}

/// Correct an `ndata`-byte codeword whose last `npar` bytes are parity.
/// Returns the number of errors corrected, or `-1` if too many errors.
/// `erasures` lists positions (counting from the start of `data`) of bytes
/// known to be in error — these count as half an error each.
pub fn rs_correct(
    gf: &RsGf256,
    m0: i32,
    data: &mut [u8],
    ndata: usize,
    npar: usize,
    erasures: &[u8],
    nerasures: usize,
) -> i32 {
    let mut lambda = [0u8; 256];
    let mut omega = [0u8; 256];
    let mut epos = [0u8; 256];
    let mut s = [0u8; 256];
    if nerasures > npar {
        return -1;
    }
    rs_calc_syndrome(gf, m0, &mut s, npar, data, ndata);
    if !s[..npar].iter().any(|&x| x != 0) {
        return 0;
    }
    let nerrors = rs_modified_berlekamp_massey(
        gf, &mut lambda, &s, &mut omega, npar, erasures, nerasures, ndata,
    );
    // nerrors-_nerasures > (_npar-_nerasures) >> 1
    if nerrors <= 0
        || (nerrors as usize) - nerasures > ((npar - nerasures) >> 1)
    {
        return -1;
    }
    let nroots = rs_find_roots(gf, &mut epos, &lambda, nerrors, ndata);
    if nroots < nerrors {
        return -1;
    }
    for i in 0..nerrors as usize {
        let alpha = epos[i] as u32;
        // Evaluate ω at α⁻¹.
        let mut a: u32 = 0;
        let alphan1 = 255 - alpha;
        let mut alphanj: u32 = 0;
        for j in 0..npar {
            a ^= rs_hgmul(gf, omega[j] as u32, alphanj);
            alphanj = gf.log[gf.exp[(alphanj + alphan1) as usize] as usize] as u32;
        }
        // Evaluate λ' at α⁻¹ (odd powers only).
        let mut b: u32 = 0;
        let alphan2 = gf.log[gf.exp[(alphan1 << 1) as usize] as usize] as u32;
        let mut alphanj = alphan1 + ((m0 as u32).wrapping_mul(alpha) % 255);
        let mut j = 1;
        while j <= npar {
            b ^= rs_hgmul(gf, lambda[j] as u32, alphanj);
            alphanj = gf.log[gf.exp[(alphanj + alphan2) as usize] as usize] as u32;
            j += 2;
        }
        data[ndata - 1 - alpha as usize] ^= rs_gdiv(gf, a, b) as u8;
    }
    nerrors
}

/// Compute an `npar`-coefficient generator polynomial for Reed-Solomon.
pub fn rs_compute_genpoly(gf: &RsGf256, m0: i32, genpoly: &mut [u8], npar: usize) {
    if npar == 0 { return; }
    rs_poly_zero(genpoly, npar);
    genpoly[0] = 1;
    for i in 0..npar {
        let n = (i + 1).min(npar - 1);
        let alphai = gf.log[gf.exp[m0 as usize + i] as usize] as u32;
        let mut j = n;
        while j > 0 {
            genpoly[j] = genpoly[j - 1] ^ rs_hgmul(gf, genpoly[j] as u32, alphai) as u8;
            j -= 1;
        }
        genpoly[0] = rs_hgmul(gf, genpoly[0] as u32, alphai) as u8;
    }
}

/// Adds `npar` parity bytes to an `ndata-npar`-byte message in-place.
pub fn rs_encode(gf: &RsGf256, data: &mut [u8], ndata: usize, genpoly: &[u8], npar: usize) {
    if npar == 0 { return; }
    let split = ndata - npar;
    for i in 0..npar { data[split + i] = 0; }
    for i in 0..ndata - npar {
        let d = data[i] ^ data[split];
        if d != 0 {
            let logd = gf.log[d as usize] as u32;
            for j in 0..npar - 1 {
                data[split + j] = data[split + j + 1] ^ rs_hgmul(gf, genpoly[npar - 1 - j] as u32, logd) as u8;
            }
            data[split + npar - 1] = rs_hgmul(gf, genpoly[0] as u32, logd) as u8;
        } else {
            // shift the parity (LFSR) left by one
            for j in 0..npar - 1 {
                data[split + j] = data[split + j + 1];
            }
            data[split + npar - 1] = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify the GF(2⁸) tables match the well-known QR values.
    #[test]
    fn gf256_tables_have_correct_size_and_round_trip() {
        let gf = RsGf256::new();
        // exp[0] == 1, exp[1] == 2 (alpha = 0x02).
        assert_eq!(gf.exp[0], 1);
        assert_eq!(gf.exp[1], 2);
        // exp/log round-trip on every non-zero value.
        for v in 1u32..256 {
            let l = gf.log[v as usize] as u32;
            assert_eq!(gf.exp[l as usize] as u32, v, "log/exp mismatch at v={}", v);
        }
        // Extended table: exp[i] == exp[i+255] for i < 256.
        for i in 0..256 {
            assert_eq!(gf.exp[i], gf.exp[i + 255]);
        }
    }

    #[test]
    fn encode_then_decode_clean() {
        let gf = RsGf256::new();
        let ndata = 100usize;
        let npar = 20usize;
        let mut genpoly = [0u8; 256];
        rs_compute_genpoly(&gf, QR_M0, &mut genpoly, npar);
        let mut data = [0u8; 256];
        for i in 0..ndata - npar { data[i] = (i as u8).wrapping_mul(7).wrapping_add(13); }
        rs_encode(&gf, &mut data, ndata, &genpoly, npar);
        let original = data;
        let n = rs_correct(&gf, QR_M0, &mut data, ndata, npar, &[], 0);
        assert_eq!(n, 0);
        assert_eq!(data, original);
    }

    #[test]
    fn correct_random_errors_within_budget() {
        // For each (ndata, npar), corrupt nerrors <= npar/2 random bytes and
        // verify the decoder recovers the codeword.
        use std::collections::HashSet;
        let gf = RsGf256::new();
        // Linear-congruential pseudo-random — keeps the test deterministic.
        let mut rng_state: u64 = 0xC0FFEE_u64;
        let mut next = || -> u32 {
            rng_state = rng_state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (rng_state >> 32) as u32
        };

        for &(ndata, npar) in &[(20usize, 8usize), (50, 20), (100, 30), (255, 50)] {
            let mut genpoly = [0u8; 256];
            rs_compute_genpoly(&gf, QR_M0, &mut genpoly, npar);
            for trial in 0..50 {
                let mut data = [0u8; 256];
                for i in 0..ndata - npar { data[i] = next() as u8; }
                rs_encode(&gf, &mut data, ndata, &genpoly, npar);
                let original = data;

                let max_errors = npar / 2;
                let nerr = (next() as usize % (max_errors + 1)).min(npar / 2);

                // Pick `nerr` distinct positions to flip.
                let mut positions = HashSet::new();
                while positions.len() < nerr {
                    positions.insert((next() as usize) % ndata);
                }
                for &p in &positions {
                    let delta = (next() as u8).max(1);
                    data[p] ^= delta;
                }

                let n = rs_correct(&gf, QR_M0, &mut data, ndata, npar, &[], 0);
                assert!(n >= 0,
                    "trial {}: rs_correct rejected {} errors in (ndata={}, npar={})",
                    trial, nerr, ndata, npar);
                assert_eq!(&data[..], &original[..],
                    "trial {}: decoded mismatch (nerr={}, ndata={}, npar={})",
                    trial, nerr, ndata, npar);
            }
        }
    }

    #[test]
    fn rejects_too_many_errors() {
        let gf = RsGf256::new();
        let ndata = 50usize;
        let npar = 10usize;
        let mut genpoly = [0u8; 256];
        rs_compute_genpoly(&gf, QR_M0, &mut genpoly, npar);
        let mut data = [0u8; 256];
        for i in 0..ndata - npar { data[i] = (i * 11) as u8; }
        rs_encode(&gf, &mut data, ndata, &genpoly, npar);
        // npar/2 = 5 corrections max. Corrupt 6 distinct bytes.
        for p in [0usize, 5, 10, 15, 20, 25] {
            data[p] ^= 0xFF;
        }
        let n = rs_correct(&gf, QR_M0, &mut data, ndata, npar, &[], 0);
        assert!(n < 0, "expected rejection, got n={}", n);
    }
}
