# 핸드오프: QR 디코더 추가 (GS25 반값택배 지원)

> 작성: 2026-05-12. 새 세션에서 이어서 진행.
> wire-printer 쪽 매칭 문서: `wire-printer/docs/handoff-qr-decoder.md` (아직
> 미작성 — 이 문서가 먼저 합의되면 그쪽도 같은 형식으로 만든다).
>
> **2026-05-12 결정 갱신: B안(zbar 직접 포팅) 단독 진행 확정.** 실행
> 계획은 `docs/handoff-qr-port.md` 참고. 본 문서는 A/B/C 비교 근거로
> 남겨 둔다 (이 아래의 "A안 기준" 작업 항목은 폐기됨).

## 배경

I2/5 포팅(`handoff-i25-port.md`, 커밋 `f1cecb0` 기준 완료)으로 롯데/cupost
운송장은 인식되는 상태. 2026-05-12 운영자(`dev@team-milestone.io`)가
KakaoTalk으로 새 라벨 1장을 추가 전송: **GS25 반값택배**.

샘플 원본: `wire-printer/samples/KakaoTalk_Photo_2026-05-12-07-48-23.jpeg`
(아직 미커밋 — 작업 시작 전 `samples/korean-delivery/07_gs25_210578897804.jpeg`
형식으로 코퍼스에 추가 필요)

이 라벨의 운송장은 **중앙 QR + 우측 가장자리 세로 1D 바코드**의 이중 인코딩
형태(샘플 사진 기준 — 좌측 가장자리에는 1D 패턴이 식별되지 않음, 다른 GS25
라벨에서 양쪽 모두 인쇄되는지는 미확인):

```
zbarimg <sample>:
  QR-Code: HALFIN;210578897804;V5M69;28;VV93;316;216;
  QR-Code: HALFIN;210578897804;V5M69;28;VV93;316;216;
```

zbar는 QR만 잡았고 우측 세로 1D는 못 읽음(흐림/각도 추정). 따라서 GS25는
사실상 **QR만 신뢰 가능한 경로**다.

## 진단 (간단 — 이번엔 알고리즘 진단할 게 없다)

masuri는 1D 전용이다. `src/scanner.rs` + `src/img_scanner.rs`는 스캔라인
기반(가로/세로 1D 스윕)이고 `decoder/mod.rs`는 width-stream 멀티플렉서다.
2D 코드는 구조적으로 통과시킬 길이 없음.

`docs/porting-guide.md` 30행에 명시된 대로 zbar QR 코드(`zbar/qrcode/*`,
3,956줄)는 의도적으로 포팅 보류 상태.

## 결정해야 할 것 (사용자 컨펌 필요)

QR 추가 방식 3안. **권장: A안 (rqrr 크레이트 의존).**

### A. `rqrr` 크레이트 의존 (권장)
- 순수 Rust QR 디코더, MIT, 잘 유지됨.
- API: `PreparedImage::prepare(luma_view)` → `detect_grids()` → 각 grid에
  `decode()` 호출. 그레이스케일 슬라이스를 그대로 받는다.
- 통합 지점: `lib.rs::decode_bytes` 안에서 기존 1D 파이프라인과 **병렬로**
  QR 패스를 한 번 더 돌리고 결과를 머지.
- 장점: 신규 코드 200줄 미만. android NDK 빌드 검증만 하면 됨.
- 단점: zbar 1:1 포팅 철학에서 벗어남. 외부 의존성 1개 추가.

### B. zbar QR 디코더 직접 포팅
- `zbar/qrcode/*` 전체를 Rust로 옮긴다. ~3,956줄.
- 장점: 의존성 0, 철학 일관성.
- 단점: 수 주 분량 작업. 본 핸드오프 1턴으로 끝낼 일 아님.

### C. Android ML Kit (자바 측에서 QR 처리)
- masuri 손대지 않고 wire-printer Scanner.kt에서 Google ML Kit
  `BarcodeScanning` 으로 QR만 디코드 후 기존 ledger 경로에 흘려 넣음.
- 장점: masuri 무수정.
- 단점: 카메라 프레임을 2개 디코더에 동시 공급해야 함(레이턴시/배터리),
  Google Play Services 의존, 디코더 일관성 파편화.

**갱신**: B안으로 확정. 아래 "남은 작업 (A안)" 섹션은 폐기. 실제 실행
계획은 `docs/handoff-qr-port.md`로 이전됨.

## 남은 작업 (A안)

### 1. masuri 쪽 (이 리포지토리)

#### 1-1. `Cargo.toml` 의존성 추가
```toml
[dependencies]
rqrr = "0.7"  # 최신 안정 버전 확인 후
```
android feature gate은 불필요(rqrr은 OS-agnostic 순수 Rust). NDK aarch64
타깃에서 빌드되는지 1-5단계에서 확인.

#### 1-2. `SymbolType`에 `QrCode` 추가
`src/lib.rs`:
```rust
pub enum SymbolType {
    // ...
    Code128 = 128,
    QrCode = 64,   // zbar의 ZBAR_QRCODE 값과 동일
}
```
`Display` impl과 `dedup_results`(있다면)의 match arm에도 동시 추가.

#### 1-3. `src/decoder/qr.rs` 신규 작성 (얇은 래퍼)
```rust
pub fn scan_qr(gray: &[u8], width: u32, height: u32) -> Vec<Decoded> {
    let img = rqrr::PreparedImage::prepare_from_greyscale(
        width as usize, height as usize, |x, y| gray[y * width as usize + x],
    );
    img.detect_grids().into_iter().filter_map(|g| {
        let mut out = String::new();
        g.decode(&mut out).ok()?;
        let (x, y) = g.bounds[0].into();  // 좌상단 코너
        Some(Decoded {
            data: out,
            sym_type: SymbolType::QrCode,
            quality: 10,  // QR은 RS 에러 정정 내장 — 통과하면 고품질로 간주
            x: x as u32,
            y: y as u32,
        })
    }).collect()
}
```
(API 시그니처는 rqrr 0.7 기준 추정 — 실제 빌드 시 조정.)

#### 1-4. `lib.rs::decode_bytes` 에서 QR 패스 머지
```rust
#[cfg(feature = "android")]
#[uniffi::export]
pub fn decode_bytes(gray: Vec<u8>, width: u32, height: u32) -> Vec<Decoded> {
    let mut out = /* 기존 1D 파이프라인 */;
    out.extend(decoder::qr::scan_qr(&gray, width, height));
    out
}
```
1D와 QR을 한 번씩 돌리고 합친다. dedup은 `data` 문자열 기준으로 한 번 더
거를 수 있지만 — QR과 1D는 페이로드가 다르므로(아래 wire-printer 항목
참조) 그대로 둬도 충돌 없음.

`decode()` / `decode_parallel()` (비-android 경로)도 동일하게 머지.

#### 1-5. 안드로이드 NDK 재빌드 검증
CLAUDE.md(wire-printer) 절차:
```
cd /Users/wonsup-mini/projects/masuri
cargo ndk --target aarch64-linux-android --platform 24 build --release --features android
cargo run --bin uniffi-bindgen --features android
```
**합격 기준: 빌드 성공 + `android/jniLibs/arm64-v8a/libuniffi_masuri.so`
생성. `.kt` 바인딩에 `QrCode` enum variant 포함.**

rqrr가 NDK aarch64에서 빌드 실패하면 거기서 멈추고 B안/C안 재논의.

#### 1-6. 코퍼스 등록
1. wire-printer 운영자 사진을 `samples/korean-delivery/` 로 복사:
   ```
   cp wire-printer/samples/KakaoTalk_Photo_2026-05-12-07-48-23.jpeg \
      samples/korean-delivery/07_gs25_210578897804.jpeg
   ```
2. `samples/korean-delivery/README.md` 에 새 항목 추가 — symbol type은
   QR, ground truth는 위 zbarimg 전체 페이로드, 운송장 추출치는
   `210578897804`.
3. 디코드 확인:
   ```
   cargo run --bin debug --release -- samples/korean-delivery/07_gs25_210578897804.jpeg
   ```
   기대: `QrCode q=10 data=HALFIN;210578897804;V5M69;28;VV93;316;216;`

#### 1-7. 회귀 확인
1D 6장(I2/5 기존 코퍼스) 디코드율이 떨어지지 않는지 확인. QR 패스가
1D 패스 결과에 간섭하지 않음 — 별도 단계이므로 회귀 위험은 낮지만
빌드 후 한번 돌려본다.

### 2. wire-printer 쪽

**중요: 현행 `Scanner.kt` 필터로는 QR 페이로드가 자동 차단된다.** 다음
필터 모두 통과 못함:
- `it.data.all(Char::isDigit)` — `HALFIN;...` 은 알파+세미콜론
- `it.data.length >= MIN_PAYLOAD_DIGITS (=10)` — 통과는 하지만 무의미

필요한 변경(별도 wire-printer 핸드오프에서 상세화 예정):

1. `SymbolType.QrCode` 분기 추가. QR일 경우 페이로드를 `;`로 split.
2. **GS25 포맷 가드**: `parts[0] == "HALFIN" && parts.size >= 2` 인지 확인.
   다른 택배사 QR(있다면 URL이나 EDI 포맷)은 일단 드롭. 운영자가 다른
   QR을 보내오면 그때 포맷 추가.
3. `parts[1]` 을 운송장 후보로 사용 → 기존 `it.data.all(Char::isDigit)` +
   `length >= 10` 가드를 이 추출 값에 동일하게 적용.
4. Ledger key / 인쇄 hero는 이 12자리 운송장을 그대로 사용 (I2/5 경로와
   동일 키 공간 — 같은 라벨을 양쪽으로 읽어도 dedup 안전).

`Scanner.kt:174-178` 의 필터 체인은 1D 경로 그대로 유지하고, QR 분기는
그 앞쪽에 별도 처리로 분리하는 게 깔끔하다.

## 빠른 시작 체크리스트 (새 세션 첫 5분)

1. `git log -3 --oneline` — `f1cecb0` 이후 커밋 상태 확인.
2. `ls samples/korean-delivery/` — 07 항목이 이미 있는지 확인. 없으면 1-6 먼저.
3. `zbarimg samples/korean-delivery/07_gs25_210578897804.jpeg` →
   `HALFIN;210578897804;V5M69;28;VV93;316;216;` 가 ground truth.
4. `cargo run --bin debug --release -- samples/korean-delivery/07_gs25_210578897804.jpeg`
   → 현재는 0건. 이게 베이스라인.
5. `Cargo.toml` 에 rqrr 추가하고 `src/decoder/qr.rs` 작성 시작.

## 위험 / 함정

- **rqrr 버전 호환**: 0.7 시리즈가 latest 가정인데 작업 시점에 확인.
  `prepare_from_greyscale` API 시그니처가 마이너 버전에서 흔들린다.
- **NDK aarch64 빌드 실패**: rqrr 내부에 SIMD/플랫폼 의존이 있으면 NDK에서
  안 붙을 가능성. 빌드 실패 시 즉시 보고하고 B/C 재논의.
- **이미지 stride/row-padding**: `decode_bytes`가 받는 `Vec<u8>` 이 packed
  (stride == width) 가정. wire-printer가 YUV `Y` 평면을 어떻게 추출해서
  넘기는지 확인 — padded라면 rqrr에 넘기기 전에 packed로 복사 필요.
- **QR이 여러 개 잡힐 수 있음**: 이 라벨처럼 양쪽에 같은 QR이 두 개
  찍힌 케이스가 흔하다. `decode_bytes` 결과에 같은 data가 2개 들어가도
  wire-printer ledger가 dedup으로 흡수하므로 masuri 측에서 별도 처리
  불필요. 단 `quality` 비교 시 첫 번째 것만 채택되도록 wire-printer 측
  `maxByOrNull { it.quality }` 가 알파벳 페이로드에도 동작하는지 한 번 더 확인.
- **GS25 외 택배사 QR**: 현재는 GS25(HALFIN)만 들어왔다. CJ, 롯데, 우체국
  등이 QR을 쓰면 페이로드 포맷이 전부 다르다. 포맷 추가는 운영자가
  새 샘플 보낼 때마다 wire-printer 측에서 가드 추가하는 식으로
  점진 확장. masuri는 raw 페이로드만 반환하면 끝.
- **rqrr가 회전된 QR 처리**: rqrr는 finder pattern 기반 회전 보정 내장.
  카메라가 비스듬한 경우도 대응되어야 정상. 안 되면 카메라 측 회전 힌트
  활용 검토.

## 참고 파일

- 직전 핸드오프 (I2/5 포팅): `docs/handoff-i25-port.md`
- wire-printer 측 I2/5 핸드오프: `wire-printer/docs/handoff-i25-decoder.md`
  (자릿수 가드 패턴의 참고가 됨)
- 포팅 가이드: `docs/porting-guide.md` 30행 — zbar QR을 의도적으로 보류한
  근거가 적혀 있음.
- rqrr 크레이트: https://crates.io/crates/rqrr
- wire-printer CLAUDE.md — 재빌드 절차 + 운영자 컨텍스트
