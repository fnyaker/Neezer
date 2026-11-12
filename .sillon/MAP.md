# Sillon — carte des zones

Branche de travail : `sillon`. Référence : `DESIGN.md` + `Sillon — prototype desktop (cliquable)-html/Main.dc.html`.
But : appliquer le design Sillon (Neezer = produit, Sillon = direction artistique) sans changer les routes, l'API, le lecteur ni les contrats de données. Un seul écrivain par fichier.

## Règles communes

- Lire `AGENTS.md`, `docs/agents/webapp.md`, `DESIGN.md` et la zone correspondante du prototype.
- Consommer les tokens et classes de la phase 1 (`--bg`, `--s1`, `--s2`, `--ln`, `--tx`, `--dim`, `--ac`, `--onac`, `--btn`, `--onbtn`, `--hrt`, `--radius`, polices, `.btn`, `.btn-primary`, `.btn-ghost`, `.chip`, `.chip-active`, `.card`, `.field`, `.label-mono`).
- Ne pas modifier la logique, les appels API, les stores hors du bloc autorisé, ni les docs.
- Ne pas renommer les sélecteurs DOM utilisés par les benches (voir § Contrats).
- Aucun commit, push ou merge : le primary relit, teste et commite chaque worktree.
- Zones A–F en parallèle, une worktree par zone. Zone G (mobile) après fusion A–F, car elle touche leurs fichiers.

## Zones et propriété des fichiers

### A — shell et navigation
- `webapp/src/App.svelte` (layout, route table, montages ; pas les overlays des autres zones)
- `webapp/src/components/Sidebar.svelte`, `BackButton.svelte`, `NetworkIndicator.svelte`
- nouveau `webapp/src/components/TopBar.svelte`
- CSS local des composants uniquement ; `app.css` appartient à la phase 1.
- Prototype : sidebar 232 px (logo disque, nav Accueil/Explorer/Bibliothèque/Nouveautés/Réglages, playlists, utilisateur), topbar sticky (retour, recherche ⌘K, niveau Flow, « Lancer mon Flow »). Retirer la recherche dupliquée de la sidebar.
- Contrats : garder `nav.sidebar a[href="#…"]`, `nav.mobilenav` (G), `--sidebar-w`, `--player-h`, `--np-w`, les montages `Player`/`PlayerBar`/`NowPlaying` et `layout.np-open`. Le `+` de playlist appelle `openNewPlaylist()` (stores.js). Le sync appelle l'action existante.

### B — accueil et explorer
- `webapp/src/routes/Home.svelte`, `Search.svelte`
- `components/EnergyPicker.svelte`, `StylePicker.svelte`, `TopResult.svelte`
- nouveaux composants Home/Search uniquement.
- Prototype : héros accent « Quelle énergie ce matin ? » + 6 chips + Flow, carte « Reprendre » ; sections Par style (chips), Nouveautés (6 pochettes, pas une TrackList), Tes artistes (avatars 104), Pour toi (liste dense). Explorer : titre, chips de source (Ma bibliothèque, Deezer, YouTube, Podcasts), récents, tuiles de styles.
- Lecture seule : `TrackList`, `Card`, `ArtistTile`, `Cover`, stores/API. `TrackList` garde le contrat `tracks/numbered/showAlbum/showCover/context/radio`.

### C — listes, bibliothèque, sorties, artiste/album/playlist
- `webapp/src/routes/Library.svelte`, `Releases.svelte`, `Artist.svelte`, `Album.svelte`, `Playlist.svelte`
- `components/PlaylistTracks.svelte`
- `components/TrackList.svelte`, `TrackRow.svelte` (structure/CSS) ; ne pas changer les props ni la fenêtrage ; `VirtualList` reste à D/partagé.
- Prototype : colonnes `36px | titre+artiste (cover 44) | album | qualité (badge mono) | + et cœur (76) | durée` ; en-têtes mono ; ligne courante accent + ▶ ; bibliothèque avec compteurs en tête, onglets pilule, barre d'actions (Tout lire, Mélanger, Exporter, filtre, tri), artistes grille 7 colonnes, playlists cartes ; pages artiste (bandeau `--s2`, avatar 168, nom 88, onglets), playlist (pochette 200, titre 72, actions nommées, menu ⋯).
- Contrats : `.row.track`, `.play[aria-label="Lire"]`, `.titles`, `.fav`, `.more`, `input[placeholder="Rechercher dans cette liste…"]`, `select.sortsel` (options `title|album|default`), `button[aria-label="Télécharger la playlist"]`, textes `"4000 titres"`, onglets `Artistes/Albums/Podcasts`, `VirtualList` par D. Ajouter un badge qualité seulement si dérivable sans requête et sans changer la hauteur de ligne (media + container query).

### D — lecteur, plein écran, file, qualité
- `components/PlayerBar.svelte`, `NowPlaying.svelte`, `DesktopNowPlaying.svelte`, `MobileNowPlaying.svelte` (layout seulement), `QueueList.svelte`, `QualityMenu.svelte`, `NowPlayingMore.svelte`, `Lyrics.svelte`, `SleepButton.svelte`, `SpeedButton.svelte`, `PartyButton.svelte`, `RemoteButton.svelte`, `SourceBadge.svelte`
- `Player.svelte` : **aucune logique audio**. Seulement copie/toasts si nécessaire, avec accord du primary.
- Prototype : barre 88 px (pochette 56, titre/artiste, ♥ et + ; transport ; badge qualité cliquable, appareils, file, volume, ⋯, plein écran) ; plein écran (réduire, genre, badge qualité ; pochette ≤330, titre 44, actions nommées, progression, transport 68 ; panneau droit File/Paroles/À propos) ; file (lire, +, ×, enregistrer en playlist) ; modale qualité.
- Contrats : `footer.player` et `.pp/.next/.prev/.max/.now .t/.info .t/.fav/input.vol/.sl/.sp/.rb` ; `.d`, `.m`, `.main`, `.side`, `.peek`, `.under`, `.more`, `.qitem[data-qi]`, `.qcount/.qclear/.qhead/.qt/.grip/.ghost/.before/.after` ; `PlayerBar` reste sans moteur (montable en remote) ; `QueueList` garde `player.move/jump/clearUpcoming`. Le bouton « Enregistrer en playlist » appelle `openQueueSave()`.

### E — modale playlists et toasts
- `components/PlaylistPicker.svelte`, `AddTracksSheet.svelte`, `Toasts.svelte`
- `lib/stores.js` : uniquement le bloc playlist (modes `new`/`queue` déjà ajoutés au commit de phase 0)
- `lib/actions.js` : uniquement les helpers playlist (`createPlaylistFrom`, sauvegarde de file)
- Prototype : modale 440 px, voile 60 %, `role=dialog`, Échap/× ; mode titre (liste avec cases, toast, « Terminé »), mode nouvelle (formulaire, suggestion « Ma playlist N », « Créer et ajouter »), mode file (titres de la file dans l'ordre) ; toast pilule 2,4 s `role=status`.
- Contrats : ne pas toucher Sidebar/TrackList/Player ; `openPlaylistPicker(track)`, `openNewPlaylist()`, `openQueueSave()` ; API existante `api.createPlaylist(title, refs)` / `addToPlaylist` ; aucune nouvelle route.

### F — réglages et apparence
- `routes/Settings.svelte` (menu groupé 200 px, recherche, section Apparence)
- nouveau `components/AppearanceSettings.svelte`
- consomme `lib/theme.js` de la phase 1 (ne pas le réécrire) ; ne pas toucher `stores.js`, `remote/settings.js`, `index.html`, `main.js` (phase 1).
- Prototype : grille `200px | contenu`, groupes Écoute (Lecture & son, Apparence, Animations), Bibliothèque (Téléchargements, Découverte), Comptes & serveur (Sources, Compte, Serveur, Assistance) ; champ « Chercher un réglage » ; cartes `--s1` ; trois cartes de thème Encre/Papier/Pochette avec aperçu, `aria-pressed`, texte FR.
- Contrats : garder les ids `playback/viz/downloads/discovery/sources/account/server/help`, `#/settings/downloads` + `.subnav` (bench offline), `#/settings/playback` + « Sortie audio » (benches output/native), route `/settings/:section?`.

### G — mobile (après fusion A–F)
- `components/MobileNav.svelte`, `MobileNowPlaying.svelte`, `ImmersivePlayer.svelte`
- nouvelle `webapp/src/mobile.css` + `lib/viewport.js`
- après fusion, droit de passer sur les fichiers A–F pour les media queries/gutters (un seul écrivain à ce moment).
- Prototype/DESIGN §9 : barre d'onglets 64 px (Accueil, Explorer, Biblio, Réglages), mini-lecteur flottant 64 px au-dessus, plein écran centré pouce (pochette carrée, contrôles ≥44, rangée qualité/+ playlist/appareils/paroles/file). Safe-area, `dvh`, tokens `--mini-h:64px`, `--tabbar-h:64px`, `--player-h:64px` à ≤640.

## Contrats partagés (ne pas casser)

- `TrackList` : props `tracks, numbered, showAlbum, showCover, context, radio`, fenêtrage `.list/.win/.vrow`, hauteur de ligne uniforme.
- `TrackRow` : `.row.track`, `.play`, `.titles`, `.fav`, `.more`, swipe tactile, badge indisponible.
- `VirtualList` : slot `let:item/let:index`, `measure`, `scrollToIndex`, `indexAt`, `autoScroll`, première ligne mesurée.
- `Cover` : `src, alt, round, size, kind, fallbackId, eager` ; blob offline, proxy, hi-res. `Avatar` : `name, src, size, round`.
- `Card` : `item, kind` (`album|artist|playlist|mix`) ; `ArtistTile` : `artist, detail` ; `ArtistLine` inline.
- `PlayerBar` : sans moteur, sélecteurs des benches, `--player-h` ; `QueueList` : `data-qi`, `player.move` sans bump de `seq`.
- Stores : `playlistPicker` (`{track}` | `{mode:"new"}` | `{mode:"queue"}`), `openPlaylistPicker`, `openNewPlaylist`, `openQueueSave`, `closePlaylistPicker`.
- Thème : `data-theme` sur `<html>`, clé `ui.theme`, tokens de la phase 1 ; Pochette met `--cover-ac`.

## Ordre de fusion

A → C → B → D → E → F. Puis G. Puis revue visuelle R1 (Playwright 1440/390 contre `DESIGN.md` + prototype) et revue fonctionnelle R2 (lecture, file, favoris, playlists, réglages, streaming). Corrections dans de nouveaux lots, puis feu vert avant merge sur `main`.
