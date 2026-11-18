#!/usr/bin/env bash
# Build the x86-64-v3 engine child beside the floor binary: `neezer-transcribe-v3`.
#
# `docs/agents/desktop.md` (*The CPU floor, and the child that picks its own ISA*)
# is the why. In one line: ggml's instruction set is fixed at BUILD time, so ONE
# engine binary is either portable and slow (x86-64-v2: 5.4× real time on the CPU)
# or fast and dead on an older CPU (`SIGILL (signal 4)`, CI run 44234946582 — the
# published packages would do the same on a stranger's laptop). The engine already
# runs in a child process, so the supervisor (`supervisor::engine_program`) starts
# the `-v3` one where the CPU answers AVX2+FMA+F16C and the floor one everywhere
# else. This script produces that second file.
#
# Two cargo builds, two target directories, on purpose: cargo builds ONE copy of a
# dependency per graph, so the same `whisper-rs-sys` cannot be compiled twice with
# two different `GGML_*` sets in one `target/`. `target/engine-v3` sits INSIDE
# `target/` so the CI's Rust cache carries it like the rest (it is warm after the
# first run: the build is ~2 min 30 s cold, the same as the baseline's whisper).
#
# Usage:
#   desktop/tools/build_engine_v3.sh [--release]     # default: --debug
#
# The result is `target/<profile>/neezer-transcribe-v3`; nothing else moves, so a
# caller's `cargo build --workspace` (the floor binary) is untouched.
set -euo pipefail

profile=debug
flag=""
for argument in "$@"; do
    case "$argument" in
        --release) profile=release; flag="--release" ;;
        --debug) profile=debug; flag="" ;;
        -h|--help) sed -n '2,20p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "argument inconnu : $argument" >&2; exit 2 ;;
    esac
done

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
desktop="$(cd "$here/.." && pwd)"
cd "$desktop"

# The Vulkan headers are whisper.cpp's CMake dependency, not ours: whisper-rs-sys
# does not read `VULKAN_SDK` on Linux, CMake's `FindVulkan` does (T2's pitfall).
if [ -z "${VULKAN_SDK:-}" ] && [ ! -f /usr/include/vulkan/vulkan.h ]; then
    echo "VULKAN_SDK (en-têtes Vulkan) ou /usr/include/vulkan/vulkan.h requis" >&2
    exit 2
fi

# `makepkg` builds into `$srcdir/target` through CARGO_TARGET_DIR (P1), so the
# subtree and the copy follow whatever target root the caller set.
targets="${CARGO_TARGET_DIR:-target}"
cargo_flags=(--locked)
[ "${NEEZER_FROZEN:-}" = 1 ] && cargo_flags=(--frozen)

cargo build -p nz-transcribe ${flag} "${cargo_flags[@]}" \
    --target-dir "$targets/engine-v3" --config .cargo/engine-v3.toml

built="$targets/engine-v3/$profile/neezer-transcribe"
[ -x "$built" ] || { echo "le binaire $built n'existe pas" >&2; exit 1; }
install -m 0755 "$built" "$targets/$profile/neezer-transcribe-v3"

# The baseline may not exist yet (a caller that only wants the variant): say the
# pair's sizes when both are there, so the packaging cost of the second file is
# never a guess.
if [ -x "$targets/$profile/neezer-transcribe" ]; then
    du -h "$targets/$profile/neezer-transcribe" "$targets/$profile/neezer-transcribe-v3"
else
    du -h "$targets/$profile/neezer-transcribe-v3"
fi
