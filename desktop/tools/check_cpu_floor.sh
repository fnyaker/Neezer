#!/usr/bin/env bash
# Prove that one shipped binary stays on the desktop's CPU floor (x86-64-v2).
#
# `docs/agents/desktop.md` (*The CPU floor*) explains why: ggml's instruction set
# is decided at BUILD time (`GGML_NATIVE` defaults to `-march=native`), so the same
# commit yields an AVX-512 binary on one builder and an SSE4.2 one on another — and
# `release.yaml` builds the desktop binaries ONCE for five package formats that land
# on other people's laptops. `desktop/.cargo/config.toml` pins the cmake flags;
# this script is the three proofs that the pin reached the artefacts, in the order a
# failure is cheapest to read:
#
#   1. the binary's census, every ISA category counted (report: a stripped binary
#      cannot be attributed, and two dependencies — aws-lc-rs's `*_avx512` AES-GCM,
#      rustfft's AVX planners — legitimately contain AVX-512 behind a CPUID check);
#   2. the exact ggml objects that binary was linked from, GATED: nothing above the
#      floor may exist there, which is the code this repository compiles;
#   3. the binary itself running on an EMULATED x86-64-v2 CPU (`qemu-x86_64 -cpu
#      Nehalem`), on a real model and real speech, which is the only check that
#      proves the whole file — its transcript must be the expected English.
#
# Usage:
#   desktop/tools/check_cpu_floor.sh <binary> [--ggml <dir>]
#   NZ_WHISPER_MODEL=<ggml-tiny.bin> NZ_WHISPER_AUDIO=<jfk.wav> \
#       desktop/tools/check_cpu_floor.sh <binary>          # + the qemu run
#
# The two fixture variables are the crate's own (`crates/nz-transcribe/tests/engine.rs`),
# so CI sets one name everywhere. Without them step 3 says it was skipped — and a
# workflow that needs it must set them, or the run is not the check this file is.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
binary=""
ggml=""
skip_ggml=0

while [ $# -gt 0 ]; do
    case "$1" in
        --ggml) ggml="$2"; shift 2 ;;
        --no-ggml) skip_ggml=1; shift ;;
        -h|--help) sed -n '2,25p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) binary="$1"; shift ;;
    esac
done

if [ -z "$binary" ] || [ ! -f "$binary" ]; then
    echo "usage: $0 <binary> [--ggml <dir avec les objets de ggml>] [--no-ggml]" >&2
    exit 2
fi

# Where `whisper-rs-sys` keeps the tree it configured: the objects next to the
# binary's profile directory, whether it is debug or release.
if [ -z "$ggml" ]; then
    profile="$(dirname "$binary")"
    ggml="$(ls -d "$profile"/build/whisper-rs-sys-*/out/build/ggml/src 2>/dev/null | head -1 || true)"
fi

echo "== 1. le relevé du binaire (toutes les catégories, aucun portillon) : $binary"
python3 "$here/check_cpu_baseline.py" --report "$binary"

if [ "$skip_ggml" = 1 ]; then
    echo "== 2. objets de ggml non vérifiés ici (--no-ggml) : l'arbre de compilation"
    echo "   n'existe plus (paquet construit par makepkg --clean) ; le job qui les"
    echo "   produit les portillonne, et l'étape 3 prouve ce binaire-ci."
elif [ -n "$ggml" ] && [ -d "$ggml" ]; then
    echo "== 2. les objets de ggml qui ont produit ce binaire, portillonnés : $ggml"
    python3 "$here/check_cpu_baseline.py" "$ggml"
else
    echo "== 2. aucun objet ggml trouvé près de $binary : étape 2 non faite." >&2
    echo "   (passez --ggml <dir>, --no-ggml, ou lancez ce script depuis un arbre qui vient d'être compilé)" >&2
    exit 2
fi

model="${NZ_WHISPER_MODEL:-}"
audio="${NZ_WHISPER_AUDIO:-}"
if [ -z "$model" ] || [ -z "$audio" ]; then
    echo "== 3. NZ_WHISPER_MODEL/NZ_WHISPER_AUDIO absents : le binaire n'a PAS été"
    echo "   lancé sous un CPU émulé de base. Posez-les pour la preuve complète." >&2
    exit 2
fi
for path in "$model" "$audio"; do
    [ -f "$path" ] || { echo "fixture introuvable : $path" >&2; exit 2; }
done
command -v qemu-x86_64 >/dev/null ||
    { echo "qemu-x86_64 absent : installez qemu-user (Debian/Ubuntu)" >&2; exit 2; }

echo "== 3. le binaire sous un CPU émulé x86-64-v2 (Nehalem), sur du vrai discours"
out="$(mktemp -d)"
trap 'rm -rf "$out"' EXIT
started=$(date +%s)
qemu-x86_64 -cpu Nehalem "$binary" file "$audio" --model "$model" \
    --lang en --cpu --out "$out/transcript.json"
seconds=$(( $(date +%s) - started ))
python3 - "$out/transcript.json" "$seconds" <<'PY'
import json
import sys

path, seconds = sys.argv[1], sys.argv[2]
document = json.load(open(path, encoding="utf-8"))
text = " ".join(line.get("text", "") for line in document["transcription"]).lower()
# `jfk.wav` is eleven seconds of English; the engine tests assert the same words.
if "ask not what your country" not in text:
    sys.exit(f"Nehalem : transcription inattendue : {text!r}")
print(f"   Nehalem : {len(document['transcription'])} ligne(s) en {seconds} s — {text.strip()[:70]}…")
PY
echo "== plancher x86-64-v2 tenu par $binary"
