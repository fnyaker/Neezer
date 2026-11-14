# Podcasts : transcrire dans un conteneur séparé

Neezer confie la transcription des épisodes de podcast à un **worker**. Le worker
par défaut de cette page est un conteneur séparé, `neezer-transcriber` : le
serveur garde la file, les leases et le budget ; le conteneur exécute
whisper.cpp sur son propre CPU, avec ses propres limites mémoire et CPU.

Le conteneur ne monte **ni la base de données ni l'archive**. Il ne reçoit que
l'audio de l'épisode qu'il a loué, par HTTP authentifié, et renvoie des
segments. Le texte est écrit par le serveur sur le disque de données, à côté de
l'épisode (`<épisode>.transcript.json` et `.srt`), et indexé dans la base.

## Ce qu'il faut avant de commencer

- Neezer avec les routes de transcription (schéma `20261121`).
- Docker Engine et Docker Compose v2, sur `amd64` ou `arm64`.
- Le serveur configuré pour un runner externe, dans `config/neezer.conf` :

```ini
[transcribe]
enabled = yes
runner = external
daily_minutes = 120
max_minutes = 360
language = auto
```

`runner` ne se règle **que** dans le fichier de configuration (l'API refuse ce
champ). `enabled`, la langue, la durée maximale et le budget se règlent aussi
dans Réglages. Redémarrez Neezer après avoir modifié le fichier. Sans
`enabled = yes`, la mise en file répond `409 disabled` ; avec
`runner = inline`, c'est le processus de l'API qui transcrirait, pas le
conteneur.

Dans la stack Docker, ce fichier doit être monté (la ligne commentée
`./config/neezer.conf:/etc/neezer:ro` de `docker-compose.yml`) ou désigné par
`NEEZER_CONFIG` : l'entrypoint ne rend que les réglages Deezer/base depuis les
variables d'environnement, pas `[transcribe]`.

En mode `external`, les chemins `bin` et `model` du serveur ne sont pas
utilisés : seul le `MODEL` monté dans le conteneur compte. Le bouton « Tester »
peut donc signaler un moteur local absent sans empêcher le worker de travailler.

- Un réseau privé entre Neezer et le conteneur. L'exemple compose place les deux
  services sur un réseau `internal` : le worker n'a aucune route vers Internet.
- De la RAM pour le modèle choisi, et un CPU qui accepte de tourner plusieurs
  heures.

### Choisir le modèle

| Modèle | Fichier | Taille | RSS mesuré | Quand |
|---|---|---|---|---|
| `large-v3-turbo-q8_0` | `ggml-large-v3-turbo-q8_0.bin` | 874 MB | ~1,13 Go | défaut si ≥ 2,5 Go de RAM libre |
| `small` | `ggml-small.bin` | 466 MB | ~0,55–0,85 Go | NAS, petit serveur, RAM serrée |
| `large-v3-turbo-q5_0` | `ggml-large-v3-turbo-q5_0.bin` | 547 MB | ~0,88 Go | compromis ; à copier à la main |
| `large-v3` | `ggml-large-v3.bin` | 2,9 Go | ~3,9 Go | 8+ cœurs seulement, lent sur CPU |

Le script `tools/fetch-whisper-model.sh` ne télécharge que les deux premiers
(ceux dont les checksums sont épinglés). Un autre fichier GGML valide peut être
copié à la main ; `MODEL` le nomme.

## Le modèle

Le modèle n'est **jamais dans l'image** et n'est **jamais téléchargé au
runtime**. Il vit sur l'hôte, dans `./models`, et il est monté en lecture seule
sur `/models`.

```sh
tools/fetch-whisper-model.sh --model large-v3-turbo-q8_0 --dest ./models
# ou, sur un serveur à RAM serrée :
tools/fetch-whisper-model.sh --model small --dest ./models
```

Le script :

- télécharge depuis Hugging Face (`ggerganov/whisper.cpp`), avec reprise ;
- vérifie la **taille exacte** et le **SHA-256 épinglé** dans le script ;
- refuse un modèle inconnu, un checksum absent ou un fichier qui ne correspond
  pas — un téléchargement raté n'est jamais installé ;
- refuse de tourner dans un conteneur (`/.dockerenv`) : c'est un outil d'hôte.

Vérifiez le montage et le binaire avant de démarrer :

```sh
docker compose -f docker-compose.yml -f docker-compose.transcribe.yml \
  run --rm --entrypoint python3 transcriber /app/worker.py --check
```

`--check` lit la taille et la magie GGML, exécute `whisper-cli --version` et
`ffmpeg -version`, et vérifie la présence du jeton. Il **n'appelle pas l'API** :
une panne du serveur ne rend pas le worker malade. Sans modèle, le conteneur
sort avec un message clair et un code non nul.

## Créer le jeton du worker

Un appareil est créé par un admin, avec un type `server` pour ce conteneur. Le
jeton est affiché **une seule fois**. Il est stocké chiffré au repos par le
serveur et n'est jamais renvoyé par une route.

Dans l'interface, si la carte Appareils est présente : **Réglages → Podcasts →
Transcription → Appareils → Ajouter un appareil**, nom `conteneur`, type
`serveur`. Copiez le jeton immédiatement.

Par l'API admin (session navigateur) :

```sh
curl -sS -X POST "$NEEZER_URL/api/transcribe/workers" \
  -H 'Content-Type: application/json' \
  -H "Cookie: session=$NEEZER_SESSION" \
  --data '{"name":"conteneur","kind":"server"}'
```

La réponse est `201` :

```json
{"worker": {"id": "…", "name": "conteneur", "kind": "server"},
 "token": "<uuid>.<secret>"}
```

Écrivez le jeton dans un fichier lisible par Docker seulement, jamais dans le
YAML compose ni dans une variable d'environnement visible par
`docker inspect` :

```sh
install -d -m 700 secrets
printf 'Jeton du worker : '
read -rs TOKEN
printf '%s' "$TOKEN" > secrets/transcriber.token
chmod 600 secrets/transcriber.token
unset TOKEN
```

`read -rs` évite l'historique du shell. Le fichier est monté comme secret
Docker sur `/run/secrets/transcriber_token`, et le worker le lit par
`NEEZER_WORKER_TOKEN_FILE`.

Révoquer un appareil :

```sh
curl -sS -X DELETE "$NEEZER_URL/api/transcribe/workers/<id>" \
  -H "Cookie: session=$NEEZER_SESSION"
```

Le prochain claim ou heartbeat reçoit `403`, et le lease en cours est relâché :
l'épisode repart en file. Il n'y a pas de rotation de jeton dans le protocole 1 :
révoquez, créez un nouvel appareil, remplacez le fichier, puis
`docker compose up -d transcriber`.

Un seul conteneur par jeton : deux workers avec le même jeton sont
indiscernables dans Réglages et une révocation les coupe tous les deux. Un
worker `server` est de toute façon seul par conception (un lease serveur à la
fois).

## Lancer le conteneur

Le fichier `docker-compose.transcribe.yml` est un **overlay** : il ajoute le
service `transcriber` et attache le service `neezer` existant au réseau privé.
Il ne remplace pas `docker-compose.yml`.

```sh
tools/fetch-whisper-model.sh --model large-v3-turbo-q8_0 --dest ./models
install -d -m 700 secrets
printf 'Jeton du worker : '
read -rs TOKEN
printf '%s' "$TOKEN" > secrets/transcriber.token
chmod 600 secrets/transcriber.token

docker compose -f docker-compose.yml -f docker-compose.transcribe.yml up -d transcriber
docker compose -f docker-compose.yml -f docker-compose.transcribe.yml logs -f transcriber
```

La première fois, Compose recrée le conteneur `neezer` pour l'attacher au réseau
`internal` (bref redémarrage de l'API) ; les fois suivantes, seul le worker
change.

Ce que l'overlay garantit :

- `read_only: true`, utilisateur non-root `10001:10001`, `cap_drop: ALL`,
  `no-new-privileges`, `pids_limit: 64` ;
- `mem_limit` **égal** à `memswap_limit` : pas de swap, un dépassement tue le
  worker, pas l'API ;
- `cpus: "2.0"` : le CPU est plafonné par le cgroup, pas seulement par `nice` ;
- `tmpfs` sur `/tmp` : l'audio téléchargé et un WAV de chunk à la fois, effacés
  en `finally` ;
- réseau `internal` : aucune sortie Internet, seulement `neezer:5722` ;
- volume `./models:/models:ro`, secret fichier, aucun port publié, aucun socket
  Docker, aucune base, aucun montage de l'archive ;
- `depends_on: neezer (healthy)` et un healthcheck **local** (`--check`).

Dimensionnement :

| Modèle | `mem_limit` | `cpus` | `THREADS` |
|---|---|---|---|
| `small` | `2g` | `2.0` | `2` |
| `large-v3-turbo-q8_0` | `3g` | `2.0` | `2` |
| `large-v3` | `5g` | `4.0` | `4` |

Le `tmpfs` de 1,25 Go est compté dans `mem_limit` : l'audio peut faire jusqu'à
1 Gio côté serveur, plus un WAV de 10 min (19,2 Mo). Si vous baissez
`max_minutes`, vous pouvez baisser le `tmpfs` et la mémoire. Pour un épisode
proche de 1 Gio avec `small`, passez `mem_limit` à `3g`.

Un worker `server` est seul par conception : un deuxième conteneur `server`
attend qu'aucun lease serveur ne soit vivant. Les workers `client` (lot
ultérieur) pourront être plusieurs.

## Premier épisode

1. Vérifiez que `enabled = yes` et `runner = external`, puis redémarrez Neezer.
2. Marquez une émission avec « Transcrire automatiquement les nouveaux
   épisodes », ou mettez un épisode en file :

```sh
neezer-cli podcast transcribe --episode <id>
neezer-cli podcast transcribe --status
```

3. Le worker réclame le job dès qu'il est libre. Ses logs montrent le
   téléchargement, la détection de langue et chaque chunk ; l'interface montre
   la progression et la cause d'un échec.
4. Un épisode `ready` apparaît avec ses segments et sa recherche ; le texte est
   aussi sur le disque de données.

Le runner **inline** n'est pas utilisé ici. Le bouton « Tester » des réglages
sonde le moteur local du serveur, pas le conteneur : pour le sidecar, la sonde
utile est `--check` (healthcheck et commande ci-dessus).

## Limites, vitesse et budget

- **Chunk** : 600 s, avec 2 s de recouvrement. Le serveur renvoie `chunk_ms`
  dans le claim et c'est lui qui fait foi ; `CHUNK_SECONDS` doit valoir 600
  (le worker avertit s'il diffère et utilise la valeur du serveur).
- **Threads** : 2 par défaut, 4 au maximum utile sur un gros CPU. Au-delà de 8,
  le gain s'effondre (bande passante mémoire). Sur un NAS 4 cœurs, `turbo` peut
  descendre **sous 1×** le temps réel.
- **Vitesse** : aucune promesse. Les mesures publiées donnent `small` à 5–13× et
  `turbo-q8` à 1,4–5× sur un CPU de bureau 8 cœurs ; mesurez votre machine sur
  les premiers épisodes (durée audio / durée réelle).
- **Budget automatique** : `daily_minutes` (120 par défaut, 0 = illimité)
  réserve les minutes d'audio des jobs automatiques sur 24 h. Un départ manuel
  passe outre le budget mais pas la file. `max_minutes` (360 par défaut) borne
  un épisode.
- **File** : 20 jobs actifs au maximum. `queue_full` est temporaire.
- **Priorité** : le worker est un job de fond. Le heartbeat renvoie `pause`
  quand une personne écoute un flux ; le worker met whisper en `SIGSTOP` et le
  reprend quand le serveur est calme. Un `cancel` (bouton Arrêter) est vu en
  quelques secondes : le processus est tué, le lease relâché, `done_ms` et les
  segments déjà reçus sont conservés.

## Arrêt, reprise, mise à jour

```sh
docker compose -f docker-compose.yml -f docker-compose.transcribe.yml stop transcriber
docker compose -f docker-compose.yml -f docker-compose.transcribe.yml start transcriber
docker compose -f docker-compose.yml -f docker-compose.transcribe.yml up -d transcriber
```

- **Arrêt propre** (`stop`, `SIGTERM`) : le worker tue whisper, relâche son
  lease et sort. Le job repart en file et reprend à `done_ms`.
- **Arrêt brutal** (kill, OOM, coupure) : le lease est expiré au bout de
  **15 min** ; au prochain claim (le worker redémarre) ou au redémarrage de
  l'API, le serveur remet le job en file. Trois expirations consécutives donnent
  `error lease_lost`. Le chunk en cours est refait, les précédents sont gardés.
- **Mise à jour** : remplacez l'image et faites `up -d transcriber` ; le serveur
  et le worker se mettent à jour indépendamment. Les jobs restent `queued`
  pendant l'arrêt. Le modèle se remplace par un nouveau fichier monté, puis un
  redémarrage.
- **Redémarrage de l'API** : le worker continue à heartbeater ; si l'API
  redémarre, le lease expire naturellement ou le job est repris. Aucun travail
  n'est perdu au-delà du chunk courant.

## Dépannage

| Cause / symptôme | Ce que c'est | Quoi faire |
|---|---|---|
| `engine_missing` | `whisper-cli` absent ou non exécutable | `docker run --rm --entrypoint /usr/local/bin/whisper-cli neezer-transcriber:local --version` ; vérifiez `WHISPER_BIN`. |
| `model_missing` | `MODEL` ne pointe pas sur un fichier monté | Vérifiez `./models:/models:ro` et le nom du fichier ; `--check` le dit. |
| `model_invalid` | fichier tronqué, page HTML, mauvaise quantification | Re-téléchargez avec le script (taille + SHA-256) ; ne copiez pas un `.bin` inconnu. |
| `exit` | whisper s'est arrêté avec un code non nul | Lisez la dernière ligne des logs ; vérifiez `OOMKilled` (`docker inspect --format '{{.State.OOMKilled}}' neezer-transcriber`), le modèle et l'audio. |
| `Illegal instruction` / redémarrages | CPU x86 sans AVX2 (antérieur à Haswell) | Utilisez un hôte plus récent, ou reconstruisez whisper.cpp avec les options `-DGGML_AVX2=OFF` (etc.) de la section multi-arch. |
| `timeout` | un chunk a dépassé `max(900 s, 6 × durée)` | CPU trop lent ou processus bloqué ; augmentez `cpus`, utilisez `small`, vérifiez le fichier. |
| `lease_lost` | trois leases de 15 min expirés sans heartbeat | Réseau vers `neezer`, conteneur arrêté/en pause, horloge ; le `done_ms` est conservé, cliquez Réessayer. |
| `worker_revoked`, `401`, `403` | jeton révoqué, remplacé, ou mauvais fichier | Créez un nouvel appareil, remplacez `secrets/transcriber.token`, redémarrez. |
| `429` | limite de requêtes du worker | Le backoff est automatique ; si cela dure, deux conteneurs partagent probablement un jeton. |
| `not_archived`, `404` | le fichier audio de l'épisode a disparu | Ré-archivez l'épisode ; le texte déjà produit reste lisible. |
| `queue_full` | 20 jobs actifs | Attendez la fin des jobs en cours. |
| tmpfs plein / `no_space` | audio + WAV dépassent `/tmp` | Augmentez `tmpfs` **et** `mem_limit`, ou baissez `max_minutes`. |
| healthcheck `unhealthy` | `--check` échoue | `docker compose ... exec transcriber python3 /app/worker.py --check` et lisez le code (`model_missing`, `engine_missing`, `ffmpeg_missing`). |
| erreur TLS | `NEEZER_URL` en `https://` avec un certificat non reconnu | Utilisez un certificat valide ; pour une CA privée, montez-la et réglez `REQUESTS_CA_BUNDLE`. Ne désactivez jamais la vérification. |
| `401` sur l'audio | jeton refusé | Vérifiez le fichier du jeton et que l'appareil n'est pas révoqué. |
| `403` sur l'audio | lease expiré ou audio demandé hors lease | Le worker reprend au prochain claim ; redémarrez s'il insiste. |

Le worker ne journalise **jamais** le jeton. Il journalise l'URL de base, le nom
du modèle, les chunks et les codes d'erreur — pas les en-têtes.

## Confidentialité et sécurité

- La transcription est **locale** : ni OpenAI, ni Mistral, ni un service cloud.
  Le worker n'appelle aucun fournisseur et ne télécharge aucun modèle.
- Le worker ne reçoit que l'épisode qu'il a loué, par
  `GET /api/transcribe/worker/audio/<eid>` avec `X-NS-Lease` ; le serveur
  vérifie le lease et ne sert jamais un autre épisode. Aucun chemin de fichier
  ne quitte le serveur.
- Le réseau `internal` de l'exemple n'a **aucune sortie** ; n'attachez pas le
  worker à un réseau partagé non maîtrisé. À l'intérieur du réseau privé,
  l'audio circule en clair ; si `NEEZER_URL` traverse un réseau non maîtrisé,
  utilisez TLS et un certificat valide.
- Le jeton est une **capacité** limitée à `transcribe` : il ne lit pas la
  bibliothèque, les réglages ou un autre épisode. Il est chiffré au repos côté
  serveur, affiché une fois, monté en fichier, jamais dans un environnement
  visible par `docker inspect`, jamais dans un log, et retiré de
  l'environnement des sous-processus whisper/ffmpeg. Révoquez-le en cas de doute.
- Le texte vit sur le **disque de données** (`<épisode>.transcript.json`,
  `.srt`) et dans l'index de la base. Supprimer une transcription supprime le
  texte dérivé, **jamais l'audio**. Le worker efface l'audio et les chunks de
  `/tmp` dans un `finally` ; le tmpfs disparaît à l'arrêt.

## Multi-arch et image

```sh
docker build -t neezer-transcriber:local docker/transcribe
docker buildx build --platform linux/amd64,linux/arm64 \
  -t neezer-transcriber:local docker/transcribe
```

- whisper.cpp v1.9.5 est compilé avec `GGML_NATIVE=OFF` et
  `BUILD_SHARED_LIBS=OFF` : pas de `-march=native`, pas de bibliothèque
  whisper partagée. Sur `amd64`, le binaire active SSE4.2/AVX/AVX2/FMA/F16C
  (Haswell, 2013+) ; un CPU x86 plus ancien lève `Illegal instruction` —
  utilisez un hôte plus récent, ou reconstruisez avec
  `-DGGML_SSE42=OFF -DGGML_AVX=OFF -DGGML_AVX2=OFF -DGGML_FMA=OFF
  -DGGML_F16C=OFF -DGGML_BMI2=OFF` (plus lent). Sur `arm64`, NEON est la base.
  ffmpeg est le build statique épinglé par le digest multi-arch de l'image de
  l'API.
- Le modèle n'est pas dans l'image ; le conteneur reste non-root et sans
  dépendance Python autre que `requests`.
- Sur `arm64` (Raspberry Pi, NAS), préférez `small` et 2 threads ; le CPU est
  souvent le facteur limitant. `-ng` est passé inconditionnellement : aucun GPU.
- Taille attendue sans modèle : ~230–280 Mo non compressés (à mesurer avec
  `docker image inspect --format '{{.Size}}' neezer-transcriber:local`).
- La CI de ce dépôt construit l'image de l'API ; l'image du sidecar se construit
  localement dans ce lot. Le contexte est `docker/transcribe` (léger), jamais la
  racine du dépôt, pour ne pas y envoyer le modèle ni le jeton.

## Références

- Protocole worker : `neezer/webui/transcribe_worker.py`.
- File, lease, reprise : `neezer/transcribe/job.py` ; moteur : `engine.py` ;
  runner inline : `runner.py`.
- Image et compose : `docker/transcribe/`, `docker-compose.transcribe.yml`.
- Notes d'ingénierie : [server.md](agents/server.md), section transcription.
