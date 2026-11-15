#!/usr/bin/env bash
set -euo pipefail

# Host-only: the model is mounted into the sidecar, never baked into an image.
if [ -f /.dockerenv ] && [ "${ALLOW_IN_CONTAINER:-0}" != "1" ]; then
    echo "refus: ce script tourne sur l'hôte, pas dans un conteneur." >&2
    echo "       Le modèle est monté en lecture seule sur /models." >&2
    exit 2
fi

usage() {
    cat <<'EOF'
Usage: tools/fetch-whisper-model.sh --model <nom> [--dest <dossier>] [--force]

Télécharge un modèle GGML officiel de whisper.cpp, vérifie sa taille et son
SHA-256, puis l'installe dans <dossier> (défaut : ./models).

Modèles :
  large-v3-turbo-q8_0   874 MB (recommandé avec >= 2,5 Go de RAM)
  small                 466 MB (RAM plus serrée)

Options :
  --model, -m <nom>     modèle à télécharger (obligatoire)
  --dest, -d <dossier>  dossier de destination (défaut : ./models)
  --force, -f           remplace un fichier existant au checksum différent
  --help, -h            cette aide

Le checksum est épinglé dans ce script : un fichier absent du tableau est
refusé, et un téléchargement dont le SHA-256 diffère n'est jamais installé.
EOF
}

MODEL=""
DEST="./models"
FORCE=0

while [ "$#" -gt 0 ]; do
    case "$1" in
        --model|-m)
            [ "$#" -ge 2 ] || { echo "valeur manquante pour $1" >&2; exit 2; }
            MODEL="$2"
            shift 2
            ;;
        --dest|-d)
            [ "$#" -ge 2 ] || { echo "valeur manquante pour $1" >&2; exit 2; }
            DEST="$2"
            shift 2
            ;;
        --force|-f)
            FORCE=1
            shift
            ;;
        --help|-h)
            usage
            exit 0
            ;;
        *)
            echo "option inconnue: $1" >&2
            usage >&2
            exit 2
            ;;
    esac
done

if [ -z "$MODEL" ]; then
    usage >&2
    exit 2
fi

# SHA-256 and exact byte sizes from the Hugging Face LFS metadata (2026-11-15).
SHA=""
SIZE=""
FILE=""
HUMAN=""
case "$MODEL" in
    large-v3-turbo-q8_0|turbo-q8|large-v3-turbo)
        FILE="ggml-large-v3-turbo-q8_0.bin"
        SHA="317eb69c11673c9de1e1f0d459b253999804ec71ac4c23c17ecf5fbe24e259a1"
        SIZE="874188075"
        HUMAN="874 MB"
        ;;
    small)
        FILE="ggml-small.bin"
        SHA="1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b"
        SIZE="487601967"
        HUMAN="466 MB"
        ;;
    *)
        echo "modèle non pris en charge: $MODEL" >&2
        echo "choisissez large-v3-turbo-q8_0 ou small" >&2
        exit 2
        ;;
esac

if [ -z "$SHA" ] || [ -z "$SIZE" ]; then
    echo "refus: aucun checksum épinglé pour $MODEL" >&2
    exit 2
fi

sha256_of() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | awk '{print $1}'
    else
        echo "ni sha256sum ni shasum ne sont disponibles" >&2
        return 1
    fi
}

mkdir -p "$DEST"
TARGET="$DEST/$FILE"
PART="$TARGET.part"
URL="https://huggingface.co/ggerganov/whisper.cpp/resolve/main/$FILE"

if [ -f "$TARGET" ]; then
    EXISTING="$(sha256_of "$TARGET")"
    if [ "$EXISTING" = "$SHA" ]; then
        echo "déjà présent et vérifié: $TARGET"
        exit 0
    fi
    if [ "$FORCE" -ne 1 ]; then
        echo "checksum incorrect pour $TARGET" >&2
        echo "relancez avec --force pour le remplacer" >&2
        exit 1
    fi
    rm -f "$TARGET"
fi

if [ -f "$PART" ]; then
    PART_SIZE="$(wc -c < "$PART" | tr -d ' ')"
    if [ "$PART_SIZE" -gt "$SIZE" ]; then
        rm -f "$PART"
    else
        echo "reprise de $PART ($PART_SIZE octets)"
    fi
fi

echo "téléchargement de $FILE ($HUMAN) vers $TARGET"
if [ -f "$PART" ]; then
    curl -fL --retry 3 --retry-delay 2 -C - -o "$PART" "$URL"
else
    curl -fL --retry 3 --retry-delay 2 -o "$PART" "$URL"
fi

ACTUAL_SIZE="$(wc -c < "$PART" | tr -d ' ')"
if [ "$ACTUAL_SIZE" -ne "$SIZE" ]; then
    echo "taille incorrecte: $ACTUAL_SIZE octets, attendu $SIZE" >&2
    rm -f "$PART"
    exit 1
fi

ACTUAL_SHA="$(sha256_of "$PART")"
if [ "$ACTUAL_SHA" != "$SHA" ]; then
    echo "SHA-256 incorrect: $ACTUAL_SHA" >&2
    echo "attendu: $SHA" >&2
    rm -f "$PART"
    exit 1
fi

chmod 0644 "$PART"
mv "$PART" "$TARGET"
echo "installé: $TARGET ($ACTUAL_SIZE octets, SHA-256 vérifié)"
