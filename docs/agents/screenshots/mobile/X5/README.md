# X5 — ce que les images montrent, et ce qui reste à prendre

Le lot X5 (les listes qui bougent : la file, la playlist, l'arbre des genres, les podcasts) change
des **déplacements de lignes**, qui ne se voient qu'en plein vol. Les PNG ci-dessous sont des images
**Robolectric** prises à **100 ms** dans l'animation (horloge Compose gelée), rendues par le vrai
écran, à 360 dp : elles montrent l'état intermédiaire que l'œil voit, ce qu'aucune assertion de
bornes ne peut montrer (voir la section *X5* de `docs/agents/mobile.md`).

- `x5-file-retrait-360dp-100ms.png` — la file, 200 lignes : « Titre 3 » vient d'être retirée et les
  lignes du dessous sont **entre deux places** (« Titre 4 » n'a pas encore rejoint son emplacement).
- `x5-playlist-retrait-360dp-100ms.png` — une playlist en édition : la ligne retirée a laissé un vide
  qui se referme (les lignes de 76 dp glissent).
- `x5-podcast-marque-360dp-100ms.png` — un marqueur « 12:05 Reprendre ici » posé sur l'épisode 1 : sa
  ligne a grandi, les épisodes suivants descendent.
- `x5-arbre-chevron-360dp.png` — l'arbre des genres : « Hardcore » est ouvert et son chevron est
  **tourné** (un seul glyphe `ChevronRight` à 90°, 200 ms au standard), « Gabber » et « Hardhouse »
  fermés restent à droite. Au repos, l'image est celle d'avant à 24 px près (anti-aliasing).

**Ce qui reste dû sur une machine à KVM** (ce worker n'a pas `/dev/kvm`) : le `FrameTimingMetric`
d'un retrait ×3 sur la file de 4 000 titres (p95 ≤ 16,7 ms) et les captures d'émulateur à 50 %. La
preuve émulateur vient du job `emulator` de `mobile.yaml` ; les commandes sont celles de
`docs/agents/mobile.md` (*The emulator, headless, as the user*) :

```sh
export ANDROID_HOME="$HOME/.local/share/android-sdk"
export ANDROID_AVD_HOME="$HOME/.local/share/android-avd"
mobile/tools/emu.sh start --port 5582 && mobile/tools/emu.sh wait --port 5582
adb -s emulator-5582 exec-out screencap -p > x5-file-retrait-50.png
```

Ce qui est prouvé sans émulateur, et où : `QueueSheetMotionTest`, `PlaylistScreenMotionTest`,
`ShowScreenMotionTest`, `GenresChevronTest` (les bornes, l'image 0, l'atterrissage en éco).
