# X3 — les captures émulateur restent à prendre

Le lot X3 (les écrans de rayons : le squelette s'efface, le contenu entre) demande des preuves
visuelles que ce worker ne peut pas produire : il n'a pas de `/dev/kvm` (règle 8 des règles de lot).
La preuve émulateur vient du job `emulator` de `mobile.yaml` ; les captures ci-dessous sont **dues**
sur une machine à KVM, avec les commandes de `docs/agents/mobile.md` (*The emulator, headless, as
the user*) :

```sh
export ANDROID_HOME="$HOME/.local/share/android-sdk"
export ANDROID_AVD_HOME="$HOME/.local/share/android-avd"
mobile/tools/emu.sh start --port 5581 && mobile/tools/emu.sh wait --port 5581
# install the debug APK, sign in, open a screen, then:
adb -s emulator-5581 exec-out screencap -p > x3-<ecran>-50.png
```

- `x3-accueil-50.png` — Accueil au froid, saisi à ~50 % du fondu squelette → contenu : la première
  ligne du contenu est à la même ordonnée que le squelette (± 1 dp), seule l'opacité diffère.
- `x3-rechercher-50.png` — Rechercher : la même mesure sur une réponse.
- `x3-podcasts-50.png` — Podcasts : squelette → grille.
- `x3-nouveautes-50.png` / `x3-sortie-50.png` / `x3-recap-50.png` / `x3-styles-50.png` — les quatre
  écrans de découverte.
- `x3-rechercher-onglet.png` — un changement d'onglet de Rechercher : la rangée de puces n'a pas
  bougé, le panneau a changé.

Ce qui est prouvé sans émulateur, et où : les tests de phase des quatre classes de capture
(`HomeGoldenTest.aPhaseChangeReplacesInTheSameFrameAtEco`, `SearchGoldenTest.theLoadingPhase…` et
`aTabSwitchKeepsTheChipsRowAndSwapsThePanel`, `PodcastsGoldenTest.aPhaseChange…`,
`DiscoveryGoldenTest.theReleases/…/theShelf/…/theRecap/…/theExplorerPhase…`) et la section *X3* de
`docs/agents/mobile.md`. Les goldens T3 restent **inchangés** : le preset dessine les mêmes pixels au
repos, ce qui est la preuve qu'aucune géométrie n'a bougé.
