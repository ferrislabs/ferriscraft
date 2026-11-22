# 0005. L'orchestrateur est le seul à écrire la progression

Status: proposed
Date: 2026-11-22

## Context

La progression (XP, levels, coins, cosmétiques débloqués) est gagnée dans les mini-jeux et dépensée au lobby. Les serveurs SheepWars sont des Pods éphémères, créés et supprimés par l'orchestrateur. L'orchestrateur reçoit déjà les heartbeats de tous les serveurs, avec un jeton Bearer, et aura sa base Postgres ([ADR 0004](0004-file-sheepwars-dans-postgres.md)).

## Decision

L'orchestrateur porte la progression : règles de récompense (via le crate pur `ferriscraft-progression`), base `progression` du cluster CNPG, et API HTTP. Les mini-jeux envoient un rapport de partie en fin de manche. Le lobby lit les profils et passe les achats par l'API. Seul le Pod orchestrateur reçoit les identifiants de la base.

## Alternatives rejected

- Un service `progression` séparé : découpage plus net, mais un déploiement, un jeton et une sonde de plus pour un domaine encore petit. On pourra l'extraire plus tard : le domaine vit dans son propre crate.
- Chaque plugin écrit directement en base : les règles de récompense seraient dupliquées dans chaque mini-jeu, et chaque Pod de jeu éphémère recevrait les identifiants Postgres.

## Consequences

- Gains : une seule implémentation des règles, une seule transaction par rapport, pas d'identifiants de base dans les Pods de jeu.
- Costs : l'orchestrateur grossit et devient nécessaire pour gagner des récompenses. S'il est absent, SheepWars renvoie le rapport pendant 2 minutes puis abandonne.
- What becomes harder : un redémarrage de l'orchestrateur coupe la boutique quelques secondes. Le lobby doit afficher « Boutique indisponible » au lieu de planter.
