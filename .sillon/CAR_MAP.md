# Mode voiture — carte du chantier

Branche : `car-mode`. Référence : `Mode voiture — prototype cliquable-html/Main.dc.html` (dimensions, couleurs, textes, états, gestes) + `DESIGN.md` (tokens Encre/Papier, citron `#D2FF3C`, polices). Aucune modification de l'API, du serveur ou de la logique de lecture : le mode voiture est un **client** du `PlayerService`/MediaSession existant.

## Techno constatée

- Application **Android Views** (Kotlin), pas de Compose. `MainActivity` héberge une WebView ; `PlayerService` est un service de premier plan avec `MediaSessionCompat` ; l'état de lecture arrive par `NSNative.publish(json)` → `PlayerService.State` ; les commandes média repartent par `PlayerService.commandSink` → `window.__nsNativeCmd` (shim `nsshim.js`).
- Préférences : `SharedPreferences("settings")` via `Prefs`. Pas de DataStore, pas de base native. Les playlists vivent dans le SPA et sont servies par `/api/me/playlists` (cookie de session de la WebView).
- Aucun moteur de lecture natif de queue : le WebView possède la file. Le mode voiture réutilise le player, il ne le remplace pas.
- `AudioRoute` sait déjà repérer une sortie Bluetooth (A2DP/BLE) sans permission ; `CarTrigger` peut s'en inspirer.

## Fondation posée avant les lots

- `android/app/src/main/res/font/{bricolage_grotesque,instrument_sans,jetbrains_mono}.ttf` + `CarType.kt` (Typefaces).
- `CarModeContract.kt` : `CarSize` (ULTRA/MINI/FULL), `CarScreen`, `CarNow`, `CarHost`.
- `CarPrefs.kt` : réglages + épingles ordonnées, opacité bornée 40–80 %, packages GPS par défaut, clavier/limite/inversion.
- `CarPermissions.kt` : superposition, usage stats, localisation, micro.
- Cette carte. Les zones ne se marchent pas dessus : un seul écrivain par fichier.

## Lots (worktrees disjoints)

### A — overlay, service, permissions, déclenchement
Fichiers : `CarModeService.kt`, `CarOverlayRoot.kt`, `CarTileService.kt`, `AndroidManifest.xml`, `build.gradle.kts` (play-services-location), `PlayerService.kt` (exposer `state`, `onState`, `transport`, `jsSink`), `MainActivity.kt` (démarrer/arrêter le service, poser `jsSink`, bridge `openCarSettings`/`carPin`/`carAvailable`, demande de permission), ressources de notification.
- Service de premier plan `specialUse` + `WindowManager` `TYPE_APPLICATION_OVERLAY`, flags `NOT_FOCUSABLE | NOT_TOUCH_MODAL`, `gravity=bottom`, marge 8 dp, bas 92 dp, alpha `CarPrefs.overlayAlpha`, flou si `isCrossWindowBlurEnabled`.
- Il instancie `CarOverlayRoot` (B/C/D) et `CarTrigger` (E), observe `PlayerService.State`, expose `CarHost`.
- Tuile rapide, notification (ouvrir le mode / réglages), démarrage si `CarPrefs` l'autorise.
- Permissions : `SYSTEM_ALERT_WINDOW`, `PACKAGE_USAGE_STATS`, `ACCESS_FINE/COARSE_LOCATION`, `BLUETOOTH_CONNECT`, `RECORD_AUDIO`, `FOREGROUND_SERVICE` + `FOREGROUND_SERVICE_SPECIAL_USE`, `POST_NOTIFICATIONS`, `<queries>` micro.

### B — trois tailles + gestes
Fichiers : `CarPlayerView.kt` (nouveau).
- Trois rendus : **FULL** (pochette 72, titre 26 Bricolage, artiste 17, ♥ 64, précédent 72, play 92, suivant 72, boutons Playlists/Recherche 64), **MINI** (pochette 48, titre 18/artiste 14, play 56, développer 56), **ULTRA** (rond 76, pochette + play/pause, glissé gauche/droite/haut, appui long).
- Détecteur maison : appui long 550 ms, glissé ≥40 dp, tap <14 dp ; `CarPrefs.invertSwipe` inverse gauche/droite. Tap développer/reculer, appui long vers ULTRA ; en ULTRA tap play/pause, glissé haut FULL, appui long MINI.
- Listeners `CarHost` uniquement ; aucune logique de lecture.

### C — clavier, recherche, voix
Fichiers : `CarSearchView.kt`, `CarSpeech.kt` (nouveaux).
- Aucun `EditText`/IME système. Affichage + clavier dessiné : AZERTY/QWERTY/QWERTZ (3 rangées) ou ABC 7×4, espace, effacer citron. « Auto » lit `InputMethodManager`/sous-type (fr→AZERTY, de→QWERTZ, sinon QWERTY), sinon fixe.
- En roulant (vitesse >5 km/h, anti-rebond E) et hors passager : 3 lettres max (2–4 réglable), la lettre suivante remplace tout, espace désactivé. Arrêté/passager : libre.
- Résultats : 3 suggestions max via `/api/search?q=`, tap = `CarHost.launch(kind,id,title,payload)`. Micro `SpeechRecognizer` (« Joue X ») avec permission runtime.
- Aucun nouvel endpoint.

### D — playlists épinglées + réglages
Fichiers : `CarPlaylistsView.kt`, `CarSettingsActivity.kt`, `CarPins.kt` si nécessaire, `webapp/src/lib/car.js`, `webapp/src/routes/Settings.svelte` + `Playlist.svelte` (entrée et action d'épinglage).
- Playlists : « Mon Flow » puis épinglées, 6 tuiles/page (2 colonnes), pagination, tuile fantôme « Épingle-en d'autres » → réglages ; tap lance via `CarHost.launch("playlist", id, title)` ; liste native depuis `/api/me/playlists` (cookie WebView).
- Réglages : écran plein dans l'app, cartes `#151513`, déclenchement GPS + apps cochables, Bluetooth, clavier/lettres, épingles réordonnables, opacité 40–80. Reprend le prototype.
- `car.js` : expose `window.__nsCarPlay(kind,id,payload)` au SPA en réutilisant `playEntity`/`startFlow`/`player` ; menu « Épingler pour la voiture » dans la page playlist via `NSNative.carPin`.

### E — détection GPS, vitesse, Bluetooth
Fichiers : `CarTrigger.kt`, `CarMotion.kt` (nouveaux).
- UsageStatsManager : événement `ACTIVITY_RESUMED` dans les ~4 s, package dans `CarPrefs.gpsPackages`, sondage ~1 s ; permission absente → pas de déclenchement, état exposé.
- Vitesse : FusedLocationProvider (fallback `LocationManager` si GMS absent), seuil ~5 km/h, anti-rebond ~5 s aux feux, « vitesse inconnue = en roulant ».
- Bluetooth : sorties A2DP/BLE via `AudioManager` (aucune permission) ; option `bluetoothTrigger`.
- Callback `onDrive(visible: Boolean, moving: Boolean, speedKmh: Double)` vers A. Aucun AccessibilityService.

## Contrats partagés

- `CarHost` : `now()`, `isDriving()`, `speedKmh()`, `isPassenger()/setPassenger`, `togglePlay/next/previous/seekTo/toggleFavorite`, `setSize`, `setScreen`, `launch(kind,id,title,payload)`, `openSettings`, `toast`, `closeOverlay`.
- `CarPrefs` : `overlayAlpha` (0.4–0.8), `gpsTrigger`, `bluetoothTrigger`, `gpsPackages`, `keyboardLayout` (`auto|azerty|qwerty|qwertz|abc`), `maxLetters` (2–4), `invertSwipe`, `pins` ordonnées.
- `PlayerService` : A ajoute `onState`, `transport(cmd,value)`, `jsSink` (évaluation JS), sans toucher au MediaSession ni au moteur.
- `MainActivity` : A pose `jsSink` sur le WebView et expose `NSNative.openCarSettings/carPin/carAvailable`.
- `webapp/src/lib/car.js` : D ajoute `window.__nsCarPlay`, qui appelle uniquement `playEntity`, `startFlow`, `player` — aucune modification de la logique de lecture.

## Ordre de fusion et revue

A → B → C → D → E (A avant car il pose service/manifest). Puis build APK CI. Puis un sous-agent de revue en lecture seule compare au prototype `Main.dc.html` (tailles, couleurs, textes, gestes, états) et rend les écarts. Feu vert avant merge sur `main`.

## Risques connus

- Pas de Gradle/SDK local : la compilation est prouvée par le workflow `android` ; les lots doivent rester conservateurs sur les API Android (minSdk 26, target 34).
- GMS peut manquer sur un appareil sideloadé : FusedLocation doit avoir un repli `LocationManager`.
- Android 12+ : opacité >80 % bloque les touches ; `CarPrefs` borne à 80.
- Le WebView possède la file : lancer une playlist passe par `window.__nsCarPlay` (SPA courant). Un serveur qui sert une SPA plus ancienne n'a pas le hook : le mode voiture garde transport + playlists, et dit pourquoi.
