# 0008. Les serveurs SheepWars sont des objets `SheepWarsServer` gérés par l'orchestrateur

Status: proposed
Date: 2026-11-22

## Context

L'orchestrateur crée des Pods nus depuis un template (`k8s.rs`, `build_sheepwars_pod`). L'état de la flotte (phase, joueurs, version) n'existe que dans sa mémoire et dans `GET /api/v1/status`, qui demande le jeton. `kubectl get pods` ne dit pas quel serveur est en partie, sur quelle version, ni pourquoi il est gardé. Le 2026-11-22, deux serveurs sont restés vides après leur partie sans qu'on puisse voir leur état depuis le cluster (#14).

## Decision

Un CRD namespacé `sheepwarsservers.ferriscraft.io` (`v1alpha1`, nom court `sws`). L'orchestrateur crée un objet `SheepWarsServer` par serveur, puis son Pod avec une `ownerReference` vers l'objet. Supprimer l'objet supprime le Pod (garbage collector Kubernetes). Le `spec` porte le hash du template et la map. Le `status` (sous-ressource) porte phase, joueurs, version, `outdated`, `ready` et la date du dernier changement de phase. Il est écrit seulement quand une de ces valeurs change. `kubectl get sws` affiche ces colonnes. L'orchestrateur reste le seul contrôleur et `FleetEngine` reste pur : seul l'adapter Kubernetes change.

## Alternatives rejected

- Agones (`GameServer`, `Fleet`) : il faut installer son contrôleur dans le cluster, intégrer son SDK dans rspigot, et remplacer notre `FleetEngine` et la file. C'est trop lourd pour une flotte de 1 à 5 serveurs, et ça ajoute une dépendance d'infrastructure.
- Garder les Pods nus et ajouter des annotations de statut sur le Pod : visible avec `kubectl`, mais sans colonnes dédiées, ni schéma, ni sous-ressource `status`. Et une annotation écrite sur un Pod ne survit pas à sa suppression.
- Écrire le `status` à chaque heartbeat (toutes les 2 s par serveur) : charge inutile sur l'API server pour une information qui ne change qu'à chaque phase.

## Consequences

- Gains : `kubectl get sws` montre la flotte sans jeton. Supprimer un objet arrête proprement son serveur. Les Pods orphelins ne sont plus possibles.
- Costs : un CRD cluster-scoped dans le chart, donc l'AppProject ArgoCD `ferriscraft` doit autoriser `apiextensions.k8s.io/CustomResourceDefinition`. Ajout de la feature `derive` de `kube` et de la dépendance `schemars`.
- What becomes harder : le schéma du CRD est versionné. Un champ renommé demande une nouvelle version (`v1alpha2`) ou une compatibilité ascendante.
