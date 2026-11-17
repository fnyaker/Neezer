#!/usr/bin/env bash
# Builds neezer-desktop-<ver>-x86_64.flatpak from the release binaries that
# already exist: a Flatpak bundle a person installs for themselves with
# `flatpak install --user ./neezer-desktop-<ver>-x86_64.flatpak`.
#
# `flatpak-builder` builds in a sandbox of its own (bubblewrap), so this needs
# unprivileged user namespaces — true on a GitHub runner and on any desktop, and
# the reason the Flatpak job is the one packaging job that does NOT run inside a
# container (a container would have to be `--privileged` to nest bubblewrap).
#
# The runtime (`org.freedesktop.Platform//25.08`) and its SDK must be installed:
#   flatpak remote-add --user --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo
#   flatpak install --user -y flathub org.freedesktop.Platform//25.08 org.freedesktop.Sdk//25.08
#
# Usage: NEEZER_DESKTOP_VERSION=1.6.0 NEEZER_BINARY_DIR=… ./build.sh [outdir]
set -euo pipefail
source "$(dirname -- "${BASH_SOURCE[0]}")/../lib.sh"

neezer_check_version
neezer_check_binaries
neezer_check_needed

command -v flatpak-builder >/dev/null || neezer_die "flatpak-builder est absent"
flatpak --user info org.freedesktop.Platform//25.08 >/dev/null 2>&1 ||
  neezer_die "le runtime org.freedesktop.Platform//25.08 n'est pas installé (voir l'en-tête)"

out="${1:-$PWD}"
work="$(mktemp -d "${TMPDIR:-/tmp}/neezer-flatpak.XXXXXX")"
trap 'rm -rf "$work"' EXIT

# The manifest is built from the copy in this repository, byte for byte, with
# its `inputs/` beside it: what a person reading `fr.neezer.Desktop.yaml` sees is
# what the release was built from.
install -d "$work/inputs"
install -m 755 "$NEEZER_BINARY_DIR/neezer-desktop" "$work/inputs/neezer-desktop"
install -m 755 "$NEEZER_BINARY_DIR/neezer-transcribe" "$work/inputs/neezer-transcribe"
install -m 644 "$NEEZER_ASSETS/neezer-desktop.desktop" "$work/inputs/neezer-desktop.desktop"
# One line differs from the shared entry: inside the sandbox the window's
# WM_CLASS is the sandbox's app id (`app/src/main.rs#flatpak_app_id`, which reads
# `FLATPAK_ID`), not `neezer-desktop`. Everything else — Name, Exec, Icon,
# Categories, the French comment — is the Arch package's file.
sed -i 's/^StartupWMClass=.*/StartupWMClass=fr.neezer.Desktop/' "$work/inputs/neezer-desktop.desktop"
grep -q '^StartupWMClass=fr[.]neezer[.]Desktop$' "$work/inputs/neezer-desktop.desktop" ||
  neezer_die "l'entrée .desktop n'a pas de StartupWMClass à remplacer"
install -m 644 "$NEEZER_ASSETS/neezer-desktop.svg" "$work/inputs/neezer-desktop.svg"
install -m 644 "$NEEZER_ROOT/LICENSE" "$work/inputs/LICENSE"
install -m 644 "$NEEZER_PACKAGING/flatpak/fr.neezer.Desktop.yaml" "$work/fr.neezer.Desktop.yaml"

mkdir -p "$out"
cd "$work"
# `--user`: nothing here needs root, and a build must never install into the
# machine's system installation. `--disable-rofiles-fuse` is the documented
# trade-off for a machine without the FUSE helper: rofiles-fuse hardlinks the
# cache into the build tree, and giving that up only costs rebuild speed.
flatpak-builder --user --force-clean --disable-rofiles-fuse \
  --state-dir "$work/state" --repo "$work/repo" --default-branch stable \
  -s "Neezer $NEEZER_VERSION" -b "Paquet desktop de la version $NEEZER_VERSION" \
  "$work/build" "$work/fr.neezer.Desktop.yaml"

flatpak build-bundle --arch="$NEEZER_ARCH" --runtime-repo=https://flathub.org/repo/flathub.flatpakrepo \
  "$work/repo" "$out/neezer-desktop-$NEEZER_VERSION-$NEEZER_ARCH.flatpak" \
  fr.neezer.Desktop stable

neezer_note "$(du -h "$out/neezer-desktop-$NEEZER_VERSION-$NEEZER_ARCH.flatpak" | cut -f1)  \
$out/neezer-desktop-$NEEZER_VERSION-$NEEZER_ARCH.flatpak"
