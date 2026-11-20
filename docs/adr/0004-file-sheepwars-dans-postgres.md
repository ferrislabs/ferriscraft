# 0004. La file SheepWars est persistée dans Postgres par l'orchestrateur

Status: proposed
Date: 2026-11-20

## Context

La file d'attente SheepWars est une `VecDeque` en mémoire dans `FleetEngine` (`engine.rs:102`). L'orchestrateur tourne avec `replicas: 1` et `strategy: Recreate`. Il redémarre à chaque changement du template SheepWars (annotation `checksum`), donc à chaque mise à jour du plugin SheepWars. Chaque redémarrage vide la file. Exigence : la file ne doit jamais être perdue.

Un joueur sort aujourd'hui de la file dès qu'il est dispatché, avant la confirmation du transfert. Un transfert raté ou un redémarrage à ce moment-là le perd aussi.

[ADR 0001](0001-etat-du-lobby-dans-postgres.md) ajoute déjà un cluster Postgres CloudNativePG au chart.

## Decision

L'orchestrateur persiste la file dans une base dédiée du même cluster CNPG. Chaque mutation (inscription, départ, dispatch, remise en file) est écrite en base avant la réponse HTTP. Au démarrage, `FleetEngine` est rechargé depuis la base. L'ordre est porté par une séquence Postgres, pas par un `Instant`. Une entrée dispatchée reste en base jusqu'à l'arrivée confirmée du joueur (son UUID dans un heartbeat). Un échec ou une expiration la remet à sa place.

## Alternatives rejected

- Reconstruction par les lobbies : chaque instance de lobby renvoie ses joueurs en file après un redémarrage de l'orchestrateur. Pas de base côté orchestrateur, mais l'ordre entre deux instances de lobby dépend de leurs horloges, et un lobby qui redémarre en même temps perd sa part.
- Redis : persistance optionnelle et un service de plus, alors que Postgres est déjà déployé.
- 2 replicas d'orchestrateur avec élection de leader : évite la coupure, mais l'état reste à partager entre replicas. Il faudrait la base quand même.

## Consequences

- Gains : la file survit aux redémarrages et aux crashs. Un transfert raté ne fait plus perdre sa place au joueur.
- Costs : une écriture Postgres par inscription, départ et dispatch (faible volume). Une base et un Job de migration de plus. L'orchestrateur ne démarre pas sans Postgres en prod.
- What becomes harder : les tests de l'API orchestrateur ont besoin de l'adapter mémoire ou d'un Postgres de test. Le schéma de `queue_entries` suit la règle expand puis contract.
