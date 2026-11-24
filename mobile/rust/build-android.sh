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

# --- the link guard --------------------------------------------------------
#
# A `NEEDED` entry the APK does not carry is an `UnsatisfiedLinkError` at the
# FIRST `System.loadLibrary`: the app dies before its first screen, and the
# emulator smoke can only say "« Accueil » never painted" unless it reads the
# crash buffer (it does now, that script carries the same lesson).
#
# Measured, and the reason this function exists: `signalsmith-stretch` (the
# time-stretch inside `nz-audio`) is C++ and the `cc` crate's Android default is
# the SHARED libc++ — so `libnz_ffi.so` carried `NEEDED libc++_shared.so`, nothing
# in this repository shipped that file, and the whole app was dead on every ABI
# from the merge that linked `nz-audio` until this check. `readelf` on the built
# library, the APK's `lib/` listing and the device's `logcat` all agreed; nothing
# looked.
#
# The rule: every `NEEDED` name is either a library ANDROID provides, or a file
# this directory really carries beside `libnz_ffi.so`. `libc++_shared.so` is
# deliberately NOT on the system list — it is the exact case this catches.
SYSTEM_LIBS="libc.so libm.so libdl.so liblog.so libandroid.so libaaudio.so libOpenSLES.so libEGL.so libGLESv2.so libvulkan.so libjnigraphics.so libz.so libstdc++.so libnativehelper.so libmediandk.so libcamera2ndk.so"

# `readelf` from binutils, or the NDK's own (the NDK is installed on every
# machine and runner this app is built on).
nz_readelf() {
    if command -v readelf >/dev/null 2>&1; then
        command -v readelf
    elif [ -x "${ANDROID_NDK_HOME:-}/toolchains/llvm/prebuilt/linux-x86_64/bin/llvm-readelf" ]; then
        printf '%s\n' "$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin/llvm-readelf"
    else
        printf 'build-android.sh: no readelf (binutils or the NDK): the link guard cannot run\n' >&2
        return 1
    fi
}

needed_of() {
    local readelf
    readelf="$(nz_readelf)" || return 1
    "$readelf" -d "$1" 2>/dev/null | sed -n 's/.*(NEEDED).*\[\(.*\)\]/\1/p'
}

# `nm` from binutils, or the NDK's own.
nz_nm() {
    if command -v nm >/dev/null 2>&1; then
        command -v nm
    elif [ -x "${ANDROID_NDK_HOME:-}/toolchains/llvm/prebuilt/linux-x86_64/bin/llvm-nm" ]; then
        printf '%s\n' "$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin/llvm-nm"
    else
        printf 'build-android.sh: no nm (binutils or the NDK): the JNI export guard cannot run\n' >&2
        return 1
    fi
}

# The four API-26 `AudioTrackPull` entry points are reached BY NAME from Kotlin
# (`external fun`), so nothing inside Rust references them: a linker flag that
# localizes exported symbols drops them without failing the build, and the app
# then dies at `System.loadLibrary` on an Android 8 phone — the same dead-app
# shape as MB3b's `libc++_shared`. Measured: `-Wl,--exclude-libs,ALL` (a 114 176 B
# saving) localized them, because a Rust crate reaches the linker as an archive;
# `mobile.yaml` saw it, this guard now refuses it locally.
JNI_ENTRY_POINTS="nativeRender nativeSetLatency nativeUnderruns nativeClose"
check_jni_exports() {
    local dir="$1" abi so nm symbol exports missing=0
    nm="$(nz_nm)" || return 1
    for abi in "${TARGETS[@]}"; do
        so="$dir/$abi/libnz_ffi.so"
        [ -f "$so" ] || continue
        for name in $JNI_ENTRY_POINTS; do
            symbol="Java_fr_neezer_mobile_playback_AudioTrackPull_$name"
            # The output is CAPTURED before grep: `nm … | grep -q` under
            # `set -o pipefail` fails on a match, because grep exits early, nm
            # takes SIGPIPE (141), and the pipeline's status is the 141 — measured,
            # it made this guard report all four symbols missing on a library that
            # exported them.
            exports="$("$nm" -D --defined-only "$so" 2>/dev/null)" || exports=""
            if ! grep -qE " T $symbol$" <<<"$exports"; then
                printf 'build-android.sh: %s/libnz_ffi.so n’exporte pas %s (la sortie AudioTrack API 26 ne la trouverait pas)\n' \
                    "$abi" "$symbol" >&2
                missing=1
            fi
        done
    done
    return "$missing"
}

# `check_libs <dir>` where <dir>/<abi>/libnz_ffi.so are the libraries an APK
# carries. Answers 1 and names every offender.
check_libs() {
    local dir="$1" abi so name missing=0 checked=0
    for abi in "${TARGETS[@]}"; do
        so="$dir/$abi/libnz_ffi.so"
        if [ ! -f "$so" ]; then
            printf 'build-android.sh: %s absent, rien à vérifier\n' "$so" >&2
            missing=1
            continue
        fi
        checked=$((checked + 1))
        while read -r name; do
            [ -n "$name" ] || continue
            case " $SYSTEM_LIBS " in
                *" $name "*) continue ;;
            esac
            [ -f "$dir/$abi/$name" ] && continue
            printf 'build-android.sh: %s/libnz_ffi.so dépend de %s, qu’Android ne fournit pas et que l’APK n’embarque pas\n' \
                "$abi" "$name" >&2
            missing=1
        done <<<"$(needed_of "$so")"
    done
    if [ "$checked" -eq 0 ]; then
        printf 'build-android.sh: aucune bibliothèque à vérifier dans %s\n' "$dir" >&2
        return 1
    fi
    return "$missing"
}

# `build-android.sh --check-libs [<dir>]` checks an already-built set (the
# jniLibs by default, or an unpacked APK's `lib/`) without building anything: the
# same command a reviewer runs on a downloaded APK.
if [ "${1:-}" = "--check-libs" ]; then
    shift || true
    check_libs "${1:-../android/bridge/src/main/jniLibs}"
    exit $?
fi

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
# `CXXSTDLIB=c++_static` links the C++ runtime INTO our library instead of
# requiring `libc++_shared.so` beside it (see the link guard above). It is set
# for THIS command only: the host build above has no `libc++_static`, and an
# exported value broke that build's link with `unable to find library -lc++_static`.
#
# `-lc++abi` is the second half of that library, and `cc` says so itself ("if
# libc++ is linked statically, the build will probably also need c++abi, since
# libc++.a usually doesn't contain it; cc doesn't link it, so link it from the
# build script"). Ours is `build-android.sh`, so the flag rides in RUSTFLAGS —
# appended, which puts it AFTER the `-lc++_static` a build script emits, the order
# a static archive needs. Measured without it: the library links, and the device
# then refuses it — `dlopen failed: cannot locate symbol "_ZTISt12length_error"`
# (libc++abi's `std::length_error` typeinfo), the same dead app with a different
# sentence.
# The one link-time shrinker left after measurement, and the two that were
# measured and REFUSED (this lot, docs/agents/mobile.md "MQ2 — la taille").
# They are set HERE and not in the exported `RUSTFLAGS` above: they are ELF
# choices, and applied to the host build they break its own build scripts —
# measured, a `RUSTFLAGS` carrying `--pack-dyn-relocs=android` made `proc-macro2`
# and `quote` die with SIGSEGV before compiling anything (glibc's loader does not
# read Android's APS2 relocations).
#
#  - `--pack-dyn-relocs=android` writes the relocations in Android's APS2
#    format, which the loader has understood since API 23 (this app's minSdk is
#    26): `.rela.dyn` 729 360 B → ~106 000 B, **−624 640 B** on the library. The
#    generic `--pack-dyn-relocs=relr` was NOT used: `DT_RELR` needs a loader newer
#    than minSdk 26.
#  - `--icf=all` folds identical functions: **−1 054 080 B**. `--icf=safe` was
#    measured first and folded nothing at all (byte-identical output), because a
#    Rust binary takes the address of almost everything (trait objects, `Drop`
#    glue); `all` is what Chrome and Soong ship for release. The one thing it must
#    never touch is a function whose ADDRESS is compared — this tree compares
#    none (`grep` over every reachable crate: no function-pointer equality), the
#    emulator journey plays on this library, and a bug that smells like folded
#    code is this flag's to answer.
#  - `--exclude-libs,ALL` was measured at **−114 176 B** and REMOVED: a Rust crate
#    reaches the linker as an rlib, which IS an archive, so the flag localized
#    `nz-droid-audio`'s four `#[no_mangle] Java_…AudioTrackPull_*` exports too and
#    the API-26 output would have died at `System.loadLibrary` — caught by
#    `mobile.yaml`'s "The API-26 JNI entry points are in libnz_ffi.so" step, and
#    the reason `check_jni_exports` below now refuses such a library locally.
ANDROID_LINK_FLAGS="-C link-arg=-Wl,--pack-dyn-relocs=android -C link-arg=-Wl,--icf=all"
CXXSTDLIB=c++_static RUSTFLAGS="$RUSTFLAGS -C link-arg=-lc++abi $ANDROID_LINK_FLAGS" \
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
        printf '   %-12s NEEDED: %s\n' "" "$(needed_of "$so" | tr '\n' ' ')"
    else
        echo "   $abi MISSING" >&2
        exit 1
    fi
done
# The guard runs on every build, before anything packs these files.
check_libs "$JNI_LIBS" || {
    echo "build-android.sh: la bibliothèque ne se chargerait pas sur l'appareil (voir ci-dessus)" >&2
    exit 1
}
check_jni_exports "$JNI_LIBS" || {
    echo "build-android.sh: la sortie AudioTrack API 26 perdrait ses points d'entrée (voir ci-dessus)" >&2
    exit 1
}
echo "   kotlin:      $(find "$GENERATED" -name '*.kt' | wc -l) file(s)"
