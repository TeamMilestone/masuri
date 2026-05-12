//! Integer arithmetic helpers for the QR decoder.
//! Rust port of zbar/qrcode/util.{c,h}.
//! Original Copyright (C) 2008-2009 Timothy B. Terriberry (tterribe@xiph.org)
//! LGPL-2.1-or-later
//!
//! Bit-twiddling idioms are preserved 1:1 so intermediate values match the
//! zbar C reference build during debugging (handoff decision).

/// `QR_MAXI(a, b)` — branchless max via `a - ((a-b) & -(b>a))`.
#[inline]
pub fn qr_maxi(a: i32, b: i32) -> i32 {
    a.wrapping_sub(a.wrapping_sub(b) & -((b > a) as i32))
}

/// `QR_MINI(a, b)` — branchless min via `a + ((b-a) & -(b<a))`.
#[inline]
pub fn qr_mini(a: i32, b: i32) -> i32 {
    a.wrapping_add(b.wrapping_sub(a) & -((b < a) as i32))
}

/// `QR_SIGNI(x)` — returns -1, 0, or +1.
#[inline]
pub fn qr_signi(x: i32) -> i32 {
    (x > 0) as i32 - (x < 0) as i32
}

/// `QR_SIGNMASK(x)` — all-ones if x<0, else 0.
#[inline]
pub fn qr_signmask(x: i32) -> i32 {
    -((x < 0) as i32)
}

/// `QR_FLIPSIGNI(a, b)` — invert sign of a when b<0.
/// Mirrors `(a + SIGNMASK(b)) ^ SIGNMASK(b)`.
#[inline]
pub fn qr_flipsigni(a: i32, b: i32) -> i32 {
    let sm = qr_signmask(b);
    a.wrapping_add(sm) ^ sm
}

/// `QR_COPYSIGNI(a, b)` — magnitude of a with sign of b.
#[inline]
pub fn qr_copysigni(a: i32, b: i32) -> i32 {
    qr_flipsigni(a.wrapping_abs(), b)
}

/// `QR_DIVROUND(x, y)` — exact rounded division (y>0).
#[inline]
pub fn qr_divround(x: i32, y: i32) -> i32 {
    x.wrapping_add(qr_flipsigni(y >> 1, x)) / y
}

/// `QR_CLAMPI(a, b, c)` — clamp b into [a, c].
#[inline]
pub fn qr_clampi(a: i32, b: i32, c: i32) -> i32 {
    qr_maxi(a, qr_mini(b, c))
}

/// `QR_CLAMP255(x)` — saturating cast to u8.
#[inline]
pub fn qr_clamp255(x: i32) -> u8 {
    let mask = ((x < 0) as i32 - 1) & (x | -((x > 255) as i32));
    mask as u8
}

/// `QR_SORT2I(a, b)` — swap so that `*a <= *b`.
#[inline]
pub fn qr_sort2i(a: &mut i32, b: &mut i32) {
    if *a > *b {
        std::mem::swap(a, b);
    }
}

/// `QR_FIXMUL(a, b, r, s)` — `(a*b + r) >> s`, clipped to i32.
#[inline]
pub fn qr_fixmul(a: i32, b: i32, r: i64, s: u32) -> i32 {
    (((a as i64).wrapping_mul(b as i64).wrapping_add(r)) >> s) as i32
}

/// `QR_EXTMUL(a, b, r)` — full 64-bit `a*b + r`.
#[inline]
pub fn qr_extmul(a: i32, b: i32, r: i64) -> i64 {
    (a as i64).wrapping_mul(b as i64).wrapping_add(r)
}

/// `qr_isqrt(val)` — floor(sqrt(val)) exactly.
/// 1:1 port using the "search the largest binary digit" method.
pub fn qr_isqrt(mut val: u32) -> u32 {
    let mut g: u32 = 0;
    let mut b: u32 = 0x8000;
    for bshift in (0..16).rev() {
        // C: t = (g<<1)+b<<bshift  ⇒  ((g<<1)+b) << bshift
        let t = (g.wrapping_shl(1).wrapping_add(b)) << bshift;
        if t <= val {
            g = g.wrapping_add(b);
            val = val.wrapping_sub(t);
        }
        b >>= 1;
    }
    g
}

/// `qr_ilog(v)` — position of highest set bit, 1-indexed; 0 if v==0.
/// Matches the `__builtin_clz`-based path in zbar.
#[inline]
pub fn qr_ilog(v: u32) -> i32 {
    if v == 0 { 0 } else { 32 - v.leading_zeros() as i32 }
}

/// `qr_ihypot(x, y)` — CORDIC `sqrt(x² + y²)` with ~27 bits precision.
/// 1:1 port; preserves all sign-mask + shift idioms so intermediate values
/// match the zbar C reference build bit-for-bit.
pub fn qr_ihypot(_x: i32, _y: i32) -> u32 {
    let xs = _x.wrapping_abs();
    let mut ys = _y.wrapping_abs();
    let mut x = xs as u32;
    let mut y = ys as u32;

    // mask = -(x>y) & (xs^ys); then swap x,y and adjust ys when x>y.
    let mut mask: i32 = -((x > y) as i32) & (xs ^ ys);
    x = ((xs ^ mask)) as u32;
    y = ((ys ^ mask)) as u32;
    ys ^= mask;
    // (xs is no longer used past this point in zbar; we drop it.)
    let _ = xs;

    let shift = qr_maxi(31 - qr_ilog(y), 0) as u32;

    // x = ((x<<shift) * 0x9B74EDAAULL) >> 32;
    x = (((x as u64).wrapping_shl(shift)).wrapping_mul(0x9B74EDAA_u64) >> 32) as u32;
    // _y = ((_y<<shift) * 0x9B74EDA9LL) >> 32;
    let ys_shifted = (ys as i64).wrapping_shl(shift);
    ys = (ys_shifted.wrapping_mul(0x9B74EDA9_i64) >> 32) as i32;

    let mut u: i32 = x as i32;
    mask = -((ys < 0) as i32);
    // x += (ys+mask)^mask;
    x = (x as i32).wrapping_add(ys.wrapping_add(mask) ^ mask) as u32;
    // ys -= (u+mask)^mask;
    ys = ys.wrapping_sub(u.wrapping_add(mask) ^ mask);

    // C: u = x+1 >> 1 — unsigned (logical) shift; mirror with u32 ops.
    u = (x.wrapping_add(1) >> 1) as i32;
    let mut v: i32 = ys.wrapping_add(1) >> 1;
    mask = -((ys < 0) as i32);
    x = (x as i32).wrapping_add(v.wrapping_add(mask) ^ mask) as u32;
    ys = ys.wrapping_sub(u.wrapping_add(mask) ^ mask);

    for i in 1..16 {
        // C: u = x+1 >> 2 — unsigned shift on `unsigned x`.
        u = (x.wrapping_add(1) >> 2) as i32;
        // r = (1<<2i) >> 1
        let r: i32 = (1_i32.wrapping_shl(2 * i)) >> 1;
        v = ys.wrapping_add(r) >> (2 * i);
        mask = -((ys < 0) as i32);
        x = (x as i32).wrapping_add(v.wrapping_add(mask) ^ mask) as u32;
        // _y = (_y - ((u+mask)^mask)) << 1;
        ys = ys.wrapping_sub(u.wrapping_add(mask) ^ mask).wrapping_shl(1);
    }

    x.wrapping_add((1_u32 << shift) >> 1) >> shift
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isqrt_small() {
        for v in 0u32..=1000 {
            let r = qr_isqrt(v);
            let s = (v as f64).sqrt().floor() as u32;
            assert_eq!(r, s, "qr_isqrt({}) = {}, expected {}", v, r, s);
        }
    }

    #[test]
    fn isqrt_edges() {
        assert_eq!(qr_isqrt(u32::MAX), 65535);
        assert_eq!(qr_isqrt(65535 * 65535), 65535);
        assert_eq!(qr_isqrt(65535 * 65535 + 1), 65535);
        assert_eq!(qr_isqrt(65536u32.wrapping_mul(65536).wrapping_sub(1)), 65535);
        // Largest perfect square fitting in u32: 65535²
        assert_eq!(qr_isqrt(123_456), 351);
    }

    #[test]
    fn ilog_basics() {
        assert_eq!(qr_ilog(0), 0);
        assert_eq!(qr_ilog(1), 1);
        assert_eq!(qr_ilog(2), 2);
        assert_eq!(qr_ilog(3), 2);
        assert_eq!(qr_ilog(4), 3);
        assert_eq!(qr_ilog(0x80000000), 32);
        assert_eq!(qr_ilog(0xFFFFFFFF), 32);
        assert_eq!(qr_ilog(0x7FFFFFFF), 31);
    }

    #[test]
    fn ihypot_pythagorean_triples() {
        // Pythagorean triples — exact integer hypotenuse, must be exact per the
        // zbar comment about "Pythagorean triples with hypotenuse < (1<<27)-1".
        for &(a, b, c) in &[(3, 4, 5), (5, 12, 13), (8, 15, 17), (7, 24, 25), (20, 21, 29)] {
            assert_eq!(qr_ihypot(a, b), c, "qr_ihypot({}, {}) expected {}", a, b, c);
            assert_eq!(qr_ihypot(b, a), c);
            assert_eq!(qr_ihypot(-a, b), c);
            assert_eq!(qr_ihypot(a, -b), c);
        }
    }

    #[test]
    fn ihypot_axis_aligned() {
        for v in [0, 1, 100, 1000, 12345, 1_000_000] {
            assert_eq!(qr_ihypot(v, 0), v as u32);
            assert_eq!(qr_ihypot(0, v), v as u32);
            assert_eq!(qr_ihypot(-v, 0), v as u32);
            assert_eq!(qr_ihypot(0, -v), v as u32);
        }
    }

    #[test]
    fn maxmin_match_std() {
        for &(a, b) in &[(0, 0), (1, 2), (-5, 3), (i32::MIN + 1, i32::MAX), (-100, -100)] {
            assert_eq!(qr_maxi(a, b), a.max(b));
            assert_eq!(qr_mini(a, b), a.min(b));
        }
    }

    #[test]
    fn signi_signmask() {
        assert_eq!(qr_signi(0), 0);
        assert_eq!(qr_signi(5), 1);
        assert_eq!(qr_signi(-5), -1);
        assert_eq!(qr_signmask(0), 0);
        assert_eq!(qr_signmask(1), 0);
        assert_eq!(qr_signmask(-1), -1);
    }

    #[test]
    fn flipsigni_copysigni() {
        assert_eq!(qr_flipsigni(5, 1), 5);
        assert_eq!(qr_flipsigni(5, -1), -5);
        assert_eq!(qr_flipsigni(-5, 1), -5);
        assert_eq!(qr_flipsigni(-5, -1), 5);
        assert_eq!(qr_copysigni(5, -1), -5);
        assert_eq!(qr_copysigni(-5, 1), 5);
    }

    #[test]
    fn clamp255() {
        assert_eq!(qr_clamp255(-1), 0);
        assert_eq!(qr_clamp255(0), 0);
        assert_eq!(qr_clamp255(128), 128);
        assert_eq!(qr_clamp255(255), 255);
        assert_eq!(qr_clamp255(256), 255);
        assert_eq!(qr_clamp255(1000), 255);
    }
}
