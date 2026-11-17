#!/usr/bin/env bash
# Installs the Flatpak bundle for the CURRENT user (`flatpak --user`, never the
# system installation) and starts the app for real, on our own display.
#
# Usage: [NEEZER_FLATPAK_REMOTE=flathub] ./test.sh <chemin du .flatpak>
#   NEEZER_FLATPAK_UNINSTALL=1  remove it again afterwards (what CI does)
#   NEEZER_REQUIRE_PIPEWIRE=1   fail when the sandbox has no PipeWire socket
#                               (a desktop running PipeWire; a CI runner has none)
set -euo pipefail
source "$(dirname -- "${BASH_SOURCE[0]}")/../lib.sh"

bundle="$(realpath "${1:?chemin du .flatpak}")"
remote="${NEEZER_FLATPAK_REMOTE:-flathub}"
app="fr.neezer.Desktop"

# The runtime comes from a remote: `--runtime-repo` in the bundle names flathub,
# and this makes a machine that never added it work.
if ! flatpak --user remotes | grep -q "^$remote"; then
  flatpak --user remote-add --if-not-exists "$remote" \
    https://dl.flathub.org/repo/flathub.flatpakrepo
fi
flatpak --user install -y "flathub" org.freedesktop.Platform//25.08 >/dev/null 2>&1 || true

neezer_note "installation de $(basename "$bundle")"
flatpak --user install -y --noninteractive "$bundle" >&2
flatpak --user info "$app" >&2

# The permissions the manifest asked for, as they were actually written.
neezer_note "permissions installées :"
flatpak info --show-permissions "$app" | sed 's/^/    /' >&2

# The sidecar is INSIDE the sandbox beside the app (/app/bin), which is where the
# app looks first. A bundle that forgets it transcribes nothing.
neezer_note "sidecar du bac à sable : $(flatpak run --command=neezer-transcribe "$app" --version | head -1)"

# What the sandbox gives the audio engine and the GPU: printed, and required
# only where the host has them.
neezer_note "périphériques du bac à sable :"
flatpak run --command=sh "$app" -c 'ls -1 "$XDG_RUNTIME_DIR" 2>/dev/null | head -20; ls /dev/dri 2>/dev/null' |
  sed 's/^/    /' >&2
runtime_files="$(flatpak run --command=sh "$app" -c 'ls -1 "$XDG_RUNTIME_DIR" 2>/dev/null')"
if [ "${NEEZER_REQUIRE_PIPEWIRE:-0}" = 1 ]; then
  printf '%s\n' "$runtime_files" | grep -qx 'pipewire-0' ||
    neezer_die "pas de socket PipeWire dans le bac à sable ($runtime_files)"
fi

neezer_xvfb_start
neezer_expect_first_frame flatpak run "$app" --self-test-quit-after-first-frame
neezer_note "l'application Flatpak a peint sa première image et quitté"

if [ "${NEEZER_FLATPAK_UNINSTALL:-0}" = 1 ]; then
  flatpak --user uninstall -y "$app" >&2
fi
