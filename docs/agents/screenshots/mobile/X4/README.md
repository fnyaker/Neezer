# X4 — ce que l'image montre, et ce qui reste à prendre

Le lot X4 (la Bibliothèque et les fiches : une feuille qui remplace une autre, une note qui apparaît)
change des **opacités**, que Compose n'expose pas à un test de sémantique et qu'un rendu Robolectric
ne peut pas prendre en plein vol : `captureRoboImage` dessine le dernier calque **évalué**, et le
`graphicsLayer` du panneau lit sa progression au moment du dessin — mesuré ici, deux captures d'affilée
montrent `p = 0` puis `p = 1` sans état intermédiaire. La courbe est donc tenue par un test
(`TransitionTest` : 0 à la frame du changement, **0,65 à 16 ms**, 1 après 140 ms) et l'image
intermédiaire reste due d'une machine à `/dev/kvm`.

- `x4-feuille-arrivee-360dp.png` — la Bibliothèque après un changement de feuille (« Titres » →
  « Artistes ») : la bande de feuilles et les outils de la feuille ont leur place, et la grille des
  artistes est arrivée entière. C'est la capture de l'état **final** : le fondu lui-même est le
  `NzPaneEnter` de `designsystem/Transition.kt`, dont la mesure est dans `TransitionTest` et dont la
  description est dans la section *X4* de `docs/agents/mobile.md`.

**Ce qui reste dû sur une machine à KVM** (ce worker n'a pas `/dev/kvm`) : le `FrameTimingMetric`
(p95 ≤ 16,7 ms) d'un dé-favori sur la bibliothèque de 8 000 titres, et les captures d'émulateur à
50 % du fondu (la feuille, la note). La preuve émulateur vient du job `emulator` de `mobile.yaml` ;
les commandes sont celles de `docs/agents/mobile.md` (*The emulator, headless, as the user*) :

```sh
export ANDROID_HOME="$HOME/.local/share/android-sdk"
export ANDROID_AVD_HOME="$HOME/.local/share/android-avd"
mobile/tools/emu.sh start --port 5582 && mobile/tools/emu.sh wait --port 5582
adb -s emulator-5582 exec-out screencap -p > x4-feuille-fondu-50.png
```

Ce qui est prouvé sans émulateur, et où : `TransitionTest` (la progression du panneau aux trois
niveaux, et `NzAppearOnMount` présent à l'image 0 en éco), `LibraryScreenMotionTest` (la feuille
remplacée dans la même image, la bande immobile ± 0 dp, le dé-favori qui déplace les lignes),
`EntityPaneMotionTest` (onglet, phase d'album, feuille de crédits), `EntityBitsMotionTest` (la note).
