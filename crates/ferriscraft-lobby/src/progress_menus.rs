use ferriscraft_progression::{Catalog, CatalogEntry};
use ferriscraft_protocol::{
    CosmeticId, CosmeticSlot, GameKind, GameStats, LevelProgress, PlayerProfile,
};
use rspigot_plugin::{CustomInventory, Enchantment, ItemStack, Material};

use crate::gadgets::{AuraTrail, LobbyGadget};
use crate::menus::fill_border;
use crate::profile::{ShopItemState, shop_item_state, unlock_label};

pub const PROFILE_ROWS: u8 = 4;
pub const SHOP_ROWS: u8 = 5;

pub const PROFILE_SLOT_NETWORK: u16 = 13;
pub const PROFILE_SLOT_GAMES_START: u16 = 20;
pub const PROFILE_SLOT_SHOP: u16 = 24;
pub const PROFILE_SLOT_BACK: u16 = 31;

pub const SHOP_SLOT_COINS: u16 = 4;
pub const SHOP_SLOT_BACK: u16 = 40;
pub const SHOP_ITEM_SLOTS: [u16; 12] = [10, 11, 12, 13, 14, 15, 19, 20, 21, 22, 23, 24];
pub const SHOP_TAB_SLOTS: [(u16, CosmeticSlot); 4] = [
    (38, CosmeticSlot::LobbyGadget),
    (39, CosmeticSlot::LobbyTrail),
    (41, CosmeticSlot::SheepTrail),
    (42, CosmeticSlot::VictoryEffect),
];
pub const UNAVAILABLE_SLOT: u16 = 22;

const BAR_WIDTH: u64 = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressMenuKind {
    Profile,
    Shop(CosmeticSlot),
}

impl ProgressMenuKind {
    pub const fn rows(self) -> u8 {
        match self {
            Self::Profile => PROFILE_ROWS,
            Self::Shop(_) => SHOP_ROWS,
        }
    }

    pub const fn title(self) -> &'static str {
        match self {
            Self::Profile => "§6§lFerrisCraft §8• §fProfil",
            Self::Shop(_) => "§a§lFerrisCraft §8• §fBoutique",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileMenuAction {
    OpenShop,
    BackToGames,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShopMenuAction {
    Page(CosmeticSlot),
    Item(CosmeticId),
    BackToGames,
}

pub fn decode_profile_click(slot: i32) -> Option<ProfileMenuAction> {
    match u16::try_from(slot).ok()? {
        PROFILE_SLOT_SHOP => Some(ProfileMenuAction::OpenShop),
        PROFILE_SLOT_BACK => Some(ProfileMenuAction::BackToGames),
        _ => None,
    }
}

pub fn decode_shop_click(slot: i32, page: CosmeticSlot) -> Option<ShopMenuAction> {
    let slot = u16::try_from(slot).ok()?;
    if slot == SHOP_SLOT_BACK {
        return Some(ShopMenuAction::BackToGames);
    }
    if let Some((_, tab)) = SHOP_TAB_SLOTS.iter().find(|(s, _)| *s == slot) {
        return Some(ShopMenuAction::Page(*tab));
    }
    let index = SHOP_ITEM_SLOTS.iter().position(|s| *s == slot)?;
    Catalog::in_slot(page)
        .nth(index)
        .map(|entry| ShopMenuAction::Item(entry.id))
}

pub const fn slot_title(slot: CosmeticSlot) -> &'static str {
    match slot {
        CosmeticSlot::LobbyGadget => "§b§lGadgets",
        CosmeticSlot::LobbyTrail => "§d§lTraînées du lobby",
        CosmeticSlot::SheepTrail => "§f§lTraînées de mouton",
        CosmeticSlot::VictoryEffect => "§6§lEffets de victoire",
    }
}

const fn slot_material(slot: CosmeticSlot) -> Material {
    match slot {
        CosmeticSlot::LobbyGadget => Material::Feather,
        CosmeticSlot::LobbyTrail => Material::BlazePowder,
        CosmeticSlot::SheepTrail => Material::WhiteWool,
        CosmeticSlot::VictoryEffect => Material::FireworkRocket,
    }
}

pub fn cosmetic_title(id: CosmeticId) -> &'static str {
    if let Some(gadget) = LobbyGadget::from_cosmetic(id) {
        return gadget.title();
    }
    if let Some(trail) = AuraTrail::from_cosmetic(id) {
        return trail.label();
    }
    match id {
        CosmeticId::SheepTrailFlame => "§6Traînée de mouton : Flamme",
        CosmeticId::SheepTrailHearts => "§dTraînée de mouton : Cœurs",
        CosmeticId::SheepTrailNotes => "§aTraînée de mouton : Notes",
        CosmeticId::SheepTrailRainbow => "§bTraînée de mouton : Arc-en-ciel",
        CosmeticId::VictoryFireworks => "§cEffet de victoire : Feux d'artifice",
        CosmeticId::VictoryLightning => "§eEffet de victoire : Éclair",
        _ => "§7Cosmétique",
    }
}

pub fn cosmetic_material(id: CosmeticId) -> Material {
    if let Some(gadget) = LobbyGadget::from_cosmetic(id) {
        return gadget.material();
    }
    if let Some(trail) = AuraTrail::from_cosmetic(id) {
        return trail.material();
    }
    match id {
        CosmeticId::SheepTrailFlame => Material::BlazePowder,
        CosmeticId::SheepTrailHearts => Material::Poppy,
        CosmeticId::SheepTrailNotes => Material::NoteBlock,
        CosmeticId::SheepTrailRainbow => Material::PinkWool,
        CosmeticId::VictoryFireworks => Material::FireworkRocket,
        CosmeticId::VictoryLightning => Material::BlazeRod,
        _ => Material::Barrier,
    }
}

pub fn progress_bar(into: u64, needed: u64) -> String {
    let filled = into
        .saturating_mul(BAR_WIDTH)
        .checked_div(needed)
        .map_or(BAR_WIDTH, |filled| filled.min(BAR_WIDTH));
    let empty = BAR_WIDTH - filled;
    format!(
        "§a{}§7{}",
        "▌".repeat(filled as usize),
        "▌".repeat(empty as usize)
    )
}

pub const fn game_label(game: GameKind) -> &'static str {
    match game {
        GameKind::SheepWars => "SheepWars",
    }
}

pub fn shop_item(entry: &CatalogEntry, state: ShopItemState) -> ItemStack {
    let title = cosmetic_title(entry.id);
    let base = ItemStack::new(cosmetic_material(entry.id), 1);
    match state {
        ShopItemState::Equipped => base
            .name(format!("{title} §a§l[ÉQUIPÉ]"))
            .lore(["§a✔ Actuellement équipé"])
            .enchant(Enchantment::Unbreaking, 1),
        ShopItemState::Unlocked => base
            .name(format!("{title} §a§l[DÉBLOQUÉ]"))
            .lore(["§e▶ Cliquez pour équiper"]),
        ShopItemState::Buyable(price) => base.name(title).lore([
            format!("§7Prix : §6{} coins", price.get()),
            "§a▶ Cliquez pour acheter".to_owned(),
        ]),
        ShopItemState::Locked(unlock) => base
            .name(format!("{title} §c§l[VERROUILLÉ]"))
            .lore([format!("§c✖ {}", unlock_label(unlock))]),
    }
}

fn clear(inv: &CustomInventory, rows: u8) {
    for slot in 0..u16::from(rows) * 9 {
        inv.set(slot, None);
    }
}

fn populate_unavailable(inv: &CustomInventory, kind: ProgressMenuKind, back: u16) {
    clear(inv, kind.rows());
    inv.set(
        UNAVAILABLE_SLOT,
        Some(
            ItemStack::new(Material::Barrier, 1)
                .name("§c§lBoutique indisponible")
                .lore([
                    "§7Le service de progression ne répond pas.",
                    "§7Réessayez dans quelques instants.",
                ]),
        ),
    );
    inv.set(
        back,
        Some(
            ItemStack::new(Material::Compass, 1)
                .name("§6§l← Retour au Menu des Jeux")
                .lore(["§7Revenir à la carte de navigation."]),
        ),
    );
}

pub fn populate_progress_menu(
    inv: &CustomInventory,
    kind: ProgressMenuKind,
    profile: Option<&PlayerProfile>,
) {
    match kind {
        ProgressMenuKind::Profile => populate_profile_menu(inv, profile),
        ProgressMenuKind::Shop(page) => populate_shop_menu(inv, page, profile),
    }
}

fn network_item(network: &LevelProgress) -> ItemStack {
    ItemStack::new(Material::Emerald, 1)
        .name(format!("§6§lLevel réseau §f{}", network.level))
        .lore([
            progress_bar(network.xp_into_level, network.xp_for_next),
            format!(
                "§7XP : §f{}§7/§f{}",
                network.xp_into_level, network.xp_for_next
            ),
            format!("§7XP totale : §f{}", network.total_xp),
        ])
        .enchant(Enchantment::Unbreaking, 1)
}

fn game_item(stats: &GameStats) -> ItemStack {
    ItemStack::new(Material::WhiteWool, 1)
        .name(format!(
            "§d§l{} §7• Level §f{}",
            game_label(stats.game),
            stats.progress.level
        ))
        .lore([
            progress_bar(stats.progress.xp_into_level, stats.progress.xp_for_next),
            format!(
                "§7XP : §f{}§7/§f{}",
                stats.progress.xp_into_level, stats.progress.xp_for_next
            ),
            String::new(),
            format!("§7Parties jouées : §f{}", stats.played),
            format!("§aVictoires : §f{}", stats.wins),
            format!("§cDéfaites : §f{}", stats.losses),
            format!("§eÉgalités : §f{}", stats.draws),
        ])
}

pub fn populate_profile_menu(inv: &CustomInventory, profile: Option<&PlayerProfile>) {
    let Some(profile) = profile else {
        populate_unavailable(inv, ProgressMenuKind::Profile, PROFILE_SLOT_BACK);
        return;
    };
    clear(inv, PROFILE_ROWS);
    fill_border(
        inv,
        PROFILE_ROWS,
        Material::YellowStainedGlassPane,
        Material::GrayStainedGlassPane,
    );
    inv.set(PROFILE_SLOT_NETWORK, Some(network_item(&profile.network)));
    if profile.games.is_empty() {
        inv.set(
            PROFILE_SLOT_GAMES_START,
            Some(
                ItemStack::new(Material::WhiteWool, 1)
                    .name("§7Aucune partie comptée")
                    .lore(["§7Jouez à SheepWars pour gagner de l'XP !"]),
            ),
        );
    }
    for (offset, stats) in profile.games.iter().enumerate() {
        let Ok(offset) = u16::try_from(offset) else {
            break;
        };
        inv.set(PROFILE_SLOT_GAMES_START + offset, Some(game_item(stats)));
    }
    inv.set(
        PROFILE_SLOT_SHOP,
        Some(
            ItemStack::new(Material::GoldIngot, 1)
                .name("§a§lBoutique")
                .lore([
                    format!("§6Coins §f{}", profile.coins),
                    String::new(),
                    "§e▶ Cliquez pour ouvrir la boutique".to_owned(),
                ]),
        ),
    );
    inv.set(
        PROFILE_SLOT_BACK,
        Some(
            ItemStack::new(Material::Compass, 1)
                .name("§6§l← Retour au Menu des Jeux")
                .lore(["§7Revenir à la carte de navigation."]),
        ),
    );
}

pub fn populate_shop_menu(
    inv: &CustomInventory,
    page: CosmeticSlot,
    profile: Option<&PlayerProfile>,
) {
    let kind = ProgressMenuKind::Shop(page);
    let Some(profile) = profile else {
        populate_unavailable(inv, kind, SHOP_SLOT_BACK);
        return;
    };
    clear(inv, SHOP_ROWS);
    fill_border(
        inv,
        SHOP_ROWS,
        Material::LimeStainedGlassPane,
        Material::GrayStainedGlassPane,
    );
    inv.set(
        SHOP_SLOT_COINS,
        Some(
            ItemStack::new(Material::GoldIngot, 1)
                .name(format!("§6Coins §f{}", profile.coins))
                .lore([slot_title(page)]),
        ),
    );
    for (slot, entry) in SHOP_ITEM_SLOTS.iter().zip(Catalog::in_slot(page)) {
        inv.set(
            *slot,
            Some(shop_item(&entry, shop_item_state(profile, &entry))),
        );
    }
    for (slot, tab) in SHOP_TAB_SLOTS {
        let mut item = ItemStack::new(slot_material(tab), 1).name(slot_title(tab));
        if tab == page {
            item = item
                .lore(["§a✔ Page affichée"])
                .enchant(Enchantment::Unbreaking, 1);
        } else {
            item = item.lore(["§e▶ Cliquez pour afficher"]);
        }
        inv.set(slot, Some(item));
    }
    inv.set(
        SHOP_SLOT_BACK,
        Some(
            ItemStack::new(Material::Compass, 1)
                .name("§6§l← Retour au Menu des Jeux")
                .lore(["§7Revenir à la carte de navigation."]),
        ),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shop_items_decode_in_catalogue_order_per_page() {
        let first = SHOP_ITEM_SLOTS[0];
        assert_eq!(
            decode_shop_click(i32::from(first), CosmeticSlot::LobbyGadget),
            Some(ShopMenuAction::Item(CosmeticId::GadgetFeatherDash))
        );
        assert_eq!(
            decode_shop_click(i32::from(first), CosmeticSlot::VictoryEffect),
            Some(ShopMenuAction::Item(CosmeticId::VictoryFireworks))
        );
        assert_eq!(
            decode_shop_click(i32::from(SHOP_ITEM_SLOTS[5]), CosmeticSlot::LobbyGadget),
            Some(ShopMenuAction::Item(CosmeticId::GadgetEnderGrapple))
        );
        assert_eq!(
            decode_shop_click(i32::from(SHOP_ITEM_SLOTS[5]), CosmeticSlot::VictoryEffect),
            None
        );
    }

    #[test]
    fn shop_tabs_and_back_decode() {
        for (slot, tab) in SHOP_TAB_SLOTS {
            assert_eq!(
                decode_shop_click(i32::from(slot), CosmeticSlot::LobbyGadget),
                Some(ShopMenuAction::Page(tab))
            );
        }
        assert_eq!(
            decode_shop_click(i32::from(SHOP_SLOT_BACK), CosmeticSlot::SheepTrail),
            Some(ShopMenuAction::BackToGames)
        );
        assert_eq!(decode_shop_click(-1, CosmeticSlot::SheepTrail), None);
        assert_eq!(decode_shop_click(0, CosmeticSlot::SheepTrail), None);
    }

    #[test]
    fn profile_slots_decode() {
        assert_eq!(
            decode_profile_click(i32::from(PROFILE_SLOT_SHOP)),
            Some(ProfileMenuAction::OpenShop)
        );
        assert_eq!(
            decode_profile_click(i32::from(PROFILE_SLOT_BACK)),
            Some(ProfileMenuAction::BackToGames)
        );
        assert_eq!(decode_profile_click(0), None);
    }

    #[test]
    fn every_page_fits_in_the_item_slots() {
        for (_, page) in SHOP_TAB_SLOTS {
            assert!(Catalog::in_slot(page).count() <= SHOP_ITEM_SLOTS.len());
        }
    }

    #[test]
    fn every_catalogue_cosmetic_has_a_title() {
        for id in Catalog::ALL {
            assert_ne!(cosmetic_title(id), "§7Cosmétique", "{id:?}");
            assert_ne!(cosmetic_material(id), Material::Barrier, "{id:?}");
        }
    }

    #[test]
    fn progress_bar_fills_proportionally_and_never_overflows() {
        assert_eq!(progress_bar(0, 100), format!("§a§7{}", "▌".repeat(20)));
        assert_eq!(
            progress_bar(50, 100),
            format!("§a{}§7{}", "▌".repeat(10), "▌".repeat(10))
        );
        assert_eq!(progress_bar(500, 100), format!("§a{}§7", "▌".repeat(20)));
        assert_eq!(progress_bar(1, 0), format!("§a{}§7", "▌".repeat(20)));
    }
}
