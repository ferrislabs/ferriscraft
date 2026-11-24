# 0011. L'orchestrateur héberge un moteur de flotte par jeu

Status: proposed
Date: 2026-11-23

## Context

L'orchestrateur est écrit pour un seul jeu. `FleetEngine` possède la file et la flotte SheepWars, `k8s.rs` crée des Pods `sheepwars-*` et un `SheepWarsServer` ([ADR 0008](0008-crd-sheepwarsserver-maison.md)), `config.rs` lit `SHEEPWARS_*`, et la table `queue_entries` ([ADR 0004](0004-file-sheepwars-dans-postgres.md)) n'a ni jeu ni colonne pour le dire. `ServerKind` ne connaît que `Lobby` et `SheepWars`. TowerWars (`docs/specs/towerwars.md`) est un second mini-jeu avec sa propre file, ses propres serveurs et sa propre politique de flotte. [ADR 0009](0009-politique-de-flotte-dans-les-values.md) garde la politique dans les values Helm : il en faut une par jeu.

La production tourne avec des versions N et N-1 ensemble ([ADR 0002](0002-plusieurs-lobbies-et-drain.md)) : le JSON et le schéma Postgres doivent rester lisibles dans les deux sens.

## Decision

Un `FleetEngine` par `GameKind`, tous détenus par un registre `Games` (`crates/ferriscraft-orchestrator/src/games.rs`) derrière un seul verrou. Chaque moteur reçoit sa `FleetPolicy`, qui porte le `game` et remplace le `ServerKind::SheepWars` codé en dur. Le moteur reste pur et ne connaît pas les autres jeux.

- Protocole : `ServerKind::TowerWars`. `GameKind` (déjà utilisé par `MatchReport`) gagne `TowerWars` et un défaut `SheepWars`. `JoinQueueRequest` et `LeaveQueueRequest` gagnent `#[serde(default)] game`. `FleetStatusSnapshot` gagne `games` (état par jeu, vide pour un orchestrateur N-1). `GET /api/v1/queue/player/{uuid}` accepte `?game=`, défaut SheepWars. Une route sans `game` est une route SheepWars.
- Routage : les instances de lobby et le drain restent dans le moteur SheepWars. Un heartbeat `Lobby` y va, un heartbeat de jeu va au moteur de son jeu. Un jeu inactif répond 400. Les snapshots proxy et de statut fusionnent tous les moteurs ; les champs historiques de `FleetStatusSnapshot` décrivent SheepWars.
- File : une colonne `queue_entries.game text NOT NULL DEFAULT 'sheepwars'` avec une migration avant-seulement et idempotente. `player_uuid` reste unique : un joueur est dans une seule file à la fois. Rejoindre l'autre jeu renvoie `AlreadyQueued`. Les numéros de séquence des joueurs remis en tête de file sont pris dans une plage par jeu, pour que deux moteurs n'écrivent jamais la même clé primaire.
- Kubernetes : un `GameSpec` (composant, préfixe de Pod, préfixe d'environnement, image par défaut, usage du catalogue S3) et un `GameConfig` par jeu actif. Le réconciliateur est écrit une fois, générique sur le type du CRD, et tourne une fois par jeu actif. `SHEEPWARS_*` ne change pas. `TOWERWARS_ENABLED` (défaut `false`) active le jeu, `TOWERWARS_*` le configure. Pods `towerwars-xxxxxx`, label `app.kubernetes.io/component=towerwars`.
- CRD : `TowerWarsServer` (`towerwarsservers.ferriscraft.io`, nom court `tws`), même `spec` et même `status` que `SheepWarsServer`. Les deux types sortent d'une macro et partagent un trait `GameServerObject` ; `status` est un type commun `GameServerStatus`.
- Cartes : le catalogue S3 (`world.zip` et `arena.json`) reste celui de SheepWars et ne change pas. TowerWars ne l'utilise pas : sans `TOWERWARS_MAP`, le plugin charge sa carte intégrée ; avec, le nom est passé au Pod (`TOWERWARS_MAP_NAME`).
- Sans variable `TOWERWARS_*`, l'orchestrateur se comporte comme avant.

## Alternatives rejected

- Un seul `FleetEngine` avec un `if kind` dans la file, le dispatch et la planification : chaque règle (minimum de joueurs, remplacement des serveurs obsolètes, standby) devient une condition de plus, et un jeu peut priver l'autre de capacité.
- Un CRD générique `GameServer` portant un champ `game` : un seul CRD à installer, mais le nom de type et `kubectl get sws|tws` perdent leur sens, et migrer les objets `SheepWarsServer` existants est un changement de schéma sans gain.
- Réutiliser `SheepWarsServer` avec un champ `game` : le nom du CRD ment.
- Une clé de file `(player_uuid, game)` : un joueur pourrait attendre dans deux jeux et être transféré deux fois. Une file à la fois suffit au besoin et garde l'unicité actuelle.
- Un orchestrateur par jeu : isole les pannes, mais duplique la base, l'API et le routage Velocity.

## Consequences

- Gains : la file, la flotte et la politique de chaque jeu sont indépendantes et testées séparément ; ajouter un troisième jeu est une variante de `GameKind`, une entrée `GameSpec` et un type CRD de plus. Le réconciliateur n'est pas dupliqué.
- Costs : le registre `Games` et ses règles entre jeux (un joueur, un lobby partagé, un transfert de retour) ; une migration Postgres (`0003_queue_game.sql`) à jouer avant le nouvel orchestrateur ; un CRD cluster-scoped de plus dans le chart, donc une entrée de plus pour l'AppProject ArgoCD ; les littéraux `JoinQueueRequest` et `LeaveQueueRequest` des clients changent.
- What becomes harder : un client N-1 (proxy Velocity, lobby) ne sait pas lire `tower_wars` dans `ServerKind`. Ordre de déploiement : tous les clients en version N, puis `TOWERWARS_ENABLED=true`. Un lobby N-1 qui envoie un join sans `game` rejoint SheepWars, comme avant. Les mutations de file de deux jeux écrites dans le même lot ne sont pas ordonnées entre elles ; le flush a lieu à la fin de chaque requête, sous un verrou.
