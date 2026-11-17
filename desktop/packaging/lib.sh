# Shared by the four distribution packages (deb, rpm, AppImage, Flatpak).
#
# Everything here exists because ONE fact has to be true in five places:
#
#   - **One release build, four packages.** The `binaries` job of
#     `.github/workflows/release.yaml` builds `neezer-desktop` and
#     `neezer-transcribe` once and the four packaging jobs download that
#     artefact; the Arch package is the exception and builds the checkout itself
#     with `makepkg` on purpose (docs/agents/desktop.md, P1). `NEEZER_BINARY_DIR`
#     is where the two binaries are read from, so a local run can point at any
#     `target/release/`.
#   - **The tag names the version.** `NEEZER_DESKTOP_VERSION` is how the release
#     workflow passes it down (`v1.6.0` -> `1.6.0`), exactly as the PKGBUILD
#     takes it: a tag can never ship a package named after the workspace's
#     0.1.0.
#   - **One launcher entry and one icon**, the Arch package's: every format
#     installs the same `.desktop` and the same `neezer-desktop.svg`, so the
#     launcher, the window (`app_id = neezer-desktop`) and the MPRIS name agree
#     everywhere and there is one place to fix them.
#   - **`depends` follow what the binaries NEED**, not what the plan expected
#     (P1: namcap's `E:` lines were the missing `libxcb` and `libxkbcommon-x11`,
#     and `fontconfig` was a build dependency nobody would have guessed). The
#     table below is that lesson written once: an unknown soname in `readelf -d`
#     fails the build of EVERY format, so a new library cannot slip into a
#     package because nobody re-read the notes.
#
# Sourced by `deb/build.sh`, `rpm/build.sh`, `appimage/build.sh` and
# `flatpak/build.sh`; not executable on its own.

# shellcheck shell=bash

NEEZER_PACKAGING="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
NEEZER_ROOT="$(cd -- "$NEEZER_PACKAGING/../.." && pwd)"
# The launcher entry (`StartupWMClass=neezer-desktop`, `Icon=neezer-desktop`) and
# the icon: the Arch package's files, reused verbatim.
NEEZER_ASSETS="$NEEZER_PACKAGING/arch"
NEEZER_VERSION="${NEEZER_DESKTOP_VERSION:-0.1.0}"
NEEZER_BINARY_DIR="${NEEZER_BINARY_DIR:-$NEEZER_ROOT/desktop/target/release}"
# The name every artefact gets: neezer-desktop-<ver>-x86_64.<ext>.
NEEZER_ARCH="x86_64"

neezer_die() {
  printf '%s: %s\n' "${0##*/}" "$*" >&2
  exit 1
}

neezer_note() {
  printf '==> %s\n' "$*" >&2
}

# A tag is `v<digits and dots>`; nothing else is a version a package may carry.
neezer_check_version() {
  if [[ ! "$NEEZER_VERSION" =~ ^[0-9]+(\.[0-9]+)*$ ]]; then
    neezer_die "'$NEEZER_VERSION' is not <version> (digits and dots): set NEEZER_DESKTOP_VERSION"
  fi
}

# The two binaries, and nothing else: a package that ships one of them is an
# app whose local transcription says "le binaire est introuvable" on every
# episode (T2).
neezer_check_binaries() {
  local name
  for name in neezer-desktop neezer-transcribe; do
    [[ -x "$NEEZER_BINARY_DIR/$name" ]] ||
      neezer_die "pas de binaire $name dans $NEEZER_BINARY_DIR (cargo build --release --workspace)"
  done
}

# What a binary NEEDS, from its own dynamic section (never `ldd`: it runs the
# loader of the build machine and reads as clean on a machine that has
# everything installed — P1's fontconfig lesson).
neezer_needed() {
  readelf -d "$1" | sed -n 's/.*(NEEDED).*\[\(.*\)\]/\1/p'
}

# The sonames the two binaries are allowed to need, and the package that
# provides each one, per distribution.
#
# Debian and Ubuntu renamed several libraries in their 64-bit `time_t` transition
# (`libasound2` -> `libasound2t64`, `libpipewire-0.3-0` -> `...-0t64`), so the
# Debian dependency is written as the alternative that installs on both sides;
# RPM needs no such list because rpmbuild derives `Requires` from the same
# sonames itself.
# The highest `GLIBC_x.y` both binaries require, read from their own version
# references (`readelf --version-info`): a binary built on Debian 13 asks for
# 2.41, the release's Ubuntu 24.04 build asks for 2.39, and the package's
# `Depends:` says the truth instead of a number somebody typed.
neezer_glibc_floor() {
  local name
  {
    for name in neezer-desktop neezer-transcribe; do
      readelf --version-info "$NEEZER_BINARY_DIR/$name" |
        grep -oE 'GLIBC_[0-9]+(\.[0-9]+)+' | sed 's/^GLIBC_//'
    done
  } | sort -u -V | tail -1
}

neezer_soname_deb() {
  case "$1" in
  # The loader and the C library are the same package, and its floor is MEASURED
  # from the binaries' version references (neezer_glibc_floor), not written by
  # hand: the release is built on Ubuntu 24.04 (glibc 2.39, the oldest this
  # stack builds on), and a locally built package says what its own build needs.
  libc.so.6) echo "libc6 (>= $(neezer_glibc_floor))" ;;
  libm.so.6) echo "libc6 (>= $(neezer_glibc_floor))" ;;
  ld-linux-x86-64.so.2) echo "libc6 (>= $(neezer_glibc_floor))" ;;
  libgcc_s.so.1) echo 'libgcc-s1' ;;
  libstdc++.so.6) echo 'libstdc++6' ;;
  # cpal: the PipeWire host first, the ALSA host as fallback (docs/agents/desktop.md).
  libpipewire-0.3.so.0) echo 'libpipewire-0.3-0 | libpipewire-0.3-0t64' ;;
  libasound.so.2) echo 'libasound2 | libasound2t64' ;;
  # GPUI's X11 backend.
  libxcb.so.1) echo 'libxcb1' ;;
  libxkbcommon.so.0) echo 'libxkbcommon0' ;;
  libxkbcommon-x11.so.0) echo 'libxkbcommon-x11-0' ;;
  # whisper.cpp links the Vulkan loader directly (neezer-transcribe).
  libvulkan.so.1) echo 'libvulkan1' ;;
  *) return 1 ;;
  esac
}

# Every soname both binaries need must be known above; anything else fails.
neezer_check_needed() {
  local name soname unknown=0
  for name in neezer-desktop neezer-transcribe; do
    while read -r soname; do
      [[ -n "$soname" ]] || continue
      neezer_soname_deb "$soname" >/dev/null ||
        {
          printf '%s: soname inconnu %s\n' "$name" "$soname" >&2
          unknown=1
        }
    done < <(neezer_needed "$NEEZER_BINARY_DIR/$name")
  done
  ((unknown == 0)) ||
    neezer_die "un binaire lie une bibliothèque que lib.sh ne connaît pas : ajoutez-la à neezer_soname_deb et à la documentation du paquet"
}

# The Debian `Depends:` line, derived from the same table: the two binaries'
# sonames, deduplicated, in the table's order.
neezer_deb_depends() {
  local name soname deps=() dep seen
  for name in neezer-desktop neezer-transcribe; do
    while read -r soname; do
      [[ -n "$soname" ]] || continue
      dep="$(neezer_soname_deb "$soname")"
      seen=0
      for existing in "${deps[@]:-}"; do
        [[ "$existing" == "$dep" ]] && seen=1
      done
      ((seen)) || deps+=("$dep")
    done < <(neezer_needed "$NEEZER_BINARY_DIR/$name")
  done
  local IFS=', '
  printf '%s' "${deps[*]}"
}

# --- starting the installed app for real -----------------------------------
#
# The person's session is never touched: a package test starts its OWN X server
# on a high display number, refuses the one that is already exported, and kills
# it on exit. Same rule as the memory measurements (docs/agents/desktop.md,
# *Memory*: `Xvfb :105`, never the panel).
neezer_xvfb_start() {
  command -v Xvfb >/dev/null || neezer_die "Xvfb est absent : le test démarre son propre serveur X"
  # GPUI prefers Wayland whenever WAYLAND_DISPLAY is set, and a test that
  # inherits the session's (an X server of our own but a compositor of theirs)
  # reports `failed to connect to Wayland compositor`. Unset it: the display this
  # function creates is the display the app must use.
  unset WAYLAND_DISPLAY
  local number attempt
  for attempt in $(seq 10); do
    number=$((140 + RANDOM % 60))
    if [ "$DISPLAY" = ":$number" ] || [ -e "/tmp/.X11-unix/X$number" ]; then
      continue
    fi
    Xvfb ":$number" -screen 0 1280x800x24 -nolisten tcp >/dev/null 2>&1 &
    NEEZER_XVFB_PID=$!
    for _ in $(seq 100); do
      [ -S "/tmp/.X11-unix/X$number" ] && break
      sleep 0.05
    done
    if [ -S "/tmp/.X11-unix/X$number" ]; then
      NEEZER_DISPLAY=":$number"
      export DISPLAY="$NEEZER_DISPLAY"
      neezer_note "serveur X de test : DISPLAY=$DISPLAY"
      return 0
    fi
    kill "$NEEZER_XVFB_PID" 2>/dev/null || true
    wait "$NEEZER_XVFB_PID" 2>/dev/null || true
    NEEZER_XVFB_PID=""
  done
  neezer_die "impossible de démarrer un serveur X de test"
}

neezer_xvfb_stop() {
  if [ -n "${NEEZER_XVFB_PID:-}" ]; then
    kill "$NEEZER_XVFB_PID" 2>/dev/null || true
    wait "$NEEZER_XVFB_PID" 2>/dev/null || true
    NEEZER_XVFB_PID=""
  fi
}

# `--self-test-quit-after-first-frame` is the app's own proof that it PAINTED
# (docs/agents/desktop.md, D0/D7): it prints `first_frame_ms=<n>` and quits. A
# package that installs but cannot paint is not a package.
neezer_expect_first_frame() {
  local log
  log="$(mktemp "${TMPDIR:-/tmp}/neezer-frame.XXXXXX")"
  if ! "$@" >"$log" 2>&1; then
    cat "$log" >&2
    rm -f "$log"
    neezer_die "l'application n'a pas peint de première image ($*)"
  fi
  if ! grep -E '^first_frame_ms=[0-9]+$' "$log"; then
    cat "$log" >&2
    rm -f "$log"
    neezer_die "pas de first_frame_ms dans la sortie ($*)"
  fi
  rm -f "$log"
}

# The sidecar must be BESIDE the app binary: `wiring/transcribe.rs#program`
# resolves `current_exe().parent()` before `PATH`, so a package that puts them in
# two directories transcribes nothing (T2's "Le binaire est introuvable").
# `--version` on top of it proves the installed copy runs and which whisper.cpp
# and Vulkan device it links.
neezer_check_sidecar_installed() {
  local app_dir="$1" version_cmd="$2"
  [ -x "$app_dir/neezer-desktop" ] || neezer_die "$app_dir/neezer-desktop est absent"
  [ -x "$app_dir/neezer-transcribe" ] ||
    neezer_die "$app_dir/neezer-transcribe est absent : l'application ne trouvera pas son sidecar"
  neezer_note "sidecar installé à côté de l'application : $app_dir/neezer-transcribe"
  neezer_note "version du sidecar : $($version_cmd --version | head -1)"
}

# The `.desktop` entry must be valid where it was installed (a launcher that
# refuses the file is a launcher with no Neezer row).
neezer_check_desktop_file() {
  command -v desktop-file-validate >/dev/null || return 0
  desktop-file-validate "$1"
  neezer_note "desktop-file-validate : $1"
}
