#!/usr/bin/env bash
# Builds neezer-desktop-<ver>-x86_64.rpm from the release binaries that already
# exist, with the distro's own `rpmbuild` (see neezer-desktop.spec for why this
# rather than `cargo-generate-rpm`).
#
# Usage: NEEZER_DESKTOP_VERSION=1.6.0 NEEZER_BINARY_DIR=… ./build.sh [outdir]
set -euo pipefail
source "$(dirname -- "${BASH_SOURCE[0]}")/../lib.sh"

neezer_check_version
neezer_check_binaries
neezer_check_needed

command -v rpmbuild >/dev/null || neezer_die "rpmbuild est absent (dnf install rpm-build)"

out="${1:-$PWD}"
work="$(mktemp -d "${TMPDIR:-/tmp}/neezer-rpm.XXXXXX")"
trap 'rm -rf "$work"' EXIT

stage="$work/stage"
install -d "$stage"
install -m 755 "$NEEZER_BINARY_DIR/neezer-desktop" "$stage/neezer-desktop"
install -m 755 "$NEEZER_BINARY_DIR/neezer-transcribe" "$stage/neezer-transcribe"
# The x86-64-v3 engine child in the SAME directory: `supervisor::engine_program`
# derives it from `current_exe()`, so a second directory would never be found.
install -m 755 "$NEEZER_BINARY_DIR/neezer-transcribe-v3" "$stage/neezer-transcribe-v3"
install -m 755 "$NEEZER_BINARY_DIR/neezer-transcribe-v4" "$stage/neezer-transcribe-v4"
install -m 644 "$NEEZER_ASSETS/neezer-desktop.desktop" "$stage/neezer-desktop.desktop"
install -m 644 "$NEEZER_ASSETS/neezer-desktop.svg" "$stage/neezer-desktop.svg"
install -m 644 "$NEEZER_ROOT/LICENSE" "$stage/LICENSE"

mkdir -p "$out"
rpmbuild --define "_topdir $work/rpmbuild" \
  --define "_neezer_stage $stage" \
  --define "neezer_version $NEEZER_VERSION" \
  --define "_binary_payload w9.xzdio" \
  --buildroot "$work/buildroot" \
  -bb "$NEEZER_PACKAGING/rpm/neezer-desktop.spec" >"$work/rpmbuild.log" 2>&1 ||
  {
    cat "$work/rpmbuild.log" >&2
    exit 1
  }

built="$(ls "$work"/rpmbuild/RPMS/*/neezer-desktop-"$NEEZER_VERSION"-*.rpm)" ||
  neezer_die "rpmbuild n'a produit aucun paquet"
# The release attaches neezer-desktop-<ver>-<arch>.rpm for all five formats; the
# RPM's own release field stays readable inside the file (`rpm -qpi`).
rpm="$out/neezer-desktop-$NEEZER_VERSION-$NEEZER_ARCH.rpm"
cp "$built" "$rpm"

neezer_note "Requires (générées par rpmbuild):"
rpm -qp --requires "$rpm" | sed 's/^/    /' >&2
neezer_note "$(du -h "$rpm" | cut -f1)  $rpm"
