# Neezer transcriber (sidecar)

Le conteneur de transcription de Neezer : whisper.cpp v1.9.5 sur CPU, ffmpeg,
Python et `requests`. Il ne contient **aucun modèle**, ne monte ni la base de
données ni l'archive, et ne parle au serveur que par l'API worker authentifiée.

- Image non-root (`10001:10001`), `amd64` et `arm64` (whisper.cpp compilé avec
  `GGML_NATIVE=OFF`, ffmpeg statique épinglé par digest multi-arch). Sur
  `amd64`, AVX2/FMA est requis (Haswell, 2013+).
- Modèle monté en lecture seule sur `/models`, validé au démarrage (taille,
  magie GGML, binaire exécutable). Sans modèle : message clair et code non zéro.
- `read_only`, `tmpfs` pour l'audio et les chunks, aucune base, aucun socket
  Docker, aucun port publié.
- Un seul worker `kind=server` à la fois ; il loue un épisode, le télécharge par
  `Range`, le transcrit en chunks de 600 s (+2 s de recouvrement), renvoie les
  segments, et reprend à `done_ms` après un arrêt.

## Construire

```sh
docker build -t neezer-transcriber:local docker/transcribe
docker buildx build --platform linux/amd64,linux/arm64 \
  -t neezer-transcriber:local docker/transcribe
```

Le contexte est **`docker/transcribe`** : le modèle et le jeton vivent à côté du
dépôt (`./models`, `./secrets`), jamais dans l'image. Le script
`tools/fetch-whisper-model.sh` les télécharge et vérifie leur SHA-256.

## Lancer

Le mode recommandé est l'overlay compose :

```sh
tools/fetch-whisper-model.sh --model large-v3-turbo-q8_0 --dest ./models
docker compose -f docker-compose.yml -f docker-compose.transcribe.yml up -d transcriber
```

Variables lues par le worker :

| Variable | Défaut | Rôle |
|---|---|---|
| `NEEZER_URL` | requis | base HTTP du serveur, préfixe inclus (ex. `http://neezer:5722`) |
| `NEEZER_WORKER_TOKEN_FILE` | — | fichier du jeton (préféré ; monté par compose) |
| `NEEZER_WORKER_TOKEN` | — | jeton en variable (si pas de fichier) |
| `MODEL` | `/models/ggml-large-v3-turbo-q8_0.bin` | modèle GGML monté en `:ro` |
| `LANGUAGE` | `auto` | langue forcée ; `auto` utilise la langue résolue par le serveur |
| `THREADS` | `2` | threads whisper (1–16 ; 2–4 utile) |
| `CHUNK_SECONDS` | `600` | doit correspondre au serveur ; le claim fait foi |
| `NEEZER_TMP_DIR` | `/tmp/neezer-transcribe` | racine des temporaires |
| `WHISPER_BIN` | `whisper-cli` | binaire whisper.cpp |
| `FFMPEG` | `ffmpeg` | binaire ffmpeg |
| `LOG_LEVEL` | `INFO` | verbosité |

Réglages avancés : `NEEZER_HEARTBEAT_SECONDS` (5 s), `POLL_SECONDS` (5 s),
`NEEZER_RETRY_BASE` (1 s), `NEEZER_RETRY_MAX` (30 s),
`NEEZER_RETRY_ATTEMPTS` (5), `NEEZER_AUDIO_TIMEOUT` (120 s par lecture).

`python3 /app/worker.py --check` valide localement le modèle, les binaires,
l'URL et la présence du jeton, sans appeler l'API. C'est le healthcheck.

Guide opérateur complet (jeton, limites, dépannage, reprise, confidentialité) :
[`docs/podcast-media.md`](../../docs/podcast-media.md).
