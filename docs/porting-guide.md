# Masuri 포팅 가이드

zbar C 라이브러리를 Rust로 포팅한 과정과 판단 근거를 기록한다.

## 포팅 대상 선정

zbar-0.10(원본)과 zbar-0.23(최신) 소스 코드를 분석하여, 바코드 디코딩에 필요한 **최소 핵심 모듈만** 선별 포팅했다.

### 포팅한 것

| 원본 파일 | Rust 파일 | 줄 수 | 포팅 기반 버전 |
|-----------|-----------|-------|---------------|
| `zbar/scanner.c` | `src/scanner.rs` | 311 → 180 | 0.10 |
| `zbar/decoder.c` + `decoder.h` | `src/decoder/mod.rs` | 401 + 206 → 175 | 0.10 + 0.23 |
| `zbar/decoder/ean.c` + `ean.h` | `src/decoder/ean.rs` | 642 + 84 → 555 | 0.10 |
| `zbar/decoder/code128.c` + `code128.h` | `src/decoder/code128.rs` | 516 + 49 → 420 | **0.23** |
| `zbar/img_scanner.c` | `src/img_scanner.rs` | 783 → 320 | 0.10 + 신규 |
| (신규) | `src/scanner_neon.rs` | 210 | 신규 |

### 포팅하지 않은 것

| 원본 | 이유 |
|------|------|
| `zbar/decoder/code39.c` | 샘플에 Code39 바코드 없음 |
| `zbar/decoder/i25.c` | 샘플에 I2/5 바코드 없음 |
| `zbar/decoder/pdf417.c` | 2D 코드, 우선순위 낮음 |
| `zbar/decoder/codabar.c` | 0.23에서 추가된 것, 샘플에 없음 |
| `zbar/decoder/code93.c` | 0.23에서 추가된 것, 샘플에 없음 |
| `zbar/decoder/databar.c` | 0.23에서 추가된 것, 29KB로 가장 큼 |
| `zbar/qrcode/*` | QR 디코더 전체 (3,956줄), 별도 프로젝트 규모 |
| `zbar/convert.c` | 이미지 포맷 변환, `image` 크레이트로 대체 |
| `zbar/video.c`, `window.c` | 비디오 캡처/디스플레이, 불필요 |
| `zbar/processor.c` | 비디오+스캐너 통합, 불필요 |
| `zbar/symbol.c`, `image.c` | C 메모리 관리(refcount, 재활용 풀), Rust 소유권으로 대체 |

## 포팅 전략

### 1단계: 0.10 기반 직접 포팅

zbar-0.10 소스를 1:1로 Rust로 번역했다. C의 구조체 → Rust struct, 함수 포인터 → 직접 호출, `malloc/free` → `Vec`, `refcnt` → Rust 소유권.

**결과: 8/71 인식 (11%)**

### 2단계: 0.23 차이점 분석 및 반영

zbar-0.23 소스를 비교 분석하여 Code128 디코더의 핵심 변경사항을 반영했다.

**Code128에서 0.10 → 0.23 주요 변경:**

1. **지연된 lock 획득**
   - 0.10: start 문자 발견 즉시 `get_lock()` → `character = 0`
   - 0.23: start 저장만 하고 `character = 1` → 첫 데이터 문자에서 `acquire_lock()`
   - 이유: 잘못된 start 감지 시 불필요한 lock 점유 방지

2. **width variance 체크 (신규)**
   ```rust
   let dw = abs_diff(self.width, self.s6);
   if dw * 4 > self.width {
       // 폭 변동이 25% 초과 → 거부
   }
   ```
   - 노이즈나 이미지 결함으로 인한 false positive 필터링

3. **`start`/`width` 필드 추가**
   - `start: u8` — start 문자를 저장해두고 lock 획득 후 buf[0]에 배치
   - `width: u32` — 문자 간 폭 변화를 추적하여 일관성 검증

**결과: 여전히 8/71** — Code128 로직은 맞았으나 다른 문제가 있었다.

### 3단계: 디버깅 — 핵심 버그 발견

성공/실패 이미지를 비교 디버깅한 결과, `decode6()` → `calc_check()` 호출에서 **character 값의 비트 마스킹 타이밍** 오류를 발견했다.

**버그:**
```rust
// 잘못된 코드 (0x7f로 이미 마스킹한 값을 calc_check에 전달)
fn decode_lo(sig: i32) -> i8 {
    ...
    (CHARACTERS[idx] & 0x7f) as i8  // ← 여기서 0x80 비트 제거
}

fn decode6() {
    let c = decode_lo(sig);  // c에는 이미 0x80 비트가 없음
    let chk = calc_check(CHARACTERS[c as usize]);  // ← 다시 테이블 조회 (잘못됨)
}
```

**수정:**
```rust
// C 원본과 동일하게: raw 값 반환 → calc_check에서 0x80 비트로 검증 → 최후에 & 0x7f
fn decode_lo(sig: i32) -> i16 {
    ...
    CHARACTERS[idx] as i16  // raw 값 반환 (0x80 비트 유지)
}

fn decode6() {
    let c = decode_lo(sig);  // raw character (0x80 비트 포함)
    let chk = calc_check(c as u8);  // 0x80 비트로 bar 검증 가능
    ...
    c & 0x7f  // 최종 반환 시에만 마스킹
}
```

C에서는 `signed char`가 `characters[]` 테이블 값(0x00~0xCB)을 담으면 자연스럽게 음수/양수로 구분되지만, Rust에서 `i8`로 변환하면서 0x7f 마스킹을 너무 일찍 적용한 것이 원인이었다.

`calc_check()`는 `c & 0x80` 여부로 bar width 검증 기준값(0x10 vs 0x20 vs 0x18)을 결정하는데, 마스킹 후에는 항상 `0x80 == 0`이 되어 모든 문자가 `0x18` 기준으로 검증되었다. 이로 인해 실제로는 유효한 문자가 bar 검증에서 탈락했다.

**결과: 68/71 인식 (96%)** — C zbar의 66/71(93%)을 초과

### 4단계: rayon 병렬화

각 스캔 라인이 완전히 독립적이므로, `rayon::par_iter`로 행/열 스캔을 병렬화했다.

```rust
row_indices.par_iter().map(|&y| {
    let mut scn = Scanner::new();    // 라인별 독립 인스턴스
    let mut dcode = Decoder::new();
    // ... 스캔 ...
    dcode.results
}).collect()
```

C zbar는 단일 scanner/decoder를 공유하며 순차 처리하지만, Rust에서는 라인별로 독립 인스턴스를 생성하여 lock 경합 없이 병렬 처리한다.

**결과: 3.3x → 6.6x (C 대비)**

### 5단계: NEON SIMD

4개 스캔 라인의 EWMA + 미분 + 영교차 검출을 `int32x4_t`로 동시 처리하는 `NeonScanner4`를 구현했다.

```rust
// 4개 라인의 EWMA를 하나의 NEON 명령어로 처리
let diff = vsubq_s32(pixel_vec, y0_prev);     // 4개 차이
let weighted = vmulq_s32(diff, ewma_w);        // 4개 가중치 곱
let y0_new = vaddq_s32(y0_prev, shifted);      // 4개 갱신

// 영교차 검출도 NEON
let zero_cross = vorrq_u32(y2_1_zero, opp_sign);
if vmaxvq_u32(zero_cross) == 0 {
    // 빠른 경로: 4개 라인 모두 엣지 없음 → 스칼라 코드 스킵
}
```

**구조:**
- EWMA, 1차/2차 미분, 영교차 검출 → NEON (항상 실행)
- 임계값 비교, 엣지 처리, 보간 → 스칼라 (영교차 발생 시에만)

**결과: ~9% 추가 개선 (6.6x → 7.2x)**

NEON의 효과가 제한적인 이유:
- 스캔 연산이 전체 시간의 ~30%만 차지 (나머지는 디코더 + 이미지 로딩)
- 영교차 발생 시 스칼라 폴백
- 4개 행의 픽셀이 `width` 바이트 간격으로 떨어져 캐시 효율 저하

## C → Rust 포팅 시 주의사항

이 프로젝트에서 겪은 C/Rust 차이점들:

### 1. signed char 오버플로우
C의 `signed char`(-128~127)는 0x80 이상 값에서 음수가 되지만, Rust `i8`도 동일하게 동작한다. 다만 **비트 마스킹 타이밍**이 중요하다. C에서 `char c = table[idx]; ... c & 0x7f`처럼 나중에 마스킹하는 패턴을 Rust로 옮길 때, 중간에 `i8`로 캐스팅하면서 정보가 손실될 수 있다.

### 2. unsigned 산술 underflow
C의 `unsigned char E = (expr - 3) / 2`에서 `expr < 3`이면 underflow로 큰 값이 되고, 범위 검사(`E >= n-3`)에서 걸린다. Rust에서는 debug 모드에서 패닉하므로, `wrapping_sub`를 쓰거나 사전 검사를 추가해야 한다.

### 3. 구조체 내 상호 참조 (borrow checker)
C의 `decoder` 구조체는 `ean` 서브구조체를 포함하며, `decode_pass(dcode, &dcode->ean.pass[i])`처럼 전체와 부분을 동시에 참조한다. Rust에서는 `&mut dcode`와 `&mut dcode.ean.pass[i]`를 동시에 빌릴 수 없으므로, pass를 `clone()`하여 독립적으로 처리한 후 다시 저장하는 방식으로 우회했다.

### 4. 메모리 관리 제거
C zbar의 symbol 재활용 풀(recycle_bucket), refcount, 수동 malloc/free는 모두 제거했다. Rust의 `Vec<DecodedSymbol>`로 결과를 수집하고, 함수 스코프를 벗어나면 자동 해제된다. 이로 인해 코드가 크게 단순화되었다.

### 5. 콜백 패턴 → 직접 수집
C zbar는 `symbol_handler` 함수 포인터 콜백으로 디코딩 결과를 전달한다. Rust에서는 `Decoder` 구조체에 `results: Vec<DecodedSymbol>`을 두고, 디코딩 성공 시 직접 push하는 방식으로 단순화했다.

## 파일별 포팅 상세

### scanner.rs ← scanner.c

`zbar_scan_y()` 함수를 `Scanner::scan_y()` 메서드로 포팅.

- `struct zbar_scanner_s` → `struct Scanner`
- 고정소수점 연산(`ZBAR_FIXED=5`) 그대로 유지
- EWMA 가중치(`0.78`), 임계값 초기화(`0.44`), 감쇠율(`8`) 동일
- `process_edge` → `ScanResult { edge, width }` 반환
- `zbar_scanner_flush` → `Scanner::flush()`
- `zbar_scanner_new_scan` → `Scanner::new_scan()`

### decoder/mod.rs ← decoder.c + decoder.h

- `struct zbar_decoder_s` → `struct Decoder`
- `w[16]` bar width 순환 버퍼 동일
- `zbar_decode_width()` → `Decoder::decode_width()`
- 각 심볼로지 디코더를 순차 호출 (EAN → Code128)
- `get_width()`, `calc_s()`, `decode_e()` 헬퍼 함수 동일 로직
- `get_lock()`/lock 해제 패턴 유지

### decoder/ean.rs ← decoder/ean.c

- `ean_pass_t` → `EanPass`, `ean_decoder_t` → `EanDecoder`
- 4개 병렬 디코딩 패스(`pass[4]`) 구조 유지
- `digits[]`, `parity_decode[]` 룩업 테이블 동일
- `decode4()` → `decode4()`, `aux_start()` → `aux_start()` 등 1:1 대응
- `integrate_partial()` → `integrate_partial()` (좌/우 반쪽 결합)
- 체크섬 검증(`ean_verify_checksum`) 동일 로직
- EAN-13 → UPC-A, ISBN-10, ISBN-13 부분집합 판별 유지

### decoder/code128.rs ← decoder/code128.c (0.23)

0.23 버전 기반 포팅. 0.10과의 차이점은 위의 "2단계" 참조.

- `characters[108]`, `lo_base[8]`, `lo_offset[128]` 테이블 동일
- `decode_lo()`, `decode_hi()` → 엣지 시그니처 → 문자 코드 매핑
- `decode6()` → 6-element 문자 디코딩 (i16 반환으로 변경)
- `validate_checksum()` → 역방향 누적합 검증
- `postprocess()` → 문자 집합(A/B/C) 해석, ASCII 변환
- `postprocess_c()` → Code Set C 확장 (2자리 숫자 쌍)

### img_scanner.rs ← img_scanner.c + 신규

C의 이미지 순회 로직을 기반으로 하되, 병렬화를 위해 재설계.

- `zbar_scan_image()` → `scan_image()` (단일 스레드), `scan_image_parallel()` (rayon), `scan_image_neon_parallel()` (NEON+rayon)
- 행 스캔: 좌→우 + 우→좌 지그재그
- 열 스캔: 상→하 + 하→상 지그재그
- C의 `symbol_handler` 콜백 → `Decoder.results` 직접 수집
- C의 심볼 중복 제거(quality 누적) → `HashMap` 기반 dedup
- C의 EAN 품질 필터(`quality < 3` 제거) → dedup에서 자연 처리
- C의 symbol 재활용 풀 → 불필요 (Rust 메모리 관리)

### scanner_neon.rs (신규)

C zbar에 없는 신규 모듈. `std::arch::aarch64` NEON intrinsics 사용.

- `NeonScanner4` — 4개 스캔 라인을 `int32x4_t`로 동시 처리
- EWMA: `vsubq_s32` + `vmulq_s32` + `vshrq_n_s32` + `vaddq_s32`
- 1차 미분 스무딩: `vabsq_s32` + `vcltq_s32` + `veorq_s32` + `vbslq_s32`
- 2차 미분: `vaddq_s32` + `vsubq_s32`
- 영교차: `vceqq_s32` + `vcltq_s32` + `vorrq_u32` + `vmaxvq_u32`
- 빠른 경로: `vmaxvq_u32(zero_cross) == 0`이면 4개 라인 모두 스킵
- 느린 경로: 영교차 발생 라인만 스칼라 `edge_check_lane()` 실행
