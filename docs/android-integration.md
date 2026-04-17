# Android 통합 가이드

Android 앱(특히 Kotlin/Java)에서 `masuri`를 호출하기 위해 필요한 변경사항과 구조 설계.

대상 사용처: `wire-printer` 앱 (Galaxy S22, aarch64-linux-android). 카메라 프레임 → grayscale → `decode()` → 결과 오버레이.

## 현재 상태 점검

| 항목 | 값 | 비고 |
|---|---|---|
| `crate-type` | `["cdylib", "rlib"]` | ✅ `.so` 생성 가능 |
| 공개 API | `decode(gray, w, h) -> Vec<Decoded>` | 순수 Rust, C ABI 없음 |
| NEON SIMD | `#[cfg(target_arch = "aarch64")]` | ✅ aarch64-linux-android에서 그대로 동작 |
| `rayon` | required | Android JVM/ART와 별개 쓰레드 풀, 프레임당 오버헤드 검토 필요 |
| `image = "0.25"` | required | decode hot path에선 불필요. 피처 게이트 권장 |
| `pyo3` | optional (`python`) | Android와 무관, 기본 off라 문제 없음 |

**핵심 차이**: Kotlin은 Rust 함수를 직접 호출할 수 없다. **C ABI를 경유한 JNI** 바인딩이 반드시 필요하다.

## 바인딩 방식 선택

세 가지 옵션. `masuri`의 표면이 작아서 어느 쪽을 택해도 코드량은 비슷하다.

| 방식 | 장점 | 단점 |
|---|---|---|
| **수작업 `extern "C"` + 수작업 JNI** | 의존성 0, 동작 투명, 빌드 단순 | JNI 코드 2곳 유지 (Rust + Kotlin) |
| **`jni` crate (Rust 쪽 JNI)** | Kotlin 쪽 `System.loadLibrary` + `native fun` 선언만 | `jni` crate가 JNIEnv 수명 관리 추가 |
| **`uniffi-rs`** | IDL 한 번 작성 → Kotlin 바인딩 자동 생성, 유지보수 최고 | 툴체인 한 단계 추가, 빌드 파이프라인 복잡도 ↑ |

**권장: `uniffi-rs`**. masuri 공개 API가 이미 데이터 타입 중심(`Decoded` 구조체, `SymbolType` enum)이라 IDL 맵핑이 깔끔하게 떨어진다. 향후 타 모바일 플랫폼(iOS)까지 재사용 가능.

## 필요한 변경사항

### 1. Cargo.toml: Android feature와 uniffi 추가

```toml
[features]
default = []
python = ["pyo3"]
android = ["uniffi"]                    # 신규

[dependencies]
# 기존 유지
uniffi = { version = "0.28", optional = true, features = ["cli"] }

[build-dependencies]
uniffi = { version = "0.28", features = ["build"], optional = true }

[lib]
# 이미 cdylib 포함 → 유지
crate-type = ["cdylib", "rlib"]
```

### 2. `image` 의존성 게이트화 (선택이지만 권장)

`decode(gray, w, h)`는 raw grayscale 바이트만 받으니 hot path에 `image`가 필요 없다. CLI 바이너리(`src/main.rs`)와 이미지 로더만 `image`를 쓴다면:

```toml
[dependencies]
image = { version = "0.25", optional = true }

[features]
default = ["image"]                     # desktop 빌드는 기존과 동일
cli = ["image"]
android = ["uniffi"]                    # image 없이 빌드 — APK 크기 ↓
```

그리고 `src/lib.rs` / `src/img_scanner.rs`에서 `image` 쓰는 부분에 `#[cfg(feature = "image")]` 게이트.

### 3. UniFFI UDL 정의 — `src/masuri.udl`

```udl
namespace masuri {
  sequence<Decoded> decode(bytes gray, u32 width, u32 height);
};

dictionary Decoded {
  string data;
  SymbolType sym_type;
  i32 quality;
  u32 x;
  u32 y;
};

enum SymbolType {
  "None", "Partial",
  "Ean2", "Ean5", "Ean8", "Upce",
  "Isbn10", "Upca", "Ean13", "Isbn13",
  "I25", "Code39", "Code128",
};
```

### 4. `build.rs` 추가

```rust
fn main() {
    #[cfg(feature = "android")]
    uniffi::generate_scaffolding("src/masuri.udl").unwrap();
}
```

### 5. `src/lib.rs`에 scaffolding include

```rust
#[cfg(feature = "android")]
uniffi::include_scaffolding!("masuri");
```

이 한 줄로 UniFFI가 C ABI + Kotlin 바인딩 코드를 자동 생성. `#[derive(uniffi::Record)]` / `#[derive(uniffi::Enum)]`을 `Decoded` / `SymbolType`에 붙이면 UDL 안 쓰고 proc-macro 방식으로도 가능 (최신 uniffi 권장).

## 크로스컴파일 파이프라인

### 요구 사항

1. **Android NDK r26+** (이미 Android Studio에 포함. `ANDROID_NDK_HOME` 지정)
2. `rustup target add aarch64-linux-android`
3. `cargo install cargo-ndk` — NDK toolchain 자동 연결

### 빌드 명령

```bash
cd projects/masuri
cargo ndk \
  --target aarch64-linux-android \
  --platform 24 \
  --output-dir ../wire-printer/app/src/main/jniLibs \
  build --release --features android --no-default-features
```

→ `app/src/main/jniLibs/arm64-v8a/libmasuri.so` 생성. Gradle이 APK에 자동 패키징.

> S22는 aarch64 단일 타깃이면 충분. 에뮬레이터까지 지원하려면 `x86_64-linux-android` 추가.

### uniffi 바인딩 생성 (Kotlin 쪽)

```bash
cargo run --features=uniffi/cli --bin uniffi-bindgen generate \
  src/masuri.udl --language kotlin \
  --out-dir ../wire-printer/app/src/main/kotlin
```

→ `io/milestone/masuri/masuri.kt` 자동 생성. `import uniffi.masuri.decode`로 바로 사용.

## API 사용 예 (Kotlin 쪽)

```kotlin
import uniffi.masuri.decode
import uniffi.masuri.SymbolType

// yPlane: CameraX ImageProxy 의 Y 플레인 (NV21/YUV_420_888). Y가 곧 grayscale.
val results = decode(yPlane, width.toUInt(), height.toUInt())
results.forEach { d ->
    Log.i("masuri", "${d.symType}: ${d.data} (q=${d.quality})")
}
```

## 성능/동작 고려 사항

### rayon 쓰레드 풀

- 프레임당 decode 호출 시 rayon 쓰레드 풀 워밍업 비용이 일회성으로 발생. 앱 시작 시 더미 호출로 워밍업하거나, **`decode_serial` 진입점 추가** 검토.
- Android P+ 에선 `libc++` `std::thread` 가능하지만, 짧은 프레임(<480p)에선 `par_iter` 오버헤드가 이득을 상쇄할 수 있음. 벤치마크 기반 분기 권장.

### NEON SIMD

`aarch64-linux-android` 타깃은 `target_arch = "aarch64"` 이므로 `scanner_neon.rs`가 자동 포함된다. 별도 feature flag 불필요. S22 Cortex-X2/A710/A510 전부 NEON 지원.

### 카메라 프레임 포맷

- CameraX `ImageProxy`는 기본 YUV_420_888 → **Y 플레인이 이미 8비트 grayscale** → 추가 변환 없이 `decode()`에 바로 전달 가능.
- 단, `ByteBuffer.array()` 접근 대신 `get(ByteArray)` 복사해야 안전. row stride ≠ width 인 경우(padded) 수동 unpack 필요.

### 메모리 / 수명

- `Vec<Decoded>` 반환값은 UniFFI가 해제까지 처리. 수작업 JNI였다면 `masuri_results_free` 쌍이 필요했을 작업.
- `String` (data 필드)은 UTF-8로 Kotlin `String`에 직접 매핑. 일부 EAN 데이터가 비-UTF8이면 `Vec<u8>`로 변경 고려.

## 테스트 경로

1. `cargo test --no-default-features --features android` — Android feature 조합 컴파일 확인 (실제 실행은 desktop에서, 로직만 검증).
2. `cargo ndk ... build` — 실제 `.so` 생성 및 크기 확인 (~2–5MB 예상, LTO 기준).
3. `adb shell am start ...` 로 앱 실행 후 `decode` 호출 → logcat에서 결과 확인.
4. 라벨에 인쇄한 Code-128 실물 촬영 → 라운드트립 검증.

## 체크리스트

- [ ] `Cargo.toml`: `android` feature 추가, uniffi build+runtime 의존성 추가
- [ ] `src/masuri.udl` 또는 `#[derive(uniffi::…)]` 매크로 적용
- [ ] `build.rs` 추가 (UDL 경로 방식인 경우)
- [ ] `src/lib.rs`에 `include_scaffolding!` (또는 proc-macro) 반영
- [ ] `image` 의존성 feature-gate (선택)
- [ ] `cargo ndk` 빌드 성공 확인
- [ ] Kotlin 바인딩 생성 → `wire-printer` 앱에서 import 성공
- [ ] 카메라 Y-plane → `decode` 호출 → 결과 로깅 성공

이 흐름을 따르면 `masuri`는 desktop Rust 용도를 그대로 유지하면서, Android feature만 빌드하면 APK에 통합 가능한 `.so` + Kotlin 바인딩이 동시에 얻어진다.
