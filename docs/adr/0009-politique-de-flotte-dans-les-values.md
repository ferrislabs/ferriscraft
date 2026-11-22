# 0009. La politique de la flotte SheepWars reste dans les values Helm

Status: proposed
Date: 2026-11-22

## Context

L'issue #16 proposait un second CRD `SheepWarsFleet`, modifiable à chaud, pour `minReplicas`, `maxReplicas`, `standby`, `idleTtl` et les délais. Aujourd'hui ces valeurs sont des variables d'environnement de l'orchestrateur, rendues par le chart depuis `orchestrator.sheepwars.*`. Le déploiement passe par ArgoCD en auto-sync avec `selfHeal`, et les values de prod vivent dans le dépôt infra.

## Decision

Pas de CRD `SheepWarsFleet`. La politique reste dans les values Helm et arrive à l'orchestrateur par variables d'environnement. Changer une valeur, c'est une PR sur le dépôt infra, puis un redémarrage de l'orchestrateur déclenché par ArgoCD.

## Alternatives rejected

- CRD `SheepWarsFleet` modifiable avec `kubectl edit` : il faudrait l'objet dans git pour ArgoCD, sinon `selfHeal` le remettrait à l'état de git, et une modification à chaud contournerait la revue. Il faudrait aussi que l'orchestrateur surveille l'objet et recalcule sa politique en cours de route. Le gain est faible : la politique change rarement.
- ConfigMap relue à chaud : même problème avec git et `selfHeal`, sans schéma.

## Consequences

- Gains : une seule source de vérité (git), revue par PR, aucun nouveau code de rechargement.
- Costs : changer la politique redémarre l'orchestrateur. La file est en mémoire jusqu'à ce que `plc` W6 la mette dans Postgres.
- What becomes harder : rien de nouveau. Si un réglage à chaud devient nécessaire (événement, pic), il faudra un nouvel ADR.
