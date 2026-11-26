# Déclencher le juge (T9)

Copie ce texte dans `send-task` (`agentId: cec572be-e9c1-4545-8f2c-8885dc8f5924`, l'agent
« Coder-Claude » — résous-le avec `get-swarm` s'il a changé), avec
`routingReason: "human_pinned"` et `routingNote: "juge visuel désigné par le propriétaire,
2026-11-25"`. Ajoute le libellé `ui` à la PR avant, si elle change
`mobile/android/**/src/main/**` :

```sh
git diff --name-only "$(git merge-base origin/master HEAD)"...HEAD |
  grep -qE '^mobile/android/.*/src/main/' && gh pr edit <n> --add-label ui
```

## Mode `pr` — après une CI verte, avant de fusionner

> Juge la PR #<n> du dépôt fnyaker/Neezer (branche `<branche>`, tête `<sha>`).
>
> Mets-toi à jour : `git fetch origin && git checkout <sha>` dans le clone du dépôt.
> Lis `mobile/tools/judge/rubric.md` et applique-le à la lettre, sans rien modifier.
>
> Télécharge l'artefact `mobile-journey-evidence` du run `mobile.yaml` de cette tête
> (`gh run list --workflow mobile.yaml --branch <branche>`), lance
> `python mobile/tools/judge/precheck.py <dir>`, regarde les 11 captures de l'artefact à côté
> des références de `mobile/tools/judge/reference/`, puis poste ta revue : `gh pr review <n>
> --comment --body-file /tmp/verdict-<n>.md`, et pose le libellé (`gh pr edit <n> --add-label
> judge:ok` ou `judge:block` ou `judge:incomplete`, en retirant les autres).
>
> Ne fusionne pas, ne pousse rien, ne corrige aucun fichier. Un artefact introuvable après
> deux `gh run download` n'est pas un blocage : c'est le verdict `INCOMPLETE`. Si tu es bloqué
> autrement (PNG illisibles, `gh` refuse la revue), `store-progress` en `failed` avec la
> commande et sa sortie.
>
> Termine par `store-progress` en `completed` : `Verdict : <PASS|BLOCK|INCOMPLETE>`, le
> nombre de critères en échec, le lien de la revue.

## Mode `release` — avant `git push origin v<version>`

> Juge la release `<version>` du dépôt fnyaker/Neezer, sur le commit `master` à `<sha>`
> (celui qui va être tagué). Lis `mobile/tools/judge/rubric.md` et applique-le.
>
> Prends l'artefact `mobile-journey-evidence` du run `mobile.yaml` de `<sha>`, lance
> `precheck.py`, regarde les captures à côté des références de
> `mobile/tools/judge/reference/`, et rends un verdict.
> Il n'y a pas de PR : écris ta revue dans `/tmp/verdict-<version>.md`, joins-la à ta
> sortie, et `store-progress` en `completed` avec `Verdict : <PASS|BLOCK|INCOMPLETE>` et
> les critères en échec. En cas de `BLOCK`, le tag ne part pas et le propriétaire doit
> le savoir : dis-le en tête de ta sortie.
>
> Ne crée pas le tag, ne pousse rien, ne touche à aucun fichier du dépôt.
