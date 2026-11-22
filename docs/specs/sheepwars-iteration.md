# Itération SheepWars : moutons, dégâts, terrain

Statut : proposé
Date : 2026-11-22
Id : `swi`

## Goal

Rééquilibrer SheepWars : nouveaux moutons (explosif, torpilleur, chercheur), dégâts maîtrisés sans one-shot, terrain destructible et irrécupérable, don de moutons plus rapide et aléatoire.

## Context

État constaté dans `crates/ferriscraft-sheepwars` :

- Onze `SheepKind`. L'actuel chercheur (`Seeker`) vole en visant l'ennemi, la Fragmentation (`Splitting`, gris) crée des `Seeker` à l'explosion.
- Toute explosion (`Action::Explode`) inflige les dégâts vanilla à tous les joueurs proches : à bout portant, puissance 3,5 = environ 25 PV, donc un one-shot sur 20 PV.
- `on_block_break` annule toute casse : un joueur ne peut rien casser à la main, ni éteindre le feu.
- Les explosions avec `breaks_blocks = true` laissent des objets ramassables.
- Le Tremblement (`Quake`) est instantané. La Distorsion (`Warp`) échange les positions des joueurs. L'Intergalactique lance 8 explosions autour de l'impact.
- Le don se fait toutes les 20 s, en rotation fixe sur `SheepKind::ALL`, même mouton pour toute l'équipe.

## Glossary

| Terme | Définition | Synonyme rejeté |
|---|---|---|
| torpilleur | `Torpedo`. Ancien chercheur : vole en ligne droite, détruit les blocs dans une sphère de 3 et explose dès qu'un joueur y entre. | chercheur (ancien) |
| chercheur | `Seeker` (nouveau). Après l'impact, court vers l'ennemi le plus proche et explose en 2 s. | mouton à tête chercheuse |
| explosif | `Explosive` (rouge). Une explosion simple, forte sur le terrain. | TNT |
| dégâts maîtrisés | Dégâts infligés par `Action::Damage`. Les dégâts d'explosion vanilla sont annulés pour les joueurs. | dégâts vanilla |
| secousse | Un saut forcé de 3 blocs infligé par le champ de tremblement. | pulse |

## Invariants

1. Aucun mouton ne tue un joueur à pleine vie, sauf l'effet de l'Intergalactique. [type : `Damage::capped` plafonne à `MAX_NON_LETHAL`, `Damage::lethal()` est le seul chemin vers l'infini]
2. Les joueurs ne subissent jamais de dégâts d'explosion vanilla dans l'arène : seul `Action::Damage` les blesse. [test]
3. Un bloc détruit par un mouton, une explosion ou un joueur ne laisse aucun objet. [test]
4. Les dégâts maîtrisés ne touchent que l'équipe ennemie du lanceur, comme `Effect`, `Ignite` et `Push`. [test]
5. Le tirage de mouton est pur : `weighted_kind(roll)` ne dépend que de `roll`. [type : fonction pure, sans état]

## Out of scope

- Équilibrage final (valeurs de `catalog.rs`) : réglé en jeu après intégration.
- Orchestrateur, lobby, protocole.
- Poser des blocs (`on_block_place` reste annulé).

## Design

Domain : `domain::damage::Damage` (newtype). `domain::SheepKind` passe à 13 variantes.

Moutons (laine) :

| Kind | Nom | Laine | Comportement |
|---|---|---|---|
| Explosive | Mouton explosif | rouge | Explosion simple, puissance 7, dégâts forts, casse beaucoup de terrain. |
| Torpedo | Mouton torpilleur | cyan | Vol droit sans gravité. Détruit les blocs dans une sphère de 3 à chaque tick, après un délai de 0,10 s (2 ticks). Détonne quand un joueur ennemi entre dans les 3 blocs : dégâts forts rayon 5. |
| Seeker | Mouton chercheur | vert | Lancé en arc. Après impact, court vers l'ennemi le plus proche (vitesse de course), explose au contact ou à 2 s. |
| Splitting | Mouton à fragmentation | gris | Petite explosion, puis 4 enfants `Fragment` qui explosent à leur tour. |
| Frozen | Mouton glacé | bleu clair | Zone de gel de rayon 5 pendant 8 s : les ennemis dedans sont ralentis (Slowness II, rafraîchi toutes les 10 ticks), avec particules de neige. La glace se pose du centre vers l'extérieur, sur deux couches. |
| Fragment | Fragment | gris | Interne, hors tirage et hors inventaire. Moitié de taille, explose dès son premier contact avec le sol, sans mèche. |
| Incendiary | Mouton enflammé | orange | Dégâts directs réduits. Enflamme le terrain sur un large rayon. |
| Intergalactic | Mouton intergalactique | bleu | Météore (particules) du ciel vers l'impact. Détruit tous les blocs au-dessus de l'impact dans le cylindre (rayon 8). Tue les ennemis dans la zone. |
| Boarding | Mouton d'abordage | blanc | Inchangé, couleur blanche. |
| Warp | Mouton à distorsion | violet | Délai 2 s, puis 12 paires de blocs aléatoires (rayon 4) échangent leur place, avec blocs en chute entre les deux. Remplace l'échange de joueurs. |
| Quake | Mouton tremblement | marron | Champ de 10 s : toutes les 2 s, les ennemis du rayon 7 sont lancés à 3 blocs, avec particules de tremblement. |

Ports et dispatch : aucun nouveau port. Le tirage prend un `roll: u32` (source : `Uuid::new_v4()`, déjà en dépendance), donc pas de nouvelle crate. Enum dispatch partout, pas de `dyn`.

Dégâts : `explosion.create` reste pour les blocs, les particules et l'effet visuel. `on_entity_damage` annule les causes `BlockExplosion` et `EntityExplosion` pour les joueurs. `Action::Damage` applique `damage * (1 - distance / radius)` aux joueurs ennemis du rayon.

Irrécupérable : `EntityExplode` et `BlockExplode` passent `set_yield(0.0)`, `ItemSpawn` dans l'arène est annulé, `BlockBreak` passe `set_drop_items(false)`.

Errors : aucune nouvelle erreur. Les échecs d'API rspigot sont journalisés (`tracing::warn!`), comme dans `apply.rs`.

## Acceptance

Chaque ligne devient un test dans le module indiqué.

- `@spec-swi-1` Un `Damage::capped(99.0)` vaut `MAX_NON_LETHAL` (18,0) et reste strictement inférieur à 20. `domain::damage`
- `@spec-swi-2` Les dégâts de chaque mouton non intergalactique, rapportés à 20 PV, laissent le joueur en vie. `sheep::impact`
- `@spec-swi-3` L'intergalactique inflige `Damage::lethal()` dans un rayon de 8, et `ClearAbove` ne touche que les blocs au-dessus de l'impact. `sheep::impact`
- `@spec-swi-4` Les dégâts d'explosion vanilla sur un joueur sont annulés, les autres causes passent. `plugin`
- `@spec-swi-5` Le torpilleur n'est soumis à aucune gravité et se déplace en ligne droite, à vitesse constante. `game::flight`
- `@spec-swi-6` Le torpilleur détonne dès qu'un joueur ennemi est à moins de 3 blocs, jamais pour un allié. `game::flight`
- `@spec-swi-7` Le torpilleur émet un `ClearSphere` de rayon 3 centré sur lui à chaque tick de vol. `game::flight`
- `@spec-swi-8` Le chercheur, après impact, se dirige vers l'ennemi le plus proche et explose au plus tard 40 ticks après l'impact. `game::flight`
- `@spec-swi-9` Les enfants de la fragmentation sont des `Fragment`, au nombre de `SPLIT_COUNT`. `sheep::impact`
- `@spec-swi-10` L'explosif a une puissance et un rayon de destruction supérieurs à ceux de l'enflammé. `sheep::catalog`
- `@spec-swi-11` L'enflammé inflige moins de dégâts que l'explosif et pose plus de feu que l'ancien. `sheep::impact`
- `@spec-swi-12` Le champ de tremblement émet une secousse à 40, 80, 120, 160 et 200 ticks, puis s'éteint. `game::effects`
- `@spec-swi-13` La distorsion n'agit qu'après 40 ticks, puis émet 12 `SwapBlocks` dont les deux cellules sont distinctes et à moins de 4 blocs du centre. `game::effects`
- `@spec-swi-14` Le météore émet des particules sur une trajectoire descendante, puis l'impact au dernier tick. `game::effects`
- `@spec-swi-15` Les couleurs de laine sont distinctes pour les 13 moutons. L'abordage est blanc et la distorsion violette. `sheep::catalog`
- `@spec-swi-16` `weighted_kind` couvre les 13 moutons, la fréquence suit les poids, `Seeker` pèse 10 et `Explosive` 20. `sheep::catalog`
- `@spec-swi-17` Le don se déclenche toutes les 12 s par défaut et tire un mouton par joueur. `game::session`
- `@spec-swi-18` Un joueur casse un bloc à la main sans objet. Bedrock et plateformes restent incassables. `plugin`
- `@spec-swi-19` Casser un bloc de feu à la main l'éteint. `plugin`
- `@spec-swi-21` Un `Fragment` n'a pas de mèche (`arms_on_impact` faux) et explose à son premier contact avec le sol, après un délai de grâce plus long que celui des autres moutons. `game::flight`
- `@spec-swi-22` Un `Fragment` explose avec dégâts non létaux et sans enfants, et sa taille est inférieure à 1. `sheep::impact`
- `@spec-swi-23` Après un abordage, le joueur ne subit aucun dégât de chute tant qu'il est en l'air, quelle que soit la durée de la chute. `game::fall_guard`
- `@spec-swi-24` La protection dure 1 s (20 ticks) après son premier contact avec le sol, puis cesse. `game::fall_guard`
- `@spec-swi-25` Seuls les spawns du plugin et des commandes sont autorisés : les créatures naturelles, de générateur, de patrouille ou de raid sont annulées. `plugin`
- `@spec-swi-26` Le torpilleur ne détruit aucun bloc avant 2 ticks de vol. `game::flight`
- `@spec-swi-27` La zone de gel couvre un rayon de 5, ralentit les ennemis toutes les 10 ticks pendant 160 ticks, et pose la glace du centre vers l'extérieur sur plus de blocs que l'ancien disque de rayon 3. `game::effects`
- `@spec-swi-28` Le passager d'un abordage n'est téléporté en vol que s'il est à plus de 6 blocs du mouton. `game::flight`
- `@spec-swi-29` Le chercheur garde son IA active après l'impact, car rspigot ignore la vélocité et la gravité d'un mob sans IA. Les autres moutons armés la désactivent. `game::flight`
- `@spec-swi-20` Les explosions n'ont aucun rendement d'objets : `set_yield(0.0)` sur `EntityExplode` et `BlockExplode`. `plugin`

## Simulation

Non. Aucun invariant ne dépend de la concurrence ni du temps réel : les machines à états de `game::effects` sont pures et testées tick par tick.

## Workstreams

Quatre workstreams indépendants, dispatchés ensemble une fois les contrats gelés. Le dépôt n'est pas un workspace partagé en écriture : chaque workstream écrit uniquement ses fichiers.

| Id | Mission | Fichiers possédés |
|---|---|---|
| A | Vol : torpilleur en ligne droite, chercheur qui court, câblage de `Effects` dans `engine.rs` (`start` à l'impact des Quake, Warp et Intergalactic, `tick` à chaque tick, `apply` des actions rendues) | `src/game/engine.rs`, `src/game/flight.rs` |
| B | Impacts : `plan()` des 13 moutons, application de `Damage`, `ClearSphere`, `ClearAbove`, `SwapBlocks` | `src/sheep/impact.rs` (hors enum `Action`), `src/sheep/apply.rs` |
| C | Effets temporisés : champ de tremblement, délai de distorsion, météore. Machines à états pures | `src/game/effects.rs` (nouveau) |
| D | Règles : casse à la main, feu, sans objets, dégâts vanilla annulés, don aléatoire toutes les 12 s | `src/plugin.rs`, `src/game/session.rs`, `src/config.rs`, `src/domain/rules.rs` |

Possédés par l'orchestrateur (convergence, aucun sous-agent n'y écrit : il rapporte la ligne exacte) : `src/domain/sheep.rs`, `src/domain/damage.rs`, `src/domain/mod.rs`, `src/sheep/catalog.rs`, `src/sheep/mod.rs`, `src/game/mod.rs`, et l'enum `Action` dans `impact.rs`.

Régénérés à l'intégration : `Cargo.lock` (`cargo build -p ferriscraft-sheepwars`).

Environnement par workstream : chacun construit avec son `CARGO_TARGET_DIR` (`/private/tmp/claude-501/target-<id>`), car cargo verrouille le dossier cible.

Dépendances : A, B, C, D sont indépendants grâce aux contrats gelés (A appelle `plan()` et `apply()` sans connaître leur corps, C produit des `Action`).

## Frozen contracts

`src/domain/damage.rs` :

```rust
pub const MAX_NON_LETHAL: f32 = 18.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Damage(f32);

impl Damage {
    pub fn capped(points: f32) -> Self;
    pub const fn lethal() -> Self;
    pub const fn is_lethal(self) -> bool;
    pub fn at_distance(self, distance: f64, radius: f64) -> f32;
}
```

`src/domain/sheep.rs` : `SheepKind` = `Lightning, Incendiary, Boarding, Healing, Seeker, Torpedo, Explosive, Splitting, Darkness, Frozen, Quake, Warp, Intergalactic` (`ALL` de longueur 13).

`src/sheep/impact.rs`, variantes ajoutées à `Action` :

```rust
Damage { team: TeamId, center: Vec3, radius: f64, damage: Damage },
ClearSphere { center: Vec3, radius: f64 },
ClearAbove { center: Vec3, radius: f64, height: f64 },
SwapBlocks { a: Cell, b: Cell },
```

`src/sheep/catalog.rs` : constantes de chaque mouton, `drop_weight(kind) -> u32`, `weighted_kind(roll: u32) -> SheepKind`, `flies_straight(kind) -> bool` (Torpedo), `chases_after_impact(kind) -> bool` (Seeker), `display_name`, `wool_color`.

`src/game/effects.rs` (contrat côté A) :

```rust
pub struct Effects;
impl Effects {
    pub fn new() -> Self;
    pub fn start(&mut self, kind: SheepKind, at: Vec3, team: TeamId);
    pub fn tick(&mut self) -> Vec<Action>;
    pub fn is_idle(&self) -> bool;
}
```

Partage `plan()` / `Effects` : pour `Quake`, `Warp` et `Intergalactic`, `plan()` ne rend que le retour sensoriel du lancer (son, particules d'impact). Le comportement temporisé est dans `Effects`, démarré par le moteur à l'impact. À la fin du délai, `Effects::tick` rend les `Action` réelles (`SwapBlocks`, `Velocity`, `Particle`, `Explode` visuel avec `breaks_blocks = false`, `ClearAbove`, `Damage`).

## Risks / open questions

| Risque | Propriétaire |
|---|---|
| `ClearAbove` rayon 8 sur 48 de haut = environ 9 600 blocs : un appel par bloc est trop lent. Chercher une API de remplissage ou étaler sur plusieurs ticks. | B |
| API de dégâts joueur : `player.health.set` ne déclenche peut-être pas le flux de mort. Utiliser l'API qui passe par `EntityDamage` avec une cause `Custom`, non annulée par le filtre d'explosion. | B |
| Mode de jeu des joueurs : si `Adventure`, la casse n'émet pas `BlockBreak`. Passer en `Survival` sans casser la pose de blocs. | D |
| Blocs en chute de la distorsion : ils peuvent se poser ailleurs qu'en position 2. Le résultat final doit être posé par `set_block`, la chute est visuelle. | B |
| Fragmentation : 4 enfants `Explosive` sur un même joueur peuvent le tuer ensemble. Réglable par `SPLIT_COUNT` en jeu. | orchestrateur |
| L'échange de joueurs de la distorsion est retiré. À rétablir si voulu. | orchestrateur |

## Verify (exit)

```
cargo fmt --check
cargo clippy -p ferriscraft-sheepwars --all-targets -- -D warnings
cargo test -p ferriscraft-sheepwars
```

Puis en jeu dans k3d : lancer chaque mouton.

Amended: 2026-11-22 Quake, Warp et Intergalactic atterrissent sans mèche de 3 s (`arms_on_impact` faux) : leur délai propre est dans `Effects`.

Amended: 2026-11-22 Les enfants de la fragmentation sont un `SheepKind::Fragment` hors de `SheepKind::ALL` (14 variantes, 13 obtenables) : plus petits, sans mèche.

Amended: 2026-11-22 La protection de chute de l'abordage n'est plus un délai fixe de 2 à 3 s : elle dure tant que le joueur est en l'air, plus 1 s après le sol (`FallGuards`).

Amended: 2026-11-22 Le glacé devient une zone de gel temporisée (`Effects`). Le ralentissement de 25 % demandé est approché par Slowness II (30 %, le palier vanilla le plus proche). Le passager de l'abordage n'est plus téléporté à chaque tick : chaque téléportation renvoie au client la rotation connue du serveur, d'où la saccade.

Amended: 2026-11-22 Le chercheur ne courait pas : le moteur coupait son IA à l'impact, or `finish_ai_step` n'appelle pas `travel` pour un mob sans IA (`living.rs`), donc sa vélocité était ignorée.
