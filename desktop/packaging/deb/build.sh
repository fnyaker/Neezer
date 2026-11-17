#!/usr/bin/env bash
# Builds neezer-desktop-<ver>-x86_64.deb from the release build that already
# exists (the `binaries` job of .github/workflows/release.yaml, or a local
# `cargo build --release --workspace`).
#
# Why a staged tree and `dpkg-deb` instead of the two cargo subcommands:
#
#   - `cargo-deb` (3.8.0, https://github.com/kornelski/cargo-deb) is the
#     obvious tool, and it does everything we need — except that it wants to run
#     `cargo build` itself. It has `--no-build`, so it CAN package a prebuilt
#     tree, but it also insists on `target/release/` asset paths and its own
#     metadata living in `Cargo.toml` of ONE crate (the workspace has nine, and
#     `neezer-desktop` is not the crate that knows about `neezer-transcribe`).
#     Keeping the packaging in `desktop/packaging/` and out of the manifests is
#     what the Arch package already does (P1).
#   - The `Depends:` line is generated from `readelf -d`'s NEEDED entries and
#     an explicit soname -> package table in `../lib.sh`, because the Debian
#     names differ per release in the 64-bit `time_t` transition
#     (`libpipewire-0.3-0t64` on trixie, `libpipewire-0.3-0` on bookworm) and
#     `dpkg-shlibdeps` would write the BUILD machine's names into the control
#     file: a package that installs nowhere else. The table is the gate; the
#     real proof is `dpkg -i` in the container (`deb/test.sh`).
#   - `Recommends: fonts-dejavu-core` is not decoration: GPUI resolves a sans font
#     at startup and ABORTS when fontconfig has none (measured in a bare Debian
#     container: `failed to resolve font '.SystemUIFont' or any of the fallbacks`).
#     Any desktop has one, so it is a Recommends, not a Depends — any of Ubuntu,
#     Adwaita Sans, Cantarell, Noto Sans, DejaVu Sans or Arial satisfies it.
#
# Usage: NEEZER_DESKTOP_VERSION=1.6.0 NEEZER_BINARY_DIR=… ./build.sh [outdir]
set -euo pipefail
source "$(dirname -- "${BASH_SOURCE[0]}")/../lib.sh"

neezer_check_version
neezer_check_binaries
neezer_check_needed

out="${1:-$PWD}"
work="$(mktemp -d "${TMPDIR:-/tmp}/neezer-deb.XXXXXX")"
trap 'rm -rf "$work"' EXIT
stage="$work/neezer-desktop"

install -d "$stage/DEBIAN" \
  "$stage/usr/bin" \
  "$stage/usr/share/applications" \
  "$stage/usr/share/icons/hicolor/scalable/apps" \
  "$stage/usr/share/doc/neezer-desktop"

# The two binaries. `neezer-transcribe` sits BESIDE `neezer-desktop` because that
# is where the app looks for it (`wiring/transcribe.rs#program`), and `PATH` is
# the second chance for a person who moved it.
install -m 755 "$NEEZER_BINARY_DIR/neezer-desktop" "$stage/usr/bin/neezer-desktop"
install -m 755 "$NEEZER_BINARY_DIR/neezer-transcribe" "$stage/usr/bin/neezer-transcribe"
install -m 644 "$NEEZER_ASSETS/neezer-desktop.desktop" \
  "$stage/usr/share/applications/neezer-desktop.desktop"
install -m 644 "$NEEZER_ASSETS/neezer-desktop.svg" \
  "$stage/usr/share/icons/hicolor/scalable/apps/neezer-desktop.svg"

# Debian policy: the licence text is `copyright` under the package's doc
# directory, and a changelog is expected there too.
install -m 644 "$NEEZER_ROOT/LICENSE" "$stage/usr/share/doc/neezer-desktop/copyright"
{
  printf 'neezer-desktop (%s-1) unstable; urgency=medium\n\n' "$NEEZER_VERSION"
  printf '  * Version %s du paquet, construite depuis l arbre etiquete.\n' "$NEEZER_VERSION"
  printf '\n -- the Neezer project <https://github.com/fnyaker/Neezer>  %s\n' \
    "$(date -u '+%a, %d %b %Y %H:%M:%S +0000')"
} | gzip -9n >"$stage/usr/share/doc/neezer-desktop/changelog.gz"
chmod 644 "$stage/usr/share/doc/neezer-desktop/changelog.gz"

depends="$(neezer_deb_depends)"
installed_size="$(du -sk --exclude=DEBIAN "$stage" | cut -f1)"
# `#` as the delimiter, not `|`: a Debian dependency alternative is written with
# a pipe (`libasound2 | libasound2t64`).
sed -e "s#@VERSION@#$NEEZER_VERSION#g" \
  -e "s#@DEPENDS@#$depends#g" \
  -e "s#@INSTALLED_SIZE@#$installed_size#g" \
  "$NEEZER_PACKAGING/deb/control.in" >"$stage/DEBIAN/control"
chmod 644 "$stage/DEBIAN/control"

mkdir -p "$out"
deb="$out/neezer-desktop-$NEEZER_VERSION-$NEEZER_ARCH.deb"
# `--root-owner-group`: no fakeroot needed, every file is root:root in the
# archive. `-Zxz` rather than the tool's default, which moved to zstd in newer
# dpkg and would be unreadable to an older `dpkg -i`.
dpkg-deb --root-owner-group -Zxz --build "$stage" "$deb" >/dev/null

neezer_note "Depends: $depends"
dpkg-deb --info "$deb" | sed -n '1,20p' >&2
neezer_note "$(du -h "$deb" | cut -f1)  $deb"
