# 핸드오프: NEON 스캐너 경로에 QR 파이프라인 통합

> 작성: 2026-05-13. 새 세션에서 이어서 진행.
> 선행 작업: `handoff-qr-port.md` / `handoff-qr-decoder.md` 의 B안
> (zbar QR 1:1 포팅) 은 완료된 상태 — 비-NEON 경로
> (`scan_image_parallel`, `decode`) 에서는 QR이 정상 디코드됨.
> 본 문서는 그 QR 파이프라인을 **NEON 진입점** 안으로 확장하는 작업.
> wire-printer 쪽 매칭 문서는 아직 없음 (이 작업은 wire-printer 코드 변경
> 없이 `libmasuri.so` + `masuri.kt` 재동기화만으로 끝난다 — 아래 §빌드/동기화 참고).

## 배경

wire-printer 운영자가 release APK를 Galaxy A31에 설치한 뒤
"QR이 인식 안 된다"고 보고. 진단 결과:

- 카메라 분석 프레임(640×480 YUV-Y → packed grayscale)을 `decodeBytes()`
  uniffi 진입점으로 넘기는 경로 자체는 정상.
- 분석 프레임 5장을 PGM으로 덤프해 호스트 CLI에 투입:
  | 호출 경로 | frame_4 결과 |
  |---|---|
  | `decode` (스칼라) | ✅ `HALFIN;210578897804;...` q=1 x=282 y=231 |
  | `decode_parallel` | ✅ 동일 |
  | `decode_neon_parallel` (= `--neon`) | ❌ no barcode found |

같은 입력 바이트, 같은 라이브러리, 다른 진입점. NEON 경로에서만 QR이 누락.

덤프 프레임 자체는 깨끗함 (frame_4 → `/tmp/wpdump/files/wp_frame_4_640x480.pgm`,
PNG 변환본 `/tmp/wpdump/frame_4.png` — 호스트에서만 보존, repo로 가져올
필요는 없음). 다른 4 프레임은 비-NEON 경로에서도 모두 실패 — QR 픽셀이
작아 단일 프레임 인식률이 낮은 건 별개 이슈고, NEON에서 0/5 vs 스칼라에서
1/5 라는 **차이 자체**가 본 작업의 회귀 대상.

## 진단 (직접 확인 완료)

`src/img_scanner.rs:454-458` 에 명시적으로 게이트가 잡혀 있다:

```rust
ScanTask::ScalarRow(y) => {
    ...
    // NEON path: QR finder lines emitted only by the scalar
    // fallback rows/cols. Phase 6 native-aarch64 QR is gated
    // behind `decode()` / `scan_image_parallel` which has the
    // full pipeline; the NEON entry stays 1D-only.
    let _ = hl;
    dcode.results
}
```

`ScanTask::ScalarCol(x)` 분기(같은 파일 538행)도 동일하게 `let _ = vl;`
로 finder lines를 버린다. NEON 4-lane 분기(`NeonRows`/`NeonCols`)는
finder line 자체를 수집하지 않는다 — `NeonScanner4` API에 hl/vl 출력 채널이
없음.

`scan_image_neon_parallel` 마지막 reduce 단계도 `DecodedSymbol`만 모으고
QR finder 클러스터링/디코딩 단계(`decode_qr_codes`)를 호출하지 않는다.

`lib.rs:82-92` 의 Android uniffi 진입점이 aarch64에서 이 NEON 함수를 직접
부르고 있어, Android 빌드는 구조적으로 QR을 절대 보지 못한다.

비교 — `scan_image_parallel` 의 `scan_at_scale` (같은 파일 169–185행 부근)
은 row/col task 결과에서 `(results, hl, vl)` 세 튜플을 모아 마지막에
`decode_qr_codes(gray, w, h, &mut hlines, &mut vlines)` 를 호출한다.
이게 NEON 경로에 빠진 파이프라인이다.

## 작업 범위 (C안 — NEON 진입점에 QR 통합)

다른 옵션(`decode_bytes`를 단순히 `scan_image_parallel`로 위임하는 A안 /
NEON 1D + 스칼라 QR 두 번 호출하는 B안)도 검토됐으나, **NEON 1D 가속을
유지하면서 QR finder도 같이 수집하는 게 정답**이라는 결론. 사용자
컨펌됨 (2026-05-13).

### 구현 가이드

1. `ScanTask::ScalarRow`/`ScalarCol` 분기 반환 타입을 확장.
   현재는 `Vec<DecodedSymbol>` 만 모음. `scan_image_parallel`의 `scan_at_scale`
   처럼 `(Vec<DecodedSymbol>, Vec<QrFinderLine>, Vec<QrFinderLine>)` 튜플로
   바꾸고 reduce 단계에서 hlines/vlines를 합산.
   - `NeonRows`/`NeonCols` 분기는 빈 hl/vl 벡터를 반환 (현재 NEON 4-lane
     `NeonScanner4`는 finder line을 내지 않음 — 이 작업에서 NeonScanner4를
     건드릴 필요는 없다, 짝수 행/열 4-lane은 1D만 잡고 홀수 fallback이
     QR finder를 잡으면 충분히 잡힌다).

2. reduce 후 `decode_qr_codes(gray, w as i32, h as i32, &mut hlines, &mut vlines)`
   를 호출해 QR 결과를 추가 (`scan_image_parallel`과 같은 패턴).

3. 마지막 `dedup_results` 는 1D 결과에만 적용. QR 결과는 1D와 다른 페이로드
   공간이라 dedup 후 `out.extend(qr_results)` 로 합치는 게 안전.

4. 다중 스케일은 일단 보류. `scan_image_parallel`은 50/75/100/110/150%를
   순회하지만, NEON 진입점에 같은 로직을 넣으면 카메라 frame당 비용이
   ~5배로 늘어난다. 1차 단일-스케일(100%)로 QR 회수율이 어디까지
   올라가는지 측정 후 결정 — wire-printer 운영자가 손 흔드는 카메라에서
   30fps로 돌리는 워크로드라, 단일 스케일이라도 같은 라벨을 여러 frame에
   걸쳐 잡으면 충분히 누적 인식됨. 다중 스케일을 도입할 거면 별도 PR로.

### 비-변경 영역

- `NeonScanner4` (`src/scanner_neon.rs`) — 손대지 않는다. 1D edge stream용
  SIMD primitives. QR finder는 zbar에서도 스칼라 경로(`zbar_scan_y` 후
  `qr_handler_fixup`) 로만 발생하는 부수효과라 SIMD 통합이 의미 없음.
- `decode_qr_codes` 본체 — 그대로 재사용.
- `lib.rs` uniffi `decode_bytes` — 시그니처/위임 대상 변경 없음. 그대로
  `scan_image_neon_parallel` 호출. NEON 함수가 QR도 반환하게 되니까.

## 검증

### 회귀 픽스처

wire-printer 운영자가 확보한 카메라 덤프가 호스트에 남아 있다 (이번 세션
한정 — repo로 가져올지는 사용자 판단):

```
/tmp/wpdump/files/wp_frame_0_640x480.pgm  → 비-NEON에서도 miss
/tmp/wpdump/files/wp_frame_1_640x480.pgm  → 비-NEON에서도 miss
/tmp/wpdump/files/wp_frame_2_640x480.pgm  → 비-NEON에서도 miss
/tmp/wpdump/files/wp_frame_3_640x480.pgm  → 비-NEON에서도 miss
/tmp/wpdump/files/wp_frame_4_640x480.pgm  → 비-NEON hit, NEON 현재 miss
```

frame_4를 `samples/korean-delivery/08_gs25_wp_frame4_640x480.pgm` (또는
유사 이름) 으로 코퍼스에 추가하는 걸 권장. PGM은 zbar 0.10 picture loader가
바로 읽는 표준 포맷이라 코퍼스 호환성 문제 없음.

### 합격 기준

```
target/release/masuri samples/korean-delivery/<frame4>.pgm --neon
  → HALFIN;210578897804;... [QR-Code] q>=1
```

추가로 기존 코퍼스(`samples/korean-delivery/01..07_*.jpeg`) 7장에 대해
`--neon` 결과가 비-`--neon` 대비 회귀 없는지 확인 (1D 회수 동일, QR
인식 추가).

벤치 — `./test_accuracy.sh` 또는 `--bench N` 으로 NEON 경로 throughput이
유의미하게 떨어지지 않는지(스칼라 fallback row/col 비율이 작은 frame에서
+1패스 클러스터링 오버헤드만 추가) 확인.

## 빌드 / wire-printer 동기화

masuri 측 변경 후:

```
cd ../masuri
cargo ndk --target aarch64-linux-android --platform 24 build --release --features android
cargo run --bin uniffi-bindgen --features android
```

uniffi 바인딩의 시그니처는 그대로지만(반환 타입 동일) **반드시 bindgen을
다시 실행**해서 .so와 .kt 의 checksum을 맞춰야 한다. 안 그러면 wire-printer
런타임에 uniffi checksum mismatch.

wire-printer 쪽은 손댈 게 없다 — `:app:syncMasuriArtifacts` Gradle task가
다음 `./gradlew assembleRelease` 호출 시 `../masuri` 의 산출물을
자동 동기화한다 (`wire-printer/app/build.gradle.kts:11-44`).

## 관련 파일

- `src/img_scanner.rs`
  - `scan_image_neon_parallel` (337–545) — 본 작업의 주 변경 대상
  - `scan_image_parallel` / `scan_at_scale` (93–185) — 참고할 reference 구현
  - `decode_qr_codes` (16–53) — 재사용할 QR 파이프라인 entry
- `src/lib.rs:82-92` — uniffi `decode_bytes` (변경 없음, 동작만 확장됨)
- `src/qrcode/finder.rs` — `QrFinderLine` 타입
- `src/scanner.rs` — `scan_single_row` / `scan_single_col` 의 hl/vl 출력
  채널 (이미 존재, 그냥 흘려보내고 있을 뿐)

## 결과 (2026-05-13 작업 완료분)

`src/img_scanner.rs` `scan_image_neon_parallel` 변경:

1. par_iter map 반환 타입을 `(Vec<DecodedSymbol>, Vec<QrFinderLine>, Vec<QrFinderLine>)` 튜플로 확장.
2. `ScalarRow`/`ScalarCol` 분기는 기존대로 hl/vl 채워서 반환.
3. **`NeonRows`/`NeonCols` 분기에 scalar QR-only 보조 패스 추가** (핸드오프 문서의
   §구현가이드 §1 가정 수정점). NEON 4-lane 1D는 그대로 두고, 같은 4 row/col에
   대해 scalar Scanner를 추가로 돌려 QR finder line을 수집한다. scalar 패스의
   1D 결과는 버린다 (1D는 NEON 결과만 사용).
4. reduce 단계에서 hlines/vlines 합산 후 `decode_qr_codes(gray, w as i32, h as i32, ..)`
   호출 → `dedup_results(&results)` 결과에 QR 추가.

### 가정 수정의 이유

핸드오프 §1 의 "짝수 행/열 4-lane은 1D만 잡고 홀수 fallback이 QR finder를
잡으면 충분" 은 잘못된 가정이었다. height/width 가 4의 배수면 fallback이
0개라 finder line 이 전혀 수집되지 않고, 4의 배수가 아니어도 fallback은
1~3 row/col 뿐이라 9 라인 임계(`decode_qr_codes` early-out)를 못 넘는다.
실측: frame_4 (640×480) 의 경우 모든 row/col scalar 패스를 거치면 hl 341 +
vl 350 = 691 라인이 수집되고 QR이 디코드되지만, fallback 만으로는 0/0 이라
인식 불가.

### 합격 검증

```
$ ./target/release/masuri samples/korean-delivery/08_gs25_wp_frame4_640x480.pgm --neon --scale 100
[NEON+rayon] 08_*.pgm: HALFIN;210578897804;V5M69;28;VV93;316;216; [QR-Code] q=1 x=283 y=232
```

`scale 100` 명시는 main.rs CLI 기본값이 82% 다운스케일이라 그렇다 — 분석
프레임이 640×480 → 524×393 으로 줄어들면 QR 픽셀이 더 작아져 단일 스케일
인식이 안 된다. wire-printer 의 실제 `decode_bytes` uniffi 진입점은 raw
camera frame을 그대로 전달하므로 (`src/lib.rs:82-92` 다운스케일 없음)
이 조건이 자동 만족된다.

코퍼스 회귀 (모두 `--scale 100`, `--neon`):
- 01–06: 1D 회수 변경 전과 동일 (NEON 4-lane 특성상 quality는 scalar
  대비 약간 낮음 — 본 작업 무관).
- 07 (jpeg): QR 인식 ✅ (변경 전 ❌).
- 08 (pgm): QR 인식 ✅ (회귀 픽스처).

### 1D 회귀 없음 확인 (2026-05-13)

`git stash` 로 본 작업 전후 비교. 02 같은 일부 케이스에서 NEON 100% 1D
누락이 있지만 stash 전후 동일 — 기존 NEON 4-lane 의 한계이며 본 작업의
회귀 아님. 별도 이슈로 추적 권장.

### 성능

frame_4 (640×480) NEON 경로 단발 평균 ~7ms (다운스케일 100%, M2 호스트).
변경 전 ~5ms 대비 +40% 정도 — NeonRows/NeonCols 마다 4 row/col 의 scalar
pass 가 추가된 비용. 30fps × ~7ms = ~21% CPU. wire-printer 의 카메라
스레드는 분석 프레임을 30fps 로 비동기 처리하므로 운영상 허용범위.
다중-스케일 도입은 본 PR 범위 밖.

### 빌드 / 동기화

핸드오프 §빌드/wire-printer 동기화 절차 그대로. uniffi 시그니처 무변경.

## 비고

- wire-printer 쪽에 임시 카메라 frame dump 코드가 들어가 있다
  (`app/src/main/kotlin/io/teammilestone/scan/scanner/Scanner.kt`의
  `debugDumpsRemaining` 블록 + `AndroidViewModel` 전환 + `MainActivity`의
  `viewModel()` 호출). 본 작업 검증이 끝나면 wire-printer 측에서 그
  덤프 코드를 되돌릴 예정이라 masuri 측 작업과는 무관.
- "왜 다중 스케일 없이 frame 1장만으로 인식되어야 하는가" 는 잘못된
  목표 설정 — wire-printer는 30fps × N초 동안 누적 인식하는 워크플로우라
  단일 frame 회수율이 20%여도 운영상 문제가 없다. 이 작업 합격 기준은
  "frame_4가 NEON 경로에서도 인식된다" 까지.
