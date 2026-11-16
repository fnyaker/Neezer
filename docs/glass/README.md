# Le verre liquide : avant / après

Captures de la retouche du matériau : opacité −8 points sur les paliers 1 et 2,
image derrière le verre normalisée (les pochettes claires baissées, les sombres
relevées) et saturation +10 points. Chaque image met côte à côte l'ancien verre
(72/86/93 %, image telle quelle) et le nouveau (64/78/93 %, image normalisée).

- `desklib` — Bibliothèque, 1280×900, Encre ;
- `phonelib` — Bibliothèque, 390×844 ;
- `menu` — menu contextuel ouvert sur la grille de pochettes ;
- `deskside` — lecteur plein écran, panneau latéral « À propos » ;
- `phoneabout` — lecteur plein écran téléphone, panneau « À propos ».

Les deux prises sont la **même page**, la même piste et les mêmes pixels : seule
la matière change (les anciennes valeurs sont reposées sur `<html>` pour la
seconde). Les contrastes et les composites sont relevés sur les pixels par
`webapp/test/glass/run.mjs` (57 surfaces, 3 thèmes, 2 formatages).
