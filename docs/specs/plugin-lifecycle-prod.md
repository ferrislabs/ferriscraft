# Cycle de vie des plugins en production

Statut : proposé
Date : 2026-11-20
Id : `plc`

## Goal

Mettre à jour, redémarrer et revenir en arrière sur les plugins `ferriscraft-lobby` et `ferriscraft-sheepwars` en production sans perdre l'état du Lobby ni déconnecter les joueurs.

## Context

État actuel, constaté dans le code :

- Un plugin est un process séparé, relié à rspigot en gRPC sur `127.0.0.1`. Il tourne en sidecar dans le même Pod que rspigot.
- Le SDK se reconnecte seul avec un backoff plafonné à 30 s (`rspigot-plugin/src/runtime.rs:163`).
- rspigot remplace un plugin quand un autre se connecte avec le même nom (`ToPlugin::Replaced`, `rspigot-core/tests/plugins.rs:52`).
- SheepWars : Pods éphémères (`restartPolicy: Never`). Un changement d'image modifie le template, l'annotation `checksum/sheepwars-template` redémarre l'orchestrateur, les nouvelles parties partent sur la nouvelle version. Ce chemin marche déjà.
- Lobby : un seul Deployment, un seul Pod (rspigot + plugin). Velocity l'atteint via le Service `*-lobby` (`velocity.yaml:30`). Une nouvelle image recrée tout le Pod et déconnecte tous les joueurs.
- Le layout (`/app/lobby.json`) est copié dans l'image (`Dockerfile:178`). Les modifications `/lobby set*` sont écrites dans le conteneur et perdues au redémarrage.
- Les sessions FerrisKey (avec `refresh_token`, `auth.rs:181`) sont dans `/tmp/.sessions.json`. Perdues au redémarrage.
- Aucun des deux `main.rs` ne gère SIGTERM.
- `Cargo.toml:12-17` pointe le SDK rspigot sur `/opt/minecraft/dev/rspigot`. La CI (`images.yml`) clone la branche par défaut de `ferrislabs/rspigot`, sans version figée.
- La CI publie déjà un tag `${{ github.sha }}`, mais le chart déploie `global.imageTag: latest`.
- La file SheepWars est une `VecDeque` en mémoire dans l'orchestrateur (`engine.rs:102`), avec `replicas: 1` et `strategy: Recreate`. Chaque redémarrage la vide. Or l'orchestrateur redémarre à chaque changement d'image SheepWars (annotation `checksum`).
- Un transfert acquitté en échec (`ack_transfer`, `engine.rs:435`) ignore `success` : le joueur est déjà sorti de la file.

Contraintes : Kubernetes >= 1.29 en prod (sidecars natifs).

Décisions : [ADR 0001](../adr/0001-etat-du-lobby-dans-postgres.md), [ADR 0002](../adr/0002-plusieurs-lobbies-et-drain.md), [ADR 0010](../adr/0010-rspigot-fige-par-commit.md) (remplace 0003), [ADR 0004](../adr/0004-file-sheepwars-dans-postgres.md).

## Glossary

Voir [`docs/glossary.md`](../glossary.md) : instance de lobby, drain, fenêtre de compatibilité.

## Invariants

1. Un redémarrage de Pod lobby ne perd ni le layout ni les sessions. [test]
2. Une mise à jour du Lobby ne déconnecte aucun joueur du proxy tant qu'il reste au moins une autre instance de lobby prête. [test]
3. Un joueur n'est jamais envoyé vers une instance de lobby en drain. [type : `LobbyState` enum, `Draining` exclu du choix]
4. Dans un même Pod, rspigot et ses plugins viennent de la même release (même tag d'image). [test : rendu Helm]
5. Pendant une mise à jour, la version N et la version N-1 des contrats JSON `ferriscraft-protocol` se comprennent. [test : fixtures N-1]
6. Pendant une mise à jour, le schéma Postgres convient à la version N et à la version N-1 du plugin (migrations expand puis contract). [test]
7. Une image déployée en staging ou en prod est référencée par un tag immuable, jamais `latest`. [test : rendu Helm des values prod]
8. Un `refresh_token` n'apparaît jamais dans un log. [test]
9. Un redémarrage de l'orchestrateur ne retire aucun joueur de la file et ne change pas leur ordre. [test]
10. Un joueur ne sort de la file que sur : départ volontaire, déconnexion du proxy, ou arrivée confirmée sur un serveur SheepWars (son UUID dans `player_uuids` d'un heartbeat). Un transfert en échec ou expiré le remet à sa place. [test]

## Out of scope

- Mise à jour sans coupure de Velocity et de son plugin Java. Velocity est modifié uniquement pour enregistrer et choisir les instances de lobby.
- Haute disponibilité de l'orchestrateur (`replicas: 1`). Il peut être indisponible quelques secondes pendant un redémarrage. Seule la file doit survivre.
- Migration à chaud d'une partie SheepWars en cours vers un autre Pod.
- Rechargement à chaud du code d'un plugin dans un process existant.
- Configuration Kargo et repo `*-infra` (spec à part, une fois les tags immuables en place).
- Chiffrement applicatif des `refresh_token` en base. Le chiffrement disque du stockage CNPG suffit pour le moment.
- Synchronisation en direct du layout entre instances. La dernière écriture gagne, les autres instances voient le changement après `/lobby reload` ou un redémarrage.
- Protection des parties SheepWars contre le vidage d'un nœud. Risque accepté : `kubectl drain --force` tue les parties en cours sur ce nœud.

## Design

### Domain

- `LobbyInstance { id: ServerId, address, state: LobbyState, player_count, max_players }`.
- `LobbyState` : `Starting | Ready | Draining`. Le choix d'un lobby ne considère que `Ready`.
- `ServerKind` : `Lobby | SheepWars`, ajouté à `HeartbeatRequest` avec `#[serde(default = SheepWars)]` pour rester lisible par N-1.
- Use case orchestrateur `drain_lobby(id)` : passe l'instance en `Draining`, émet `ServerRemove`, émet un `TransferPlayer` par joueur vers l'instance `Ready` la moins chargée. Renvoie le nombre de joueurs restants.
- Use case `pick_lobby()` : instance `Ready` la moins chargée, `None` si aucune.
- File : chaque entrée porte `seq: u64` (ordre, attribué par Postgres) et `state: QueueEntryState` (`Waiting | Dispatched { server_id, transfer_id }`). Les entrées `Dispatched` comptent dans la file mais ne sont plus redistribuées. Un échec ou une expiration de transfert repasse l'entrée en `Waiting` avec le même `seq`.

### Ports

| Port | Owner | Dispatch |
|---|---|---|
| `LayoutRepository` (load, save) | `ferriscraft-lobby` | générique statique |
| `SessionRepository` (get, put, delete) | `ferriscraft-lobby` | générique statique |
| `QueueRepository` (load_all, insert, set_state, remove) | `ferriscraft-orchestrator` | générique statique |

`FleetEngine` reste pur et synchrone. La couche API écrit chaque mutation de la file en base avant de répondre (write-through), puis l'applique à l'engine. Au démarrage, l'engine est rechargé depuis `load_all`.

Deux adapters par port : Postgres (prod) et fichier (dev local, comportement actuel). Le choix se fait au démarrage, dans `main.rs`, via un enum de deux variantes. Pas de `dyn`.

### Adapters

- **Postgres** : `sqlx`, pool partagé. Tables `lobby_layout` (une ligne, JSON, `updated_at`, dernière écriture gagne) et `player_sessions` (clé `player_uuid` + `realm`).
- **Postgres lobby** : cluster CNPG `ferriscraft-lobby-db` (base `lobby`), déclaré dans l'app `ferriscraft-deps` du dépôt infra. Secret `ferriscraft-lobby-db-app`. Le lobby n'a jamais accès à la base de l'orchestrateur.
- **Postgres orchestrateur** : la base de l'orchestrateur (`ferriscraft-db`, déjà utilisée par la progression). Migrations dans `crates/ferriscraft-progression-pg/migrations`, une seule source pour cette base. Table `queue_entries` (`seq bigserial`, `player_uuid` unique, `name`, `joined_at timestamptz`, `state`, `server_id`, `transfer_id`). Adapter mémoire pour le dev local et les tests.
- **Orchestrateur indisponible** : le plugin lobby réessaie `queue/join` et `queue/leave` pendant 10 s avant d'afficher une erreur au joueur. Les heartbeats en échec sont ignorés, le suivant part 2 s plus tard. Le registre des serveurs se reconstruit avec les heartbeats.
- **Fichier** : code actuel de `LayoutStore` et `SessionStore`, déplacé derrière le port.
- **Migrations** : `sqlx migrate`, lancées par un Job Helm (hook ArgoCD `PreSync`). Le plugin ne migre jamais au démarrage.
- **Heartbeat lobby** : le plugin lobby envoie `HeartbeatRequest { kind: Lobby, server_id: $POD_NAME, address: $POD_IP:port }` toutes les 2 s, comme SheepWars.
- **Orchestrateur** : registre des instances de lobby, `POST /api/v1/servers/{id}/drain`, `pick_lobby` utilisé par `send-lobby`.
- **Velocity** (Java, minimal) : choix du serveur initial et du repli après kick parmi les serveurs `lobby-*` reçus par SSE, au lieu du nom fixe `lobby`. Le Service `*-lobby` sert de repli si le flux SSE est coupé.
- **Helm** :
  - `lobby.replicas: 2`, `maxUnavailable: 0`, `maxSurge: 1`, PDB `minAvailable: 1`.
  - Plugin lobby en sidecar natif (`initContainers` + `restartPolicy: Always`). Il démarre avant rspigot et s'arrête après lui, donc il reste actif pendant le drain.
  - `preStop` sur le conteneur rspigot : appelle `drain`, attend `player_count == 0`, plafonné à `lobby.drainTimeoutSecs` (défaut 60). `terminationGracePeriodSeconds` = drain + 15.
  - `POD_NAME` et `POD_IP` via Downward API.
  - Secret de connexion lobby (`lobby.storage.existingDatabaseSecret`), vide = fichiers (dev). Les clusters CNPG sont dans le dépôt infra.
  - Values prod : `global.imageTag` obligatoire (`required`), refus de `latest`.

### Errors

- `RepositoryError` (lobby) : `Unavailable`, `Corrupt`, `Conflict`. Si Postgres est injoignable au démarrage, le plugin réessaie avec backoff et ne démarre pas avec un layout vide.
- `DrainError` (orchestrateur) : `UnknownServer`, `NotALobby`, `NoOtherLobby`. Avec `NoOtherLobby`, le drain attend jusqu'au timeout puis laisse Kubernetes terminer.

### Signaux

Les deux `main.rs` écoutent SIGTERM. À réception : fin des écritures en cours, dernier heartbeat (`ending` pour SheepWars), sortie code 0.

## Acceptance (Gherkin)

```gherkin
@spec-plc-1  Scenario: le layout survit à un redémarrage
  Given un lobby branché sur Postgres
  When un admin fait /lobby setspawn puis le Pod redémarre
  Then le spawn est celui défini par l'admin

@spec-plc-2  Scenario: une session survit à un redémarrage
  Given un joueur connecté à FerrisKey via /login
  When le Pod lobby redémarre et le joueur revient
  Then /whoami affiche son compte sans nouveau /login

@spec-plc-3  Scenario: une mise à jour du lobby ne déconnecte personne
  Given 2 instances de lobby Ready et 10 joueurs répartis dessus
  When on déploie une nouvelle image du plugin lobby
  Then aucun joueur n'est déconnecté du proxy
  And tous les joueurs finissent sur une instance de la nouvelle version

@spec-plc-4  Scenario: aucun joueur n'arrive sur un lobby en drain
  Given une instance A en Draining et une instance B Ready
  When un joueur se connecte au proxy
  Then il arrive sur B

@spec-plc-5  Scenario: drain sans autre lobby
  Given une seule instance de lobby avec des joueurs
  When le Pod reçoit l'ordre d'arrêt
  Then le drain renvoie NoOtherLobby
  And le Pod s'arrête au plus tard après drainTimeoutSecs + 15 s

@spec-plc-6  Scenario: l'orchestrateur N lit un heartbeat N-1
  Given le JSON d'un HeartbeatRequest sans champ kind
  When l'orchestrateur le désérialise
  Then kind vaut SheepWars

@spec-plc-7  Scenario: le plugin N-1 tourne sur le schéma N
  Given la base migrée à la version N
  When le plugin lobby N-1 lit et écrit layout et sessions
  Then aucune requête n'échoue

@spec-plc-8  Scenario: le plugin s'arrête proprement sur SIGTERM
  Given un plugin lobby connecté à rspigot
  When il reçoit SIGTERM
  Then il sort avec le code 0 en moins de 5 s

@spec-plc-9  Scenario: le chart prod refuse latest
  Given les values prod avec global.imageTag absent ou égal à latest
  When on lance helm template
  Then le rendu échoue avec un message explicite

@spec-plc-10 Scenario: rspigot et plugins d'un même Pod partagent le tag
  Given les values prod avec global.imageTag = <sha>
  When on lance helm template
  Then les images rspigot et plugin du Pod lobby et du template SheepWars ont toutes le tag <sha>

@spec-plc-11 Scenario: la CI construit une version figée de rspigot
  Given Cargo.toml pointe le SDK sur un tag git rspigot
  When la CI construit les images
  Then le commit rspigot utilisé est celui du tag, lu dans Cargo.lock

@spec-plc-12 Scenario: aucun refresh_token dans les logs
  Given un /login suivi d'un rafraîchissement de token
  When on capture les logs du plugin lobby
  Then aucune ligne ne contient la valeur du refresh_token

@spec-plc-13 Scenario: la file survit à un redémarrage de l'orchestrateur
  Given 5 joueurs dans la file, dans l'ordre A, B, C, D, E
  When le Pod orchestrateur redémarre
  Then GET /api/v1/queue/player/{uuid} donne les positions 1 à 5 dans le même ordre

@spec-plc-14 Scenario: un transfert raté remet le joueur à sa place
  Given A en tête de file, dispatché vers sheepwars-x
  When le transfert est acquitté en échec ou expire
  Then A est de nouveau en tête de file, à l'état Waiting

@spec-plc-15 Scenario: un joueur dispatché pendant un redémarrage n'est pas perdu
  Given A dispatché vers sheepwars-x, transfert non acquitté
  When l'orchestrateur redémarre avant l'acquittement
  Then A reste dans la file jusqu'à ce que son UUID apparaisse dans un heartbeat de sheepwars-x
  And s'il n'y apparaît pas avant transfer_timeout, A repasse en Waiting à sa place

@spec-plc-16 Scenario: rejoindre la file pendant le redémarrage de l'orchestrateur
  Given l'orchestrateur indisponible pendant 5 s
  When un joueur clique sur le plot SheepWars
  Then le joueur est inscrit dans la file dès que l'orchestrateur répond, sans recliquer
```

## Simulation

Pas nécessaire pour le drain. Pour la file, `plc-14` et `plc-15` se testent sur `FleetEngine` avec une horloge injectée (déjà basé sur `Instant` passé en paramètre). Le drain est séquentiel (un Pod à la fois, garanti par `maxUnavailable: 0` et le PDB). `plc-3` et `plc-4` se testent avec un orchestrateur réel et un faux proxy SSE. `plc-3` est aussi vérifié une fois à la main sur k3d.

## Workstreams

| # | Nom | Mission | Dépend de | Fichiers |
|---|---|---|---|---|
| W1 | SDK figé | `rspigot.ref`, CI clone ce commit, `RSPIGOT_REF` dans les images (ADR 0010) | — | `rspigot.ref`, `.github/workflows/images.yml`, `docker-bake.hcl`, `Dockerfile` |
| W2 | Stockage lobby | Ports + adapters Postgres/fichier, migrations sqlx | W1 | `crates/ferriscraft-lobby/src/{config,auth,storage/*}.rs`, `crates/ferriscraft-lobby/migrations/` |
| W3 | Registre et drain | `ServerKind`, `LobbyState`, registre lobby, `/drain`, `pick_lobby`, heartbeat lobby | W1 | `crates/ferriscraft-protocol/src/lib.rs`, `crates/ferriscraft-orchestrator/src/{api,engine}.rs`, `crates/ferriscraft-lobby/src/orchestrator.rs` |
| W4 | Velocity | Choix du lobby initial et de repli parmi `lobby-*` | W3 | `proxy/velocity-plugin/**` |
| W5 | Signaux | SIGTERM dans les deux plugins | — | `crates/ferriscraft-{lobby,sheepwars}/src/main.rs` |
| W6 | Chart | CNPG (2 bases), Jobs de migration, sidecar natif, preStop, PDB, Downward API, garde-fou tag | W2, W3, W7 | `deploy/charts/ferriscraft/**` |
| W7 | File persistante | `QueueEntryState`, `seq`, `QueueRepository` Postgres/mémoire, write-through, rechargement au démarrage, remise en file sur échec, retry côté lobby | W1 | `crates/ferriscraft-orchestrator/src/{engine,api,storage/*}.rs`, `crates/ferriscraft-orchestrator/migrations/`, `crates/ferriscraft-lobby/src/orchestrator.rs` |

## Frozen contracts

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ServerKind {
    Lobby,
    #[default]
    SheepWars,
}

pub struct HeartbeatRequest {
    pub server_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    #[serde(default)]
    pub kind: ServerKind,
    pub phase: ServerPhase,
    pub player_count: u32,
    pub max_players: u32,
    #[serde(default)]
    pub player_uuids: Vec<Uuid>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LobbyState {
    Starting,
    Ready,
    Draining,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueueEntryState {
    Waiting,
    Dispatched { server_id: String, transfer_id: Uuid },
}
```

Ajouts (avec les restes des issues #17 et #18) :

- `ServerSummary` gagne `kind: ServerKind` et `draining: bool`. `FleetStatusSnapshot` gagne `lobbies: Vec<ServerSummary>` (les instances de lobby, `servers` reste SheepWars) et `proxy_version`.
- `ProxyServerEntry` et `ProxyEvent::ServerUpsert` gagnent `kind`. Velocity choisit un lobby parmi les serveurs `kind == lobby`, et se replie sur le serveur statique `lobby` (Service) s'il n'en connaît aucun.
- Velocity envoie sa version dans l'en-tête `x-ferriscraft-version` (`PROXY_VERSION_HEADER`) sur `GET /api/v1/proxy/events` et `GET /api/v1/proxy/state`.
- `PlayerQueueStatus.spectate_server` : rempli pendant 60 s après `POST /servers/{id}/spectate`. SheepWars le lit à l'arrivée du joueur et le met en spectateur au lieu de l'inscrire dans une équipe.
- Un serveur SheepWars en `waiting` ou `countdown` qui disparaît avec des joueurs : ces joueurs repassent en tête de file, dans leur ordre d'arrivée.

```http
POST /api/v1/servers/{server_id}/drain
→ 200 { "remaining_players": 3 }
→ 404 UnknownServer | 409 NotALobby | 409 NoOtherLobby
```

## Risks / open questions

Aucune question ouverte. Décisions du 2026-11-20 :

| Sujet | Décision |
|---|---|
| `refresh_token` en clair en base | Chiffrement disque CNPG suffisant pour le moment |
| File perdue au redémarrage de l'orchestrateur | Interdit : file persistée en Postgres ([ADR 0004](../adr/0004-file-sheepwars-dans-postgres.md)) |
| Deux admins modifient le layout | Dernière écriture gagne |
| Vidage d'un nœud avec des parties SheepWars | Risque accepté |
| Version de Kubernetes | >= 1.29 en prod |

## Verify (exit)

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
helm lint deploy/charts/ferriscraft
helm template ferriscraft deploy/charts/ferriscraft -f deploy/charts/ferriscraft/values-k3d.yaml | kubeconform -strict -ignore-missing-schemas
helm template ferriscraft deploy/charts/ferriscraft --set global.imageTag=latest   # doit échouer
```

Puis à la main sur k3d :

1. 2 lobbies, 2 clients connectés, `helm upgrade` avec un nouveau tag lobby : les 2 clients restent connectés (`plc-3`).
2. 2 clients dans la file, `kubectl rollout restart deploy/<release>-orchestrator` : les 2 clients gardent leur position (`plc-13`).

Amended: 2026-11-20 réponses aux questions ouvertes, ajout de la file persistante (invariants 9-10, plc-13 à plc-16, W7).
