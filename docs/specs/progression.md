# Progression : XP, levels, coins et cosmétiques

Spec id : `prg`. Statut : proposée.

## Goal

Chaque partie comptée rapporte de l'XP et des coins. L'XP fait monter un level par mini-jeu et un level réseau. Les coins achètent des cosmétiques du lobby et des mini-jeux. Les grades FerrisKey donnent des avantages cosmétiques et un multiplicateur de coins.

## Context

- Aucune donnée joueur n'est persistée aujourd'hui. Postgres (CloudNativePG) est prévu par [ADR 0001](../adr/0001-etat-du-lobby-dans-postgres.md) et [ADR 0004](../adr/0004-file-sheepwars-dans-postgres.md), workstream `plc` W6, pas encore livré.
- SheepWars connaît l'issue d'une manche : `Phase::Ended { winner: Option<TeamId> }` et `RoundEvent::{Won, Draw, Eliminated}` (`crates/ferriscraft-sheepwars/src/domain/round.rs`). Il ne sait pas qui a tué qui.
- Le lobby résout un grade depuis les rôles FerrisKey du joueur connecté par `/login` : `Rank::{Admin, Mod, Dev, Vip, Member, Guest}` (`crates/ferriscraft-lobby/src/auth.rs:43`). Le grade sert aujourd'hui au tri de la tablist et au cooldown du chat.
- Le lobby propose 6 gadgets (`LobbyGadget`) et 4 trails (`AuraTrail`), tous gratuits (`crates/ferriscraft-lobby/src/gadgets.rs`).
- Les plugins parlent déjà à l'orchestrateur en HTTP avec un jeton Bearer.

Décisions de cadrage (2026-11-22) :

| Sujet | Décision |
|---|---|
| Identité de la progression | UUID Minecraft ([ADR 0006](../adr/0006-progression-par-uuid-minecraft.md)) |
| Monnaie | Coins réseau uniques ([ADR 0007](../adr/0007-coins-reseau-uniques.md)) |
| Source des grades | Rôles FerrisKey, attribués par le staff |
| Boutique v1 | Cosmétiques du lobby et de SheepWars |
| Qui écrit la progression | L'orchestrateur seul ([ADR 0005](../adr/0005-progression-dans-l-orchestrateur.md)) |

## Glossary

Voir `docs/glossary.md` : XP, level de jeu, level réseau, coins, partie comptée, rapport de partie, cosmétique, déblocage, grade.

## Invariants

1. Le solde de coins d'un joueur n'est jamais négatif. [type : `Coins(u64)`, `Coins::checked_sub` renvoie `Result` ; contrainte `CHECK` en base]
2. Un rapport de partie est crédité au plus une fois. [test : `match_id` clé primaire, rejeu idempotent]
3. Le grade ne change jamais l'XP, les levels ni les statistiques. Il agit seulement sur le multiplicateur de coins et l'accès aux cosmétiques. [type : `xp_reward(outcome)` ne prend pas de grade ; test]
4. Un cosmétique n'a aucun effet de gameplay dans un mini-jeu : ni dégâts, ni vitesse, ni vies, ni moutons. Règle de l'EULA Minecraft. [type : le catalogue ne contient que des effets visuels ; revue]
5. Le level réseau est calculé depuis la somme des XP de jeu, jamais stocké à part. [type : pas de colonne, fonction dérivée]
6. Une partie non comptée ne crédite rien, à personne. [test]
7. Seul l'orchestrateur écrit la progression. Les plugins lisent un profil et envoient des rapports. [test : seul le Pod orchestrateur reçoit le Secret Postgres de la base `progression`, rendu Helm]
8. Un serveur SheepWars ne passe en `ending` qu'après l'accusé de réception de son rapport, ou après épuisement du budget de renvoi (journalisé en `error`). [test]
9. Chaque mouvement de coins est une ligne de journal avec sa raison. Le solde est égal à la somme du journal. [test]
10. Un cosmétique ne s'équipe que s'il est débloqué pour ce joueur. [test]
11. La progression survit à un redémarrage de l'orchestrateur. [test]

## Out of scope

- Classements et leaderboards. Les données (XP, victoires) les permettent, aucune API ni affichage n'est fait ici.
- Vente de grades, de coins ou de cosmétiques contre de l'argent réel.
- Attribution des kills dans SheepWars, et toute statistique autre que parties jouées, victoires, défaites, égalités.
- Échange ou don de coins entre joueurs.
- Quêtes, succès, prestige, bonus quotidiens.
- Mini-jeux autres que SheepWars. `GameKind` est un enum prévu pour en accueillir d'autres.
- Interface web d'administration. Seule une commande admin de crédit de coins est prévue.
- Expiration des grades : gérée dans FerrisKey.

## Design

### Domain

Nouveau crate pur `ferriscraft-progression`, sans I/O :

- `Xp(u64)`, `Coins(u64)`, `Level(u32)` (newtypes).
- `LevelCurve` : XP nécessaire pour passer du level `L` à `L+1`. Deux courbes constantes : `GAME` et `NETWORK`.
- `MatchOutcome { Win, Loss, Draw }`.
- `count_match(report) -> Result<CountedMatch, NotCounted>` : applique les règles de partie comptée.
- `reward(outcome, rank) -> Reward { xp: Xp, coins: Coins }` : `xp` dépend de `outcome` seul, `coins` du résultat multiplié par le multiplicateur du grade.
- `Catalog` : prix et condition de déblocage de chaque `CosmeticId`.
- `Unlock { Free, Price(Coins), Rank(PlayerRank), NetworkLevel(Level) }`.
- `can_unlock(profile, cosmetic) -> Result<Unlock, UnlockError>`.

Valeurs par défaut, à ajuster après la bêta :

| Règle | Valeur |
|---|---|
| Partie comptée | pas de démarrage forcé, au moins 2 participants, durée au moins 60 s |
| Participant récompensé | a joué jusqu'à la fin de la manche ou jusqu'à son élimination |
| Victoire | 150 XP, 30 coins |
| Égalité | 75 XP, 15 coins |
| Défaite | 50 XP, 10 coins |
| Multiplicateur de coins | x1,5 (arrondi vers le bas) pour `Vip`, `Dev`, `Mod`, `Admin` ; x1 sinon |
| Courbe de jeu | `min(250 × L, 2 500)` XP pour passer de `L` à `L+1` (level 30 ≈ 63 750 XP, ≈ 425 victoires) |
| Courbe réseau | `min(1 000 × L, 10 000)` XP |

Catalogue v1 :

| Cosmétique | Où | Déblocage |
|---|---|---|
| Gadget Feather Dash | lobby | gratuit |
| Gadget Firework Rocket | lobby | gratuit |
| Gadget Wind Charge | lobby | 500 coins |
| Gadget Sheep Cannon | lobby | 1 000 coins |
| Gadget Vortex Wand | lobby | 1 500 coins |
| Gadget Ender Grapple | lobby | grade `Vip` |
| Trail Flame | lobby | 750 coins |
| Trail Heart | lobby | 750 coins |
| Trail Magic | lobby | level réseau 10 |
| Trail Emerald | lobby | grade `Vip` |
| Traînée de mouton Flamme | SheepWars | 500 coins |
| Traînée de mouton Cœurs | SheepWars | 500 coins |
| Traînée de mouton Notes | SheepWars | 800 coins |
| Traînée de mouton Arc-en-ciel | SheepWars | grade `Vip` |
| Effet de victoire Feux d'artifice | SheepWars | 1 000 coins |
| Effet de victoire Éclair | SheepWars | 1 500 coins |

Un cosmétique débloqué par grade est utilisable tant que le joueur a le grade. Un cosmétique acheté reste acquis.

Emplacements d'équipement : `LobbyGadget`, `LobbyTrail`, `SheepTrail`, `VictoryEffect`. Un cosmétique par emplacement.

### Ports

| Port | Propriétaire | Dispatch | Rôle |
|---|---|---|---|
| `ProgressionRepository` | orchestrateur | générique statique | enregistrer un rapport et ses crédits (atomique, idempotent), lire un profil, mettre à jour pseudo et grade, acheter, équiper, créditer (admin) |

Les plugins n'ont pas de nouveau port. Ils étendent leur client orchestrateur existant (`crates/ferriscraft-{lobby,sheepwars}/src/orchestrator.rs`).

### Adapters

- `ProgressionRepository` mémoire : tests et dev local sans Postgres.
- `ProgressionRepository` Postgres (`sqlx`) : base `progression` du cluster CNPG. Achat et crédit dans une transaction avec `SELECT … FOR UPDATE` sur la ligne du joueur.

Schéma (migrations expand puis contract, comme `plc`) :

| Table | Contenu |
|---|---|
| `players` | `uuid` PK, `name`, `rank`, `rank_seen_at` |
| `game_progress` | (`uuid`, `game`) PK, `xp`, `played`, `wins`, `losses`, `draws` |
| `coin_ledger` | `id`, `uuid`, `delta` (signé), `reason`, `match_id` ou `cosmetic` ou `admin`, `created_at` |
| `coin_balance` | `uuid` PK, `coins` avec `CHECK (coins >= 0)`, mis à jour dans la même transaction que le journal |
| `matches` | `match_id` PK, `game`, `server_id`, `reported_at` |
| `unlocks` | (`uuid`, `cosmetic`) PK, `source`, `unlocked_at` |
| `equipped` | (`uuid`, `slot`) PK, `cosmetic` |

### Flux

1. **Fin de manche SheepWars.** Le plugin construit un `MatchReport`, l'envoie sur `POST /api/v1/matches` et renvoie en cas d'échec (backoff 1 s à 10 s, 2 minutes au total). Il passe en `ending` après l'accusé de réception ou à la fin du budget.
2. **Crédit.** L'orchestrateur ignore tout participant jamais vu dans un heartbeat de ce serveur, vérifie que la partie est comptée, calcule les récompenses avec le grade en cache de chaque joueur, puis enregistre le tout dans une seule transaction. Un `match_id` déjà connu renvoie l'accusé d'origine avec `duplicate: true`.
3. **Annonce.** L'accusé contient les récompenses par joueur. SheepWars les affiche (« +150 XP, +30 coins », « Level 12 SheepWars ! ») avant le retour au hub.
4. **Grade.** À chaque arrivée d'un joueur au lobby, et à chaque `/login` réussi, le lobby envoie `POST /api/v1/players/{uuid}/identity` avec son pseudo et son grade.
5. **Profil.** Le lobby et SheepWars lisent `GET /api/v1/players/{uuid}/profile` à l'arrivée du joueur et après chaque achat ou équipement.
6. **Boutique.** Le menu du lobby appelle `POST /api/v1/players/{uuid}/purchases`, puis `PUT /api/v1/players/{uuid}/equipped`.

### Affichage

- Lobby : level réseau devant le pseudo dans la tablist et le chat (`[12✫] Pseudo`), coins sur le scoreboard, menu « Profil » (level réseau, level et statistiques par jeu), menu « Boutique » par emplacement.
- SheepWars : level SheepWars devant le pseudo dans la tablist, traînée de mouton et effet de victoire équipés, récompenses en fin de manche.
- Commande admin `/coins give <joueur> <montant>` (grade `Admin`), journalisée avec la raison `admin:<pseudo de l'admin>`.

### Errors

| Frontière | Enum | Variantes |
|---|---|---|
| Domaine | `NotCounted` | `ForcedStart`, `TooFewPlayers`, `TooShort` |
| Domaine | `UnlockError` | `AlreadyUnlocked`, `InsufficientCoins { missing: Coins }`, `RankRequired(PlayerRank)`, `LevelRequired(Level)`, `UnknownCosmetic` |
| Domaine | `EquipError` | `NotUnlocked`, `WrongSlot` |
| Repository | `ProgressionStoreError` | `Unavailable`, `Conflict`, `Corrupted(String)` |
| HTTP | voir Frozen contracts | |

Si l'orchestrateur est injoignable, le lobby affiche « Boutique indisponible » et les cosmétiques gratuits. Il ne plante pas.

## Acceptance (Gherkin)

```gherkin
@spec-prg-1  Scenario: une victoire crédite XP et coins
  Given une manche SheepWars comptée entre A (rouge) et B (bleu)
  When l'équipe rouge gagne
  Then A gagne 150 XP SheepWars et 30 coins
  And B gagne 50 XP SheepWars et 10 coins
  And l'XP réseau de A et de B augmente du même montant que leur XP SheepWars

@spec-prg-2  Scenario: un rapport rejoué ne crédite pas deux fois
  Given un rapport déjà crédité
  When SheepWars renvoie le même match_id
  Then la réponse est 200 avec duplicate=true
  And aucun solde ni aucune XP ne change

@spec-prg-3  Scenario: un démarrage forcé ne rapporte rien
  Given une manche lancée par la commande de démarrage forcé
  When elle se termine
  Then aucun participant ne gagne d'XP ni de coins

@spec-prg-4  Scenario: une manche à un seul joueur ou trop courte ne rapporte rien
  Given une manche avec 1 participant, ou de moins de 60 secondes
  When elle se termine
  Then aucun participant ne gagne d'XP ni de coins

@spec-prg-5  Scenario: un joueur parti avant la fin ne gagne rien
  Given une manche comptée où C se déconnecte avant d'être éliminé
  When la manche se termine
  Then C ne gagne rien et sa partie n'est pas comptée dans ses statistiques

@spec-prg-6  Scenario: passage de level
  Given A à 240 XP SheepWars (level 1, 250 XP pour le level 2)
  When A perd une manche comptée
  Then A est level 2 SheepWars
  And SheepWars lui affiche son nouveau level avant le retour au hub

@spec-prg-7  Scenario: achat réussi
  Given A a 600 coins
  When A achète la traînée de mouton Flamme (500 coins)
  Then A a 100 coins et la traînée est débloquée
  And le journal contient une ligne -500 de raison cosmetic

@spec-prg-8  Scenario: achat refusé faute de coins
  Given A a 400 coins
  When A achète la traînée de mouton Flamme
  Then la réponse est 402 avec missing=100
  And le solde de A reste 400

@spec-prg-9  Scenario: deux achats simultanés ne rendent jamais le solde négatif
  Given A a 600 coins
  When A achète en même temps deux cosmétiques à 500 coins
  Then un seul achat réussit
  And le solde de A est 100

@spec-prg-10  Scenario: le multiplicateur VIP agit sur les coins, pas sur l'XP
  Given A a le grade Vip en cache, B le grade Member
  When A et B gagnent la même manche comptée
  Then A gagne 150 XP et 45 coins
  And B gagne 150 XP et 30 coins

@spec-prg-11  Scenario: cosmétique réservé à un grade
  Given A est Vip et B est Member
  When ils équipent le trail Emerald
  Then A l'équipe
  And B reçoit 403 RankRequired(Vip)

@spec-prg-12  Scenario: cosmétique équipé visible en jeu
  Given A a équipé la traînée de mouton Cœurs
  When A lance un mouton dans SheepWars
  Then le mouton laisse des particules de cœurs

@spec-prg-13  Scenario: orchestrateur absent en fin de manche
  Given l'orchestrateur est arrêté
  When une manche comptée se termine
  And l'orchestrateur redémarre dans les 2 minutes
  Then le rapport est crédité une fois
  And le serveur SheepWars ne passe en ending qu'après l'accusé de réception

@spec-prg-14  Scenario: level affiché au lobby
  Given A est level réseau 12
  When A rejoint le lobby
  Then la tablist et ses messages de chat affichent [12✫] devant son pseudo

@spec-prg-15  Scenario: la progression survit à un redémarrage
  Given A a 600 coins et 3 000 XP SheepWars
  When l'orchestrateur redémarre
  Then le profil de A est identique

@spec-prg-16  Scenario: boutique sans orchestrateur
  Given l'orchestrateur est injoignable
  When A ouvre la boutique du lobby
  Then le menu affiche « Boutique indisponible »
  And A peut utiliser les gadgets gratuits

@spec-prg-17  Scenario: crédit admin journalisé
  Given un admin X
  When X fait /coins give A 200
  Then A gagne 200 coins
  And le journal contient une ligne +200 de raison admin:X
```

Les scénarios `prg-1` à `prg-11`, `prg-15` et `prg-17` sont testés contre l'API de l'orchestrateur avec l'adapter mémoire, puis `prg-9` et `prg-15` contre Postgres. `prg-12`, `prg-14` et `prg-16` sont vérifiés à la main sur k3d.

## Simulation

Non. La seule concurrence (achats et crédits simultanés) est confinée dans des transactions Postgres. `prg-9` la couvre avec un test d'intégration qui lance les deux achats en parallèle. Les règles de récompense et la courbe de levels ont des tests de propriété : XP croissante, level monotone, `NETWORK` dérivé de la somme.

## Workstreams

| # | Nom | Mission | Dépend de | Fichiers |
|---|---|---|---|---|
| W0 | Postgres | Cluster CNPG et Job de migration dans le chart, base `progression` | `plc` W6 (même cluster) | `deploy/charts/ferriscraft/**` |
| W1 | Domaine | Crate `ferriscraft-progression` : newtypes, courbes, règles, catalogue, tests de propriété | — | `crates/ferriscraft-progression/**`, `Cargo.toml` |
| W2 | Contrats | DTO de progression dans `ferriscraft-protocol` | W1 | `crates/ferriscraft-protocol/src/lib.rs` |
| W3 | Orchestrateur | `ProgressionRepository` mémoire et Postgres, migrations, routes HTTP | W1, W2, W0 pour Postgres | `crates/ferriscraft-orchestrator/src/{api,progression/*}.rs`, `crates/ferriscraft-orchestrator/migrations/` |
| W4 | SheepWars | Rapport de fin de manche avec renvoi, `ending` après accusé, affichage des récompenses, traînée et effet de victoire, level dans la tablist | W2 | `crates/ferriscraft-sheepwars/src/{orchestrator,plugin,game/*,hud/*}.rs` |
| W5 | Lobby | Envoi pseudo et grade, profil en cache, level dans tablist et chat, coins sur le scoreboard, menus Profil et Boutique, verrouillage gadgets et trails, `/coins give` | W2 | `crates/ferriscraft-lobby/src/{orchestrator,plugin,menus,gadgets,hud,chat}.rs` |

W1 et W2 d'abord. W3, W4 et W5 en parallèle ensuite, contre l'adapter mémoire. W0 avant le déploiement en prod.

## Frozen contracts

```rust
// crates/ferriscraft-protocol/src/lib.rs

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GameKind {
    SheepWars,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchOutcome {
    Win,
    Loss,
    Draw,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlayerRank {
    Admin,
    Mod,
    Dev,
    Vip,
    Member,
    Guest,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchParticipant {
    pub uuid: Uuid,
    pub name: String,
    pub outcome: MatchOutcome,
    /// A joué jusqu'à la fin de la manche ou jusqu'à son élimination.
    pub completed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchReport {
    pub match_id: Uuid,
    pub game: GameKind,
    pub server_id: String,
    pub forced_start: bool,
    pub duration_secs: u32,
    pub participants: Vec<MatchParticipant>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LevelProgress {
    pub level: u32,
    pub xp_into_level: u64,
    pub xp_for_next: u64,
    pub total_xp: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerReward {
    pub uuid: Uuid,
    pub xp: u64,
    pub coins: u64,
    pub game: LevelProgress,
    pub game_level_up: bool,
    pub network: LevelProgress,
    pub network_level_up: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchReportAck {
    pub match_id: Uuid,
    pub counted: bool,
    pub duplicate: bool,
    pub rewards: Vec<PlayerReward>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CosmeticSlot {
    LobbyGadget,
    LobbyTrail,
    SheepTrail,
    VictoryEffect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CosmeticId {
    GadgetFeatherDash,
    GadgetFireworkRocket,
    GadgetWindCharge,
    GadgetSheepCannon,
    GadgetVortexWand,
    GadgetEnderGrapple,
    TrailFlame,
    TrailHeart,
    TrailMagic,
    TrailEmerald,
    SheepTrailFlame,
    SheepTrailHearts,
    SheepTrailNotes,
    SheepTrailRainbow,
    VictoryFireworks,
    VictoryLightning,
    /// Cosmétique ajouté par une version plus récente : ignoré.
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameStats {
    pub game: GameKind,
    pub progress: LevelProgress,
    pub played: u32,
    pub wins: u32,
    pub losses: u32,
    pub draws: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerProfile {
    pub uuid: Uuid,
    pub name: String,
    pub rank: PlayerRank,
    pub coins: u64,
    pub network: LevelProgress,
    pub games: Vec<GameStats>,
    pub unlocked: Vec<CosmeticId>,
    pub equipped: BTreeMap<CosmeticSlot, CosmeticId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerIdentityUpdate {
    pub name: String,
    pub rank: PlayerRank,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PurchaseRequest {
    pub cosmetic: CosmeticId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EquipRequest {
    pub slot: CosmeticSlot,
    pub cosmetic: Option<CosmeticId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminCoinGrant {
    pub amount: u64,
    pub admin: String,
}
```

Routes de l'orchestrateur, toutes derrière le jeton Bearer existant :

| Route | Corps | Réponses |
|---|---|---|
| `POST /api/v1/matches` | `MatchReport` | `200 MatchReportAck` · `422 EmptyMatch` |
| `GET /api/v1/players/{uuid}/profile` | — | `200 PlayerProfile` (profil vide pour un joueur inconnu) |
| `POST /api/v1/players/{uuid}/identity` | `PlayerIdentityUpdate` | `204` |
| `POST /api/v1/players/{uuid}/purchases` | `PurchaseRequest` | `200 PlayerProfile` · `402 InsufficientCoins { missing }` · `403 RankRequired \| LevelRequired` · `409 AlreadyUnlocked` · `422 UnknownCosmetic` |
| `PUT /api/v1/players/{uuid}/equipped` | `EquipRequest` | `200 PlayerProfile` · `403 NotUnlocked` · `422 WrongSlot` |
| `POST /api/v1/players/{uuid}/coins` | `AdminCoinGrant` | `200 PlayerProfile` |

## Risks / open questions

| Risque | Décision ou responsable |
|---|---|
| Postgres pas encore dans le chart | W0 dépend de `plc` W6. Sans lui, la progression tourne sur l'adapter mémoire et se perd au redémarrage : interdit en prod. |
| Grade en cache périmé : un VIP retiré garde le x1,5 jusqu'à son prochain passage au lobby | Accepté. Le cache est rafraîchi à chaque arrivée au lobby. |
| Rapport perdu si le Pod SheepWars est tué pendant le renvoi (vidage de nœud) | Accepté, comme dans `plc`. |
| Valeurs d'XP, de coins et de prix | À ajuster après la bêta. Ce sont des constantes du crate `ferriscraft-progression`. |
| Gadgets aujourd'hui gratuits qui deviennent payants | Accepté : le serveur n'est pas encore ouvert au public. |
| Un plugin compromis peut forger un rapport | Accepté : les plugins sont de confiance (jeton Bearer). L'orchestrateur refuse un participant jamais vu dans un heartbeat de ce serveur. |

## Verify (exit)

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test -p ferriscraft-orchestrator --features postgres-tests   # prg-9, prg-15 contre Postgres
helm lint deploy/charts/ferriscraft
```

Puis à la main sur k3d, avec 2 clients :

1. Une manche comptée : récompenses affichées en fin de manche, coins et level visibles au lobby (`prg-1`, `prg-14`).
2. Achat puis équipement de la traînée Cœurs, mouton lancé en jeu (`prg-7`, `prg-12`).
3. `kubectl rollout restart deploy/<release>-orchestrator` pendant une manche : le rapport est crédité après le redémarrage (`prg-13`).
