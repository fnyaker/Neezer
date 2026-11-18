#!/usr/bin/env bash
# Prove that the engine binaries stay on their CPU floor, and that the machine
# picks the right one — `docs/agents/desktop.md`, *The CPU floor, and the child
# that picks its own ISA*.
#
# Three binaries matter and each has its own floor:
#
#   neezer-transcribe      x86-64-v2 (SSE4.2 + POPCNT: Nehalem 2008, Bulldozer 2011)
#   neezer-transcribe-v3   x86-64-v3 (AVX2 + FMA + F16C: Haswell 2013, Excavator 2015)
#   (an absent `-v3` is not an error: a build that ships only the floor binary is a
#    working install, just slower)
#
# The checks, in the order a failure is cheapest to read:
#   1. each binary's census, every ISA category counted (report: a stripped binary
#      cannot be attributed, and aws-lc-rs's `*_avx512` AES-GCM and rustfft's AVX
#      planners legitimately contain AVX-512 behind a CPUID check);
#   2. the exact ggml objects each binary was linked from, GATED at its own level —
#      the code this repository compiles;
#   3. the floor binary transcribing real speech under an EMULATED x86-64-v2 CPU
#      (`qemu-x86_64 -cpu Nehalem`) — the crash this exists for is `SIGILL` in 0.7 s —
#      and the `-v3` child transcribing NATIVELY on a host that has AVX2+FMA+F16C
#      (qemu's TCG emulates AVX2 through helper calls: eleven seconds took >10 min);
#   4. the DISPATCH: `engine-program` under both emulated CPUs must answer the
#      floor binary on Nehalem and the `-v3` one on Haswell.
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

# Where `whisper-rs-sys` keeps the tree it configured, per binary: the floor one
# next to `<profile>/`, the v3 one in `target/engine-v3/<profile>/` (its own target
# directory, see tools/build_engine_v3.sh).
if [ -z "$ggml" ]; then
    profile="$(dirname "$binary")"
    ggml="$(ls -d "$profile"/build/whisper-rs-sys-*/out/build/ggml/src 2>/dev/null | head -1 || true)"
    ggml_v3="$(ls -d "$profile"/../engine-v3/"$(basename "$profile")"/build/whisper-rs-sys-*/out/build/ggml/src 2>/dev/null | head -1 || true)"
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

if [ -f "$variant" ]; then
    echo "== 1bis. le relevé de la variante x86-64-v3 : $variant"
    python3 "$here/check_cpu_baseline.py" --report "$variant"
    if [ -n "${ggml_v3:-}" ] && [ -d "${ggml_v3:-}" ]; then
        echo "== 2bis. les objets de ggml de la variante, portillonnés à x86-64-v3 : $ggml_v3"
        python3 "$here/check_cpu_baseline.py" --baseline x86-64-v3 "$ggml_v3"
    elif [ "$skip_ggml" != 1 ]; then
        echo "== 2bis. aucun objet ggml pour la variante : étape 2bis non faite." >&2
        echo "   Compilez-la avec tools/build_engine_v3.sh (target/engine-v3)." >&2
        exit 2
    fi
fi

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

if [ -f "$variant" ]; then
    # The v3 child under EMULATION is not worth its wall clock: qemu's TCG emulates
    # AVX2/FMA through helper calls, and eleven seconds of speech took more than ten
    # minutes here (the floor binary under Nehalem: 140 s), which is not a check a
    # workflow can pay. What matters is proven twice, cheaper: the census above reads
    # its instructions (nothing above v3), and it is RUN here — natively, on a CPU that
    # has the three features, or not at all (a host without AVX2 must not run it, and
    # the dispatch below is what says such a host gets the floor binary).
    flags="$(grep -m1 '^flags' /proc/cpuinfo | cut -d: -f2)"
    can_v3=1
    for flag in avx2 fma f16c; do
        case " $flags " in *" $flag "*) ;; *) can_v3=0 ;; esac
    done
    echo "== 3bis. la variante x86-64-v3 sur ce CPU (natif)"
    if [ "$can_v3" = 1 ]; then
        transcribe_natively "$variant" "$(basename "$variant")"
    else
        echo "   ce CPU n'a pas AVX2+FMA+F16C : la variante n'est pas lancée (le"
        echo "   superviseur choisirait le binaire de plancher, étape 4)"
    fi
fi


# The dispatch: the supervisor asks the CPU, and the answer must be the binary whose
# floor this CPU meets. This is the check that a Nehalem-class laptop keeps working
# AND that a Haswell one gets the fast child.
echo "== 4. le choix du moteur selon le CPU"
# `engine-program` prints `current_exe()`, i.e. an ABSOLUTE path: compare resolved
# paths, or a relative `$binary` reads as a wrong answer on a right choice.
resolve() { readlink -f "$1" 2>/dev/null || printf '%s' "$1"; }
nehalem_choice="$(qemu-x86_64 -cpu Nehalem "$binary" engine-program)"
haswell_choice="$(qemu-x86_64 -cpu Haswell "$binary" engine-program)"
echo "   Nehalem -> $nehalem_choice"
echo "   Haswell -> $haswell_choice"
if [ "$(resolve "$nehalem_choice")" != "$(resolve "$binary")" ]; then
    echo "un CPU sans AVX2 doit garder $binary (il a choisi $nehalem_choice)" >&2
    exit 1
fi
if [ -f "$variant" ] && [ "$(resolve "$haswell_choice")" != "$(resolve "$variant")" ]; then
    echo "un CPU x86-64-v3 doit choisir $variant (il a choisi $haswell_choice)" >&2
    exit 1
fi

echo "== planchers tenus : $(basename "$binary") en x86-64-v2${variant:+ et $(basename "$variant") en x86-64-v3}"
