#!/usr/bin/env bash
# Build the faster engine children beside the floor binary:
# `neezer-transcribe-v3` and `neezer-transcribe-v4`.
#
# `docs/agents/desktop.md` (*The CPU floor, and the children that pick their own
# ISA*) is the why. In one line: ggml's instruction set is fixed at BUILD time, so
# ONE engine binary is either portable and slow (x86-64-v2: 5.0× real time on the
# CPU) or fast and dead on an older CPU (`SIGILL (signal 4)`, CI run 44234946582 —
# the published packages would do the same on a stranger's laptop). The engine runs
# in a child process, so the supervisor (`supervisor::engine_program`) starts the
# best child the CPU can run — `-v4` (x86-64-v4) then `-v3` (x86-64-v3) then the
# floor binary. Measured on one 330 s excerpt: 5.0× / 23.5× / 26.7× (native 28.7×).
#
# One cargo build per level, each in its OWN target directory: cargo builds ONE copy
# of a dependency per graph, so the same `whisper-rs-sys` cannot be compiled three
# times with three `GGML_*` sets in one `target/`. `target/engine-<level>` sits
# INSIDE `target/` so the Rust cache carries them like the rest (~2 min 30 s each
# cold, warm afterwards).
#
# Usage:
#   desktop/tools/build_engine_variants.sh [--release] [v3|v4 ...]  # default: both, --debug
#
# The results are `target/<profile>/neezer-transcribe-v3` and `-v4`; nothing else
# moves, so a caller's `cargo build --workspace` (the floor binary) is untouched.
set -euo pipefail

profile=debug
flag=""
levels=()
for argument in "$@"; do
    case "$argument" in
        --release) profile=release; flag="--release" ;;
        --debug) profile=debug; flag="" ;;
        v3|v4) levels+=("$argument") ;;
        -h|--help) sed -n '2,24p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "argument inconnu : $argument" >&2; exit 2 ;;
    esac
done
[ ${#levels[@]} -gt 0 ] || levels=(v3 v4)

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
# subtrees and the copies follow whatever target root the caller set.
targets="${CARGO_TARGET_DIR:-target}"
cargo_flags=(--locked)
[ "${NEEZER_FROZEN:-}" = 1 ] && cargo_flags=(--frozen)

for level in "${levels[@]}"; do
    cargo build -p nz-transcribe ${flag} "${cargo_flags[@]}" \
        --target-dir "$targets/engine-$level" --config ".cargo/engine-$level.toml"
    built="$targets/engine-$level/$profile/neezer-transcribe"
    [ -x "$built" ] || { echo "le binaire $built n'existe pas" >&2; exit 1; }
    install -m 0755 "$built" "$targets/$profile/neezer-transcribe-$level"
done

# The baseline may not exist yet (a caller that only wants the children): say the
# sizes when it does, so the packaging cost of each extra file is never a guess.
for name in "" "-v3" "-v4"; do
    [ -x "$targets/$profile/neezer-transcribe$name" ] &&
        du -h "$targets/$profile/neezer-transcribe$name"
done
