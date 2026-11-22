# 0010. Le SDK rspigot est figé par un commit écrit dans `rspigot.ref`

Status: proposed
Date: 2026-11-22
Remplace : [ADR 0003](0003-sdk-rspigot-fige-par-tag-git.md)

## Context

L'ADR 0003 prévoyait des dépendances `rspigot-*` en `git` + `tag`. Le dépôt `ferrislabs/rspigot` n'a aucun tag et il est privé : avec des dépendances `git`, `cargo` doit s'authentifier sur GitHub dans chaque build Docker et sur chaque poste. Aujourd'hui la CI clone la branche par défaut de rspigot, donc deux builds du même commit FerrisCraft peuvent embarquer deux SDK différents.

## Decision

Le fichier `rspigot.ref`, à la racine du dépôt, contient le sha complet du commit rspigot utilisé. La CI clone rspigot à ce commit et le transmet au build. Les dépendances `rspigot-*` restent en `path`, ce qui ne change rien pour le développement local. Monter rspigot, c'est une PR qui change `rspigot.ref`. Les images exposent ce commit dans `RSPIGOT_REF`.

## Alternatives rejected

- Tag git (ADR 0003) : demande de publier des tags sur rspigot et d'authentifier `cargo` dans les builds Docker (secret BuildKit) et sur les postes.
- `git` + `rev` dans `Cargo.toml` : même problème d'authentification, sans gain par rapport au fichier.

## Consequences

- Gains : build reproductible, pas d'authentification `cargo`, montée de version visible dans une PR.
- Costs : rien n'empêche un poste local de compiler contre un autre commit de rspigot. La CI fait foi.
- What becomes harder : rien.
