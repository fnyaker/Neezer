#!/usr/bin/env bash
# Runs the AppImage for real: no install step, one file, our own display.
#
# `APPIMAGE_EXTRACT_AND_RUN=1` is the AppImage runtime's own documented way to
# run without FUSE (a container usually has none); on a desktop the file is
# simply executed and mounted. Both paths run the same `AppRun`.
#
# Usage: ./test.sh <chemin du .AppImage>
set -euo pipefail
source "$(dirname -- "${BASH_SOURCE[0]}")/../lib.sh"

image="$(realpath "${1:?chemin du .AppImage}")"
chmod +x "$image"

work="$(mktemp -d "${TMPDIR:-/tmp}/neezer-appimage-test.XXXXXX")"
cleanup() {
  neezer_xvfb_stop
  rm -rf "$work"
}
trap cleanup EXIT

# What is inside, before running it: the two binaries, the launcher entry and the
# icon, and NO bundled graphics stack (see build.sh). `--appimage-extract` is the
# runtime's own command and unpacks into the current directory.
(
  cd "$work"
  "$image" --appimage-extract >/dev/null 2>&1
) || neezer_die "l'AppImage ne s'extrait pas"
for expected in usr/bin/neezer-desktop usr/bin/neezer-transcribe \
  neezer-desktop.desktop neezer-desktop.svg neezer-desktop.png .DirIcon AppRun; do
  [ -e "$work/squashfs-root/$expected" ] || neezer_die "$expected manque dans l'AppImage"
done
if find "$work/squashfs-root" -name 'libvulkan*' | grep -q .; then
  neezer_die "l'AppImage embarque un chargeur Vulkan : il doit venir de l'hôte"
fi
neezer_note "contenu vérifié : les deux binaires, l'entrée .desktop, l'icône, aucun Vulkan embarqué"

neezer_xvfb_start
neezer_expect_first_frame env APPIMAGE_EXTRACT_AND_RUN=1 "$image" \
  --self-test-quit-after-first-frame
neezer_note "l'AppImage a peint sa première image et quitté"

# The sidecar the app starts is the copy inside the image, beside the app: ask
# that one, not the host's, for its version.
neezer_note "sidecar de l'AppImage : $("$work/squashfs-root/usr/bin/neezer-transcribe" --version | head -1)"
