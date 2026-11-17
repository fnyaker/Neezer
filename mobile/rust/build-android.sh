#!/usr/bin/env bash
# Builds the mobile app's ONE native library and its Kotlin bindings.
#
#   1. the host DEBUG build of `nz-ffi` (unstripped: `strip = true` on the
#      release profile deletes UniFFI's metadata, and bindgen on a stripped
#      library answers "No UniFFI metadata found");
#   2. `uniffi-bindgen` over that library -> the Kotlin in
#      `:bridge`'s generated source dir;
#   3. `cargo ndk` for the three ABIs the app ships, into
#      `:bridge/src/main/jniLibs/` where Gradle packs them into the APK.
#
# Needs: the Rust targets below (`rustup target add`), `cargo-ndk` 4.1 (pinned in
# .github/workflows/mobile.yaml) and an NDK. `ANDROID_NDK_HOME` wins; otherwise
# `$ANDROID_HOME/ndk/<the pinned one>` is used, as this machine and the CI image
# both lay it out.
#
# CARGO_BUILD_JOBS defaults to 4: this machine builds at most four Rust lots at
# once and a fifth cargo would only make every one of them slower.
set -euo pipefail
cd "$(dirname "$0")"

NDK_VERSION=30.0.16248370
API=26 # the app's minSdk
TARGETS=(arm64-v8a armeabi-v7a x86_64)
: "${CARGO_BUILD_JOBS:=4}"
export CARGO_BUILD_JOBS

if [ -z "${ANDROID_NDK_HOME:-}" ]; then
    if [ -n "${ANDROID_HOME:-}" ] && [ -d "$ANDROID_HOME/ndk/$NDK_VERSION" ]; then
        export ANDROID_NDK_HOME="$ANDROID_HOME/ndk/$NDK_VERSION"
    elif [ -d "$HOME/.local/share/android-sdk/ndk/$NDK_VERSION" ]; then
        export ANDROID_NDK_HOME="$HOME/.local/share/android-sdk/ndk/$NDK_VERSION"
    else
        echo "build-android.sh: no NDK $NDK_VERSION; set ANDROID_NDK_HOME" >&2
        exit 1
    fi
fi

GENERATED=../android/bridge/build/generated/uniffi
JNI_LIBS=../android/bridge/src/main/jniLibs

# 16 KB pages: Android 15 devices may use them, and a library aligned for 4 KB
# refuses to load there (android/nscore/build-android.sh's rule, same reason).
export RUSTFLAGS="${RUSTFLAGS:-} -C link-arg=-Wl,-z,max-page-size=16384"

echo "== build-android.sh: host debug library (unstripped, for bindgen)"
cargo build --locked -p nz-ffi

echo "== build-android.sh: Kotlin bindings"
mkdir -p "$GENERATED"
cargo run --quiet --locked -p nz-bindgen -- generate \
    --library target/debug/libnz_ffi.so \
    --language kotlin \
    --no-format \
    --config uniffi.toml \
    --out-dir "$GENERATED"

echo "== build-android.sh: three ABIs (NDK $ANDROID_NDK_HOME, API $API)"
mkdir -p "$JNI_LIBS"
cargo ndk -t arm64-v8a -t armeabi-v7a -t x86_64 -P "$API" -o "$JNI_LIBS" \
    build --locked --release -p nz-ffi

# cargo-ndk copies EVERY cdylib it built, and four of the shared crates are
# `cdylib + rlib` (rhythm, appcore, trainer, measure) plus nscore: their code is
# already linked INTO libnz_ffi.so, so the copies are dead weight in the APK
# (measured: 4 extra libraries, ~1.4 MB on arm64). ONE library ships — the plan's
# "one `cdylib` (`libnz_ffi.so`)".
for abi in "${TARGETS[@]}"; do
    find "$JNI_LIBS/$abi" -maxdepth 1 -name 'lib*.so' ! -name 'libnz_ffi.so' -delete
done

echo "== build-android.sh: what was produced"
for abi in "${TARGETS[@]}"; do
    so="$JNI_LIBS/$abi/libnz_ffi.so"
    if [ -f "$so" ]; then
        printf '   %-12s %8s bytes\n' "$abi" "$(stat -c %s "$so")"
    else
        echo "   $abi MISSING" >&2
        exit 1
    fi
done
echo "   kotlin:      $(find "$GENERATED" -name '*.kt' | wc -l) file(s)"
