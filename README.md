# FerrisCraft — Réseau Mini-Jeux (`Velocity` + `rspigot`)

Workspace Rust des plugins **FerrisCraft** ciblant `rspigot` (`/opt/minecraft/dev/rspigot`) derrière un proxy **Velocity 4.3.1** (compatible **Minecraft 26.3 / protocole 777**).

## Architecture réseau locale

| Composant | Rôle | Port Minecraft | Port gRPC Plugin | Répertoire |
|---|---|---|---|---|
| **Velocity 4.3.1** | Proxy d'entrée (`online-mode = true`, forwarding HMAC-SHA256 `modern`) | **`0.0.0.0:25565`** | — | `./proxy/` |
| **`rspigot` Lobby** | Hub principal + plugin `ferriscraft-lobby` | `127.0.0.1:25566` | `127.0.0.1:25580` | `./server/` |
| **`rspigot` SheepWars** | Serveur mini-jeu SheepWars | `127.0.0.1:25567` | `127.0.0.1:25581` | `./server-sheepwars/` |

- Les joueurs se connectent toujours sur **`localhost:25565`** (Velocity).
- Cliquer sur le plot **SheepWars** (ou dans la **Boussole** / `/sheepwars`) vérifie si `127.0.0.1:25567` est démarré :
  - **S'il tourne** : transfert automatique vers `sheepwars` via Velocity (`sheepwars.localhost:25565` → `forced-hosts`).
  - **S'il est éteint** : inscription propre dans la file d'attente locale du Lobby sans déconnecter le joueur.
- Les commandes proxy natives `/server sheepwars` et `/server lobby` fonctionnent également en jeu.

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

### Terminal 4 *(optionnel)* — Serveur `rspigot` SheepWars (`:25567`, gRPC `:25581`)

```bash
/opt/minecraft/dev/rspigot/target/release/rspigot --world-dir ./server-sheepwars
```

*(Le plugin SheepWars se connectera ensuite avec `RSPIGOT_URL=http://127.0.0.1:25581`, et pourra renvoyer les joueurs au Hub en fin de partie via `player.connection.transfer("lobby.localhost", 25565).await`.)*

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
| **`ferriscraft-protocol`** | Contrat JSON partagé (queue, heartbeat, phases, retours Hub) pour le futur plugin SheepWars | [`crates/ferriscraft-protocol`](./crates/ferriscraft-protocol) |

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

Voir [`docs/SHEEPWARS_PLUGIN_CONTRACT.md`](./docs/SHEEPWARS_PLUGIN_CONTRACT.md) pour brancher le futur plugin SheepWars en sidecar.

