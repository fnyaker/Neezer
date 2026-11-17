#!/usr/bin/env bash
# Prints the version a package must carry.
#
# A release tag names it (`v1.6.0` -> `1.6.0`); anything else — a branch push, a
# pull request — keeps the workspace's own 0.1.0, so the package built there is
# the one the tag will ship (the Arch PKGBUILD's rule, P1). A tag that is not
# `v<digits and dots>` fails instead of shipping a package with a made-up
# version.
#
# `NEEZER_DESKTOP_VERSION` wins when it is already set, which is how the release
# workflow passes the resolved version down to the four build scripts.
set -euo pipefail

source "$(dirname -- "${BASH_SOURCE[0]}")/lib.sh"

if [ -n "${NEEZER_DESKTOP_VERSION:-}" ]; then
  version="$NEEZER_DESKTOP_VERSION"
elif [ "${GITHUB_EVENT_NAME:-}" = "workflow_dispatch" ] && [ -n "${TAG:-}" ]; then
  version="${TAG#v}"
elif [[ "${GITHUB_REF:-}" == refs/tags/v* ]]; then
  version="${GITHUB_REF#refs/tags/v}"
else
  version="0.1.0"
fi

if [[ ! "$version" =~ ^[0-9]+(\.[0-9]+)*$ ]]; then
  neezer_die "'$version' is not <version> (digits and dots)"
fi
printf '%s\n' "$version"
