#!/bin/sh
set -eu

: "${NEEZER_URL:?NEEZER_URL est requis (ex. http://neezer:5722)}"
if [ -z "${NEEZER_WORKER_TOKEN:-}" ] && [ -z "${NEEZER_WORKER_TOKEN_FILE:-}" ]; then
    echo "NEEZER_WORKER_TOKEN ou NEEZER_WORKER_TOKEN_FILE est requis" >&2
    exit 2
fi
: "${MODEL:?MODEL est requis (ex. /models/ggml-small.bin)}"
if [ ! -r "$MODEL" ]; then
    echo "MODEL=$MODEL est illisible ; montez le dossier du modèle en lecture seule sur /models" >&2
    exit 2
fi
exec python3 -u /app/worker.py "$@"
