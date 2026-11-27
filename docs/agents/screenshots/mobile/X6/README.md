# X6 — les derniers changements d'état : ce que les images montrent

Le lot X6 (`Réglages, admin, mise en route, import, soirée`) remplace les phases de ces écrans en
fondu et répare la rangée d'actions de l'import. Les PNG de ce dossier sont des images
**Robolectric** (backend NATIVE, 360 et 600 dp, Encre et Papier), pas des captures d'émulateur :

- `x6-import-actions-360dp-encre.png` / `…-papier.png` — la rangée d'actions de l'import **après** :
  « Lire » et « Mélanger » tiennent sur la première ligne, « Enregistrer » passe à la seconde avec
  son libellé **entier** (130,5 dp). Le troisième bouton n'est pas hors écran : il est **tronqué**
  (« Enregi… », 111,5 dp) sur l'ancienne `Row` — c'est le défaut réel, mesuré.
- `x6-import-actions-360dp-encre-avant.png` / `…-papier-avant.png` — le même écran sur `Row` : le
  troisième libellé est coupé.
- `x6-import-actions-600dp-encre.png` / `…-papier.png` — les trois actions sur **une** ligne.
- `x6-import-actions-600dp-encre-avant.png` / `…-papier-avant.png` — la `Row` à 600 dp (référence).
- `x6-import-phase-360dp-50ms.png` — la phase de l'import (le corps) à **50 %** de son fondu
  (140 ms), le lien d'intro déjà sorti.
- `x6-onboarding-step-360dp-50ms.png` — le passage artistes → compte à **50 %** : le squelette est
  sorti, l'étape « compte » entre (titre à demi-opacité), la barre et « Étape 2 sur 2 » restent
  dehors du swap.

**Ce qui reste dû sur une machine à KVM** (ce worker n'a pas `/dev/kvm`) : le `FrameTimingMetric`
(p95 ≤ 16,7 ms) des changements de phase et les captures d'émulateur à 50 % ; la preuve émulateur
vient du job `emulator` de `mobile.yaml`. Aucun de ces chemins n'ajoute de requête (chaque écran
dessine l'état qu'il a déjà lu) : le delta `NzCore::stats()` est **0**.

Les bornes et les phases sont tenues par `ImportScreenMotionTest`, `OnboardingScreenMotionTest`,
`SettingsPhaseTest`, `OfflinePhaseTest`, `AudioFxPhaseTest`, `AdminPhaseTest` et
`PartyGuestPhaseTest`.
