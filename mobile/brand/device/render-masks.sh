#!/usr/bin/env bash
# LG1 — the icon evidence, on the emulator, without the user's display.
#
#   mobile/brand/device/render-masks.sh --serial emulator-5600 --out ~/shots [--apk FILE]
#
# It installs the APK (or reuses the installed one), compiles IconMasks.java with
# the platform's d8, runs it through `app_process` on the device, pulls the PNGs,
# writes strips, and captures the system's own App info page in BOTH themes —
# the surface that proves the light variant, since it is the system that picks
# drawable/ against drawable-night/.
#
# Needs: ANDROID_HOME, a JDK 17+ (JAVA_HOME or javac on PATH), and an emulator
# this lot started itself (mobile/tools/emu.sh once MQ1 has landed; the same
# flags by hand before that). Nothing touches the user's screen or speakers.
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)

SERIAL=""
OUT=""
APK=""
PKG="fr.neezer.mobile"
while [ $# -gt 0 ]; do
    case "$1" in
        --serial) SERIAL="$2"; shift ;;
        --out) OUT="$2"; shift ;;
        --apk) APK="$2"; shift ;;
        --package) PKG="$2"; shift ;;
        *) echo "render-masks.sh: unknown argument $1" >&2; exit 2 ;;
    esac
    shift
done
[ -n "$SERIAL" ] || { echo "render-masks.sh: --serial emulator-<port> is required" >&2; exit 2; }
[ -n "$OUT" ] || OUT="$PWD/masks-out"
mkdir -p "$OUT"

: "${ANDROID_HOME:?ANDROID_HOME must point at the SDK}"
ADB="$ANDROID_HOME/platform-tools/adb"
BT=$(ls -d "$ANDROID_HOME"/build-tools/* 2>/dev/null | sort -V | tail -1)
AJAR=$(ls "$ANDROID_HOME"/platforms/android-*/android.jar | sort -V | tail -1)
JAVAC="${JAVA_HOME:+$JAVA_HOME/bin/}javac"
command -v "$JAVAC" >/dev/null || { echo "render-masks.sh: javac missing" >&2; exit 1; }

# Refuse to touch a device this lot did not start: the serial must be an
# emulator port, never a phone.
case "$SERIAL" in
    emulator-*) ;;
    *) echo "render-masks.sh: refusing '$SERIAL' (only emulator-<port>)" >&2; exit 2 ;;
esac

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

if [ -n "$APK" ]; then
    "$ADB" -s "$SERIAL" install -r -t "$APK" >/dev/null
    echo "installed $APK"
fi
APK_PATH=$("$ADB" -s "$SERIAL" shell pm path "$PKG" | tr -d '\r' | sed 's/package://' | head -1)
[ -n "$APK_PATH" ] || { echo "render-masks.sh: $PKG is not installed" >&2; exit 1; }

echo "== compiling IconMasks for the device"
mkdir -p "$WORK/classes" "$WORK/dex"
"$JAVAC" --release 11 -cp "$AJAR" -d "$WORK/classes" "$HERE/IconMasks.java" 2>&1 |
    grep -v 'deprecat' || true
"$BT/d8" --release --lib "$AJAR" --min-api 26 --output "$WORK/dex" \
    "$WORK"/classes/*.class

"$ADB" -s "$SERIAL" shell mkdir -p /data/local/tmp/lg1 >/dev/null
"$ADB" -s "$SERIAL" push "$WORK/dex/classes.dex" /data/local/tmp/lg1/ >/dev/null
"$ADB" -s "$SERIAL" shell \
    "CLASSPATH=/data/local/tmp/lg1/classes.dex app_process /system/bin IconMasks '$APK_PATH' /data/local/tmp/lg1 $PKG"

for f in icon-day icon-night \
         mask-circle mask-squircle mask-teardrop \
         mask-circle-night mask-squircle-night mask-teardrop-night \
         mono-dark mono-light glyph-24dp glyph-24dp-x4; do
    "$ADB" -s "$SERIAL" pull "/data/local/tmp/lg1/$f.png" "$OUT/$f.png" >/dev/null 2>&1 ||
        echo "  (no $f.png: the layer is missing from the APK)"
done

# The system's own surface, in both themes: App info reloads the drawable when
# the ui mode changes, which is exactly the thing to prove.
capture_appinfo() {
    local mode="$1" out="$2"
    "$ADB" -s "$SERIAL" shell cmd uimode night "$mode" >/dev/null 2>&1
    "$ADB" -s "$SERIAL" shell am force-stop com.android.settings >/dev/null 2>&1
    "$ADB" -s "$SERIAL" shell am start -a android.settings.APPLICATION_DETAILS_SETTINGS \
        -d "package:$PKG" >/dev/null 2>&1
    for _ in $(seq 1 40); do
        local focus
        focus=$("$ADB" -s "$SERIAL" shell dumpsys window 2>/dev/null |
                grep -m1 mCurrentFocus || true)
        case "$focus" in *settings*) break ;; esac
        sleep 0.5
    done
    sleep 2
    "$ADB" -s "$SERIAL" exec-out screencap -p > "$out"
    echo "wrote $out"
}
capture_appinfo no "$OUT/system-light.png"
capture_appinfo yes "$OUT/system-dark.png"

# Strips to look at in one go.
if command -v magick >/dev/null; then
    magick montage "$OUT/icon-day.png" "$OUT/icon-night.png" -tile 2x1 \
        -geometry +10+10 -background '#2a2a26' "$OUT/icons-both-themes.png" 2>/dev/null &&
        echo "wrote $OUT/icons-both-themes.png"
    magick montage "$OUT/mask-circle.png" "$OUT/mask-squircle.png" \
        "$OUT/mask-teardrop.png" -tile 3x1 -geometry +10+10 \
        -background '#2a2a26' "$OUT/masks.png" 2>/dev/null && echo "wrote $OUT/masks.png"
    magick montage "$OUT/mask-circle-night.png" "$OUT/mask-squircle-night.png" \
        "$OUT/mask-teardrop-night.png" -tile 3x1 -geometry +10+10 \
        -background '#2a2a26' "$OUT/masks-night.png" 2>/dev/null &&
        echo "wrote $OUT/masks-night.png"
    magick montage "$OUT/mono-dark.png" "$OUT/mono-light.png" -tile 2x1 \
        -geometry +10+10 -background '#2a2a26' "$OUT/mono.png" 2>/dev/null &&
        echo "wrote $OUT/mono.png"
fi
