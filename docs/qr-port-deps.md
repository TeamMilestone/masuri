# QR 포팅 의존 그래프 + 순서 (Phase 0 산출물)

> 작성: 2026-05-12. `docs/handoff-qr-port.md`의 Phase 0 산출물.
> 분석 대상: `/Users/wonsup-mini/Downloads/zbar-0.10/zbar/qrcode/` (총 6,611 줄)

## 1. 외부 노출 함수 (img_scanner.c가 호출)

`qrdec.c` 진입점 5개 + 결과 리스트 관리 2개:

| 함수 | 위치 | 역할 |
|---|---|---|
| `_zbar_qr_create()` | qrdec.c:84 | `qr_reader` 할당 (`isaac_init`, `rs_gf256_init`) |
| `_zbar_qr_destroy()` | qrdec.c:92 | 정리 |
| `_zbar_qr_reset()` | qrdec.c:105 | 프레임 사이 finder line 누적 초기화 |
| `_zbar_qr_found_line()` | qrdec.c:3875 | 1D 스캐너가 검출한 finder line 누적 |
| `_zbar_qr_decode()` | qrdec.c:3912 | 메인 디코드 진입점 (binarize → finder → 디코드) |
| `qr_code_data_list_init/clear()` | qrdec.c (말미) | 결과 리스트 관리 |

masuri 측 동등물: `src/qrcode/mod.rs`에 `QrReader` 구조체로 흡수. C에서는 `qr_reader*` 포인터 + 함수, Rust에서는 `impl QrReader`.

## 2. 함수 그룹 + 의존 순서

핸드오프의 4a~4h 분류를 의존 그래프 결과로 재정렬:

### 그룹 A — 기하 (Geometry, ~600줄)
순수 함수. 다른 그룹에 의존 없음. 가장 먼저 포팅.
- `qr_point_*` (translate, distance, CCW)
- `qr_line_*` (eval, fit, intersect)
- `qr_aff_*` (affine: init, unproject)
- `qr_hom_*` (homography: init, unproject, cell ops)

### 그룹 B — Finder 검출 + 클러스터링 (~1300줄)
1D 라인 누적 → finder 3개 그룹 + RANSAC + 변환 적합. **얽혀 있어 분할 불가** — 핸드오프의 4b/4c/4d 일부를 한 묶음으로.
- `qr_finder_vline_cmp`, `cluster_lines`, `qr_finder_find_crossings`
- `qr_finder_centers_locate` → isaac
- `qr_finder_ransac` (qrdec.c:1033) — Phase B의 `qr_finder_edge_pts_aff_classify`를 호출 (**역의존**)
- `qr_hom_fit` (qrdec.c:1909) → ransac + line_fit_finder_*
- `qr_finder_estimate_module_size_and_version`
- `qr_finder_version_decode`, `fmt_info_decode` → bch15_5, bch18_6

### 그룹 C — 그리드 샘플링 (~700줄)
B 끝난 뒤에만 가능.
- `qr_alignment_pattern_fetch/search`
- `qr_sampling_grid_init` → `qr_hom_cell_init`
- `qr_sampling_grid_sample` → `qr_data_mask_fill`
- `qr_samples_unpack`

### 그룹 D — 디코드 (~1000줄)
- `qr_code_decode` → `rs_correct` (외부 — 본격 호출은 RS 포팅 끝나야)
- `qr_code_data_parse` (qrdec.c:3293, ~200줄) — numeric/alphanumeric/byte/kanji **+ structured append inline**

### 그룹 E — 오케스트레이션 (~400줄)
- `_zbar_qr_decode` → `qr_reader_match_centers` → `qr_reader_try_configuration`

## 3. 결합도와 포팅 전략 — **수정 권고**

핸드오프 문서는 Phase 4를 4a~4g로 잘게 쪼개려 했으나, 의존 그래프 분석 결과:

- `qr_finder` 구조체가 B→C→D를 관통한다. 같은 `extent` 필드를 RANSAC inlier 표시로도, affine 분류로도 재사용.
- `qr_finder_ransac`이 Phase B의 helper(`qr_finder_edge_pts_aff_classify`)를 호출 → **위상 정렬 불가능 구간 존재**.

**수정안**: Phase 4를 4단계가 아닌 **3 모듈**로 재편.

| 신규 단계 | 매핑 | 줄 수 | 검증 지점 |
|---|---|---|---|
| 4-G (Geometry) | 핸드오프 4a의 기하 부분 | ~600 | 단위 테스트만 (입출력 비교) |
| 4-F (Finder+Transform) | 핸드오프 4b+4c+4d 통합 | ~1300 | 07 샘플 finder 좌표 ±1px 일치 |
| 4-D (Decode+Parse) | 핸드오프 4e+4f+4g | ~1500 | 코드워드 → 비트스트림 → 텍스트 |

빅뱅 전환 임계점: 4-F 안에서도 ransac↔classify 역의존 때문에 한 번에 옮기는 것이 안전. 4-F는 한 PR/한 세션에 끝내는 것을 목표로 한다.

## 4. ISAAC PRNG 결정 — **1:1 포팅** (2026-05-12 컨펌)

근거 (qrdec.c:79, 1034, 1059–1060):
- 초기화 시 zero seed (`isaac_init(&reader->isaac, NULL, 0)`) → 이미 결정론적.
- 호출처: `qr_finder_ransac` 안에서 0..n 범위 인덱스 2개 추출.

분석 단계에서는 카운터 대체를 권고했으나, **사용자 결정으로 1:1 포팅 채택**. 이유: zbar C와 비트 단위 일치 검증 유지. Phase 0~Phase 4-F 사이 중간값 diff가 가능해 디버깅 비용이 절감됨.

→ `isaac.c` 139줄 + `isaac.h` 41줄 = `src/qrcode/isaac.rs`로 포팅. Phase 2 예산에 포함.

## 5. RS 결정 — **그대로 포팅**

근거 (rs.h:32–39, rs.c:30–42):
- `rs_gf256` 단일 구조체. log/exp 256/511 테이블.
- 다른 갈루아 필드 다루지 않음. Primitive polynomial은 `QR_PPOLY=0x1D` 1개.
- `_nerrors<=4` 경로가 QR에서 항상 성립하지만, 단순화 이득 ~50줄 미만이라 위험 대비 이득 부족.

→ rs.c 799줄 1:1 포팅. 단, `rs_gf256_init`의 primitive polynomial 파라미터는 `QR_PPOLY` 상수로 고정 호출.

## 6. zbar 0.10 vs 0.23 diff — **추후 검토**

핸드오프 결정대로 Phase 6 이후 별도 검토. 본 작업은 0.10 베이스로 진행.

## 7. Structured Append — **전체 구현** (2026-05-12 컨펌)

`qr_code_data_parse`(qrdec.c:3293) 안에 mode 4의 한 분기로 inline되어 있음. 분석 단계에서는 분기 가드로 스킵 권고했으나, **사용자 결정으로 전체 구현 채택**. 이유: inline 구조라 가드 처리도 본문 이해가 선행돼야 하고, 어차피 본문 포팅이 필요. 향후 다중 QR을 쓰는 택배사가 들어와도 마스리 측 변경 불필요.

→ qrdec.c의 structured append 블록(~400줄)도 포팅. Phase 4-D 예산에 포함.

## 8. 수정된 포팅 순서 + 의존성

```
Phase 1: 인프라 (finder line 누적)
  ↓
Phase 2: util.rs, bch15_5.rs, rs.rs
  ↓                    ↓
Phase 3: binarize.rs   ↓
  ↓                    ↓
Phase 4-G: 기하 모듈 (qr_point/line/aff/hom)
  ↓
Phase 4-F: finder + transform (한 PR)
  ↓
Phase 4-D: decode + parse (한 PR, structured append guard)
  ↓
Phase 5: text.rs (qrdectxt.c)
  ↓
Phase 6: 통합 + 회귀 + NDK
```

## 9. 새 예산

| 단계 | 줄 수 | 추정 세션 |
|---|---|---|
| Phase 1 | ~150 (스캐너 패치) | 1 |
| Phase 2 (util+bch+rs, isaac 제외) | 944 | 1–2 |
| Phase 3 (binarize) | 639 | 1 |
| Phase 4-G (geom) | ~600 | 1 |
| Phase 4-F (finder+transform) | ~1300 | 2–3 |
| Phase 4-D (decode+parse) | ~1500 | 2–3 |
| Phase 5 (text) | 394 | 1 |
| Phase 6 (통합+NDK) | ~150 (img_scanner 훅) | 1 |
| **합계** | **~5,677줄** | **10–14** |

(원본 핸드오프 6,761에서 isaac 180줄 + img_scanner 일부 절감)

## 10. Phase 1 진입 전 컨펌 필요 사항

핸드오프 문서를 다음과 같이 갱신해야 한다:

1. **Phase 4 재편**: 4a~4h → 4-G / 4-F / 4-D로 변경 (위 8번 참조).
2. **ISAAC 스킵 확정**: `src/qrcode/`에 isaac 모듈 만들지 않음. 결정론적 카운터로 대체.
3. **Structured Append**: 함수 분리가 아닌 분기 가드. 디코드 시도 시 mode 4 만나면 그 QR만 포기하고 다음 후보로.
4. **0.23 diff**: Phase 6 이후로 미룸. 본 작업 범위 밖.

위 4건 사용자 컨펌 후 Phase 1 진행.
