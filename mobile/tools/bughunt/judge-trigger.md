# Déclencher le juge de la chasse (B3)

Copie ce texte dans `send-task` (agentId de Coder-Claude, `cec572be-e9c1-4545-8f2c-8885dc8f5924`
au 2026-11-27, à résoudre par `get-swarm` à l'envoi), avec `routingReason: "human_pinned"` et
`routingNote: "juge de la chasse aux bugs, 2026-11-26"`. Le schedule `neezer-bughunt-judge`
(lundi 06:30 `Europe/Paris`, timeout 20 min) envoie le même texte.

## Mode `bughunt` — après un run nocturne de `bughunt.yaml`

> Juge la chasse aux bugs du dépôt fnyaker/Neezer : le dernier run réussi de `bughunt.yaml`
> (`gh run list --workflow bughunt.yaml --limit 5`, conclusion `success` ou `failure` avec
> artefact : le job conclut `failure` quand il trouve un bug bloquant, c'est voulu), son
> `<run-id>`, sa date et sa tête `<sha>`.
> Avant tout : cherche dans les tâches récentes (`get-tasks`) un verdict déjà rendu pour ce
> `<run-id>`. S'il existe pour les deux applications, écris « déjà jugée » et arrête-toi : on
> ne rejuge jamais un même `run-id`.
> Lis `mobile/tools/judge/rubric.md` et applique-le à la lettre, sans le modifier.
> Télécharge les artefacts `bughunt-web` et `bughunt-native` de ce run
> (`gh run download <run-id> -n bughunt-web -D /tmp/bh-web`, idem `bughunt-native`).
> Pour chaque application : lis `bughunt-<app>.md`, ouvre les captures (`screens/`), remplis le
> tableau des six critères **par écran** (une ligne par écran, la preuve citée : fichier +
> ligne du rapport), puis une section « Bugs confirmés » (ceux dont la capture et le tableau du
> rapport établissent le défaut) et « Faux positifs » (ceux que la preuve ne soutient pas),
> avec pour chacun la gravité que tu retiens et pourquoi. Ajoute les défauts que seules les
> images portent. Une gravité abaissée ou relevée tient en une phrase vérifiable sur la capture.
> Écris `/tmp/bughunt-verdict-<app>.md` et joins-le à ta sortie. Ne clique pas, ne lance rien,
> ne modifie aucun fichier du dépôt, ne pousse rien.
> Termine par `store-progress` en `completed` avec `Verdict : <PASS|BLOCK|INCOMPLETE>`, le nombre
> de bugs confirmés par gravité, et le chemin des deux verdicts.
> Si un artefact manque après deux `gh run download`, ce n'est pas un blocage : `INCOMPLETE`.
> Si tu ne peux pas afficher les PNG, arrête-toi et rapporte-le : on ne juge pas à l'aveugle.
