# Contrat d'intégration du plugin TowerWars ↔ Orchestrateur FerrisCraft

Ce document décrit comment le plugin **`ferriscraft-towerwars`** ([`crates/ferriscraft-towerwars`](../crates/ferriscraft-towerwars)) — exécuté en sidecar dans chaque Pod `towerwars-*` — s'intègre avec `ferriscraft-orchestrator` et `ferriscraft-velocity`. Il suit le même contrat que SheepWars ([`SHEEPWARS_PLUGIN_CONTRACT.md`](./SHEEPWARS_PLUGIN_CONTRACT.md)) avec le préfixe `TOWERWARS_`. Le choix d'un moteur de flotte par jeu est décrit dans l'[ADR 0011](./adr/0011-un-moteur-de-flotte-par-jeu.md) et les règles du jeu dans [`specs/towerwars.md`](./specs/towerwars.md).

Les DTOs Rust partagés sont dans [`crates/ferriscraft-protocol`](../crates/ferriscraft-protocol/src/lib.rs).

---

## 1. Variables d'environnement

### Injectées automatiquement dans le Pod

| Variable | Exemple | Rôle |
|---|---|---|
| `SERVER_ID` | `towerwars-a1b2c3` | Identifiant unique du serveur (nom du serveur dans Velocity) |
| `ORCHESTRATOR_URL` | `http://ferriscraft-orchestrator:8080` | URL HTTP interne du control plane |
| `ORCHESTRATOR_TOKEN` | *(optionnel)* | Jeton Bearer (`Authorization: Bearer <token>`) si activé |
| `RSPIGOT_URL` | `http://127.0.0.1:25582` | Endpoint gRPC du conteneur `rspigot` local au Pod |
| `RSPIGOT_PLUGIN_NAME` | `ferriscraft-towerwars` | Nom d'enregistrement du plugin gRPC |
| `TOWERWARS_MAX_PLAYERS` | `6` | Capacité maximale du serveur |
| `TOWERWARS_MAP_NAME` | *(optionnel)* | Nom de carte choisi par l'orchestrateur (`TOWERWARS_MAP`) |

### Paramètres optionnels du plugin

| Variable | Défaut | Rôle |
|---|---|---|
| `TOWERWARS_MODE` | `solo` | `solo` (6 voies, 4 à 6 joueurs) ou `teams` (2 joueurs par voie, 4 à 12 joueurs) |
| `TOWERWARS_MIN_PLAYERS` | `1` | Seuil de départ sans orchestrateur ; avec orchestrateur, le seuil est `expected_players` (réponse de heartbeat) |
| `TOWERWARS_MAX_PLAYERS` | `6` | Plafonné par le mode |
| `TOWERWARS_COUNTDOWN_SECS` | `15` | Durée du compte à rebours |
| `TOWERWARS_END_DELAY_SECS` | `10` | Délai de célébration avant le retour au Hub |
| `TOWERWARS_EPHEMERAL` | `true` si `ORCHESTRATOR_URL` est défini | Termine en phase `ending` pour recyclage du Pod |
| `TOWERWARS_HEARTBEAT_INTERVAL_MS` | `2000` | Cadence du heartbeat (minimum `500` ms) |
| `TOWERWARS_MAP_PATH` | *(carte intégrée)* | Fichier JSON de carte (6 voies numérotées de 0 à 5) |
| `TOWERWARS_SEED` | *(aléatoire)* | Graine de la partie, pour rejouer une partie à l'identique |

TowerWars construit son arène dans un monde vide au démarrage : il n'y a ni `world.zip`, ni `arena.json`, ni catalogue S3.

### Droits

La permission `towerwars.admin` (ou `op`) donne accès à `/towerwars forcestart` et `/towerwars stop`.

---

## 2. Cycle de vie d'une partie (`ServerPhase`)

Un heartbeat est envoyé toutes les 2 secondes (et à chaque connexion, déconnexion ou transition de phase) sur `POST ${ORCHESTRATOR_URL}/api/v1/servers/heartbeat`, avec les mêmes phases que SheepWars (`starting`, `waiting`, `countdown`, `in_game`, `ending`). Le heartbeat est routé vers le moteur qui possède le `server_id`. Si `TOWERWARS_ENABLED` est `false` côté orchestrateur, il répond 400.

| Phase | Comportement Orchestrateur & Velocity |
|---|---|
| `starting` | Pod en cours d'initialisation |
| `waiting` | L'orchestrateur y dispatche les joueurs de la file TowerWars jusqu'à `max_players` |
| `countdown` | Continue d'accepter les joueurs restants |
| `in_game` | Retiré du matchmaking ; un serveur `waiting` est provisionné si la file le demande |
| `ending` | Désenregistré de Velocity, Pod supprimé dès `player_count == 0` |

La file est propre au jeu : `POST /api/v1/queue/join` avec `"game": "tower_wars"`. Un joueur n'est que dans une seule file à la fois.

---

## 3. Retour au Hub

Identique à SheepWars : `POST ${ORCHESTRATOR_URL}/api/v1/players/{player_uuid}/send-lobby`, puis `POST ${ORCHESTRATOR_URL}/api/v1/servers/${SERVER_ID}/phase` avec `{ "phase": "ending" }`. Commande joueur : `/towerwars hub`.

---

## 4. Déploiement dans Helm

TowerWars est **désactivé par défaut** dans [`deploy/charts/ferriscraft/values.yaml`](../deploy/charts/ferriscraft/values.yaml). Tant que `orchestrator.towerwars.enabled` vaut `false`, le chart ne rend ni ConfigMap de Pod, ni CRD, ni règle RBAC, ni variable `TOWERWARS_*`.

```yaml
orchestrator:
  towerwars:
    enabled: false
    minReplicas: 0
    maxReplicas: 3
    standbyServers: 1
    maxPlayers: 6
    queueMinPlayers: 4   # ignoré : la taille est choisie au lobby
    mode: solo
    map: ""
    crd:
      enabled: false
towerwars:
  pluginSidecar:
    enabled: true
    image:
      repository: ferriscraft-towerwars-plugin
      tag: ""
    extraEnv: []
```

Ordre de déploiement : tous les clients (lobby, proxy Velocity) en version N, la migration `0003_queue_game.sql`, puis `orchestrator.towerwars.enabled=true`. Avec `crd.enabled=true`, le chart installe le CRD cluster-scoped `towerwarsservers.ferriscraft.io` (nom court `tws`) : l'AppProject ArgoCD doit autoriser `CustomResourceDefinition`.
