#!/usr/bin/env bash
# mobile/tools/size-guard.sh — the ceiling the shipped library and APK may not
# cross (lot « MQ2 — la taille », docs/agents/mobile.md "Measures").
#
#   mobile/tools/size-guard.sh [--jni <dir>] [--apk-dir <dir>]
#
# `apk`'s Gradle step builds `libnz_ffi.so` for three ABIs and four APKs (one per
# ABI plus the universal one); this is the one place that says how big each may
# be. A PR that doubles the library — which happened once, from 9,2 MB at MB3a to
# 19,8 MB at MQ2 — passes every other check there is: it compiles, it plays, and
# the only symptom is a download nobody measures.
#
# WHY THESE NUMBERS. Each ceiling is the size MEASURED on 2026-11-24 (the run
# that cut the library by 18,2 %) plus ~5 %, so a real regression is caught and a
# routine a-few-bytes change is not. They are deliberately NOT the note's targets:
# see "MQ2 — la taille" for why 9 MB for the library cannot be reached with the
# features the app owes, and why 45 MB is a per-ABI download, not the universal
# APK's three ABIs in one file.
#
# Artifact                              measured B    ceiling B
# ------------------------------------  ----------    ---------
# libnz_ffi.so arm64-v8a                 16 322 520   17 150 000
# libnz_ffi.so armeabi-v7a               11 473 160   12 050 000
# libnz_ffi.so x86_64                    20 739 864   21 800 000
# app-arm64-v8a-release.apk              21 473 481   22 550 000
# app-armeabi-v7a-release.apk            16 574 985   17 400 000
# app-x86_64-release.apk                 25 841 660   27 150 000
# app-universal-release.apk              54 580 998   57 300 000
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
jni="$repo/mobile/android/bridge/src/main/jniLibs"
apk_dir="$repo/mobile/android/app/build/outputs/apk/release"

while [[ $# -gt 0 ]]; do
    case "$1" in
        --jni) jni="$2"; shift 2 ;;
        --jni=*) jni="${1#*=}"; shift ;;
        --apk-dir) apk_dir="$2"; shift 2 ;;
        --apk-dir=*) apk_dir="${1#*=}"; shift ;;
        -h | --help)
            sed -n '2,10p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
            exit 0
            ;;
        *)
            printf 'size-guard.sh: unknown argument: %s\n' "$1" >&2
            exit 2
            ;;
    esac
done

ceiling_of() {
    case "$1" in
        libnz_ffi.so:arm64-v8a) printf '%s\n' 17150000 ;;
        libnz_ffi.so:armeabi-v7a) printf '%s\n' 12050000 ;;
        libnz_ffi.so:x86_64) printf '%s\n' 21800000 ;;
        app-arm64-v8a-release.apk) printf '%s\n' 22550000 ;;
        app-armeabi-v7a-release.apk) printf '%s\n' 17400000 ;;
        app-x86_64-release.apk) printf '%s\n' 27150000 ;;
        app-universal-release.apk) printf '%s\n' 57300000 ;;
        *) printf '' ;;
    esac
}

fail=0
checked=0

# `check <what> <path> <ceiling>`: a missing artifact is a failure of its own —
# a guard that silently passes because the file does not exist is the bug this
# script exists to avoid.
check() {
    local what="$1" path="$2" ceiling="$3" size delta
    if [[ ! -f "$path" ]]; then
        printf 'size-guard: %s absent (%s)\n' "$what" "$path" >&2
        fail=$((fail + 1))
        return
    fi
    size="$(stat -c %s "$path")"
    checked=$((checked + 1))
    if (( size > ceiling )); then
        delta=$((size - ceiling))
        printf 'size-guard: %s %s B dépasse le plafond de %s B (%+d B, %d %%)\n' \
            "$what" "$size" "$ceiling" "$delta" "$((delta * 100 / ceiling))" >&2
        fail=$((fail + 1))
    else
        printf '   %-38s %10s B  (plafond %9s B, %d %%)\n' \
            "$what" "$size" "$ceiling" "$((size * 100 / ceiling))"
    fi
}

for abi in arm64-v8a armeabi-v7a x86_64; do
    check "libnz_ffi.so:$abi" "$jni/$abi/libnz_ffi.so" "$(ceiling_of "libnz_ffi.so:$abi")"
done

for apk in app-arm64-v8a-release.apk app-armeabi-v7a-release.apk app-x86_64-release.apk app-universal-release.apk; do
    check "$apk" "$apk_dir/$apk" "$(ceiling_of "$apk")"
done

if (( checked == 0 )); then
    printf 'size-guard: nothing was measured, which is never a pass\n' >&2
    exit 1
fi
if (( fail )); then
    printf 'size-guard: %d problème(s) sur %d artefacts sous plafond (voir "MQ2 — la taille", docs/agents/mobile.md)\n' \
        "$fail" "$checked" >&2
    exit 1
fi
printf 'size-guard: %d artifacts under their ceiling\n' "$checked"
