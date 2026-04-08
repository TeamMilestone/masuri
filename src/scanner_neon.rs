//! NEON SIMD optimized scanner - processes 4 scan lines simultaneously.
//! Each NEON lane handles one independent scan line's EWMA + derivative computation.

#[cfg(target_arch = "aarch64")]
use std::arch::aarch64::*;

const ZBAR_FIXED: i32 = 5;
const ROUND: u32 = 1 << (ZBAR_FIXED - 1);
const EWMA_WEIGHT: i32 = (0.78_f64 * ((1 << (ZBAR_FIXED + 1)) as f64) + 1.0) as i32 / 2;
const THRESH_INIT: u32 = (0.44_f64 * ((1 << (ZBAR_FIXED + 1)) as f64) + 1.0) as u32 / 2;
const THRESH_FADE: u32 = 8;
const THRESH_MIN: u32 = 4;

#[derive(Clone, Copy)]
pub struct EdgeResult {
    pub has_edge: bool,
    pub width: u32,
}

impl EdgeResult {
    #[inline(always)]
    const fn none() -> Self {
        EdgeResult { has_edge: false, width: 0 }
    }
}

/// 4-lane NEON scanner
#[cfg(target_arch = "aarch64")]
pub struct NeonScanner4 {
    x: u32,
    // y0 circular buffer: 4 history slots, each an int32x4 (4 lanes)
    y0: [int32x4_t; 4],
    // Per-lane scalar state for edge detection
    y1_sign: [i32; 4],
    y1_thresh: [u32; 4],
    cur_edge: [u32; 4],
    last_edge: [u32; 4],
    width: [u32; 4],
}

#[cfg(target_arch = "aarch64")]
impl NeonScanner4 {
    pub fn new() -> Self {
        unsafe {
            NeonScanner4 {
                x: 0,
                y0: [vdupq_n_s32(0); 4],
                y1_sign: [0; 4],
                y1_thresh: [THRESH_MIN; 4],
                cur_edge: [0; 4],
                last_edge: [0; 4],
                width: [0; 4],
            }
        }
    }

    pub fn reset(&mut self) {
        unsafe {
            self.x = 0;
            self.y0 = [vdupq_n_s32(0); 4];
            self.y1_sign = [0; 4];
            self.y1_thresh = [THRESH_MIN; 4];
            self.cur_edge = [0; 4];
            self.last_edge = [0; 4];
            self.width = [0; 4];
        }
    }

    /// Process 4 pixels (one from each lane/scan line) simultaneously.
    #[inline(always)]
    pub fn scan_y_4(&mut self, p0: i32, p1: i32, p2: i32, p3: i32) -> [EdgeResult; 4] {
        unsafe { self.scan_y_4_neon(p0, p1, p2, p3) }
    }

    #[target_feature(enable = "neon")]
    unsafe fn scan_y_4_neon(&mut self, p0: i32, p1: i32, p2: i32, p3: i32) -> [EdgeResult; 4] {
        let x = self.x as usize;
        let pixels = [p0, p1, p2, p3];
        let pixel_vec = vld1q_s32(pixels.as_ptr());

        if x == 0 {
            self.y0 = [pixel_vec; 4];
            self.x = 1;
            return [EdgeResult::none(); 4];
        }

        // Load history from circular buffer
        let y0_prev = self.y0[x.wrapping_sub(1) & 3];
        let y0_2 = self.y0[x.wrapping_sub(2) & 3];
        let y0_3 = self.y0[x.wrapping_sub(3) & 3];

        // --- NEON EWMA ---
        // y0_new = y0_prev + ((pixel - y0_prev) * EWMA_WEIGHT) >> FIXED
        let diff = vsubq_s32(pixel_vec, y0_prev);
        let ewma_w = vdupq_n_s32(EWMA_WEIGHT);
        let weighted = vmulq_s32(diff, ewma_w);
        let shifted = vshrq_n_s32(weighted, ZBAR_FIXED);
        let y0_new = vaddq_s32(y0_prev, shifted);
        self.y0[x & 3] = y0_new;

        // --- NEON 1st derivative with smoothing ---
        let y1_1_vec = vsubq_s32(y0_prev, y0_2);
        let y1_2_vec = vsubq_s32(y0_2, y0_3);

        // if |y1_1| < |y1_2| && same_sign => use y1_2
        let abs1 = vabsq_s32(y1_1_vec);
        let abs2 = vabsq_s32(y1_2_vec);
        let less: uint32x4_t = vcltq_s32(abs1, abs2);
        // same sign: (y1_1 ^ y1_2) >= 0
        let xored = veorq_s32(y1_1_vec, y1_2_vec);
        let zero = vdupq_n_s32(0);
        let same_sign: uint32x4_t = vcgeq_s32(xored, zero);
        let use_y1_2 = vandq_u32(less, same_sign);
        let y1_final = vbslq_s32(use_y1_2, y1_2_vec, y1_1_vec);

        // --- NEON 2nd derivatives ---
        let y0_prev_x2 = vaddq_s32(y0_prev, y0_prev); // 2 * y0_prev
        let y0_2_x2 = vaddq_s32(y0_2, y0_2);
        let y2_1 = vsubq_s32(vaddq_s32(y0_new, y0_2), y0_prev_x2);
        let y2_2 = vsubq_s32(vaddq_s32(y0_prev, y0_3), y0_2_x2);

        // --- NEON zero-crossing detection ---
        // y2_1 == 0 OR sign(y2_1) != sign(y2_2)
        let y2_1_zero: uint32x4_t = vceqq_s32(y2_1, zero);
        let y2_xored = veorq_s32(y2_1, y2_2);
        let opp_sign: uint32x4_t = vcltq_s32(y2_xored, zero);
        let zero_cross = vorrq_u32(y2_1_zero, opp_sign);

        // Fast path: if no lane has zero crossing, skip
        let any_cross = vmaxvq_u32(zero_cross);

        self.x += 1;

        if any_cross == 0 {
            return [EdgeResult::none(); 4];
        }

        // --- Scalar fallback for edge processing ---
        let cross_arr: [u32; 4] = std::mem::transmute(zero_cross);
        let y1_arr: [i32; 4] = std::mem::transmute(y1_final);
        let y2_1_arr: [i32; 4] = std::mem::transmute(y2_1);
        let y2_2_arr: [i32; 4] = std::mem::transmute(y2_2);

        let mut results = [EdgeResult::none(); 4];
        for lane in 0..4 {
            if cross_arr[lane] != 0 {
                results[lane] = self.edge_check_lane(lane, x, y1_arr[lane], y2_1_arr[lane], y2_2_arr[lane]);
            }
        }
        results
    }

    #[inline]
    fn calc_thresh_lane(&self, lane: usize) -> u32 {
        let thresh = self.y1_thresh[lane];
        if thresh <= THRESH_MIN || self.width[lane] == 0 {
            return THRESH_MIN;
        }
        let dx = (self.x << ZBAR_FIXED as u32).wrapping_sub(self.last_edge[lane]);
        let t = (thresh as u64 * dx as u64 / self.width[lane] as u64 / THRESH_FADE as u64) as u32;
        if thresh > t {
            let new_thresh = thresh - t;
            if new_thresh > THRESH_MIN {
                return new_thresh;
            }
        }
        THRESH_MIN
    }

    #[inline]
    fn edge_check_lane(&mut self, lane: usize, x: usize, y1: i32, y2_1: i32, y2_2: i32) -> EdgeResult {
        let thresh = self.calc_thresh_lane(lane);
        if thresh > y1.unsigned_abs() {
            return EdgeResult::none();
        }

        let y1_rev = if self.y1_sign[lane] > 0 { y1 < 0 } else if self.y1_sign[lane] < 0 { y1 > 0 } else { false };
        let mut result = EdgeResult::none();

        if y1_rev {
            result = self.process_edge_lane(lane, y1);
        }

        if y1_rev || self.y1_sign[lane].unsigned_abs() < y1.unsigned_abs() {
            self.y1_sign[lane] = y1;
            self.y1_thresh[lane] = ((y1.unsigned_abs() * THRESH_INIT) + ROUND) >> ZBAR_FIXED as u32;
            if self.y1_thresh[lane] < THRESH_MIN {
                self.y1_thresh[lane] = THRESH_MIN;
            }

            let d = y2_1 - y2_2;
            self.cur_edge[lane] = if d == 0 {
                1u32 << (ZBAR_FIXED as u32 - 1)
            } else if y2_1 != 0 {
                ((1i32 << ZBAR_FIXED) - ((y2_1 << ZBAR_FIXED) + 1) / d) as u32
            } else {
                1u32 << ZBAR_FIXED as u32
            };
            self.cur_edge[lane] += (x as u32) << ZBAR_FIXED as u32;
        }

        result
    }

    fn process_edge_lane(&mut self, lane: usize, _y1: i32) -> EdgeResult {
        if self.y1_sign[lane] == 0 {
            let init = (1u32 << ZBAR_FIXED as u32) + ROUND;
            self.last_edge[lane] = init;
            self.cur_edge[lane] = init;
        } else if self.last_edge[lane] == 0 {
            self.last_edge[lane] = self.cur_edge[lane];
        }

        self.width[lane] = self.cur_edge[lane].wrapping_sub(self.last_edge[lane]);
        self.last_edge[lane] = self.cur_edge[lane];

        EdgeResult { has_edge: true, width: self.width[lane] }
    }

    pub fn flush_lane(&mut self, lane: usize) -> EdgeResult {
        if self.y1_sign[lane] == 0 {
            return EdgeResult::none();
        }
        let x = (self.x << ZBAR_FIXED as u32) + ROUND;
        if self.cur_edge[lane] != x || self.y1_sign[lane] > 0 {
            let result = self.process_edge_lane(lane, -self.y1_sign[lane]);
            self.cur_edge[lane] = x;
            self.y1_sign[lane] = -self.y1_sign[lane];
            return result;
        }
        self.y1_sign[lane] = 0;
        self.width[lane] = 0;
        EdgeResult::none()
    }

    pub fn new_scan(&mut self) {
        for lane in 0..4 {
            while self.y1_sign[lane] != 0 {
                self.flush_lane(lane);
            }
        }
        self.reset();
    }
}
