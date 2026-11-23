use rspigot_plugin::Material;

use super::names::{effect_text, level_name, mob_name, next_priority, priority_name, tower_name};
use super::render::mob_material;
use crate::domain::{
    Branch, LaneId, Level, MobKind, PlayerId, SELL_REFUND_PERCENT, STOCK_MAX, TICKS_PER_SECOND,
    TIER_UNLOCK_TICKS, TowerId, TowerKind,
};
use crate::hud::{clock, lane_color, lane_names};
use crate::sim::Match;

pub const ROW: u16 = 9;
pub const SEND_BULK: u8 = 5;
const MOB_FIRST_COLUMN: u16 = 2;
const BUILD_FIRST_SLOT: u16 = 10;
const LANE_FIRST_SLOT: u16 = 10;
const LANES_RETURN_SLOT: u16 = 4;
const TOWER_INFO_SLOT: u16 = 4;
const TOWER_UPGRADE_SLOT: u16 = 11;
const TOWER_BRANCH_A_SLOT: u16 = 10;
const TOWER_BRANCH_B_SLOT: u16 = 12;
const TOWER_PRIORITY_SLOT: u16 = 14;
const TOWER_SELL_SLOT: u16 = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuAction {
    Build(TowerKind),
    Send(MobKind),
    View(LaneId),
    ViewOwn,
    Upgrade(Option<Branch>),
    CyclePriority,
    Sell,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuItem {
    pub slot: u16,
    pub material: Material,
    pub count: u8,
    pub name: String,
    pub lore: Vec<String>,
    pub action: Option<MenuAction>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Menu {
    pub title: String,
    pub rows: u8,
    pub items: Vec<MenuItem>,
}

impl Menu {
    pub fn action_at(&self, slot: u16) -> Option<MenuAction> {
        self.items
            .iter()
            .find(|item| item.slot == slot)
            .and_then(|item| item.action)
    }

    pub fn item_at(&self, slot: u16) -> Option<&MenuItem> {
        self.items.iter().find(|item| item.slot == slot)
    }
}

pub const fn send_count(right_click: bool, shift: bool) -> u8 {
    if right_click || shift { SEND_BULK } else { 1 }
}

pub const fn tower_icon(kind: TowerKind) -> Material {
    match kind {
        TowerKind::Archer => Material::Bow,
        TowerKind::Ice => Material::BlueIce,
        TowerKind::Mage => Material::BlazeRod,
        TowerKind::Cannon => Material::Tnt,
        TowerKind::Tesla => Material::LightningRod,
        TowerKind::Slime => Material::SlimeBlock,
        TowerKind::Beacon => Material::Beacon,
    }
}

fn affordable(gold: u32, cost: u32) -> &'static str {
    if gold >= cost { "§a" } else { "§c" }
}

fn seconds(ticks: u16) -> f32 {
    f32::from(ticks) / TICKS_PER_SECOND as f32
}

pub fn tower_stat_lines(kind: TowerKind, level: Level) -> Vec<String> {
    let stats = kind.stats(level);
    let mut lines = Vec::new();
    if stats.damage > 0.0 {
        lines.push(format!("§7Dégâts : §f{:.0}", stats.damage));
    }
    lines.push(format!(
        "§7Cadence : §f{:.2}s",
        seconds(stats.cooldown_ticks)
    ));
    lines.push(format!("§7Portée : §f{:.1} blocs", stats.range));
    if let Some(effect) = effect_text(stats.effect) {
        lines.push(format!("§d{effect}"));
    }
    lines
}

pub fn build_menu(gold: u32, selected: Option<TowerKind>) -> Menu {
    let items = TowerKind::ALL
        .into_iter()
        .zip(BUILD_FIRST_SLOT..)
        .map(|(kind, slot)| {
            let cost = kind.stats(Level::T1).cost;
            let chosen = selected == Some(kind);
            let mut lore = vec![format!("§7Coût : {}{cost} or", affordable(gold, cost))];
            lore.extend(tower_stat_lines(kind, Level::T1));
            lore.push(if chosen {
                "§aSélectionnée : clic droit sur une case pour construire".to_owned()
            } else {
                "§eClic pour sélectionner".to_owned()
            });
            MenuItem {
                slot,
                material: tower_icon(kind),
                count: 1,
                name: format!("{}{}", if chosen { "§a▶ " } else { "§f" }, tower_name(kind)),
                lore,
                action: Some(MenuAction::Build(kind)),
            }
        })
        .collect();
    Menu {
        title: "§8Construire une tour".to_owned(),
        rows: 3,
        items,
    }
}

pub const fn tier_unlock_ticks(tier: u8) -> u64 {
    TIER_UNLOCK_TICKS[tier as usize - 1]
}

pub fn send_menu(game: &Match, player: PlayerId) -> Option<Menu> {
    let state = game.world().player(player)?;
    let elapsed = game.elapsed();
    let gold = state.gold.get();
    let mut items = Vec::new();
    for tier in 1..=4u8 {
        let row = u16::from(tier - 1) * ROW;
        let unlock = tier_unlock_ticks(tier);
        let locked = elapsed < unlock;
        items.push(MenuItem {
            slot: row,
            material: if locked {
                Material::Barrier
            } else {
                Material::Paper
            },
            count: tier,
            name: format!("§eTier {tier}"),
            lore: vec![if locked {
                format!("§cDébloqué dans §f{}", clock(unlock - elapsed))
            } else {
                "§aDébloqué".to_owned()
            }],
            action: None,
        });
        let kinds = MobKind::SENDABLE
            .into_iter()
            .filter(|kind| kind.spec().tier == tier);
        for (kind, column) in kinds.zip(MOB_FIRST_COLUMN..) {
            let spec = kind.spec();
            let slot = row + column;
            if locked {
                items.push(MenuItem {
                    slot,
                    material: Material::Barrier,
                    count: 1,
                    name: format!("§c{} §8(verrouillé)", mob_name(kind)),
                    lore: vec![
                        format!("§7Coût : §6{} or", spec.cost),
                        format!("§7Income : §a+{}", spec.income),
                        format!("§cDébloqué dans §f{}", clock(unlock - elapsed)),
                    ],
                    action: None,
                });
                continue;
            }
            let charges = kind
                .stock_index()
                .map_or(0, |index| state.stock.charges[index]);
            let color = if charges == 0 || gold < spec.cost {
                "§c"
            } else {
                "§f"
            };
            let mut lore = vec![
                format!("§7Coût : {}{} or", affordable(gold, spec.cost), spec.cost),
                format!("§7Income : §a+{}", spec.income),
                format!("§7Stock : §f{charges}/{STOCK_MAX}"),
                format!(
                    "§7PV : §f{:.0} §8• §7Vitesse : §f{:.1} b/s §8• §7Dégâts : §c{}",
                    spec.hp, spec.speed, spec.damage
                ),
            ];
            if spec.flying {
                lore.push("§bVole".to_owned());
            }
            if spec.fire_immune {
                lore.push("§6Immunisé au feu".to_owned());
            }
            if spec.armor > 0.0 {
                lore.push(format!("§7Armure : §f{:.0}%", spec.armor * 100.0));
            }
            lore.push("§eClic gauche : envoyer 1".to_owned());
            lore.push(format!("§eClic droit / Shift : envoyer {SEND_BULK}"));
            items.push(MenuItem {
                slot,
                material: mob_material(kind),
                count: charges.clamp(1, STOCK_MAX),
                name: format!("{color}{}", mob_name(kind)),
                lore,
                action: Some(MenuAction::Send(kind)),
            });
        }
    }
    Some(Menu {
        title: "§8Envoyer des mobs".to_owned(),
        rows: 4,
        items,
    })
}

pub fn refund_for(invested: u32) -> u32 {
    invested * SELL_REFUND_PERCENT / 100
}

pub fn tower_menu(game: &Match, player: PlayerId, tower: TowerId) -> Option<Menu> {
    let state = game.world().player(player)?;
    let found = game.lane(state.lane).tower(tower)?;
    let gold = state.gold.get();
    let mut items = vec![MenuItem {
        slot: TOWER_INFO_SLOT,
        material: tower_icon(found.kind),
        count: 1,
        name: format!("§e{} {}", tower_name(found.kind), level_name(found.level)),
        lore: {
            let mut lore = tower_stat_lines(found.kind, found.level);
            lore.push(format!("§7Investi : §6{} or", found.invested));
            lore
        },
        action: None,
    }];
    let upgrade = |slot: u16, branch: Option<Branch>, label: &str| {
        found.level.next(branch).ok().map(|next| {
            let stats = found.kind.stats(next);
            let mut lore = vec![format!(
                "§7Coût : {}{} or",
                affordable(gold, stats.cost),
                stats.cost
            )];
            lore.extend(tower_stat_lines(found.kind, next));
            lore.push("§eClic pour améliorer".to_owned());
            MenuItem {
                slot,
                material: Material::Anvil,
                count: 1,
                name: format!("§a{label} §8→ §f{}", level_name(next)),
                lore,
                action: Some(MenuAction::Upgrade(branch)),
            }
        })
    };
    match found.level {
        Level::T1 | Level::T2 => items.extend(upgrade(TOWER_UPGRADE_SLOT, None, "Améliorer")),
        Level::T3 => {
            items.extend(upgrade(TOWER_BRANCH_A_SLOT, Some(Branch::A), "Branche A"));
            items.extend(upgrade(TOWER_BRANCH_B_SLOT, Some(Branch::B), "Branche B"));
        }
        Level::T4(_) => items.push(MenuItem {
            slot: TOWER_UPGRADE_SLOT,
            material: Material::Barrier,
            count: 1,
            name: "§7Niveau maximal".to_owned(),
            lore: Vec::new(),
            action: None,
        }),
    }
    items.push(MenuItem {
        slot: TOWER_PRIORITY_SLOT,
        material: Material::Hopper,
        count: 1,
        name: format!("§bPriorité : §f{}", priority_name(found.priority)),
        lore: vec![
            format!(
                "§7Suivante : §f{}",
                priority_name(next_priority(found.priority))
            ),
            "§eClic pour changer".to_owned(),
        ],
        action: Some(MenuAction::CyclePriority),
    });
    items.push(MenuItem {
        slot: TOWER_SELL_SLOT,
        material: Material::GoldBlock,
        count: 1,
        name: "§cVendre".to_owned(),
        lore: vec![
            format!("§7Remboursement : §6{} or", refund_for(found.invested)),
            "§eClic pour vendre".to_owned(),
        ],
        action: Some(MenuAction::Sell),
    });
    Some(Menu {
        title: format!("§8{} {}", tower_name(found.kind), level_name(found.level)),
        rows: 3,
        items,
    })
}

pub fn lane_menu(
    game: &Match,
    viewer: PlayerId,
    name_of: &impl Fn(PlayerId) -> Option<String>,
    is_bot: &impl Fn(PlayerId) -> bool,
) -> Menu {
    let mut items = Vec::new();
    if game.world().player(viewer).is_some() {
        items.push(MenuItem {
            slot: LANES_RETURN_SLOT,
            material: Material::Emerald,
            count: 1,
            name: "§aRetour à ma voie".to_owned(),
            lore: Vec::new(),
            action: Some(MenuAction::ViewOwn),
        });
    }
    for (lane, slot) in game.world().lanes.iter().zip(LANE_FIRST_SLOT..) {
        if lane.owners.iter().all(|owner| is_bot(*owner)) {
            continue;
        }
        let state = if lane.eliminated {
            "§8éliminée".to_owned()
        } else {
            format!("§c{} ❤", lane.hp.current())
        };
        items.push(MenuItem {
            slot,
            material: Material::Compass,
            count: lane.id.get() + 1,
            name: format!(
                "{}Voie {} §8— §f{}",
                lane_color(lane.id),
                lane.id.get() + 1,
                lane_names(lane, name_of)
            ),
            lore: vec![state, "§eClic pour observer cette voie".to_owned()],
            action: Some(MenuAction::View(lane.id)),
        });
    }
    Menu {
        title: "§8Observer une voie".to_owned(),
        rows: 3,
        items,
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;
    use crate::domain::GameMode;
    use crate::map::Map;
    use crate::sim::Seat;

    fn pid(index: u128) -> PlayerId {
        PlayerId(Uuid::from_u128(index + 1))
    }

    fn lane(index: u8) -> LaneId {
        LaneId::try_from(index).expect("lane")
    }

    fn game() -> Match {
        let seats: Vec<Seat> = (0..4u8)
            .map(|index| Seat {
                player: pid(u128::from(index)),
                lane: lane(index),
            })
            .collect();
        Match::new(GameMode::Solo, 3, &Map::test_map(), &seats).expect("match")
    }

    fn names(id: PlayerId) -> Option<String> {
        Some(format!("P{}", id.0.as_u128()))
    }

    fn slots(menu: &Menu) -> Vec<u16> {
        menu.items.iter().map(|item| item.slot).collect()
    }

    fn fits(menu: &Menu) -> bool {
        menu.items
            .iter()
            .all(|item| item.slot < u16::from(menu.rows) * ROW)
    }

    #[test]
    fn the_build_menu_offers_the_seven_towers_with_cost_and_stats() {
        let menu = build_menu(100, None);
        assert_eq!(menu.items.len(), 7);
        assert!(fits(&menu));
        let archer = menu.item_at(10).expect("archer");
        assert_eq!(archer.action, Some(MenuAction::Build(TowerKind::Archer)));
        assert!(archer.lore[0].contains("50 or"));
        assert!(archer.lore.iter().any(|line| line.contains("Dégâts")));
        assert!(archer.lore.iter().any(|line| line.contains("Portée")));
    }

    #[test]
    fn unaffordable_towers_are_costed_in_red_and_the_selection_is_marked() {
        let menu = build_menu(60, Some(TowerKind::Archer));
        let ice = menu.item_at(11).expect("ice");
        assert!(ice.lore[0].contains("§c75 or"), "{:?}", ice.lore);
        let archer = menu.item_at(10).expect("archer");
        assert!(archer.lore[0].contains("§a50 or"));
        assert!(archer.name.contains('▶'));
        assert!(!ice.name.contains('▶'));
    }

    #[test]
    fn the_send_menu_has_four_tiers_of_four_mobs() {
        let game = game();
        let menu = send_menu(&game, pid(0)).expect("menu");
        assert!(fits(&menu));
        let sendable: Vec<MenuAction> = menu.items.iter().filter_map(|item| item.action).collect();
        assert_eq!(sendable.len(), 4);
        assert_eq!(
            menu.item_at(2).and_then(|item| item.action),
            Some(MenuAction::Send(MobKind::Silverfish))
        );
        let locked: Vec<&MenuItem> = menu
            .items
            .iter()
            .filter(|item| item.material == Material::Barrier)
            .collect();
        assert_eq!(locked.len(), 3 + 12);
        assert!(locked.iter().all(|item| item.action.is_none()));
    }

    #[test]
    fn a_sendable_mob_shows_cost_income_and_stock() {
        let game = game();
        let menu = send_menu(&game, pid(0)).expect("menu");
        let silverfish = menu.item_at(2).expect("silverfish");
        assert_eq!(silverfish.count, STOCK_MAX);
        let text = silverfish.lore.join("|");
        assert!(
            text.contains("15 or") && text.contains("+2") && text.contains("10/10"),
            "{text}"
        );
        assert!(text.contains("Clic gauche") && text.contains("Shift"));
    }

    #[test]
    fn the_stock_shown_follows_the_sends() {
        let mut game = game();
        game.send(pid(0), MobKind::Silverfish, 3).expect("send");
        let menu = send_menu(&game, pid(0)).expect("menu");
        let silverfish = menu.item_at(2).expect("silverfish");
        assert_eq!(silverfish.count, 7);
        assert!(silverfish.lore.join("|").contains("7/10"));
    }

    #[test]
    fn locked_tiers_show_their_countdown_and_unlock_with_time() {
        let mut game = game();
        let menu = send_menu(&game, pid(0)).expect("menu");
        let spider = menu.item_at(ROW + 2).expect("spider slot");
        assert_eq!(spider.material, Material::Barrier);
        assert!(spider.lore.iter().any(|line| line.contains("02:30")));
        for _ in 0..3_000 {
            game.tick();
        }
        let menu = send_menu(&game, pid(0)).expect("menu");
        assert_eq!(
            menu.item_at(ROW + 2).and_then(|item| item.action),
            Some(MenuAction::Send(MobKind::Spider))
        );
    }

    #[test]
    fn a_stranger_has_no_send_menu() {
        assert!(send_menu(&game(), pid(9)).is_none());
    }

    #[test]
    fn clicking_sends_one_and_right_or_shift_sends_five() {
        assert_eq!(send_count(false, false), 1);
        assert_eq!(send_count(true, false), 5);
        assert_eq!(send_count(false, true), 5);
    }

    #[test]
    fn the_tower_menu_offers_upgrade_priority_and_sell_with_refund() {
        let mut game = game();
        let tower = game
            .place(pid(0), TowerKind::Archer, (2, 1))
            .expect("place");
        let menu = tower_menu(&game, pid(0), tower).expect("menu");
        assert!(fits(&menu));
        assert_eq!(
            menu.action_at(TOWER_UPGRADE_SLOT),
            Some(MenuAction::Upgrade(None))
        );
        assert_eq!(
            menu.action_at(TOWER_PRIORITY_SLOT),
            Some(MenuAction::CyclePriority)
        );
        assert_eq!(menu.action_at(TOWER_SELL_SLOT), Some(MenuAction::Sell));
        let sell = menu.item_at(TOWER_SELL_SLOT).expect("sell");
        assert!(sell.lore[0].contains("40 or"), "{:?}", sell.lore);
        let upgrade = menu.item_at(TOWER_UPGRADE_SLOT).expect("upgrade");
        assert!(upgrade.lore[0].contains("100 or"));
        assert!(
            menu.item_at(TOWER_PRIORITY_SLOT)
                .expect("p")
                .name
                .contains("Premier")
        );
    }

    fn upgraded_to(game: &mut Match, tower: TowerId, steps: &[Option<Branch>]) {
        for branch in steps {
            loop {
                match game.upgrade(pid(0), tower, *branch) {
                    Ok(()) => break,
                    Err(crate::domain::SimError::NotEnoughGold) => {
                        for _ in 0..120 {
                            game.tick();
                        }
                    }
                    Err(error) => panic!("{error}"),
                }
            }
        }
    }

    #[test]
    fn a_third_level_tower_offers_both_branches() {
        let mut game = game();
        let tower = game
            .place(pid(0), TowerKind::Archer, (2, 1))
            .expect("place");
        upgraded_to(&mut game, tower, &[None, None]);
        let menu = tower_menu(&game, pid(0), tower).expect("menu");
        assert_eq!(
            menu.action_at(TOWER_BRANCH_A_SLOT),
            Some(MenuAction::Upgrade(Some(Branch::A)))
        );
        assert_eq!(
            menu.action_at(TOWER_BRANCH_B_SLOT),
            Some(MenuAction::Upgrade(Some(Branch::B)))
        );
        assert_eq!(menu.action_at(TOWER_UPGRADE_SLOT), None);
        assert!(menu.item_at(TOWER_BRANCH_A_SLOT).expect("a").lore[0].contains("600 or"));
        let sell = menu.item_at(TOWER_SELL_SLOT).expect("sell");
        assert!(sell.lore[0].contains("320 or"), "{:?}", sell.lore);
    }

    #[test]
    fn a_max_level_tower_cannot_be_upgraded() {
        let mut game = game();
        let tower = game
            .place(pid(0), TowerKind::Archer, (2, 1))
            .expect("place");
        upgraded_to(&mut game, tower, &[None, None, Some(Branch::A)]);
        let menu = tower_menu(&game, pid(0), tower).expect("menu");
        assert!(
            menu.items
                .iter()
                .all(|item| !matches!(item.action, Some(MenuAction::Upgrade(_))))
        );
        assert!(
            menu.item_at(TOWER_UPGRADE_SLOT)
                .expect("max")
                .name
                .contains("maximal")
        );
        assert_eq!(slots(&menu).len(), 4);
    }

    #[test]
    fn the_tower_menu_is_only_for_the_owner() {
        let mut game = game();
        let tower = game
            .place(pid(0), TowerKind::Archer, (2, 1))
            .expect("place");
        assert!(tower_menu(&game, pid(1), tower).is_none());
        assert!(tower_menu(&game, pid(0), TowerId(99)).is_none());
    }

    #[test]
    fn the_lane_menu_lists_the_occupied_lanes_and_a_way_back() {
        let game = game();
        let menu = lane_menu(&game, pid(0), &names, &|_| false);
        assert!(fits(&menu));
        assert_eq!(menu.action_at(LANES_RETURN_SLOT), Some(MenuAction::ViewOwn));
        let views: Vec<MenuAction> = menu
            .items
            .iter()
            .filter_map(|item| item.action)
            .filter(|action| matches!(action, MenuAction::View(_)))
            .collect();
        assert_eq!(views.len(), 4);
        assert_eq!(
            menu.action_at(LANE_FIRST_SLOT + 2),
            Some(MenuAction::View(lane(2)))
        );
        assert!(
            menu.item_at(LANE_FIRST_SLOT + 1)
                .expect("lane")
                .name
                .contains("P2")
        );
    }

    #[test]
    fn a_spectator_has_no_way_back() {
        let game = game();
        let menu = lane_menu(&game, pid(9), &names, &|_| false);
        assert_eq!(menu.action_at(LANES_RETURN_SLOT), None);
    }

    #[test]
    fn refunds_floor_at_eighty_percent() {
        assert_eq!(refund_for(50), 40);
        assert_eq!(refund_for(51), 40);
        assert_eq!(refund_for(0), 0);
    }
}
