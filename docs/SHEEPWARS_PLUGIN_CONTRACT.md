# Contrat d'intégration du plugin SheepWars ↔ Orchestrateur FerrisCraft

Ce document décrit comment le plugin **`ferriscraft-sheepwars`** ([`crates/ferriscraft-sheepwars`](../crates/ferriscraft-sheepwars)) — exécuté en sidecar dans chaque Pod `sheepwars-*` — s'intègre avec `ferriscraft-orchestrator` et `ferriscraft-velocity`.

Les DTOs Rust partagés sont disponibles dans le crate [`crates/ferriscraft-protocol`](../crates/ferriscraft-protocol/src/lib.rs).

---

## 1. Variables d'environnement

### Injectées automatiquement dans le Pod

L'orchestrateur et le template Helm injectent ces variables dans chaque conteneur du Pod SheepWars :

| Variable | Exemple | Rôle |
|---|---|---|
| `SERVER_ID` | `sheepwars-a1b2c3` | Identifiant unique du serveur (correspond aussi au nom du serveur dans Velocity) |
| `ORCHESTRATOR_URL` | `http://ferriscraft-orchestrator:8080` | URL HTTP interne du control plane |
| `ORCHESTRATOR_TOKEN` | *(optionnel)* | Jeton Bearer (`Authorization: Bearer <token>`) si activé |
| `RSPIGOT_URL` | `http://127.0.0.1:25581` | Endpoint gRPC du conteneur `rspigot` local au Pod |
| `RSPIGOT_PLUGIN_NAME` | `ferriscraft-sheepwars` | Nom d'enregistrement du plugin gRPC |
| `SHEEPWARS_MAX_PLAYERS` | `16` | Capacité maximale configurée pour l'arène |

### Paramètres optionnels du plugin (`ferriscraft-sheepwars`)

| Variable | Défaut | Rôle |
|---|---|---|
| `SHEEPWARS_MIN_PLAYERS` | `2` | Nombre minimum de joueurs pour lancer le compte à rebours |
| `SHEEPWARS_COUNTDOWN_SECS` | `15` | Durée du compte à rebours avant le début de la partie |
| `SHEEPWARS_SHEEP_INTERVAL_SECS` | `20` | Intervalle de distribution des moutons |
| `SHEEPWARS_LIVES` | `3` | Nombre de vies par joueur |
| `SHEEPWARS_END_DELAY_SECS` | `10` | Délai de célébration avant le retour au Hub |
| `SHEEPWARS_EPHEMERAL` | `true` si `ORCHESTRATOR_URL` est défini | Termine en phase `ending` pour recyclage du Pod K8s (sinon réinitialise l'arène pour une nouvelle manche) |
| `SHEEPWARS_HEARTBEAT_INTERVAL_MS` | `2000` | Cadence d'envoi du heartbeat (minimum `500` ms) |
| `SHEEPWARS_ARENA_PATH` | `arena.json` (`/tmp/arena.json` en K8s) | Chemin JSON optionnel de personnalisation d'arène |

---

## 2. Cycle de vie d'une partie (`ServerPhase`)

Un heartbeat est envoyé toutes les **2 secondes** (et immédiatement à chaque connexion, déconnexion ou transition de phase) sur `POST ${ORCHESTRATOR_URL}/api/v1/servers/heartbeat` :

```json
{
  "server_id": "sheepwars-a1b2c3",
  "phase": "waiting",
  "player_count": 4,
  "max_players": 16,
  "player_uuids": ["..."]
}
```

Valeurs de `phase` :

| Phase | Comportement Orchestrateur & Velocity |
|---|---|
| `starting` | Pod en cours d'initialisation (non joignable par la file) |
| `waiting` | Serveur prêt : l'orchestrateur y dispatche les joueurs de la file jusqu'à `max_players` |
| `countdown` | Compte à rebours avant lancement : continue d'accepter les joueurs restants si places libres |
| `in_game` | Partie lancée (inclut la célébration) : retiré du matchmaking, l'orchestrateur provisionne un nouveau serveur `waiting` si la file demande (jusqu'à `k` max) |
| `ending` | Partie terminée : désenregistré de Velocity puis **Pod supprimé proprement** dès `player_count == 0` (et remplacé par un Pod vierge si besoin) |

> Sans plugin SheepWars actif, l'orchestrateur passe automatiquement le Pod en `waiting` dès que le port Minecraft TCP (`25565`) répond, afin de valider la file d'attente, le routage Velocity et le scaling sur `k` Pods.

---

## 3. Retourner les joueurs au Hub en fin de partie

Lorsque la partie se termine (ou via `/sheepwars hub` / `/hub`) :

1. Appel pour chaque joueur :
   ```http
   POST ${ORCHESTRATOR_URL}/api/v1/players/{player_uuid}/send-lobby
   Content-Type: application/json

   { "reason": "fin de partie" }
   ```
   Velocity re-bascule instantanément le joueur sur `lobby` sans déconnexion.

2. Passage du serveur en phase `ending` :
   ```http
   POST ${ORCHESTRATOR_URL}/api/v1/servers/${SERVER_ID}/phase
   Content-Type: application/json

   { "phase": "ending" }
   ```
   Une fois `player_count == 0` (ou déconnexion confirmée), l'orchestrateur supprime le Pod Kubernetes et sa map éphémère (`emptyDir`) est automatiquement réinitialisée au prochain Pod.

---

## 4. Déploiement dans Helm

Le sidecar `ferriscraft-sheepwars` est activé par défaut dans [`deploy/charts/ferriscraft/values.yaml`](../deploy/charts/ferriscraft/values.yaml) :

```yaml
sheepwars:
  pluginSidecar:
    enabled: true
    image:
      repository: ferriscraft-sheepwars-plugin
      tag: ""
    extraEnv: []
```
