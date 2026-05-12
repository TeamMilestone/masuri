//! ISAAC PRNG — used by `qr_finder_ransac` for sample selection.
//! Rust port of zbar/qrcode/isaac.{c,h}.
//! Original public-domain implementation by Robert J. Jenkins Jr., 1996.
//! Adapted by Timothy B. Terriberry (1999-2009).
//! LGPL-2.1-or-later (the zbar wrapper; the algorithm itself is PD).

pub const ISAAC_SZ_LOG: u32 = 8;
pub const ISAAC_SZ: usize = 1 << ISAAC_SZ_LOG; // 256
pub const ISAAC_SEED_SZ_MAX: usize = ISAAC_SZ << 2; // 1024 bytes

pub struct IsaacCtx {
    pub n: u32,
    pub r: [u32; ISAAC_SZ],
    pub m: [u32; ISAAC_SZ],
    pub a: u32,
    pub b: u32,
    pub c: u32,
}

impl IsaacCtx {
    pub fn new() -> Self {
        IsaacCtx {
            n: 0,
            r: [0; ISAAC_SZ],
            m: [0; ISAAC_SZ],
            a: 0,
            b: 0,
            c: 0,
        }
    }
}

// Wrapping helpers — ISAAC_MASK in C truncates to 32-bit. With u32, wrapping_*
// is sufficient.
#[inline(always)]
fn wadd(a: u32, b: u32) -> u32 { a.wrapping_add(b) }
#[inline(always)]
fn wmul(a: u32, b: u32) -> u32 { a.wrapping_mul(b) }
#[inline(always)]
fn wshl(a: u32, n: u32) -> u32 { a.wrapping_shl(n) }
#[inline(always)]
fn wshr(a: u32, n: u32) -> u32 { a.wrapping_shr(n) }

/// Core ISAAC state update. Refills `ctx.r` with 256 fresh u32 values.
fn isaac_update(ctx: &mut IsaacCtx) {
    let mut a = ctx.a;
    ctx.c = ctx.c.wrapping_add(1);
    let mut b = wadd(ctx.b, ctx.c);

    // C macro: `(x & (ISAAC_SZ-1)<<2) >> 2` selects an index into m[0..256].
    let idx_mask: u32 = (ISAAC_SZ as u32 - 1) << 2; // 0x3FC

    macro_rules! mix_step {
        ($i:expr, $half_off:expr, $a_op:expr) => {{
            let i: usize = $i;
            let x = ctx.m[i];
            a = wadd($a_op(a), ctx.m[$half_off(i)]);
            let y = wadd(wadd(ctx.m[((x & idx_mask) >> 2) as usize], a), b);
            ctx.m[i] = y;
            // `y >> (ISAAC_SZ_LOG+2) & (ISAAC_SZ-1)` — note operator precedence:
            //   `+`, `>>`, `&` — `+` highest, `&` lowest → (y >> 10) & 0xFF
            b = wadd(ctx.m[(wshr(y, ISAAC_SZ_LOG + 2) & (ISAAC_SZ as u32 - 1)) as usize], x);
            ctx.r[i] = b;
        }};
    }

    // First half: m[i + 128]
    let off_hi = |i: usize| i + ISAAC_SZ / 2;
    let mut i = 0;
    while i < ISAAC_SZ / 2 {
        mix_step!(i, off_hi, |a: u32| a ^ wshl(a, 13));
        i += 1;
        mix_step!(i, off_hi, |a: u32| a ^ wshr(a, 6));
        i += 1;
        mix_step!(i, off_hi, |a: u32| a ^ wshl(a, 2));
        i += 1;
        mix_step!(i, off_hi, |a: u32| a ^ wshr(a, 16));
        i += 1;
    }

    // Second half: m[i - 128]
    let off_lo = |i: usize| i - ISAAC_SZ / 2;
    let mut i = ISAAC_SZ / 2;
    while i < ISAAC_SZ {
        mix_step!(i, off_lo, |a: u32| a ^ wshl(a, 13));
        i += 1;
        mix_step!(i, off_lo, |a: u32| a ^ wshr(a, 6));
        i += 1;
        mix_step!(i, off_lo, |a: u32| a ^ wshl(a, 2));
        i += 1;
        mix_step!(i, off_lo, |a: u32| a ^ wshr(a, 16));
        i += 1;
    }

    ctx.a = a;
    ctx.b = b;
    ctx.n = ISAAC_SZ as u32;
}

/// Initialization mixing function — 1:1 port of `isaac_mix`.
fn isaac_mix(x: &mut [u32; 8]) {
    const SHIFT: [u32; 8] = [11, 2, 8, 16, 10, 4, 8, 9];
    let mut i = 0;
    while i < 8 {
        x[i] ^= wshl(x[(i + 1) & 7], SHIFT[i]);
        x[(i + 3) & 7] = wadd(x[(i + 3) & 7], x[i]);
        x[(i + 1) & 7] = wadd(x[(i + 1) & 7], x[(i + 2) & 7]);
        i += 1;
        x[i] ^= wshr(x[(i + 1) & 7], SHIFT[i]);
        x[(i + 3) & 7] = wadd(x[(i + 3) & 7], x[i]);
        x[(i + 1) & 7] = wadd(x[(i + 1) & 7], x[(i + 2) & 7]);
        i += 1;
    }
    let _ = wmul; // silence unused; reserved for future use
}

/// Initialize from an optional byte seed (≤ ISAAC_SEED_SZ_MAX). Empty seed
/// gives the deterministic stream that zbar's `qr_reader` uses.
pub fn isaac_init(ctx: &mut IsaacCtx, seed: &[u8]) {
    ctx.a = 0; ctx.b = 0; ctx.c = 0;

    let mut x = [0x9E3779B9_u32; 8];
    for _ in 0..4 { isaac_mix(&mut x); }

    let nseed = seed.len().min(ISAAC_SEED_SZ_MAX);
    for s in ctx.r.iter_mut() { *s = 0; }
    let mut i = 0usize;
    while i < nseed >> 2 {
        // Little-endian u32 assembly of seed[i*4..i*4+4].
        ctx.r[i] = (seed[i * 4 + 3] as u32) << 24
            | (seed[i * 4 + 2] as u32) << 16
            | (seed[i * 4 + 1] as u32) << 8
            | (seed[i * 4] as u32);
        i += 1;
    }
    if nseed & 3 != 0 {
        // Partial trailing word.
        let mut w = seed[i * 4] as u32;
        for j in 1..(nseed & 3) {
            w = w.wrapping_add((seed[i * 4 + j] as u32) << (j << 3));
        }
        ctx.r[i] = w;
        // Skip incrementing i — zbar does increment but only to advance past
        // the partial; remaining r[] are already zeroed.
    }

    // First pass: x[j] += r[i+j]; mix; copy into m.
    let mut i = 0usize;
    while i < ISAAC_SZ {
        for j in 0..8 { x[j] = wadd(x[j], ctx.r[i + j]); }
        isaac_mix(&mut x);
        ctx.m[i..i + 8].copy_from_slice(&x);
        i += 8;
    }
    // Second pass: x[j] += m[i+j]; mix; copy into m.
    let mut i = 0usize;
    while i < ISAAC_SZ {
        for j in 0..8 { x[j] = wadd(x[j], ctx.m[i + j]); }
        isaac_mix(&mut x);
        ctx.m[i..i + 8].copy_from_slice(&x);
        i += 8;
    }
    isaac_update(ctx);
}

/// Convenience: initialize with no seed — zero-init then mix.
#[inline]
pub fn isaac_init_empty(ctx: &mut IsaacCtx) {
    isaac_init(ctx, &[]);
}

/// Returns the next 32-bit random value.
pub fn isaac_next_uint32(ctx: &mut IsaacCtx) -> u32 {
    if ctx.n == 0 {
        isaac_update(ctx);
    }
    ctx.n -= 1;
    ctx.r[ctx.n as usize]
}

/// Uniform integer in `[0, n)`. `n` must be > 0.
/// Mirrors zbar's rejection-sampling loop.
pub fn isaac_next_uint(ctx: &mut IsaacCtx, n: u32) -> u32 {
    loop {
        let r = isaac_next_uint32(ctx);
        let v = r % n;
        let d = r.wrapping_sub(v);
        // Reject if (d + n - 1) wrapped past u32::MAX (would alias to a low bin).
        if d.wrapping_add(n.wrapping_sub(1)) >= d {
            return v;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// First 16 u32 from a zero-seed ISAAC stream, captured from the zbar C
    /// reference (build: `cc -O2 isaac.c`, init with `(NULL, 0)`).
    const ZERO_SEED_FIRST16: [u32; 16] = [
        0x182600F3, 0x300B4A8D, 0x301B6622, 0xB08ACD21,
        0x296FD679, 0x995206E9, 0xB3FFA8B5, 0x0FC99C24,
        0x5F071FAF, 0x52251DEF, 0x894F41C2, 0xCC4C9AFB,
        0x96C33F74, 0x347CB71D, 0xC90F8FBD, 0xA658F57A,
    ];

    /// Following the 16 u32 above: first 16 outputs of `isaac_next_uint(_, 10)`.
    const ZERO_SEED_BOUNDED_10: [u32; 16] = [
        8, 7, 2, 9, 8, 3, 5, 4, 5, 1, 5, 2, 5, 5, 8, 0,
    ];

    /// First 8 outputs of `isaac_next_uint(_, 100)` continuing the above.
    const ZERO_SEED_BOUNDED_100: [u32; 8] = [40, 73, 63, 21, 31, 7, 82, 31];

    #[test]
    fn zero_seed_stream_matches_zbar_c() {
        let mut ctx = IsaacCtx::new();
        isaac_init_empty(&mut ctx);
        for (i, &expected) in ZERO_SEED_FIRST16.iter().enumerate() {
            let v = isaac_next_uint32(&mut ctx);
            assert_eq!(v, expected, "stream u32 #{}: got 0x{:08X}, want 0x{:08X}", i, v, expected);
        }
        for (i, &expected) in ZERO_SEED_BOUNDED_10.iter().enumerate() {
            let v = isaac_next_uint(&mut ctx, 10);
            assert_eq!(v, expected, "next_uint(10) #{}: got {}, want {}", i, v, expected);
        }
        for (i, &expected) in ZERO_SEED_BOUNDED_100.iter().enumerate() {
            let v = isaac_next_uint(&mut ctx, 100);
            assert_eq!(v, expected, "next_uint(100) #{}: got {}, want {}", i, v, expected);
        }
    }

    #[test]
    fn bounded_range_in_bounds() {
        let mut ctx = IsaacCtx::new();
        isaac_init_empty(&mut ctx);
        for _ in 0..1000 {
            assert!(isaac_next_uint(&mut ctx, 1) == 0);
        }
        for _ in 0..1000 {
            assert!(isaac_next_uint(&mut ctx, 17) < 17);
        }
    }
}
