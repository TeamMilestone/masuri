# Japan Post EMS — Code 39 회귀 코퍼스

일본發 EMS 송장(S10 번호)의 추적 바코드. **Code 39**로 인쇄된다
(사람 읽는 글자 양끝 `*` start/stop 기호로 식별).

| 파일 | ground truth | 비고 |
|------|--------------|------|
| `01_ems_EN521985499JP_full.jpg` | `EN521985499JP` | 작업자 원본 사진 4000×3000. 저울 위 박스 + 비닐 파우치 송장. 바코드가 프레임에서 작고 90° 회전돼 있음 |
| `01_ems_EN521985499JP_crop.jpg` | `EN521985499JP` | 위 사진에서 바코드만 타이트 크롭 + 정렬한 깨끗한 1060×320. 디코더 단위 테스트용 |

## 레퍼런스 (zbar 0.23)

```
$ zbarimg --quiet 01_ems_EN521985499JP_full.jpg
CODE-39:EN521985499JP
$ zbarimg --quiet 01_ems_EN521985499JP_crop.jpg
CODE-39:EN521985499JP
```

zbar은 **풀프레임에서도** 읽는다 → masuri의 실패는 방향/스케일이 아니라
Code 39 디코더 부재가 원인. (`docs/handoff-code39-decoder.md` 참조)

## 현재 masuri 베이스라인 (Code 39 포팅 전)

```
$ cargo run --bin debug --release -- samples/japan-ems/01_ems_EN521985499JP_crop.jpg
Results: 0 (전 배율 0건)
```

S10 번호 형식: `[2 letters][9 digits][2 letters]`, 마지막 두 글자는 발송국
코드(`JP`). 9자리 중 끝 8번째는 check digit.
