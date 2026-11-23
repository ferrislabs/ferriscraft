# TowerWars — Game Design Document (source des chiffres)

Source de vérité des valeurs (coûts, PV, vitesses, portées). Les règles de mise en œuvre et les arbitrages sont dans [`specs/towerwars.md`](specs/towerwars.md) ; en cas de conflit, le spec gagne.

## 1. Boucle de jeu

| Mode | Joueurs | Voies | PV de départ |
| :--- | :--- | :--- | :--- |
| Solo | 6 (min. 4) | 6 voies indépendantes | 20 |
| Teams | 12 (6 équipes de 2) | 6 voies partagées (or individuel, limite de tours partagée) | 25 |

- Or de départ : 100. Income de départ : 10 or toutes les 6 s (120 ticks).
- Acheter un mob l'envoie à tous les adversaires encore en vie. Acheter un mob est le seul moyen d'augmenter l'income.
- Un mob qui atteint le portail de fin disparaît et retire ses « Dégâts Base » en PV au défenseur.
- Mort subite à 15:00 : tous les mobs envoyés gagnent +25 % de vitesse et +50 % de PV pour chaque minute supplémentaire. Régénération et vol de vie désactivés.
- Victoire : dernier joueur ou dernière équipe avec au moins 1 PV.

## 2. Tours

- Limite : 80 tours par voie (partagée en Teams). Empreinte 1x1.
- Ciblage : `FIRST` (défaut), `LAST`, `STRONGEST`, `WEAKEST`.
- Revente : 80 % du coût total investi.
- Arbre : T1 → T2 → T3, puis choix exclusif T4-A ou T4-B.

### Archer (monocible)
| Niveau | Coût | Dégâts | Cadence | Portée |
|---|---|---|---|---|
| T1 | 50 | 12 | 1.0 s | 8 |
| T2 | 100 | 26 | 0.9 s | 9 |
| T3 | 250 | 60 | 0.8 s | 10 |
| T4-A Marksman | 600 | 180 | 1.2 s | 14 | 25 % de critique (x2.5) |
| T4-B Machine Gun | 600 | 45 | 0.25 s | 9 |

### Ice (contrôle)
| Niveau | Coût | Dégâts | Ralentissement | Cadence | Rayon AoE |
|---|---|---|---|---|---|
| T1 | 75 | 5 | 25 % (2.5 s) | 1.5 s | 3.5 |
| T2 | 150 | 12 | 35 % (3.0 s) | 1.4 s | 4.0 |
| T3 | 350 | 25 | 45 % (3.5 s) | 1.3 s | 4.5 |
- T4-A Permafrost (800) : ralentissement 60 % (4.0 s), les cibles ralenties subissent +15 % de dégâts de toutes sources.
- T4-B Blizzard (800) : 65 dégâts/s en aura continue autour de la tour, 10 % de chance de gel complet (0.75 s d'étourdissement, immunité interne 4 s).

### Mage (dégâts magiques / DoT, ignore l'armure)
| Niveau | Coût | Dégâts | Brûlure | Cadence | Portée |
|---|---|---|---|---|---|
| T1 | 100 | 15 | 5/s (3 s) | 1.2 s | 8 |
| T2 | 220 | 35 | 12/s (3 s) | 1.1 s | 9 |
| T3 | 480 | 80 | 25/s (4 s) | 1.0 s | 10 |
- T4-A Pyromancer (1100) : explosion de feu (AoE 2.5), brûlure cumulable 3 fois (45/s par stack).
- T4-B Necromancer (1100) : maudit la cible ; si elle meurt maudite, elle explose pour 30 % de ses PV max en dégâts de zone.

### Cannon (artillerie AoE)
| Niveau | Coût | Dégâts | Rayon AoE | Cadence | Portée |
|---|---|---|---|---|---|
| T1 | 125 | 28 | 2.5 | 2.0 s | 9 |
| T2 | 275 | 65 | 3.0 | 1.9 s | 10 |
| T3 | 600 | 150 | 3.5 | 1.8 s | 11 |
- T4-A Big Bertha (1400) : 480 dégâts, rayon 4.5, cadence 2.5 s, portée 15.
- T4-B Cluster Bomb (1400) : l'impact génère 4 sous-munitions de 110 dégâts, rayon 2.

### Tesla (chaîne)
| Niveau | Coût | Dégâts | Cibles | Rebond | Cadence | Portée |
|---|---|---|---|---|---|---|
| T1 | 150 | 20 | 3 | 3.5 | 1.1 s | 7.5 |
| T2 | 320 | 45 | 4 | 4.0 | 1.0 s | 8.5 |
| T3 | 700 | 100 | 5 | 4.5 | 0.9 s | 9.5 |
- T4-A Arc Reactor (1600) : jusqu'à 9 rebonds sans perte (190 par cible).
- T4-B Overload (1600) : 3 rebonds, 320 dégâts, étourdit la cible principale 0.4 s toutes les 3 attaques.

### Slime / Acid (corrosion)
| Niveau | Coût | Dégâts | Armure | Cadence | Portée |
|---|---|---|---|---|---|
| T1 | 120 | 14 | -15 % (4 s) | 1.3 s | 7.5 |
| T2 | 260 | 32 | -25 % (4 s) | 1.2 s | 8.5 |
| T3 | 550 | 75 | -35 % (5 s) | 1.1 s | 9.5 |
- T4-A Corrosive Pool (1250) : flaque (rayon 3, 4 s) à 60/s, armure -50 %.
- T4-B Toxic Mutator (1250) : dégâts bonus = 2.5 % des PV actuels de la cible par coup.

### Beacon (aura)
Booste les tours alliées dans son rayon ; au plus 2 beacons cumulés sur une même tour.
| Niveau | Coût | Vitesse d'attaque | Portée | Rayon |
|---|---|---|---|---|
| T1 | 200 | +10 % | — | 4.5 |
| T2 | 450 | +18 % | +10 % | 5.0 |
| T3 | 950 | +25 % | +15 % | 5.5 |
- T4-A Overclock (2000) : +40 % vitesse d'attaque, +20 % dégâts, rayon 6.
- T4-B Gold Mine (2000) : +15 or par cycle d'income (max 4 par joueur), +15 % vitesse d'attaque.

## 3. Mobs

- Envoi : coût en or, income permanent crédité immédiatement, apparition chez tous les adversaires vivants (décalage de 2 ticks par mob en envoi groupé).
- Stock : max 10 charges par type, recharge 1 charge / 1.5 s.
- Tiers débloqués par le chronomètre : T1 à 0:00, T2 à 2:30, T3 à 5:30, T4 à 9:00.
- Kill bounty : 10 à 15 % du coût d'envoi en or direct, sans income.

### Tier 1
| Mob | Coût | Income | PV | Vitesse (b/s) | Dégâts | Capacité |
|---|---|---|---|---|---|---|
| Silverfish | 15 | +2 | 35 | 3.2 | 1 | — |
| Zombie | 30 | +4 | 95 | 2.4 | 1 | armure 10 % |
| Endermite | 50 | +5 | 65 | 4.6 | 1 | rush (très rapide) |
| Skeleton | 80 | +9 | 180 | 2.7 | 1 | esquive 15 % des projectiles monocibles |

### Tier 2
| Mob | Coût | Income | PV | Vitesse | Dégâts | Capacité |
|---|---|---|---|---|---|---|
| Spider | 120 | +13 | 220 | 3.8 | 1 | immunité partielle 50 % aux ralentissements |
| Creeper | 200 | +21 | 420 | 2.5 | 2 | à sa mort, étourdit les tours à 2.5 blocs pendant 1.0 s |
| Blaze | 320 | +32 | 550 | 3.0 | 2 | immunisé feu / brûlure |
| Witch | 450 | +44 | 680 | 2.6 | 2 | soigne les mobs alliés à 3 blocs (30 PV/s) |

### Tier 3
| Mob | Coût | Income | PV | Vitesse | Dégâts | Capacité |
|---|---|---|---|---|---|---|
| Slime géant | 700 | +65 | 1100 | 2.2 | 2 | à sa mort, 2 slimes moyens (300 PV, 2.8 b/s) |
| Piglin Brute | 1000 | +90 | 1850 | 2.8 | 3 | armure 25 %, +30 % de vitesse sous 40 % PV |
| Enderman | 1450 | +125 | 1600 | 3.5 | 3 | blink : téléportation de 4 blocs vers l'avant sous 50 % PV |
| Ghast | 2000 | +170 | 2400 | 2.4 | 3 | volant : ignore les flaques, vulnérable Archer/Tesla |

### Tier 4
| Mob | Coût | Income | PV | Vitesse | Dégâts | Capacité |
|---|---|---|---|---|---|---|
| Iron Golem | 3000 | +240 | 5500 | 1.9 | 4 | armure 40 %, immunisé étourdissements |
| Ravager | 4500 | +340 | 7800 | 2.6 | 5 | écrase les effets de zone au sol, résiste à 50 % des ralentissements |
| Warden | 7000 | +500 | 13500 | 2.1 | 8 | boss : -20 % vitesse d'attaque des tours à 4 blocs |
| Wither | 12000 | +800 | 22000 | 2.5 | 15 | boss : régénère 150 PV/s, +2 PV à l'attaquant s'il passe |

## 4. Maps

- 6 voies isolées. Chemin de 75 à 95 blocs, 4 à 6 virages.
- Données d'une voie : `lane_id` (0..5), `spawn_point`, `waypoints`, `build_bounds` (AABB), `spectate_anchor`.

## 5. Technique (rspigot)

- 20 TPS constants avec 6 voies, 80 tours par voie, des centaines de mobs.
- Pas d'IA vanilla : un mob = `progress` (distance parcourue) + `lateral_offset` dans [-0.35, 0.35]. Position et yaw interpolés sur les segments.
- Paquets de mouvement uniquement vers les joueurs qui observent la voie.
- Tours en `BlockDisplay`/`ItemDisplay`, barres de PV en `TextDisplay`, projectiles en particules.

## 6. UX

- Hotbar : slot 1 construire (GUI des 7 tours, aperçu de portée en particules), slot 2 envoyer des mobs (clic gauche 1, clic droit ou Shift 5), slot 5 téléporteur de voies, slot 9 auto-send (optionnel).
- Clic droit sur une tour : améliorer, changer la priorité, vendre.
- ActionBar toutes les 5 ticks : `Or: 450 ⛃  |  Income: +68 / 6s  |  Tours: 24/80`.
- Scoreboard : temps écoulé, prochain tier, classement des voies. BossBar : progression vers le prochain income.
