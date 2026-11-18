#!/usr/bin/env bash
# Prove that the engine binaries stay on their CPU floor, and that the machine
# picks the right one — `docs/agents/desktop.md`, *The CPU floor, and the child
# that picks its own ISA*.
#
# Three binaries matter and each has its own floor:
#
#   neezer-transcribe      x86-64-v2 (SSE4.2 + POPCNT: Nehalem 2008, Bulldozer 2011)
#   neezer-transcribe-v3   x86-64-v3 (AVX2 + FMA + F16C: Haswell 2013, Excavator 2015)
#   neezer-transcribe-v4   x86-64-v4 (AVX512F/CD/BW/DQ/VL: Skylake-Server 2017, Zen 4 2022)
#   (an absent child is not an error: a build that ships only the floor binary is a
#    working install, just slower — the dispatch then skips it)
#
# The checks, in the order a failure is cheapest to read:
#   1. each binary's census, every ISA category counted (report: a stripped binary
#      cannot be attributed, and aws-lc-rs's `*_avx512` AES-GCM and rustfft's AVX
#      planners legitimately contain AVX-512 behind a CPUID check);
#   2. the exact ggml objects each binary was linked from, GATED at its own level —
#      the code this repository compiles (v4's gate still refuses VNNI/BF16/VBMI: a
#      child that needs one SIGILLs on a CPU that has v4 without it);
#   3. the floor binary transcribing real speech under an EMULATED x86-64-v2 CPU
#      (`qemu-x86_64 -cpu Nehalem`) — the crash this exists for is `SIGILL` in 0.7 s —
#      and each child transcribing NATIVELY when this host has its features (qemu's
#      TCG emulates AVX2 through helper calls: eleven seconds took >10 min);
#   4. the DISPATCH: `engine-program` under an emulated CPU of each level must answer
#      Nehalem -> floor, Haswell -> -v3, Skylake-Server -> -v4.
#
# Usage:
#   desktop/tools/check_cpu_floor.sh <binary> [--ggml <dir>] [--no-ggml]
#   NZ_WHISPER_MODEL=<ggml-tiny.bin> NZ_WHISPER_AUDIO=<jfk.wav> \
#       desktop/tools/check_cpu_floor.sh <binary>          # + steps 3 and 4
#
# The two fixture variables are the crate's own (`crates/nz-transcribe/tests/engine.rs`),
# so CI sets one name everywhere. Without them the script says it skipped the
# emulated runs — and a workflow that ships these binaries must set them.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
binary=""
ggml=""
skip_ggml=0

while [ $# -gt 0 ]; do
    case "$1" in
        --ggml) ggml="$2"; shift 2 ;;
        --no-ggml) skip_ggml=1; shift ;;
        -h|--help) sed -n '2,32p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) binary="$1"; shift ;;
    esac
done

if [ -z "$binary" ] || [ ! -f "$binary" ]; then
    echo "usage: $0 <binary> [--ggml <dir>] [--no-ggml]" >&2
    exit 2
fi
variant="${binary}-v3"
variant4="${binary}-v4"

# Where `whisper-rs-sys` keeps the tree it configured, per binary: the floor one
# next to `<profile>/`, the v3 one in `target/engine-v3/<profile>/` (its own target
# directory, see tools/build_engine_variants.sh).
if [ -z "$ggml" ]; then
    profile="$(dirname "$binary")"
    ggml="$(ls -d "$profile"/build/whisper-rs-sys-*/out/build/ggml/src 2>/dev/null | head -1 || true)"
    ggml_v3="$(ls -d "$profile"/../engine-v3/"$(basename "$profile")"/build/whisper-rs-sys-*/out/build/ggml/src 2>/dev/null | head -1 || true)"
    ggml_v4="$(ls -d "$profile"/../engine-v4/"$(basename "$profile")"/build/whisper-rs-sys-*/out/build/ggml/src 2>/dev/null | head -1 || true)"
fi

echo "== 1. le relevé du binaire (toutes les catégories, aucun portillon) : $binary"
python3 "$here/check_cpu_baseline.py" --report "$binary"

if [ "$skip_ggml" = 1 ]; then
    echo "== 2. objets de ggml non vérifiés ici (--no-ggml) : l'arbre de compilation"
    echo "   n'existe plus (paquet construit par makepkg --clean) ; le job qui les"
    echo "   produit les portillonne, et l'étape 3 prouve ces binaires-ci."
elif [ -n "$ggml" ] && [ -d "$ggml" ]; then
    echo "== 2. les objets de ggml de $binary, portillonnés à x86-64-v2 : $ggml"
    python3 "$here/check_cpu_baseline.py" --baseline x86-64-v2 "$ggml"
else
    echo "== 2. aucun objet ggml trouvé près de $binary : étape 2 non faite." >&2
    echo "   (passez --ggml <dir>, --no-ggml, ou lancez ce script depuis un arbre compilé)" >&2
    exit 2
fi

# Each child, at ITS level: its census, then its own ggml objects gated.
for level in v3 v4; do
    case "$level" in
        v3) child="$variant"; objects="${ggml_v3:-}" ;;
        v4) child="$variant4"; objects="${ggml_v4:-}" ;;
    esac
    [ -f "$child" ] || continue
    case "$level" in
        v3) baseline=x86-64-v3 ;;
        v4) baseline=x86-64-v4 ;;
    esac
    echo "== 1bis. le relevé de la variante $baseline : $child"
    python3 "$here/check_cpu_baseline.py" --report "$child"
    if [ -n "$objects" ] && [ -d "$objects" ]; then
        echo "== 2bis. les objets de ggml de la variante, portillonnés à $baseline : $objects"
        python3 "$here/check_cpu_baseline.py" --baseline "$baseline" "$objects"
    elif [ "$skip_ggml" != 1 ]; then
        echo "== 2bis. aucun objet ggml pour $child : étape non faite." >&2
        echo "   Compilez-la avec tools/build_engine_variants.sh (target/engine-$level)." >&2
        exit 2
    fi
done

model="${NZ_WHISPER_MODEL:-}"
audio="${NZ_WHISPER_AUDIO:-}"
if [ -z "$model" ] || [ -z "$audio" ]; then
    echo "== 3. NZ_WHISPER_MODEL/NZ_WHISPER_AUDIO absents : les binaires n'ont PAS été"
    echo "   lancés sous un CPU émulé de leur plancher. Posez-les pour la preuve." >&2
    exit 2
fi
for path in "$model" "$audio"; do
    [ -f "$path" ] || { echo "fixture introuvable : $path" >&2; exit 2; }
done
command -v qemu-x86_64 >/dev/null ||
    { echo "qemu-x86_64 absent : installez qemu-user (Debian/Ubuntu)" >&2; exit 2; }

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

# One transcription under one emulated CPU, asserted on what was said: `jfk.wav` is
# eleven seconds of English, the same sentence the engine tests pin.
transcribe_under() {
    local cpu="$1" program="$2" label="$3"
    local out="$work/$(basename "$program")-$cpu.json"
    started=$(date +%s)
    qemu-x86_64 -cpu "$cpu" "$program" file "$audio" --model "$model" \
        --lang en --cpu --out "$out"
    seconds=$(( $(date +%s) - started ))
    python3 - "$out" "$cpu" "$seconds" "$label" <<'PY'
import json
import sys

path, cpu, seconds, label = sys.argv[1:5]
document = json.load(open(path, encoding="utf-8"))
text = " ".join(line.get("text", "") for line in document["transcription"]).lower()
if "ask not what your country" not in text:
    sys.exit(f"{label} sous {cpu} : transcription inattendue : {text!r}")
print(f"   {label} sous {cpu} : {len(document['transcription'])} ligne(s) en {seconds} s"
      f" — {text.strip()[:60]}…")
PY
}

# Native: the same assertion, on this machine's own CPU.
transcribe_natively() {
    local program="$1" label="$2"
    local out="$work/$(basename "$program")-native.json"
    started=$(date +%s)
    "$program" file "$audio" --model "$model" --lang en --cpu --out "$out"
    seconds=$(( $(date +%s) - started ))
    python3 - "$out" "$label" "$seconds" <<'PY2'
import json
import sys

path, label, seconds = sys.argv[1:4]
document = json.load(open(path, encoding="utf-8"))
text = " ".join(line.get("text", "") for line in document["transcription"]).lower()
if "ask not what your country" not in text:
    sys.exit(f"{label} (natif) : transcription inattendue : {text!r}")
print(f"   {label} (natif) : {len(document['transcription'])} ligne(s) en {seconds} s"
      f" — {text.strip()[:60]}…")
PY2
}

echo "== 3. le binaire de plancher sous un CPU émulé x86-64-v2 (Nehalem)"
transcribe_under Nehalem "$binary" "$(basename "$binary")"

flags="$(grep -m1 '^flags' /proc/cpuinfo | cut -d: -f2)"
has() { case " $flags " in *" $1 "*) return 0 ;; *) return 1 ;; esac; }
can_run() {
    local level="$1" flag
    case "$level" in
        v3) set -- avx2 fma f16c ;;
        v4) set -- avx2 fma f16c avx512f avx512cd avx512bw avx512dq avx512vl ;;
    esac
    for flag in "$@"; do has "$flag" || return 1; done
    return 0
}

for level in v3 v4; do
    case "$level" in
        v3) child="$variant" ;;
        v4) child="$variant4" ;;
    esac
    [ -f "$child" ] || continue
    echo "== 3bis. la variante x86-64-$level sur ce CPU (natif)"
    if can_run "$level"; then
        transcribe_natively "$child" "$(basename "$child")"
    else
        echo "   ce CPU n'a pas les instructions de $level : la variante n'est pas"
        echo "   lancée (le superviseur choisirait un moteur plus bas, étape 4)"
    fi
done

echo "== 4. le choix du moteur selon le CPU"
# `engine-program` prints `current_exe()`, i.e. an ABSOLUTE path: compare resolved
# paths, or a relative `$binary` reads as a wrong answer on a right choice.
resolve() { readlink -f "$1" 2>/dev/null || printf '%s' "$1"; }
# One emulated CPU per level QEMU's TCG can actually emulate: Nehalem (no AVX2) and
# Haswell (AVX2+FMA+F16C). AVX-512 is NOT among them — measured: `-cpu Skylake-Server`
# and even `-cpu max` clear all five v4 bits with "TCG doesn't support requested
# feature: … avx512f/dq/cd/bw/vl" — so the v4 rung is proven twice instead: natively
# here below (the choice must match THIS host's own flags), and by supervisor's own
# unit test with an injected `Simd` (a v4 CPU with `-v4` installed gets it).
for pair in "Nehalem:$binary" "Haswell:$variant"; do
    cpu="${pair%%:*}"
    expected="${pair#*:}"
    [ -f "$expected" ] || continue
    choice="$(qemu-x86_64 -cpu "$cpu" "$binary" engine-program)"
    echo "   $cpu -> $choice"
    if [ "$(resolve "$choice")" != "$(resolve "$expected")" ]; then
        echo "un CPU $cpu doit choisir $expected (il a choisi $choice)" >&2
        exit 1
    fi
done
# The same question on the real CPU: the implementation and this expectation are
# written from the same three facts, so a divergence is a red check.
expected_native="$binary"
if can_run v3 && [ -f "$variant" ]; then expected_native="$variant"; fi
if can_run v4 && [ -f "$variant4" ]; then expected_native="$variant4"; fi
choice="$("$binary" engine-program)"
echo "   ce CPU -> $choice"
if [ "$(resolve "$choice")" != "$(resolve "$expected_native")" ]; then
    echo "ce CPU doit choisir $expected_native (il a choisi $choice)" >&2
    exit 1
fi

echo "== planchers tenus : $(basename "$binary") en x86-64-v2, et les variantes installées à leur niveau"
