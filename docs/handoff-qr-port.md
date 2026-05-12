# 핸드오프: zbar QR 디코더 직접 포팅 (B안 단독)

> 작성: 2026-05-12. 새 세션에서 이어서 진행.
> 결정 근거 / 선행 논의: `docs/handoff-qr-decoder.md` (A/B/C 비교).
> 본 문서는 **B안 단독 진행**으로 확정된 이후의 실행 계획.

## 배경

GS25 반값택배 라벨(`samples/korean-delivery/07_gs25_210578897804.jpeg`)이
QR 인코딩을 사용하지만 masuri는 1D 전용이다. `docs/handoff-qr-decoder.md`
에서 A안(rqrr 의존), B안(zbar 직접 포팅), C안(Android ML Kit) 중 B안을
채택했다.

채택 이유:
- masuri 전체 철학("zbar 1:1 포팅, 외부 의존성 0")과 일관성 유지.
- 1D 디코더 포팅(I2/5, EAN, Code128)에서 누적된 zbar 코드 이해도가
  큐레이션된 자산이라 QR도 같은 방식이 장기적으로 비용 우위.
- 운영 부담 — 외부 크레이트의 NDK aarch64 빌드 깨짐/유지보수 리스크가
  zbar 코드를 직접 들고 가는 것보다 크다고 판단.

비용: 약 6,761줄(zbar 0.10 qrcode/ 디렉터리 6,611줄 + img_scanner/scanner
QR 통합 ~150줄). 예상 10–14 작업 세션.

## 포팅 범위 (zbar-0.10 기준)

소스 경로: `/Users/wonsup-mini/Downloads/zbar-0.10/zbar/`

| 원본 파일 | 줄 수 | 역할 | 포팅 대상 Rust 파일 |
|---|---|---|---|
| `qrcode/qrdec.c` | 3956 | finder → grid → 데이터 디코딩 (메인) | `src/qrcode/qrdec.rs` (단계별 분리) |
| `qrcode/rs.c` | 799 | Reed-Solomon 에러 정정 | `src/qrcode/rs.rs` |
| `qrcode/binarize.c` | 639 | 적응적 이진화 | `src/qrcode/binarize.rs` |
| `qrcode/qrdectxt.c` | 394 | 페이로드 텍스트 변환 (ECI/kanji) | `src/qrcode/text.rs` |
| `qrcode/bch15_5.c` | 184 | format info BCH(15,5) 코드 | `src/qrcode/bch15_5.rs` |
| `qrcode/util.c` | 140 | isqrt/ilog 등 정수 유틸 | `src/qrcode/util.rs` |
| `qrcode/isaac.c` | 139 | PRNG (사용처 확인 필요) | TBD (Phase 0에서 결정) |
| `qrcode/*.h` | 360 | 헤더 | 각 .rs로 흡수 |
| `img_scanner.c` QR 훅 | ~120 | finder line 누적 + 최종 호출 | `src/img_scanner.rs` 패치 |
| `scanner.c`/`decoder` finder line 콜백 | ~30 | 1D 스캐닝 부산물로 finder 검출 | `src/scanner.rs`, `src/decoder/mod.rs` 패치 |
| **합계** | **~6,761줄** | | |

zbar의 구조적 특징: 1D 스캐너가 스캔라인을 훑으며 `1:1:3:1:1` finder
패턴을 부수적으로 검출한다. img_scanner가 이 라인들을 누적했다가
마지막에 `_zbar_qr_decode()`로 일괄 처리. **즉 QR은 별도 패스가 아니라
기존 1D 패스의 부산물을 재사용**한다. masuri도 동일 구조 유지.

## 확정된 결정 사항

- **베이스 버전**: zbar-0.10. I2/5/Code128 포팅과 동일 베이스라 finder line
  검출 콜백을 기존 1D 디코더에 끼워 넣기가 자연스럽다. (0.23 QR 변경점은
  Phase 0 끝에 따로 검토하지만 별도 작업으로 미룬다.)
- **Structured Append (다중 QR 합성)**: 스킵. GS25 라벨엔 불필요. zbar
  `qrdec.c`의 해당 블록은 stub으로 두고 single QR만 디코딩한다.
- **ECI / Kanji**: 최소 구현. Byte 모드 + UTF-8 + ASCII만 우선 지원.
  GS25는 ASCII만 사용하므로 충분. ECI/Shift_JIS는 placeholder.
- **외부 의존성**: 0 유지. `isaac.c` 사용처가 사소하면 포팅, 마스크 검증에
  본질적으로 필요하면 결정론적 카운터로 교체.
- **부동소수점 정책**: zbar 0.10은 정수 산술 위주. 동일하게 유지하여
  C와 비교 검증 시 비트단위 일치를 노린다.

## 사전 준비 (Phase 0 전에 1회만)

zbar C 라이브러리를 macOS에서 디버그 빌드해서, 중간값 비교를 위한 instrumented binary를 만들어 둔다. Phase 2~4의 단위 검증 속도가 결정된다.

```
cd /Users/wonsup-mini/Downloads/zbar-0.10
./configure --without-gtk --without-qt --without-python --without-imagemagick --enable-debug
make -C zbar
```

빌드 산출물에서 `qrdec.c` 안의 `_zbar_qr_decode` 진입/리턴에 printf로
중간값(detected finder count, format info bits, RS 결과 코드워드,
raw bitstream) 덤프하는 패치 파일을 따로 보관한다. Rust 측 동일 지점에서
같은 입력으로 같은 값이 나오는지 라인별 diff 가능해진다.

## 단계별 계획

### Phase 0: 의존 그래프 + 사용처 정리 (1세션)

목표: 본격 포팅 전 작업 순서 확정.

작업:
- `qrdec.c` 함수 60여 개의 호출 그래프 작성. 위상 정렬 → Phase 4의 하위
  단계 4a~4h 순서 검증.
- `isaac.c` 호출 위치 grep. 마스크 검증/finder 노이즈 처리 어디 쓰는지
  확인 → 드롭 / `rand` 크레이트 / 직접 포팅 셋 중 결정.
- `rs.c`가 GF(256) 외 다른 필드 다루는지 확인. QR 전용이면 단순화 여지.
- zbar 0.23 `qrcode/` 와 0.10 diff. 의미 있는 변경 있으면 별도 메모.

산출물:
- `docs/qr-port-deps.md` — 의존 그래프 + 포팅 순서표 + isaac 결정.

검증:
- Phase 1 들어가기 전 사용자 컨펌. 4h(Structured Append) 스킵 확정도
  여기서 다시 명문화.

### Phase 1: 인프라 + finder line 파이프 (1세션)

목표: 1D 스캐너에서 QR finder line이 어떻게 수집되는지만 먼저 연결.
포팅 자체는 안 함.

작업:
- `src/qrcode/mod.rs` 생성 (빈 모듈).
- `src/qrcode/finder.rs` 신규 — `QrFinderLine { pos: u32, len: u32, eoffs: [u32; 4] }` 등
  자료구조. zbar `qr_finder_line` 동등물.
- `src/scanner.rs` / `src/decoder/mod.rs` 에 1:1:3:1:1 패턴 검출 콜백.
  zbar `_zbar_decoder_get_qr_finder_line` 동등.
- `src/img_scanner.rs` 에 finder line 누적 벡터. 결과는 일단 디버그 출력만.
- `SymbolType::QrCode = 64` 추가 (zbar `ZBAR_QRCODE` 값과 동일).

검증:
- `cargo run --bin debug --release -- samples/korean-delivery/07_gs25_*.jpeg`
  → finder line 수십~수백 라인 누적 확인 (QR 라벨엔 finder 3개 × 스캔라인 다수).
- 1D 회귀: 6장 코퍼스 디코드율 유지. finder 콜백 추가가 기존 디코더에
  간섭하지 않는지 확인 (가장 큰 회귀 위험 지점).

### Phase 2: 산술 / 유틸 / RS (1–2세션)

목표: 디코더 본체 없이 단위 테스트 가능한 모듈부터.

작업:
- `qrcode/util.c` (140줄) → `src/qrcode/util.rs`. isqrt, ilog.
- `qrcode/bch15_5.c` (184줄) → `src/qrcode/bch15_5.rs`.
- `qrcode/rs.c` (799줄) → `src/qrcode/rs.rs`. GF(256) 테이블, RS decode.
- isaac (Phase 0 결정에 따라).

검증:
- 모듈별 `cargo test`. zbar 테스트 벡터가 있으면 그대로 차용. 없으면
  Phase 0의 instrumented C 빌드에서 동일 입력 동일 출력 확인.
- 본체 미구현 상태로도 끝나는 단계 — 빠르게 안정 마일스톤 확보.

### Phase 3: 이진화 (1세션)

작업:
- `qrcode/binarize.c` (639줄) → `src/qrcode/binarize.rs`. 적응적 임계값
  (블록 단위 평균 기반).
- 입력: 그레이스케일 슬라이스. 출력: 비트맵.

검증:
- 07 샘플 이진화 결과를 PNG로 덤프해 zbar C 동일 함수 출력과 픽셀 단위
  비교. 99% 이상 일치 목표(JPEG 노이즈로 1% 미만 차이는 허용).

### Phase 4: 코어 디코더 `qrdec.c` (4–6세션, 최대 위험)

3,956줄을 하위 단계로 분할. 각 하위 단계마다 중간 산출물 검증.

- **4a. 자료구조 (~400줄)**: `QrReader`, `QrFinder`, `QrAff` (affine 변환),
  `QrHom` (homography). 메모리 레이아웃만 옮기고 함수는 stub.
- **4b. Finder 클러스터링 (~500줄)**: 1D 라인들을 finder 3개 그룹으로 묶기.
  검증: 07 샘플에서 finder center 3개 좌표가 zbar C와 ±1px 일치.
- **4c. Alignment pattern + version 결정 (~400줄)**.
  검증: version 추출 결과 동일.
- **4d. Homography + 그리드 샘플링 (~700줄)**.
  검증: 추출된 비트 그리드를 PNG로 덤프 → zbar C 결과와 비교.
- **4e. Format info + mask 디코딩, 데이터 비트 추출 (~500줄)**.
  검증: format info 비트 패턴 일치.
- **4f. RS 디코딩 호출, 코드워드 → 비트스트림 (~400줄)**.
  검증: raw 코드워드 / RS 정정 후 비트스트림 동일.
- **4g. 페이로드 모드별 추출 (~600줄)**: numeric / alphanumeric / byte /
  kanji. 검증: 07 샘플 `HALFIN;...` 텍스트 완전 일치.
- **4h. Structured Append (~400줄)**: **스킵**. stub만.

위험 분기: 4a 끝났는데 4b~4d의 자료구조 결합도가 너무 높아 단계 검증이
불가능해 보이면 **빅뱅 포팅(전체 한 번에 옮기고 끝)** 으로 전환 가능성.
판단 시점은 4a 종료 후.

### Phase 5: 텍스트 변환 (1세션)

작업:
- `qrcode/qrdectxt.c` (394줄) → `src/qrcode/text.rs`.
- Byte 모드 + ASCII/UTF-8 우선. ECI/Shift_JIS 최소.

검증:
- 07 샘플 `HALFIN;210578897804;V5M69;28;VV93;316;216;` 완전 일치.

### Phase 6: 통합 + 회귀 + NDK 빌드 (1세션)

작업:
- `decode_bytes`에 QR 패스 결과 머지 (1D 결과 + QR 결과).
- `Decoded` 구조체에 QR symbol type 흘림.
- 안드로이드 NDK 재빌드:
  ```
  cargo ndk --target aarch64-linux-android --platform 24 build --release --features android
  cargo run --bin uniffi-bindgen --features android
  ```
- 1D 코퍼스 6장 회귀: 디코드율 유지.
- QR 코퍼스 1장(07_gs25): 디코드 성공.

검증:
- `android/jniLibs/arm64-v8a/libuniffi_masuri.so` 생성.
- `.kt` 바인딩에 `QrCode` enum variant 포함.
- 1D 디코드율 비회귀.

## wire-printer 측 작업 (별도 핸드오프)

masuri Phase 6 통과 후 wire-printer에서 별도 작업이 필요하다. 현재
`Scanner.kt:174-178`의 필터(`it.data.all(Char::isDigit)`)가 `HALFIN;...`
페이로드를 자동 차단한다. 필요한 변경:

1. `SymbolType.QrCode` 분기 추가. QR 경로는 페이로드를 `;`로 split.
2. GS25 포맷 가드: `parts[0] == "HALFIN" && parts.size >= 2`.
3. `parts[1]` 을 운송장 후보로 추출 → 기존 길이/숫자 가드를 추출값에 적용.
4. Ledger key/인쇄 hero는 12자리 운송장 그대로 사용 (1D 경로와 동일
   키 공간 — dedup 안전).

상세는 본 작업 완료 후 `wire-printer/docs/handoff-qr-decoder.md`로
별도 작성.

## 위험 / 함정

- **qrdec.c 결합도**: 3,956줄이 finder 클러스터링부터 텍스트 추출까지
  한 파일에 엉켜 있다. Phase 0의 의존 그래프가 깔끔히 안 그려지면
  4a~4g의 단계별 검증이 어려워지고, 빅뱅 포팅으로 전환해야 한다.
- **정수 산술 vs 부동소수점**: zbar 0.10은 정수 위주. 안일하게 `f64`로
  바꾸면 중간값 비트 일치 검증이 깨진다. 모든 산술은 원본 타입 보존.
- **Finder line 콜백의 1D 회귀**: Phase 1에서 가장 큰 위험. 1D 디코더
  내부 상태 머신에 finder 검출 코드를 추가할 때 기존 상태 전이가
  바뀌지 않도록 주의. 코퍼스 회귀 테스트로 즉시 검출.
- **NDK aarch64 빌드 호환성**: 본질적으로 순수 Rust 포팅이라 큰 문제는
  없을 가능성. 정수 오버플로우 동작이 디버그/릴리스에서 다를 수 있으니
  `wrapping_*` / `saturating_*` 명시.
- **이미지 stride/row-padding**: `decode_bytes`가 받는 `Vec<u8>`이 packed
  (stride == width) 가정. wire-printer가 YUV `Y` 평면을 어떻게
  추출해서 넘기는지 확인 필요. padded라면 binarize 전에 packed 복사.
- **GS25 외 택배사 QR**: 현재는 GS25(HALFIN)만 들어왔다. 다른 택배사가
  QR을 쓰면 페이로드 포맷이 다르다. masuri는 raw 페이로드만 반환하면
  되고, 포맷 가드는 wire-printer 측에서 점진 확장.

## 빠른 시작 체크리스트 (새 세션 첫 5분)

1. `git log -3 --oneline` — `f1cecb0` (I2/5 포팅) 이후 커밋 상태 확인.
2. `ls /Users/wonsup-mini/Downloads/zbar-0.10/zbar/qrcode/` — 8개 .c + 7개 .h
   존재 확인. 없으면 zbar-0.10 tarball 재배포.
3. `zbarimg samples/korean-delivery/07_gs25_210578897804.jpeg` →
   `HALFIN;210578897804;V5M69;28;VV93;316;216;` 이 ground truth.
4. `cargo run --bin debug --release -- samples/korean-delivery/07_gs25_*.jpeg`
   → 현재 0건이 베이스라인. Phase 6 끝나면 QrCode 1건이어야 함.
5. 현재 진행 중인 Phase 확인 (커밋 로그 또는 본 문서 갱신본).

## 참고 파일

- 결정 근거: `docs/handoff-qr-decoder.md` (A/B/C 비교 — B 채택)
- 직전 1D 포팅: `docs/handoff-i25-port.md` (I2/5 작업 패턴 참고)
- 포팅 가이드: `docs/porting-guide.md` (zbar QR 보류 사유 30행)
- 코퍼스: `samples/korean-delivery/README.md` (07 GS25 항목)
- zbar 0.10 소스: `/Users/wonsup-mini/Downloads/zbar-0.10/zbar/qrcode/`
- wire-printer 측 통합: 본 작업 완료 후 별도 핸드오프
