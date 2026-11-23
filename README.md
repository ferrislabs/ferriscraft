# FerrisCraft — Réseau Mini-Jeux (`Velocity` + `rspigot`)

Workspace Rust des plugins **FerrisCraft** ciblant `rspigot` (`/opt/minecraft/dev/rspigot`) derrière un proxy **Velocity 4.3.1** (compatible **Minecraft 26.3 / protocole 777**).

## Architecture réseau locale

| Composant | Rôle | Port Minecraft | Port gRPC Plugin | Répertoire |
|---|---|---|---|---|
| **Velocity 4.3.1** | Proxy d'entrée (`online-mode = true`, forwarding HMAC-SHA256 `modern`) | **`0.0.0.0:25565`** | — | `./proxy/` |
| **`rspigot` Lobby** | Hub principal + plugin `ferriscraft-lobby` | `127.0.0.1:25566` | `127.0.0.1:25580` | `./server/` |
| **`rspigot` SheepWars** | Serveur mini-jeu SheepWars | `127.0.0.1:25567` | `127.0.0.1:25581` | `./server-sheepwars/` |
| **`rspigot` TowerWars** | Serveur mini-jeu TowerWars | `127.0.0.1:25568` | `127.0.0.1:25582` | `./server-towerwars/` |

- Les joueurs se connectent toujours sur **`localhost:25565`** (Velocity).
- Cliquer sur le plot **SheepWars** (ou dans la **Boussole** / `/sheepwars`) vérifie si `127.0.0.1:25567` est démarré :
  - **S'il tourne** : transfert automatique vers `sheepwars` via Velocity (`sheepwars.localhost:25565` → `forced-hosts`).
  - **S'il est éteint** : inscription propre dans la file d'attente locale du Lobby sans déconnecter le joueur.
- Les commandes proxy natives `/server sheepwars`, `/server towerwars` et `/server lobby` fonctionnent également en jeu.

---

## 1. Préparer Velocity et la map du Lobby (1ère fois uniquement)

`proxy/velocity.jar` et le dossier décompressé `server/world/` ne sont pas suivis par Git :

```bash
./proxy/download-velocity.sh
unzip -o server/world.zip -d server
```

*(Télécharge `velocity-4.3.1-45.jar`, vérifie son SHA-256, initialise `server/spigot.yml` et `server-sheepwars/spigot.yml` avec `proxy/forwarding.secret`, et extrait la map du Lobby.)*

---

## 2. Configurer l'environnement (`.env`)

```bash
cp .env.example .env
```

Variables principales :
- `RSPIGOT_URL=http://127.0.0.1:25580`
- `FERRISKEY_URL=http://localhost:3333`
- `FERRISKEY_REALM=capitainerie`
- `FERRISKEY_CLIENT_ID=minecraft` *(client autorisé par Capitainerie — activer **Device Code Grant** ; renseigner `FERRISKEY_CLIENT_SECRET` si confidentiel)*
- `FERRISKEY_SCOPE="openid profile email roles"`
- `CAPITAINERIE_URL=http://localhost:4100` / `CAPITAINERIE_RESOURCE=http://localhost:4100` / `CAPITAINERIE_CONSOLE_URL=http://localhost:4080`
- `SHEEPWARS_HOST=sheepwars.localhost` / `SHEEPWARS_PORT=25565` / `SHEEPWARS_BACKEND=127.0.0.1:25567`
- `LOBBY_SESSIONS_PATH=.sessions.json` *(peut être partagé avec le plugin SheepWars pour conserver la session FerrisKey d'un serveur à l'autre)*

---

## 3. Lancer l'infrastructure depuis `ferriscraft`

### Terminal 1 — Proxy Velocity (`:25565`)

```bash
./proxy/run.sh
```

### Terminal 2 — Serveur `rspigot` Lobby (`:25566`, gRPC `:25580`)

```bash
/opt/minecraft/dev/rspigot/target/release/rspigot --world-dir ./server
```

*(Taper `op <VotrePseudo>` dans la console `rspigot` pour obtenir les droits admin `/lobby` et `/build`.)*

### Terminal 3 — Plugin `ferriscraft-lobby`

```bash
cargo run --release -p ferriscraft-lobby
```

### Terminal 4 — Serveur `rspigot` SheepWars (`:25567`, gRPC `:25581`)

```bash
/opt/minecraft/dev/rspigot/target/release/rspigot --world-dir ./server-sheepwars
```

### Terminal 5 — Plugin `ferriscraft-sheepwars`

```bash
RSPIGOT_URL=http://127.0.0.1:25581 RSPIGOT_PLUGIN_NAME=ferriscraft-sheepwars cargo run --release -p ferriscraft-sheepwars
```

*(Optionnel : ajouter `ORCHESTRATOR_URL=http://127.0.0.1:8080` et `SERVER_ID=sheepwars-1` si l'orchestrateur local tourne ; sinon le plugin enchaîne les manches sur place. En jeu : `/sheepwars join|leave|hub` et admin `/sheepwars forcestart|stop`.)*

### Terminal 6 — Serveur `rspigot` TowerWars (`:25568`, gRPC `:25582`)

```bash
/opt/minecraft/dev/rspigot/target/release/rspigot --world-dir ./server-towerwars
```

### Terminal 7 — Plugin `ferriscraft-towerwars`

```bash
RSPIGOT_URL=http://127.0.0.1:25582 RSPIGOT_PLUGIN_NAME=ferriscraft-towerwars cargo run --release -p ferriscraft-towerwars
```

*(Optionnel : ajouter `ORCHESTRATOR_URL=http://127.0.0.1:8080` et `SERVER_ID=towerwars-1` si l'orchestrateur local tourne. En jeu : `/towerwars join|leave|hub` et admin `/towerwars forcestart|stop`, réservés à `op` ou à la permission `towerwars.admin`.)*

Variables du plugin : `TOWERWARS_MODE` (`solo` ou `teams`), `TOWERWARS_MIN_PLAYERS` (1), `TOWERWARS_MAX_PLAYERS` (6), `TOWERWARS_COUNTDOWN_SECS`, `TOWERWARS_EPHEMERAL`, `TOWERWARS_SEED`, `TOWERWARS_MAP_PATH`. Sans `TOWERWARS_MAP_PATH`, le plugin construit sa carte intégrée dans un monde vide au démarrage : aucun `world.zip` ni map S3 n'est nécessaire.

---

## 4. Importer / mettre à jour une map Vanilla 26.x

Si vous ré-exportez une map depuis Vanilla 26.x dans `server/world/` (couper `rspigot` avant) :

```bash
cp -R /chemin/vers/votre_map/* ./server/world/
cargo run --release -p ferriscraft-lobby --bin normalize-anvil -- ./server/world
```

*(Convertit les palettes Vanilla 26.x et force `persistent = "true"` sur toutes les feuilles d'arbres custom.)*

---

## Commandes en jeu

### Joueurs
- `/menu` — Ouvrir le menu de navigation des mini-jeux
- `/options` — Ouvrir les préférences personnelles & compte FerrisKey
- `/sheepwars` — Rejoindre le serveur ou la file **SheepWars**
- `/server sheepwars` / `/server lobby` — Basculer directement via Velocity
- `/spawn` — Retourner au spawn (`13.5, 29.0, -82.5` — vue Sud)
- `/login` / `/logout` / `/whoami` — Lier, délier ou inspecter son compte **FerrisKey** (Lobby)
- `/caplogin` / `/caplogout` — Lier ou délier son compte **Capitainerie** (si realm dédié)
- `/mp <joueur> <message>` et `/r <message>` — Messagerie privée

### Administration & Calibration (`op` ou permission `ferriscraft.admin`)
- `/build` — Activer/désactiver l'édition de blocs dans le lobby
- `/lobby setspawn` / `/lobby setpoi <nom>` / `/lobby setpedestal <1..7>`
- `/lobby setcaptain` / `/lobby setstation <index>` / `/lobby setwaypoint <pnj> <index>`
- `/lobby reload` / `/lobby info`

---

## 5. Déploiement Kubernetes (`k3s` / `k3d`) & gestion de flotte ($k$ serveurs SheepWars)

Composants de gestion de flotte :

| Composant | Rôle | Sources |
|---|---|---|
| **`ferriscraft-orchestrator`** | File d'attente FIFO, matchmaking par lots, scaling dynamique de `min` à `k` (`SHEEPWARS_MAX_REPLICAS`) Pods SheepWars via `kube-rs`, flux SSE proxy | [`crates/ferriscraft-orchestrator`](./crates/ferriscraft-orchestrator) |
| **`ferriscraft-velocity`** | Plugin Velocity 4.3.1 enregistrant dynamiquement les Pods `sheepwars-*` et transférant les joueurs sans déconnexion client | [`proxy/velocity-plugin`](./proxy/velocity-plugin) |
| **`ferriscraft-sheepwars`** | Plugin de mini-jeu SheepWars (arène, moutons spéciaux, HUD, heartbeat orchestrateur, retour au Hub) | [`crates/ferriscraft-sheepwars`](./crates/ferriscraft-sheepwars) |
| **`ferriscraft-protocol`** | Contrat JSON partagé (queue, heartbeat, phases, retours Hub) | [`crates/ferriscraft-protocol`](./crates/ferriscraft-protocol) |

### Construire les images Docker (avec contexte `rspigot` local)

```bash
RSPIGOT_DIR=/opt/minecraft/dev/rspigot IMAGE_TAG=dev ./deploy/scripts/build-images.sh
```

Pour importer directement dans un cluster `k3d` existant :

```bash
K3D_CLUSTER=capitainerie IMAGE_TAG=dev ./deploy/scripts/build-images.sh
```

### Déployer via Helm

```bash
helm upgrade --install ferriscraft ./deploy/charts/ferriscraft \
  --namespace ferriscraft --create-namespace \
  -f ./deploy/charts/ferriscraft/values-k3d.yaml
```

Pour ajuster le nombre max de serveurs SheepWars (`k`) et le seuil de lancement de partie :

```bash
helm upgrade --install ferriscraft ./deploy/charts/ferriscraft \
  --namespace ferriscraft --create-namespace \
  --set orchestrator.sheepwars.maxReplicas=10 \
  --set orchestrator.queue.minPlayers=8
```

### Maps SheepWars multi-arènes sur S3

En mode S3 (`s3.enabled=true`), l'orchestrateur découvre les maps complètes sous `s3://<bucket>/<prefix><nom>/{world.zip,arena.json}`, effectue une rotation à la création de chaque Pod SheepWars et injecte la map choisie via l'`initContainer` `map-sync` (`/usr/local/bin/ferriscraft-map-sync`). En dev local (`values-k3d.yaml`), un serveur **RustFS** compatible S3 est déployé dans le cluster.

Téléverser une nouvelle map (vers `k3d-ferriscraft` via port-forward automatique, ou vers un `S3_ENDPOINT` externe) :

```bash
./deploy/scripts/upload-map.sh steampunk server-sheepwars
# ou explicitement :
./deploy/scripts/upload-map.sh ma-map /chemin/world.zip /chemin/arena.json
```

### TowerWars sur Kubernetes

TowerWars est désactivé par défaut (`orchestrator.towerwars.enabled=false`) : le chart ne rend alors rien de plus qu'avant. `values-k3d.yaml` l'active pour les tests locaux. La taille de partie (1 à 4) est choisie au lobby : la file est groupée par taille et `queueMinPlayers` est ignoré pour TowerWars. Il a sa propre file, sa propre flotte (`TOWERWARS_*`) et son CRD `towerwarsservers.ferriscraft.io` (`kubectl get tws`, activé par `orchestrator.towerwars.crd.enabled`).

```bash
K3D_CLUSTER=capitainerie IMAGE_TAG=dev ./deploy/scripts/build-images.sh towerwars
helm upgrade --install ferriscraft ./deploy/charts/ferriscraft \
  --namespace ferriscraft --create-namespace \
  -f ./deploy/charts/ferriscraft/values-k3d.yaml \
  --set orchestrator.towerwars.maxReplicas=4
```

Voir [`docs/TOWERWARS_PLUGIN_CONTRACT.md`](./docs/TOWERWARS_PLUGIN_CONTRACT.md) pour le contrat du sidecar `ferriscraft-towerwars`, et [`docs/adr/0011-un-moteur-de-flotte-par-jeu.md`](./docs/adr/0011-un-moteur-de-flotte-par-jeu.md) pour l'architecture.

Voir [`docs/SHEEPWARS_PLUGIN_CONTRACT.md`](./docs/SHEEPWARS_PLUGIN_CONTRACT.md) pour les variables d'environnement et le contrat du sidecar `ferriscraft-sheepwars`.

