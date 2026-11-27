# X5b — ce que les images montrent, et ce qui reste à prendre

Le lot X5b applique le seam de X4 aux deux écrans dont le corps de phase est un `LazyColumn` : la
fiche playlist et la fiche d'un podcast. Ce qui change est une **opacité** et une **translation** de
calque : le panneau entre (140 ms de fondu, 4 dp), et l'en-tête de l'écran — la bande du titre pour la
playlist, la tête du podcast — **n'entre pas** : il était déjà là et il reste immobile.

Les deux PNG ci-dessous sont des images **Robolectric** prises à **70 ms** dans le fondu (horloge
Compose gelée, progress ≈ 0,65), rendues par les vrais écrans à 360 dp, après le passage de la phase
« contenu » à la phase « vide ». L'en-tête y est **opaque** et le panneau **à demi effacé** : c'est
exactement ce que `NzSwap` autour du `LazyColumn` ne faisait pas (il emportait la bande avec le
panneau).

- `x5b-playlist-panneau-360dp-70ms.png` — la fiche playlist : la bande, les actions et la barre de
  recherche sont à leur place et à pleine opacité ; « Cette playlist est vide » et les suggestions
  entrent en fondu.
- `x5b-podcast-panneau-360dp-70ms.png` — la fiche d'un podcast : la tête (pochette, titre, « Lire »,
  « Tout télécharger ») ne bouge pas ; « Aucun épisode » entre.

**Ce qui reste dû sur une machine à KVM** (ce worker n'a pas `/dev/kvm`) : les captures d'émulateur à
50 % du fondu et le `FrameTimingMetric` (p95 ≤ 16,7 ms) d'un changement de phase sur la playlist de
4 000 titres. La preuve émulateur vient du job `emulator` de `mobile.yaml` ; les commandes sont celles
de `docs/agents/mobile.md` (*The emulator, headless, as the user*) :

```sh
export ANDROID_HOME="$HOME/.local/share/android-sdk"
export ANDROID_AVD_HOME="$HOME/.local/share/android-avd"
mobile/tools/emu.sh start --port 5582 && mobile/tools/emu.sh wait --port 5582
adb -s emulator-5582 exec-out screencap -p > x5b-playlist-panneau-50.png
```

Ce qui est prouvé sans émulateur, et où : `PlaylistScreenPaneTest` (3), `ShowScreenPaneTest` (3) et
`EpisodePanelTabTest` (2) — le nouveau panneau dans la même image en éco, l'ancien sans aucun nœud,
l'en-tête dans la même boîte (± 0 dp) et la ré-émission muette ; la courbe elle-même est
`TransitionTest`'s. **Une honnêteté mesurée** : ces assertions de bornes passent aussi sur l'arbre
d'avant, parce que `NzSwap` comme `nzPaneEnter` passent par un calque et que
`getUnclippedBoundsInRoot` ignore les deux — le fait visible que ce lot corrige est donc tenu par ces
images, pas par une borne (section *X5b* de `docs/agents/mobile.md`).
