# WS2.M7 — Décision Q2 sanitizer policy source

## Décision

Le sanitizer MCP de production est désormais une responsabilité `mcpd`, pas une dépendance implicite de `hermes-agent`.

Source retenue : `/etc/acos/policy.md`, embarqué dans l'image ACOS et vérifié au boot par `mcpd`.

## Rationale

- `mcpd` est la frontière native du scheme `mcp:` : une sortie tool hostile doit être filtrable même si l'agent appelant est minimal, mal configuré ou compromis.
- `hermes-agent` peut garder ses défenses côté orchestration, mais elles deviennent une défense en profondeur, pas la source d'autorité.
- Un fichier policy en image rend la décision auditable et versionnable sans lier le runtime à une config externe de la VPS.
- La vérification boot-time évite le mode dangereux "sanitizer absent mais service actif".

## Contract boot-time

`mcpd` appelle `verify_policy_files_or_die()` au démarrage, avant l'enregistrement du scheme `mcp:`.

Comportement :

- build développement : fichier absent => warning et boot autorisé ; fichier présent mais vide => fatal.
- build `production` : fichier absent ou vide => fatal exit code 78.
- chemin canonique : `/etc/acos/policy.md`.

## Non-objectifs

- Pas de nouveau DSL policy dans ce PR.
- Pas de migration du corpus sanitizer.
- Pas de changement de seuils parity WS2.M4.
- Pas de dépendance runtime à `hermes-agent`.

## Done criterion mapping

Backlog : `Décision Q2 sanitizer — source policy file mcpd (build-time hash, /etc/acos/policy.md, ou rester hermes-agent). Done : décision Khéri enregistrée ; si "in mcpd", ajouter verify_policy_files_or_die() à main.rs.`

Cette note enregistre la décision `in mcpd` avec `/etc/acos/policy.md`, et le code `mcpd/src/main.rs` ajoute le hook `verify_policy_files_or_die()`.
