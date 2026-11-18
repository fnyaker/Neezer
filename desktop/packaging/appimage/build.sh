#!/usr/bin/env bash
# Builds neezer-desktop-<ver>-x86_64.AppImage: ONE executable file, no install
# step, from the release binaries that already exist.
#
# The shape is the manual AppDir from the AppImage documentation
# (https://docs.appimage.org/packaging-guide/manual.html): AppRun, the
# `.desktop`, the icon, `usr/bin/<payload>`, then `appimagetool`. What is NOT in
# it is the decision this script is really about:
#
#   - **Vulkan comes from the host, never bundled.** The rule is the project's
#     ("Always archive FLAC, transcode to Opus" has a sibling here: a GPU stack
#     shipped inside an app is a GPU stack that does not know the machine's
#     driver). `neezer-transcribe` links `libvulkan.so.1`; the loader and the
#     ICDs (`libvulkan_radeon.so`, `nvidia_icd.json`, …) stay the person's, which
#     is also what the app's own fallback expects (no usable device -> the CPU,
#     and it says why, T2).
#   - **Nor the rest of the graphics/audio stack.** `linuxdeploy`
#     (https://github.com/linuxdeploy/linuxdeploy) is the usual way to fill
#     `usr/lib`: it walks the binary's dependencies with `ldd` and copies every
#     library that is not on its exclusion list, and its `--exclude-library`
#     only exists because doing that to the graphics stack breaks apps. Blocking
#     a distribution's `libwayland-client.so.0` / `libxkbcommon.so.0` into an
#     AppImage means the person's compositor and the app's client library are two
#     different builds of the same protocol: the window then does not open on the
#     next distribution, and that is the failure this package must not ship. The
#     AppImage therefore carries the app and its two binaries, and declares the
#     desktop it needs as any binary of it would; `deb/`, `rpm/` and `flatpak/`
#     are the formats that own the dependency question, each the way its
#     distribution answers it.
#
# `appimagetool` is pinned by SHA-256: the `continuous` tag is a moving build,
# so a fresh download is a deliberate act (bump APPIMAGETOOL_SHA256 with it).
#
# Usage: NEEZER_DESKTOP_VERSION=1.6.0 NEEZER_BINARY_DIR=… ./build.sh [outdir]
set -euo pipefail
source "$(dirname -- "${BASH_SOURCE[0]}")/../lib.sh"

neezer_check_version
neezer_check_binaries
neezer_check_needed

APPIMAGETOOL_URL="https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage"
APPIMAGETOOL_SHA256="e8c1169c4af66c3184d7ee110713d4be04befe671fa358e2c1591ee840553b03"

out="${1:-$PWD}"
work="$(mktemp -d "${TMPDIR:-/tmp}/neezer-appimage.XXXXXX")"
trap 'rm -rf "$work"' EXIT

tool="${APPIMAGETOOL:-}"
if [[ -z "$tool" ]]; then
  if command -v appimagetool >/dev/null; then
    tool="$(command -v appimagetool)"
  else
    neezer_note "téléchargement d'appimagetool (épinglé par SHA-256)"
    curl -sSLo "$work/appimagetool" "$APPIMAGETOOL_URL"
    echo "$APPIMAGETOOL_SHA256  $work/appimagetool" | sha256sum -c - >/dev/null
    chmod +x "$work/appimagetool"
    tool="$work/appimagetool"
  fi
fi

appdir="$work/Neezer.AppDir"
install -d "$appdir/usr/bin" \
  "$appdir/usr/share/applications" \
  "$appdir/usr/share/icons/hicolor/scalable/apps" \
  "$appdir/usr/share/doc/neezer-desktop"

install -m 755 "$NEEZER_BINARY_DIR/neezer-desktop" "$appdir/usr/bin/neezer-desktop"
install -m 755 "$NEEZER_BINARY_DIR/neezer-transcribe" "$appdir/usr/bin/neezer-transcribe"
# The x86-64-v3 engine child in the SAME directory: `supervisor::engine_program`
# derives it from `current_exe()`, so a second directory would never be found.
install -m 755 "$NEEZER_BINARY_DIR/neezer-transcribe-v3" "$appdir/usr/bin/neezer-transcribe-v3"
install -m 755 "$NEEZER_BINARY_DIR/neezer-transcribe-v4" "$appdir/usr/bin/neezer-transcribe-v4"
install -m 755 "$NEEZER_PACKAGING/appimage/AppRun" "$appdir/AppRun"
install -m 644 "$NEEZER_ASSETS/neezer-desktop.desktop" "$appdir/neezer-desktop.desktop"
install -m 644 "$NEEZER_ASSETS/neezer-desktop.desktop" \
  "$appdir/usr/share/applications/neezer-desktop.desktop"
install -m 644 "$NEEZER_ASSETS/neezer-desktop.svg" "$appdir/neezer-desktop.svg"
install -m 644 "$NEEZER_ASSETS/neezer-desktop.svg" \
  "$appdir/usr/share/icons/hicolor/scalable/apps/neezer-desktop.svg"
# The AppDir's own icon is a PNG, and it is the WEB APP's committed one
# (`webapp/public/icon-512.png`, pixel-identical to `neezer-desktop.svg`: 512x512,
# measured with `compare -metric AE`). appimagetool builds `.DirIcon` from it and
# refuses nothing: an SVG `.DirIcon` is a warning today ("SVG is hard to parse…
# .DirIcon must be a PNG") and a hard error in a future release, and no rasteriser
# has to be installed for a file the repository already has.
install -m 644 "$NEEZER_ROOT/webapp/public/icon-512.png" "$appdir/neezer-desktop.png"
install -Dm 644 "$NEEZER_ROOT/webapp/public/icon-512.png" \
  "$appdir/usr/share/icons/hicolor/512x512/apps/neezer-desktop.png"
install -m 644 "$NEEZER_ROOT/LICENSE" "$appdir/usr/share/doc/neezer-desktop/LICENSE"

mkdir -p "$out"
appimage="$out/neezer-desktop-$NEEZER_VERSION-$NEEZER_ARCH.AppImage"
# `ARCH` and `VERSION` are appimagetool's own environment (it writes them into
# the runtime and into the AppImage's name when it picks the name itself).
ARCH="$NEEZER_ARCH" VERSION="$NEEZER_VERSION" \
  "$tool" --comp zstd --no-appstream "$appdir" "$appimage" >&2

neezer_note "$(du -h "$appimage" | cut -f1)  $appimage"
