# 핸드오프: I2/5 (Interleaved 2 of 5) 디코더 포팅

> 작성: 2026-05-12. 새 세션에서 이어서 진행.

## 배경

wire-printer (소비자: `dev@team-milestone.io`) 현장 운영자가 2026-05-07에
"라벨 6장이 인식이 안 된다"고 KakaoTalk으로 사진 6장을 보냈다. 모두
한국 택배 라벨 (롯데택배 5장, cupost 1장). 와이어프린터는 카메라 프레임을
`uniffi.masuri.decodeBytes`에 넘기고 결과를 필터링하는 얇은 레이어이므로
인식 실패의 책임은 사실상 masuri에 있다.

샘플 원본: `wire-printer/samples/` (ff880c0 커밋)
정규화 사본: `masuri/samples/korean-delivery/` (e797086 커밋)

## 진단 (완료, 커밋됨)

1. **`cargo run --bin debug --release -- <sample>`** 으로 6장 모두 디코더 통과 →
   Code128 결과 0건 또는 무관한 단편 1건 (`462`).
   EAN-13 q=1 거짓양성만 다수 (wire-printer `Scanner.kt` q≥3 필터가 잘 막고 있음).

2. **`src/bin/scan_tiles.rs`** (이번에 추가) — 다중 스케일 × 다중 타일 위치로
   브루트포스 슬라이딩 윈도우 → 여전히 Code128 0건.
   결론: **위치/스케일 문제가 아니라 알고리즘 문제.**

3. **`zbarimg`(레퍼런스 구현)** 비교:
   ```
   01_cupost_460335055375.jpeg : I2/5:460335055375 ✓
   02_lotte_263733057943.jpeg  : I2/5:263733057943 ✓
   03_lotte_410663725441.jpeg  : I2/5:410663725441 ✓
   04_lotte_409695576625.jpeg  : I2/5:409695576625 ✓
   05_lotte_263733045004.jpeg  : (decoder도 실패 — blur/skew 한계, 코퍼스에 의도적 잔존)
   06_lotte_536788954376.jpeg  : I2/5:536788954376 ✓
   ```

   진단 결론:
   - **운송장 바코드는 전부 `I2/5` (Interleaved 2 of 5).** Code128 아님.
   - 동반된 작은 보조 코드(`169`, `462`)는 Code-39.
   - masuri는 `decoder/{ean,code128}.rs`만 마운트 → I2/5/Code-39 둘 다 부재.
     `SymbolType::I25 = 25`는 enum에 이미 정의되어 있으나 디코더 모듈이 없음.

## 결정 (사용자 확인 완료)

- **I2/5 디코더만 포팅한다.** Code-39는 운송장 번호가 아니므로 인쇄 키로
  부적절 (3자리). wire-printer 측에서 길이 가드(≥10)로 막을 것이므로
  무시한다.
- 동시에 wire-printer `Scanner.kt`에 자릿수 가드를 추가해 운송장이 아닌
  짧은 숫자 코드가 ledger key/인쇄로 흘러가는 것을 방지한다.

## 남은 작업

### 1. masuri 쪽 (이 리포지토리)

#### 1-1. `src/decoder/i25.rs` 신규 작성
reference: [zbar 0.23 i25.c](https://raw.githubusercontent.com/mchehab/zbar/0.23/zbar/decoder/i25.c)
+ [i25.h](https://raw.githubusercontent.com/mchehab/zbar/0.23/zbar/decoder/i25.h)
(둘 다 `/tmp/i25.c`, `/tmp/i25.h`에 받아둠 — 캐시 만료되면 재다운로드)

포트할 함수와 대응:

| C 원본 | Rust 매핑 | 비고 |
|--------|-----------|------|
| `i25_decoder_t` 구조체 | `pub struct I25Decoder` | direction, element, character, s10, width, buf[4], config |
| `_zbar_decode_i25(dcode)` | `pub fn decode_i25(dcode: &mut Decoder) -> SymbolType` | mod hub의 `decode_width`에서 호출 |
| `i25_decode1(enc, e, s)` | `fn i25_decode1` (private) | `decode_e(e, s, 45)` 사용 — 기존 `decoder::decode_e` 재사용 |
| `i25_decode10(dcode, offset)` | `fn i25_decode10` | parity 체크, weighted decode |
| `i25_decode_start(dcode)` | `fn i25_decode_start` | quiet zone 검증 후 lock 안 잡음 (Code128과 동일 패턴) |
| `i25_decode_end(dcode)` | `fn i25_decode_end` | min/max len 체크 (CFG 매크로 → Rust에서는 상수로) |
| `i25_acquire_lock(dcode)` | 인라인 처리 | masuri의 `dcode.get_lock(SymbolType::I25)` |

핵심 알고리즘 메모:
- I2/5는 두 자리 단위(쌍)로 디코드. 5개 바와 5개 스페이스가 인터리브.
- `s10`은 10-element 슬라이딩 윈도우 합 (`s6` Code128 패턴과 동일 구조).
- 각 character는 9-bit pattern `enc`로 인코딩. 8개 width를 `decode_e(., 45)`로
  양자화 → 5-bit `enc`. 그 중 정확히 2비트가 wide여야 parity 통과.
- 바이너리 weight: 0,1,2,4,7 → 0=12, 1=1, 2=2, 4=4, 7=7 ... (zbar 원본 참고)
- min_len/max_len: zbar 기본은 6/0(무제한). 운송장 12자리니까 그대로 유지.

#### 1-2. `src/decoder/mod.rs` 수정
- `pub mod i25;` 추가
- `Decoder` 구조체에 `pub i25: i25::I25Decoder,` 필드
- `Decoder::new()`, `reset()`, `new_scan()`에 i25 처리 추가
- `decode_width()` 끝부분에 EAN/Code128 사이 또는 이후로 I2/5 호출:
  ```rust
  if self.i25.enabled() {
      let sym = i25::decode_i25(self);
      if sym as i32 > SymbolType::Partial as i32 {
          self.sym_type = sym;
      }
  }
  ```
- `dedup_results`의 match arm에 `25 => SymbolType::I25` 추가

#### 1-3. 검증
```
cargo build --bin debug --release
for f in samples/korean-delivery/*.jpeg; do
  ./target/release/debug "$f" 2>&1 | head -5
done
```
**합격 기준: 6장 중 ≥4장에서 파일명 ground truth와 일치하는 I2/5 결과 디코드.**
샘플 05는 zbar도 실패하니까 제외해도 됨. 즉 4/5 또는 5/5.

#### 1-4. 회귀 확인
기존 Code128 회귀가 깨지지 않는지 확인. (기존 `test_accuracy.sh`가 가리키는
`../samples/`는 로컬에 없음 — 수정한 `code128.rs`는 손대지 않으니 코드 레벨
회귀 위험은 낮음. 그래도 빌드는 통과해야.)

#### 1-5. 안드로이드 재빌드
CLAUDE.md(wire-printer) 절차 그대로:
```
cd /Users/alfonso/projects/masuri
cargo ndk --target aarch64-linux-android --platform 24 build --release --features android
cargo run --bin uniffi-bindgen --features android
```
산출물:
- `android/jniLibs/arm64-v8a/libuniffi_masuri.so`
- `android/kotlin/uniffi/masuri/masuri.kt`

(둘 다 `.gitignore`된 경로. wire-printer가 빌드 시점에 가져감.)

### 2. wire-printer 쪽

`wire-printer/docs/handoff-i25-decoder.md` 참조.

## 빠른 시작 체크리스트 (새 세션 첫 5분)

1. `git log -3 --oneline` — 마지막 커밋이 `e797086 Add Korean delivery label
   regression corpus and tile scanner` 인지 확인.
2. `ls samples/korean-delivery/` — 6 jpeg + README.
3. `zbarimg samples/korean-delivery/01_cupost_460335055375.jpeg` — ground truth
   로 `460335055375` 출력 확인 (zbar 설치 안 됐으면 `brew install zbar`).
4. `cargo run --bin debug --release -- samples/korean-delivery/01_cupost_460335055375.jpeg`
   → 현재는 Code128 0건. 이게 베이스라인.
5. `src/decoder/i25.rs` 작성 시작.

## 위험 / 함정

- **i25 lock 시점**: zbar 원본에서 `i25_acquire_lock`은 `i25_decode_end`와
  `decode_i25` 본문 두 군데에서 호출 (character==4에서 한 번, 끝낼 때 다시).
  Code128 포트가 lock 타이밍 때문에 한 번 깨졌던 적이 있음
  (`docs/issues.md` 참조). 두 호출 시점 모두 반드시 보존할 것.
- **decode_e 매개변수**: I2/5는 `n=45` (45 = 9 elements × 5 modules). Code128은
  `n=11`. 헷갈리지 말 것.
- **direction**: I2/5는 forward(space-first)와 reverse(bar-first) 둘 다 지원.
  reverse는 buf를 뒤집어야 함 (zbar 원본 169행). Code128과 동일 패턴.
- **min_len 기본값**: zbar 0.23 기본 `CFG_MIN_LEN`은 i25에 대해 6. 우리는
  6 그대로 둬도 운송장(12자리)에는 영향 없음. 단 6보다 짧은 노이즈 디코드를
  걸러주므로 그대로 유지 권장.
- **i25는 거짓양성이 잘 나는 심볼**이다 (parity가 2-of-5밖에 안 됨). 멀티
  스케일/멀티 라인 q≥3 필터가 wire-printer 측에 이미 있지만, masuri 자체적으로
  `min_len` 게이트는 6 이상으로 유지.

## 참고 파일

- masuri 기존 Code128 포트: `src/decoder/code128.rs` — 가장 유사한 참고 구현
- masuri 디코더 hub: `src/decoder/mod.rs` — 새 디코더를 어떻게 끼우는지
- 진단 도구: `src/bin/scan_tiles.rs` (e797086에서 추가)
- 코퍼스 README: `samples/korean-delivery/README.md`
