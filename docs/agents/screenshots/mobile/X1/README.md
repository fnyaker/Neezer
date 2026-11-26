# X1 — les captures émulateur restent à prendre

Le lot X1 (l'écran qui entre : 200 ms, 8 dp, rien pour un balayage d'onglet) demande deux preuves
visuelles que ce worker ne peut pas produire : il n'a pas de `/dev/kvm` (règle 8 des règles de lot).
La preuve émulateur vient du job `emulator` de `mobile.yaml` ; les captures ci-dessous sont **dues**
sur une machine à KVM, avec les commandes de `docs/agents/mobile.md` (*The emulator, headless, as
the user*) :

```sh
export ANDROID_HOME="$HOME/.local/share/android-sdk"
export ANDROID_AVD_HOME="$HOME/.local/share/android-avd"
mobile/tools/emu.sh start --port 5581 && mobile/tools/emu.sh wait --port 5581
# install the debug APK, sign in, then:
adb -s emulator-5581 exec-out screencap -p > x1-poussee-50.png
```

- `x1-poussee-50.png` — une carte → un album, saisi à ~50 % de la poussée : le contenu de l'écran
  entrant est **8 dp plus bas** que sa place au repos (le fondu est déjà à mi-course).
- `x1-poussee-repos.png` — le même écran une fois la poussée finie (l'image de référence).
- `x1-onglet-image-1.png` / `x1-onglet-image-2.png` — deux images **consécutives** après un tap
  d'onglet, identiques : le routeur n'a pas bougé la page (le pager seul anime).

Ce qui est prouvé sans émulateur, et où : `RouteMotionTest` (identité des quatre transitions,
balayage d'onglet à Full et à Off, retour qui tient ses deux pages pendant le fondu, première
composition statique) et la section *X1* de `docs/agents/mobile.md`.
