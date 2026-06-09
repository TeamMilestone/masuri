# 핸드오프: Code 39 디코더 포팅

> 작성: 2026-06-10. 새 세션(이 masuri 리포지토리)에서 이어서 진행.
> 선행 사례 `docs/handoff-i25-port.md`와 동일한 1D 리니어 디코더 포팅이다.
> 그 문서의 절차를 그대로 따르면 된다.

## 배경

wire-printer(소비자: `dev@team-milestone.io`) 운영자가 일본發 EMS 소포의
송장 추적번호를 인식시키려 했으나 와이어프린터가 읽지 못했다. 와이어프린터는
카메라 프레임을 `uniffi.masuri.decodeBytes`에 넘기는 얇은 레이어이므로 인식
실패의 책임은 masuri에 있다.

원본 사진: `samples/japan-ems/01_ems_EN521985499JP_full.jpg`
(ground truth `EN521985499JP`).

## 진단 (완료)

1. **`cargo run --bin debug -- samples/japan-ems/01_ems_EN521985499JP_full.jpg`**
   → 실 결과 0건. 확대 배율에서 EAN-13 거짓양성(`0027881000080` 등)만, 추적번호와 무관.

2. **바코드만 타이트 크롭 + 90° 정렬**(`01_ems_EN521985499JP_crop.jpg`)해서 다시
   넣어도 **전 배율 0건.** → 위치/스케일/방향 문제가 아니라 **알고리즘 부재.**

3. **사람 읽는 글자 = `*EN521985499JP*`.** 양끝 `*`는 **Code 39**의 start/stop
   기호다. 일본우편 EMS S10 추적번호는 Code 39로 인쇄된다.

4. **zbarimg(레퍼런스) 비교** — full / crop 둘 다:
   ```
   CODE-39:EN521985499JP
   ```
   zbar은 풀프레임에서도 읽는다 → masuri 실패는 순수하게 **Code 39 디코더 부재.**

5. **masuri 현황**: `decoder/` 에 `ean.rs`, `code128.rs`, `i25.rs`만 마운트.
   `code39` 모듈 없음. 단,
   - `SymbolType::Code39 = 39` 은 enum에 이미 정의됨 (`src/lib.rs:48`).
   - `Display` 도 `"CODE-39"` 로 이미 구현됨 (`src/lib.rs:67`).
   - `ean.rs:577` 의 `i32_to_sym` 에 `39 => SymbolType::Code39` 매핑도 이미 있음.

   **즉 타입/표시/매핑은 전부 준비돼 있고, 빠진 건 디코더 본체와 hub 배선뿐이다.**

## 결정 (사용자 확인 완료)

- **Code 39 디코더를 포팅한다.** EMS 추적번호(13자 영숫자)가 대상.
- 와이어프린터 쪽은 영숫자를 허용하고 "끝 6자 소문자화 → `5499jp`" 포맷 규칙을
  넣는다. **이건 앱 작업이고 masuri 범위 밖.** (wire-printer 세션에서 별도 진행)

## 남은 작업 (masuri)

### 1. `src/decoder/code39.rs` 신규 작성

reference: zbar 0.23
- [code39.c](https://raw.githubusercontent.com/mchehab/zbar/0.23/zbar/decoder/code39.c)
- [code39.h](https://raw.githubusercontent.com/mchehab/zbar/0.23/zbar/decoder/code39.h)

**`src/decoder/i25.rs` 가 가장 가까운 참고 구현이다.** 구조를 그대로 미러링하라.

포트할 함수와 대응:

| C 원본 | Rust 매핑 | 비고 |
|--------|-----------|------|
| `code39_decoder_t` 구조체 | `pub struct Code39Decoder` | direction, element, character, s9, width, buf, config |
| `_zbar_decode_code39(dcode)` | `pub fn decode_code39(dcode: &mut Decoder) -> SymbolType` | hub의 `decode_width`에서 호출 |
| `code39_decode9(dcode)` | `fn code39_decode9` (private) | 9-element char (5 bar + 4 space), `decode_e(., 12)` |
| `code39_decode_start(dcode)` | `fn code39_decode_start` | start 문자 `*` 검출 + quiet zone |
| `code39_postprocess(dcode)` | `fn code39_postprocess` | full-ASCII 옵션은 EMS엔 불필요. 기본 charset만으로 충분 |
| `c39_acquire_lock` | 인라인 `dcode.get_lock(SymbolType::Code39)` | i25와 동일 lock 패턴 |

핵심 알고리즘 메모:
- Code 39는 **문자당 9 element**(5 bar + 4 space), 그 중 정확히 **3개가 wide**
  (3-of-9 → "Code 3 of 9"). inter-character gap(narrow space) 1개로 구분.
- `decode_e(e, s, n)` 의 **`n=12`** (9 elements 기준 모듈 합). i25는 45,
  Code128은 11. **헷갈리지 말 것** — i25 핸드오프에서도 같은 함정 경고됨.
- start/stop 문자는 `*` (인코딩 `0x094`). 데이터엔 `*` 안 나옴.
- charset: `0-9 A-Z - . SPACE $ / + %` (43 글자) + `*`.
- check digit(mod 43)은 zbar 기본 비활성(`CFG_ADD_CHECK` off). EMS S10도
  자체 check digit을 데이터에 포함하므로 **그대로 문자열로 통과**시키면 됨.
  → ground truth `EN521985499JP` 13자가 그대로 나와야 한다.
- min_len: zbar 기본 `CFG_MIN_LEN`=1. **거짓양성 억제 위해 6 이상으로 올릴 것
  권장**(i25 핸드오프와 동일 사유; Code 39도 parity가 약함).

### 2. `src/decoder/mod.rs` 수정 (i25와 완전 동일 패턴)

- 6행 근처: `pub mod code39;` 추가
- `Decoder` 구조체(43행 `pub i25:` 옆)에 `pub code39: code39::Code39Decoder,`
- `Decoder::new()`(66행), `reset()`(84행), `new_scan()`(94행)에 각각
  `code39` 초기화/리셋 추가 — i25 라인 바로 밑에 복붙 후 이름만 교체.
- `decode_width()` 의 i25 호출(206–212행) **바로 뒤**에:
  ```rust
  // Code 39 decoder
  if self.code39.enabled() {
      let sym = code39::decode_code39(self);
      if sym as i32 > SymbolType::Partial as i32 {
          self.sym_type = sym;
      }
  }
  ```
- `i32_to_sym`(ean.rs:577)은 **이미 `39 => Code39` 가 있으므로 손대지 않는다.**

### 3. 검증

```
cargo build --bin debug --release
./target/release/debug samples/japan-ems/01_ems_EN521985499JP_crop.jpg 2>&1 | grep CODE-39
./target/release/debug samples/japan-ems/01_ems_EN521985499JP_full.jpg 2>&1 | grep CODE-39
```
**합격 기준:**
- **필수**: crop 이미지에서 `CODE-39:EN521985499JP` 디코드 (단위 정확성).
- **목표**: full 사진에서도 디코드 (zbar은 읽으므로 기존 멀티오리엔테이션
  스캔이 잡아야 정상). 안 잡히면 그건 별개의 스캔 커버리지 이슈 —
  먼저 crop 합격부터 확보하고, full은 `scan_tiles`로 추가 진단.

### 4. 회귀 확인

기존 Code128/EAN/i25/QR 디코드가 안 깨지는지 빌드+기존 코퍼스로 확인.
`code128.rs`/`i25.rs`/`ean.rs`는 손대지 않으니 코드 레벨 회귀 위험은 낮다.
단 `decode_width`에 분기가 하나 늘어나므로 거짓양성이 늘지 않는지
`samples/korean-delivery/`(i25 코퍼스)로 교차 확인.

### 5. 안드로이드 재빌드 (wire-printer CLAUDE.md 절차)

```
cargo ndk --target aarch64-linux-android --platform 24 build --release --features android
cargo run --bin uniffi-bindgen --features android
```
산출물(`.gitignore`된 경로, wire-printer가 빌드 시점에 가져감):
- `android/jniLibs/arm64-v8a/libuniffi_masuri.so`
- `android/kotlin/uniffi/masuri/masuri.kt`

> uniffi 시그니처는 안 바뀐다(`Decoded`/`SymbolType` 그대로). 그래도 bindgen은
> 재실행할 것 — 빼먹으면 런타임 checksum mismatch.

## 빠른 시작 체크리스트 (새 세션 첫 5분)

1. `git log -3 --oneline` — masuri HEAD가 `e3f8b6a Phase 6 (NEON)…` 인지 확인.
2. `ls samples/japan-ems/` — full + crop + README.
3. `zbarimg --quiet samples/japan-ems/01_ems_EN521985499JP_crop.jpg`
   → ground truth `CODE-39:EN521985499JP` 확인 (없으면 `brew install zbar`).
4. `cargo run --bin debug -- samples/japan-ems/01_ems_EN521985499JP_crop.jpg`
   → 현재 0건. 이게 베이스라인.
5. `src/decoder/i25.rs` 정독 후 `src/decoder/code39.rs` 작성 시작.

## 위험 / 함정

- **`decode_e`의 n=12** (Code 39). i25(45)/Code128(11)과 다르다.
- **lock 타이밍**: i25/Code128 포트가 lock 시점 때문에 깨진 전례 있음
  (`docs/issues.md`). `i25_acquire_lock` 호출 시점을 그대로 미러링할 것.
- **full-ASCII 모드 불필요**: EMS는 대문자+숫자만. zbar의 full-ASCII
  postprocess(`<char> pairing`)는 구현 생략 가능. 단 charset에 `*`가 데이터로
  새어나오지 않게 start/stop 처리만 정확히.
- **거짓양성**: Code 39도 3-of-9 parity라 노이즈에서 잘 뜬다. masuri 자체
  `min_len`을 6 이상으로, wire-printer 쪽 q≥3 필터(`Scanner.kt`)가 2차 방어.
- **EMS S10 끝 두 글자는 발송국 코드**(`JP`). 와이어프린터에서 끝 6자
  (`5499JP`)를 잘라 쓰므로, masuri는 **13자 전체를 그대로** 돌려주면 된다.
  masuri에서 자르지 말 것.

## 참고 파일

- 가장 유사한 참고 구현: `src/decoder/i25.rs` (직전 1D 포팅)
- 디코더 hub / 새 디코더 끼우는 법: `src/decoder/mod.rs`
- 진단 도구: `src/bin/debug.rs`, `src/bin/scan_tiles.rs`
- 코퍼스: `samples/japan-ems/README.md`
- 선행 핸드오프: `docs/handoff-i25-port.md` (절차 동일)
