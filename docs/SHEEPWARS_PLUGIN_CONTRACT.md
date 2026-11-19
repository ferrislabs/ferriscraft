# Contrat d'intégration du plugin SheepWars ↔ Orchestrateur FerrisCraft

Ce document décrit comment le futur plugin **SheepWars** (exécuté en sidecar dans chaque Pod `sheepwars-*`) s'intègre avec `ferriscraft-orchestrator` et `ferriscraft-velocity`.

Les DTOs Rust partagés sont déjà disponibles dans le crate [`crates/ferriscraft-protocol`](../crates/ferriscraft-protocol/src/lib.rs).

---

## 1. Variables d'environnement injectées automatiquement dans le Pod

L'orchestrateur injecte ces variables dans chaque conteneur du Pod SheepWars :

| Variable | Exemple | Rôle |
|---|---|---|
| `SERVER_ID` | `sheepwars-a1b2c3` | Identifiant unique du serveur (correspond aussi au nom du serveur dans Velocity) |
| `ORCHESTRATOR_URL` | `http://ferriscraft-orchestrator:8080` | URL HTTP interne du control plane |
| `ORCHESTRATOR_TOKEN` | *(optionnel)* | Jeton Bearer (`Authorization: Bearer <token>`) si activé |
| `RSPIGOT_URL` | `http://127.0.0.1:25581` | Endpoint gRPC du conteneur `rspigot` local au Pod |
| `SHEEPWARS_MAX_PLAYERS` | `16` | Capacité maximale configurée pour l'arène |

---

## 2. Cycle de vie d'une partie (`ServerPhase`)

Envoyer un heartbeat toutes les **2 à 3 secondes** sur `POST ${ORCHESTRATOR_URL}/api/v1/servers/heartbeat` :

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
| `in_game` | Partie lancée : retiré du matchmaking, l'orchestrateur provisionne un nouveau serveur `waiting` si la file demande (jusqu'à `k` max) |
| `ending` | Partie terminée : désenregistré de Velocity puis **Pod supprimé proprement** dès `player_count == 0` (et remplacé par un Pod vierge si besoin) |

> Sans plugin SheepWars actif, l'orchestrateur passe automatiquement le Pod en `waiting` dès que le port Minecraft TCP (`25565`) répond, afin de valider la file d'attente, le routage Velocity et le scaling sur `k` Pods.

---

## 3. Retourner les joueurs au Hub en fin de partie

Lorsque la partie se termine (ou via `/hub` géré aussi par Velocity) :

1. Appeler pour chaque joueur :
   ```http
   POST ${ORCHESTRATOR_URL}/api/v1/players/{player_uuid}/send-lobby
   Content-Type: application/json

   { "reason": "fin de partie" }
   ```
   Velocity re-bascule instantanément le joueur sur `lobby` sans déconnexion.

2. Passer le serveur en phase `ending` :
   ```http
   POST ${ORCHESTRATOR_URL}/api/v1/servers/${SERVER_ID}/phase
   Content-Type: application/json

   { "phase": "ending" }
   ```
   Une fois `player_count == 0` (ou déconnexion confirmée), l'orchestrateur supprime le Pod Kubernetes et sa map éphémère (`emptyDir`) est automatiquement réinitialisée au prochain Pod.

---

## 4. Déployer le plugin SheepWars dans Helm

Dans [`deploy/charts/ferriscraft/values.yaml`](../deploy/charts/ferriscraft/values.yaml) :

```yaml
sheepwars:
  pluginSidecar:
    enabled: true
    image: ghcr.io/ferrislabs/ferriscraft-sheepwars-plugin:latest
```
