#!/usr/bin/env bash
# Installs the .deb on THIS machine (a Debian or Ubuntu container) and starts the
# app for real.
#
# Run as root inside the container: `apt-get install ./neezer-desktop-*.deb`
# resolves `Depends:` from the archive, so a dependency the package forgets is a
# package that does not install — which is the defect this script exists to
# catch (P1's `namcap E:` lesson, on Debian).
#
# Usage: ./test.sh <chemin du .deb>
set -euo pipefail
source "$(dirname -- "${BASH_SOURCE[0]}")/../lib.sh"

deb="${1:?chemin du .deb}"
export DEBIAN_FRONTEND=noninteractive

neezer_note "installation de $(basename "$deb")"
apt-get update -qq
# `xvfb` and `mesa-vulkan-drivers` (lavapipe) are the test's own tools, not the
# package's dependencies: the first paints the window, the second gives the
# sidecar a Vulkan device so `--version` reports one instead of finding none.
# `fonts-dejavu-core` is part of the test's own desktop: a container that
# installs its packages with --no-install-recommends has no font at all, and GPUI
# aborts on a machine where fontconfig resolves none (see control.in).
apt-get install -y --no-install-recommends "$(realpath "$deb")" \
  xvfb xauth mesa-vulkan-drivers desktop-file-utils fonts-dejavu-core >/dev/null

dpkg-query -W -f='${Package} ${Version} ${Architecture}\n' neezer-desktop >&2
neezer_check_desktop_file /usr/share/applications/neezer-desktop.desktop
neezer_check_sidecar_installed /usr/bin neezer-transcribe

# Every library both binaries need must resolve on the installed system: `ldd`
# says `not found` for a dependency the package forgot to declare, and that is
# exactly the silent half of a broken install.
for binary in /usr/bin/neezer-desktop /usr/bin/neezer-transcribe; do
  if ldd "$binary" | grep -q 'not found'; then
    ldd "$binary" | grep 'not found' >&2
    neezer_die "$binary a une bibliothèque non résolue"
  fi
done
neezer_note "ldd : aucune bibliothèque manquante"

trap neezer_xvfb_stop EXIT
neezer_xvfb_start
neezer_expect_first_frame neezer-desktop --self-test-quit-after-first-frame
neezer_note "l'application a peint sa première image et quitté"
