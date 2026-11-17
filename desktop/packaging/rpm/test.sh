#!/usr/bin/env bash
# Installs the .rpm on THIS machine (a Fedora container) and starts the app for
# real.
#
# `dnf install ./neezer-desktop-*.rpm` resolves the soname `Requires` rpmbuild
# generated from the two binaries, so a missing runtime library is an install
# that fails here rather than an app that refuses to start on the person's
# machine.
#
# Usage: ./test.sh <chemin du .rpm>
set -euo pipefail
source "$(dirname -- "${BASH_SOURCE[0]}")/../lib.sh"

rpm="${1:?chemin du .rpm}"

neezer_note "installation de $(basename "$rpm")"
# `xorg-x11-server-Xvfb` paints the window and `mesa-vulkan-drivers` gives the
# sidecar a (software) Vulkan device: the test's tools, not the package's
# dependencies.
# `dejavu-sans-fonts` is the test desktop's font: a container that skips weak
# dependencies has none, and GPUI aborts when fontconfig resolves none.
dnf install -y --setopt=install_weak_deps=False "$(realpath "$rpm")" \
  xorg-x11-server-Xvfb mesa-vulkan-drivers desktop-file-utils \
  dejavu-sans-fonts >/dev/null

rpm -q neezer-desktop >&2
neezer_check_desktop_file /usr/share/applications/neezer-desktop.desktop
neezer_check_sidecar_installed /usr/bin neezer-transcribe

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
