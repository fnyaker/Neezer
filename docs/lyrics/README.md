# La colonne de paroles séparée : avant / après

Captures de la retouche de la vue des paroles dans le lecteur plein écran.

- `avant-1440-encre-clair` / `apres-1440-encre-clair` — le cas qui change :
  à 1440×900 (PC large, colonne de paroles séparée), thème Encre, pochette
  claire. Avant, la colonne était une carte opaque à bordure ; après, les mots
  flottent sur la pochette floutée, sans fond, sans cadre.
- `apres-1440-encre-sombre` — la même colonne au-dessus d'une pochette noire :
  l'ombre de texte est peinte dans la couleur de fond du thème, elle ne dépend
  donc pas de la pochette.
- `apres-1440-papier-clair` — thème Papier, pochette claire : l'encre sombre
  reste lisible sans carte.
- `avant-1024-integree` / `apres-1024-integree` — la carte INTÉGRÉE (fenêtre
  étroite, onglet « Paroles » de la carte de file) : elle garde son verre
  liquide. Les deux prises sont identiques à 0,01 % des pixels près (JPEG).
- `apres-390-telephone` — le panneau de paroles du téléphone, inchangé (verre
  du palier 3 sur la pochette) ; la prise est identique à l'octet avant/après.

Les deux prises d'une même paire sont la même page, la même piste, la même
pochette et le même format : seule la feuille de style de la vue des paroles
change. Les contrastes sont relevés sur les pixels peints par
`webapp/test/lyrics/run.mjs` (3 thèmes × pochette claire et sombre, 26 contrôles).
