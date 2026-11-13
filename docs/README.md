# Documentation — carte et règles

Il n'y a **qu'une seule documentation vivante** : `docs/agents/`, une note par zone, lue seulement
quand une tâche touche cette zone. `CLAUDE.md` (et son lien `AGENTS.md`) porte le processus et les
règles communes ; `README.md` s'adresse à la personne qui installe Neezer ; `todo.md` est la liste
des fonctions qui manquent encore.

## Ce qui vit ici

| Fichier | Rôle |
|---|---|
| [`agents/`](agents/) | Les 12 notes d'ingénierie, la référence à jour : règle, pourquoi, chiffre mesuré. |
| [`detachement-deezer.md`](detachement-deezer.md) | Référence livrée : Deezer comme source de sons, plus comme compte à synchroniser (décisions et raisons). |
| [`multi-provider.md`](multi-provider.md) | Référence livrée (sauf le registre de réglages §13) : plusieurs fournisseurs, une seule bibliothèque, doublons, qualité et accès. |
| [`playlist-export.md`](playlist-export.md) | Référence de la fonction d'export (Spotify, YouTube Music, Deezer, fichier), à jour. |
| [`audit-securite.md`](audit-securite.md) | Instantané daté de la passe offensive de juillet 2026 : causes, correctifs, constats ouverts. |
| [`repro/`](repro/) | Harnais de reproduction (transport, file, pause) ; ils restent exécutables. |
| [`screenshots/`](screenshots/) | Visuels du README. |

## Les règles du ménage

- **Un plan terminé se supprime**, ou devient une référence **explicitement marquée livrée** s'il
  porte des décisions durables (c'est le cas de `detachement-deezer.md` et `multi-provider.md`).
  Ce qui doit survivre (une règle, un piège, un chiffre) est fusionné dans la note `docs/agents/`
  de la zone **avant** la suppression ; le plan lui-même n'est pas un journal.
- **Pas de nouveau markdown à la racine** en dehors de `README.md`, `CLAUDE.md`/`AGENTS.md` et
  `todo.md`. Une étude, un handoff de design ou une carte de chantier vit le temps du chantier.
- **Une note d'aire dit la règle, pourquoi elle existe et le nombre qui l'a prouvée** — pas un
  changelog. Les liens entre notes sont vérifiés par `tests/test_agent_docs.py`.
- **Une note stale est pire que pas de note** : quand le code change un comportement documenté, la
  note change dans le même commit.

## Où lire quoi

- Comprendre le produit : [`../README.md`](../README.md).
- Travailler sur le code : [`../CLAUDE.md`](../CLAUDE.md), puis la ou les notes de la zone touchée.
- Savoir quoi faire ensuite : [`../todo.md`](../todo.md).
- Vérifier une commande, un bench, une mesure : [`agents/commands.md`](agents/commands.md) et
  [`agents/testing.md`](agents/testing.md).
