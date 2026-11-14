# TODO

Fonctions de Spotify et d'Apple Music qui manquent encore à Neezer, classées par difficulté
d'ajout. `[x]` = fait, `[ ]` = à faire. Les notes sous chaque ligne disent sur quoi s'appuyer dans
le code.

> Subsonic est désormais une fonction secondaire : le lecteur web (`/app`) est la référence. On ne
> se contraint plus à rester compatible avec le modèle Subsonic quand une fonction demande de
> s'en écarter (dossiers de playlists, historique, etc.).

---

## Niveau 1 — quelques heures, surtout côté interface

- [x] **Modifier la file d'attente** — réordonner par glisser-déposer, vider la file.
  Le store `player` n'a que `removeAt`, `playNext`, `addToQueue` : il manque `move` et `clear`.
  Le glisser-déposer existe déjà dans `Playlist.svelte` (mode édition). La file est une liste
  virtualisée (`VirtualList`) : le glisser doit fonctionner sans monter toutes les lignes.
- [x] **Minuteur de sommeil** — arrêt après N minutes, ou à la fin du titre / de l'album en cours,
  avec un fondu sur les dernières secondes. À porter par le store `player` pour que la
  télécommande (`lib/remote`) et le lecteur natif Android restent cohérents.
- [x] **Vitesse de lecture pour les podcasts** — 0,5× à 3×, mémorisée par émission. Le lecteur
  lit `playbackRate` mais ne le règle jamais. Penser à `preservesPitch`, aux marqueurs et à la
  reprise de position (les positions sont en secondes de média, pas en temps réel).
- [x] **Mono et balance gauche/droite** — accessibilité. La carte « Mono et balance » agit sur
  le graphe traité (`lib/audio/pan.js`, `graph.js`) : sommation mono et panoramique à puissance
  constante ; l'analyseur de rythme garde le stéréo d'origine. Désactivée en lecture directe et
  dans le lecteur natif Android, avec la raison affichée.
- [x] **Mode voiture** — écran plein format, très gros boutons (précédent / lecture / suivant,
  file, favori), lisible d'un coup d'œil et sans geste fin. Livré en overlay natif : tailles,
  gestes, playlists épinglées, recherche, déclenchement GPS/Bluetooth et démarrage au boot
  (`docs/agents/android.md`).
- [x] **Panneau « sous le lecteur »** — dans le lecteur plein écran, glisser vers le haut fait
  monter une page (crédits, artiste, puis titres similaires tout à la fin) :
  - [x] titres similaires (via l'API Deezer aujourd'hui — `/radio/track/<id>` ; à terme via nos
    propres embeddings, voir « Lecture aléatoire intelligente ») ;
  - [x] à propos de l'artiste (photo, fans, titres populaires, artistes similaires — l'API
    publique de Deezer ne donne pas de biographie) ;
  - [x] crédits complets du titre (auteur, compositeur, producteur, label, ISRC…).

## Niveau 2 — une journée, un petit endpoint ou un petit écran

- [x] **Mode exploration** (Spotify « Parcourir », Apple « Explorer ») — classements, genres,
  ambiances. `deezerpy` sait déjà faire `get_chart*`, `get_genres`, `get_channels`,
  `get_editorial*` ; il manque l'endpoint `/api` et l'écran.
- [x] **Radios** — stations par genre, livrées : le disque d'une tuile Explorer lance une station
  continue ancrée sur le genre et ses sous-genres (`/api/radio/genre`, `genre-radio`), avec un
  refill piloté par les écoutes et les skips. « Toutes les stations » liste les genres analysés.
  Les radios par titre et par artiste existaient déjà.
- [x] **Vrai historique d'écoute** — journal de chaque écoute (titre, date, durée écoutée,
  contexte) et écran « Historique » dans la Bibliothèque : pagination keyset, filtres
  écoutes/sauts, résumé 30 jours et tops artistes/titres (`neezer/webui/history.py`,
  `routes/History.svelte`). Le journal `listen_event` existait déjà ; l'écran et l'API le lisent.
- [x] **Crédits complets** (auteur, compositeur, producteur, label, ISRC…) — `GET /api/track/<id>/credits` :
  le sidecar d'abord, Deezer ensuite (et alors écrit dans le sidecar), la base en dernier recours.
  Affichés dans le panneau sous le lecteur.
- [x] **Phrase de paroles en carte image** — choisir une à trois lignes des paroles synchronisées
  et en tirer une image 1080×1350 à partager (`LyricCard.svelte`, `lib/lyriccard.js`) : pochette
  depuis le cache hors-ligne, sinon la rainure Sillon, couleurs dérivées, texte crème contrasté.
- [x] **Titres recommandés en bas d'une playlist** — suggestions à ajouter en un tap, à partir de
  la playlist entière comme graine (`GET /api/playlist/<id>/recommendations`) : bibliothèque
  seule, 12 par défaut, ajout optimiste et « Tout ajouter » en une requête. Un échantillon
  régulier de 80 seeds garde la réponse à ~140 ms sur une playlist de 4 000 titres.

## Niveau 3 — quelques jours, avec migration de base de données

- [x] **Récap** (type Spotify Wrapped / Apple Replay) — top titres, artistes, genres, minutes
  écoutées, par mois et par année, en « stories ». `GET /api/recap/years` + `/api/recap/<year>`
  (4 statements, cache process, aucun fournisseur), écran `#/recap/:year`, carte saisonnière sur
  l'Accueil, partage d'affiche 1080×1350 dessinée localement. 29 tests serveur, 19 tests JS purs,
  banc navigateur de 102 assertions (`webapp/test/recap/run.mjs`).
- [x] **Dossiers de playlists** — regrouper les playlists dans la barre latérale et la
  bibliothèque. Table locale (ni Deezer ni Subsonic n'ont ce concept), ordre libre. Deux tables
  (`playlist_folder`, `playlist_placement`), vue par personne, place aussi les playlists partagées ;
  sidebar, onglet Playlists, mode Organiser (boutons, clavier, VirtualList au-delà de 60 lignes),
  feuilles de création/déplacement, annulation. 38 tests serveur, 12 tests purs, 576 tests SPA.
- [x] **Lien public en lecture seule vers une playlist** — capacité signée (`/pl/<token>`), un lien
  vivant par playlist, révocable et mort à la requête suivante. La page publique n'a ni compte ni
  session : elle liste la playlist et joue uniquement les fichiers déjà archivés, jamais un appel
  fournisseur. Les uploads privés d'autrui sont exclus. Entrée : « Exporter » → « Lien public ».
- [x] **Import de playlists** — depuis un lien ou un fichier, correspondance des titres par ISRC
  puis par artiste + titre, avec un écran de revue des titres non trouvés (`neezer/linkimport.py`,
  `neezer/webui/playlist_import.py`, `routes/Import.svelte`) :
  - [x] Spotify
  - [x] Apple Music
  - [x] Deezer (lien de playlist publique, en plus de l'import CLI existant)
  - [x] YouTube / YouTube Music

## Niveau 4 — une à deux semaines, vraie conception

- [x] **Lecture aléatoire intelligente** — couverte par le Flow, qui choisit déjà chaque titre à
  partir du goût, du niveau de découverte et de la session. Le bouton « Aléatoire » reste un
  tirage simple, volontairement : intercaler des recommandations dans un shuffle explicite n'a
  plus de besoin propre.
- [x] **Mix par ambiance** (type Daylist) — six mixes construits sur des bandes mesurées (énergie,
  tempo, couleur spectrale, flux) : Calme, Concentration, Bonne humeur, Énergie, Soirée, Nuit.
  Rangée « Mixes d'ambiance » sur l'Accueil, bibliothèque seule, file finie de 40 titres
  (`neezer/moods.py`, `GET /api/moods`).
- [ ] **Playlists collaboratives** — *mis de côté.* Plusieurs comptes qui écrivent dans une même
  playlist. La difficulté est la synchronisation entre plusieurs comptes et les droits d'écriture ;
  le miroir Deezer n'existe plus.
- [ ] **Chapitres et transcription de podcasts** — chapitres quand l'épisode en porte
  (ID3 `CHAP`, ou `podcast:chapters`), transcription en texte avec recherche. Les épisodes
  arrivent de Deezer en MP3 brut, sans flux RSS : à voir d'où viennent les chapitres. La
  transcription demande un modèle de reconnaissance vocale.

## Niveau 5 — administration et onboarding (gros chantier)

- [ ] **Dashboard admin des utilisateurs** — dans Réglages, visible seulement pour un admin :
  liste et recherche des comptes, rôle, état (actif / invité / désactivé), dernière connexion,
  création d'une invitation, révocation, désactivation/suppression avec garde-fous (pas de
  suppression du dernier admin, pas d'auto-suppression), renvoi du lien. Routes `/api/admin/users…`
  avec `@login_required` + contrôle admin, pagination, aucune énumération pour un non-admin.
- [ ] **Invitation à usage unique** — lien copiable (pas d'email tant qu'aucun SMTP n'est
  configuré) : table `invite` (id, empreinte du jeton, créateur, dates, expiration, utilisé par,
  révoqué), jeton opaque signé, une seule création de compte par lien, expiration et révocation
  immédiate. Page publique de création : username limité (3–32 caractères `[a-z0-9_.-]`, replié,
  noms réservés, unicité insensible à la casse), mot de passe fort + confirmation, limitation de
  débit, CSRF, aucune fuite d'existence de compte, hachage argon2 existant.
- [ ] **Onboarding par les artistes** — après la création du compte : une page plein écran montre
  une liste d'artistes connus (Deezer public : `get_chart_artists` / artistes populaires, cache,
  sélection diverse par genres et pays), scroll infini et barre de recherche en haut. Un like
  insère **juste après** l'artiste des artistes similaires (`artist/<id>/related`), sans casser la
  diversité ; minimum 1 artiste pour terminer (3–5 recommandés), bouton « Terminé ». Les artistes
  choisis deviennent des artistes suivis/étoilés pour le profil de goût et les Nouveautés ; rien
  n'est téléchargé.
- [ ] **Étape compte et import** — après les artistes, proposer sans bloquer : lier son compte
  Deezer (ARL ou connexion navigateur), ou importer ses playlists/favoris depuis YouTube, Spotify
  et Apple Music, via `neezer/services/` et `neezer/webui/playlist_import.py`. Chaque option est
  skippable ; les imports sont des jobs de fond avec progression et cause d'échec ; rien n'est
  jamais écrit sur le compte du fournisseur.
- [ ] **Finitions** — l'onboarding est une route gardée avant l'Accueil (reprise après refresh),
  français, mobile/desktop, cibles 44 px, états vides/erreurs ; tests d'invitation (usage unique,
  expiration, révocation), d'unicité/validation du username, de mot de passe, de similarité des
  artistes et d'import ; mesure du nombre de requêtes et du temps de première peinture. Migration
  pour SQLite/PostgreSQL/MySQL et upgrade d'un serveur existant (les comptes actuels ne voient
  jamais l'onboarding).

---

## Écarté ou à réévaluer

Non retenu pour l'instant, à rediscuter si l'envie revient :

- Recherche dans les paroles, traduction des paroles, mode karaoké (atténuation de la voix).
- Concerts et dates de tournée (API tierce, clé et quotas).
- Vidéos musicales, Canvas : Deezer ne fournit aucune vidéo.
- Audio spatial / Dolby Atmos, Hi-Res 24 bits : le plafond de la source est le FLAC 16 bits.
- DJ vocal par IA, playlists générées par prompt.
- Android Auto (navigation dans la bibliothèque), livres audio.
