//! Linear intensity scanner - edge detection via 2nd derivative zero-crossing.
//! Rust port of zbar/scanner.c
//! Original Copyright (C) 2007-2009 Jeff Brown <spadix@users.sourceforge.net>
//! LGPL-2.1-or-later

const ZBAR_FIXED: i32 = 5;
const ROUND: i32 = 1 << (ZBAR_FIXED - 1);
const ZBAR_SCANNER_THRESH_MIN: u32 = 4;
const EWMA_WEIGHT: u32 = ((0.78_f64 * ((1 << (ZBAR_FIXED + 1)) as f64) + 1.0) / 2.0) as u32;
const THRESH_INIT: u32 = ((0.44_f64 * ((1 << (ZBAR_FIXED + 1)) as f64) + 1.0) / 2.0) as u32;
const THRESH_FADE: u32 = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeType {
    None,
    Bar,
    Space,
}

pub struct Scanner {
    y1_min_thresh: u32,
    x: u32,
    y0: [i32; 4],
    y1_sign: i32,
    y1_thresh: u32,
    cur_edge: u32,
    last_edge: u32,
    width: u32,
}

/// Result from scanning a single pixel
pub struct ScanResult {
    pub edge: EdgeType,
    pub width: u32,
}

impl Scanner {
    pub fn new() -> Self {
        Scanner {
            y1_min_thresh: ZBAR_SCANNER_THRESH_MIN,
            x: 0,
            y0: [0; 4],
            y1_sign: 0,
            y1_thresh: ZBAR_SCANNER_THRESH_MIN,
            cur_edge: 0,
            last_edge: 0,
            width: 0,
        }
    }

    pub fn reset(&mut self) {
        self.x = 0;
        self.y0 = [0; 4];
        self.y1_sign = 0;
        self.y1_thresh = self.y1_min_thresh;
        self.cur_edge = 0;
        self.last_edge = 0;
        self.width = 0;
    }

    #[inline(always)]
    fn calc_thresh(&mut self) -> u32 {
        let thresh = self.y1_thresh;
        if thresh <= self.y1_min_thresh || self.width == 0 {
            return self.y1_min_thresh;
        }
        let dx = (self.x << ZBAR_FIXED as u32) - self.last_edge;
        let t = (thresh as u64 * dx as u64 / self.width as u64 / THRESH_FADE as u64) as u32;
        if thresh > t {
            let new_thresh = thresh - t;
            if new_thresh > self.y1_min_thresh {
                return new_thresh;
            }
        }
        self.y1_thresh = self.y1_min_thresh;
        self.y1_min_thresh
    }

    #[inline(always)]
    fn process_edge(&mut self, y1: i32) -> ScanResult {
        if self.y1_sign == 0 {
            self.last_edge = (1u32 << ZBAR_FIXED as u32) + ROUND as u32;
            self.cur_edge = self.last_edge;
        } else if self.last_edge == 0 {
            self.last_edge = self.cur_edge;
        }

        self.width = self.cur_edge.wrapping_sub(self.last_edge);
        self.last_edge = self.cur_edge;

        let edge = if y1 > 0 { EdgeType::Space } else { EdgeType::Bar };
        ScanResult {
            edge,
            width: self.width,
        }
    }

    /// Feed one pixel intensity value. Returns edge detection result.
    #[inline(always)]
    pub fn scan_y(&mut self, y: i32) -> ScanResult {
        let x = self.x as usize;
        let y0_1 = self.y0[(x.wrapping_sub(1)) & 3];
        let y0_0;

        if x > 0 {
            y0_0 = y0_1 + (((y - y0_1) * EWMA_WEIGHT as i32) >> ZBAR_FIXED);
            self.y0[x & 3] = y0_0;
        } else {
            self.y0 = [y; 4];
            self.x = 1;
            return ScanResult { edge: EdgeType::None, width: 0 };
        }

        let y0_2 = self.y0[(x.wrapping_sub(2)) & 3];
        let y0_3 = self.y0[(x.wrapping_sub(3)) & 3];

        // 1st differential @ x-1
        let mut y1_1 = y0_1 - y0_2;
        {
            let y1_2 = y0_2 - y0_3;
            if y1_1.unsigned_abs() < y1_2.unsigned_abs()
                && (y1_1 >= 0) == (y1_2 >= 0)
            {
                y1_1 = y1_2;
            }
        }

        // 2nd differentials
        let y2_1 = y0_0 - (y0_1 * 2) + y0_2;
        let y2_2 = y0_1 - (y0_2 * 2) + y0_3;

        let mut result = ScanResult { edge: EdgeType::None, width: 0 };

        // 2nd zero-crossing detection
        let zero_cross = y2_1 == 0
            || (if y2_1 > 0 { y2_2 < 0 } else { y2_2 > 0 });

        if zero_cross && self.calc_thresh() <= y1_1.unsigned_abs() {
            let y1_rev = if self.y1_sign > 0 { y1_1 < 0 } else { y1_1 > 0 };
            if y1_rev {
                result = self.process_edge(y1_1);
            }

            if y1_rev || self.y1_sign.unsigned_abs() < y1_1.unsigned_abs() {
                self.y1_sign = y1_1;

                // adaptive thresholding
                self.y1_thresh =
                    ((y1_1.unsigned_abs() * THRESH_INIT) + ROUND as u32) >> ZBAR_FIXED as u32;
                if self.y1_thresh < self.y1_min_thresh {
                    self.y1_thresh = self.y1_min_thresh;
                }

                // update current edge with interpolation
                let d = y2_1 - y2_2;
                self.cur_edge = if d == 0 {
                    1u32 << (ZBAR_FIXED as u32 - 1)
                } else if y2_1 != 0 {
                    let interp = ((y2_1 << ZBAR_FIXED) + 1) / d;
                    ((1i32 << ZBAR_FIXED) - interp) as u32
                } else {
                    1u32 << ZBAR_FIXED as u32
                };
                self.cur_edge += (x as u32) << ZBAR_FIXED as u32;
            }
        }

        self.x += 1;
        result
    }

    /// Flush scanner pipeline
    pub fn flush(&mut self) -> ScanResult {
        if self.y1_sign == 0 {
            return ScanResult { edge: EdgeType::None, width: 0 };
        }

        let x = (self.x << ZBAR_FIXED as u32) + ROUND as u32;

        if self.cur_edge != x || self.y1_sign > 0 {
            let result = self.process_edge(-self.y1_sign);
            self.cur_edge = x;
            self.y1_sign = -self.y1_sign;
            return result;
        }

        self.y1_sign = 0;
        self.width = 0;
        ScanResult { edge: EdgeType::None, width: 0 }
    }

    /// Start new scan line
    pub fn new_scan(&mut self) {
        while self.y1_sign != 0 {
            self.flush();
        }
        self.x = 0;
        self.y0 = [0; 4];
        self.y1_sign = 0;
        self.y1_thresh = self.y1_min_thresh;
        self.cur_edge = 0;
        self.last_edge = 0;
        self.width = 0;
    }

    pub fn get_width(&self) -> u32 {
        self.width
    }

    pub fn get_edge(&self, offset: u32, prec: i32) -> u32 {
        let edge = self.last_edge.wrapping_sub(offset).wrapping_sub(1u32 << ZBAR_FIXED as u32).wrapping_sub(ROUND as u32);
        let p = ZBAR_FIXED - prec;
        if p > 0 { edge >> p as u32 } else if p == 0 { edge } else { edge << (-p) as u32 }
    }
}
