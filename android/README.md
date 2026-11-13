# Neezer — application Android

Application Kotlin native qui embarque le lecteur web (`webapp/`) dans une WebView plein écran et
lui ajoute ce que le navigateur ne sait pas faire :

- **Service média foreground + MediaSession** : la musique continue après une longue pause en
  arrière-plan, la notification, l'écran de verrouillage et les commandes Bluetooth routes vers le
  lecteur. Le pont est `app/src/main/assets/nsshim.js`, injecté au chargement : il remplace
  `navigator.mediaSession` et relaie tout vers le service natif. Aucun code spécifique dans la SPA,
  donc l'app fonctionne avec n'importe quelle version déployée sur le serveur.
- **Lecteur natif optionnel** (Media3/ExoPlayer, Réglages → Audio → *Lecteur de l'application*) :
  quand il est actif, le son sort dans le processus de l'app et non dans la WebView, avec l'égaliseur,
  la normalisation et les animations. Le noyau Rust `android/nscore` porte la chaîne DSP et
  l'analyseur.
- **Mode voiture** : superposition flottante (taille, gestes, playlists épinglées, recherche,
  déclenchement GPS/Bluetooth), service foreground et démarrage au boot.
- **Mise à jour depuis le serveur** : `/api/android/apk`, vérification de la signature et de la
  version avant de confier l'APK à Android.
- **Écran de configuration** au premier lancement : adresse du serveur, port, case « Vérifier le
  certificat SSL ». Décochée, la case ne veut pas dire « tout accepter » : le certificat présenté à
  la première connexion est affiché (empreinte SHA-256) puis épinglé, et seul celui-là est accepté
  ensuite. Un certificat renouvelé se fait accepter en revalidant l'écran.

## Build

L'APK est construit par le workflow CI `.github/workflows/android.yaml` : artefact `neezer-apk` sur
chaque build, et fichier attaché à la release GitHub sur les tags `v*`. Sans secrets configurés, la
signature change à chaque run (il faut désinstaller pour mettre à jour) ; pour une signature stable,
définir `ANDROID_KEYSTORE_BASE64`, `ANDROID_KEYSTORE_PASSWORD`, `ANDROID_KEY_ALIAS`,
`ANDROID_KEY_PASSWORD`.

En local :

```sh
cd android && gradle assembleDebug   # ou ./gradlew après `gradle wrapper`
```

Nécessite un JDK 17 et le SDK Android (API 34).

## Où sont les règles

Lire **[docs/agents/android.md](../docs/agents/android.md)** avant de toucher au pont, au lecteur
natif, à l'offline, à l'updater ou au mode voiture ; **[docs/agents/playback.md](../docs/agents/playback.md)**
pour le chemin audio et **[docs/agents/commands.md](../docs/agents/commands.md)** pour les benches.
