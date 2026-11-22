use std::collections::HashMap;
use std::sync::RwLock;

use rspigot_plugin::{
    CustomInventory, Enchantment, Error, InventoryRef, InventorySlot, ItemStack, Material, Player,
    Server, SlotItem, Uuid,
};

use crate::auth::UserClaims;
use crate::gadgets::LobbyGadget;
use crate::prefs::PlayerPrefs;
use crate::profile::{gadget_usable, unlock_label};
use crate::progress_menus::{ProgressMenuKind, populate_progress_menu};
use crate::servers_menu::{SERVERS_ROWS, populate_servers_menu, sorted_server_ids};
use ferriscraft_progression::Catalog;
use ferriscraft_protocol::{FleetStatusSnapshot, PlayerProfile};

pub const HOTBAR_GAMES_SLOT: u8 = 0;
pub const HOTBAR_SHEEPWARS_SLOT: u8 = 1;
pub const HOTBAR_ACCOUNT_SLOT: u8 = 4;
pub const HOTBAR_GADGETS_MENU_SLOT: u8 = 5;
pub const HOTBAR_EQUIPPED_GADGET_SLOT: u8 = 6;
pub const HOTBAR_VISIBILITY_SLOT: u8 = 7;
pub const HOTBAR_OPTIONS_SLOT: u8 = 8;

pub const GAMES_ROWS: u8 = 5;
pub const OPTIONS_ROWS: u8 = 4;
pub const GADGETS_ROWS: u8 = 4;

pub const SLOT_SHEEPWARS: u16 = 13;
pub const SLOT_PITCHOUT: u16 = 20;
pub const SLOT_FK: u16 = 21;
pub const SLOT_PVPSWAP: u16 = 23;
pub const SLOT_OPEN_PROFILE: u16 = 30;
pub const SLOT_OPEN_SHOP: u16 = 32;
pub const SLOT_MOUTRON: u16 = 24;
pub const SLOT_OPEN_GADGETS: u16 = 37;
pub const SLOT_TP_SPAWN: u16 = 38;
pub const SLOT_TP_PORTAL: u16 = 39;
pub const SLOT_TP_CAPITAINERIE: u16 = 40;
pub const SLOT_TP_CASTLE: u16 = 41;
pub const SLOT_OPEN_OPTIONS: u16 = 42;

pub const OPT_SLOT_VISIBILITY: u16 = 10;
pub const OPT_SLOT_FLIGHT: u16 = 12;
pub const OPT_SLOT_SPEED: u16 = 14;
pub const OPT_SLOT_CHAT_SOUNDS: u16 = 16;
pub const OPT_SLOT_FERRISKEY: u16 = 22;
pub const OPT_SLOT_BACK: u16 = 31;

pub const GAD_SLOT_FEATHER: u16 = 10;
pub const GAD_SLOT_ROCKET: u16 = 11;
pub const GAD_SLOT_SHEEP: u16 = 12;
pub const GAD_SLOT_VORTEX: u16 = 13;
pub const GAD_SLOT_WIND: u16 = 14;
pub const GAD_SLOT_ENDER: u16 = 15;
pub const GAD_SLOT_UNEQUIP: u16 = 16;
pub const GAD_SLOT_FLIGHT: u16 = 20;
pub const GAD_SLOT_TRAIL: u16 = 22;
pub const GAD_SLOT_SPEED: u16 = 24;
pub const GAD_SLOT_BACK: u16 = 31;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GamesMenuAction {
    JoinSheepWars,
    ComingSoon(&'static str),
    OpenGadgets,
    OpenProfile,
    OpenShop,
    TeleportPoi(&'static str),
    OpenOptions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptionsMenuAction {
    ToggleVisibility,
    ToggleFlight,
    CycleSpeed,
    ToggleChatSounds,
    FerrisKeyAction,
    BackToGames,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GadgetsMenuAction {
    Equip(LobbyGadget),
    UnequipGadget,
    ToggleFlight,
    CycleTrail,
    CycleSpeed,
    BackToGames,
}

pub fn decode_games_click(slot: i32) -> Option<GamesMenuAction> {
    let slot = u16::try_from(slot).ok()?;
    match slot {
        SLOT_SHEEPWARS => Some(GamesMenuAction::JoinSheepWars),
        SLOT_PITCHOUT => Some(GamesMenuAction::ComingSoon("Pitchout")),
        SLOT_FK => Some(GamesMenuAction::ComingSoon("Fallen Kingdoms")),
        SLOT_PVPSWAP => Some(GamesMenuAction::ComingSoon("PvP Swap")),
        SLOT_MOUTRON => Some(GamesMenuAction::ComingSoon("Moutron")),
        SLOT_OPEN_GADGETS => Some(GamesMenuAction::OpenGadgets),
        SLOT_OPEN_PROFILE => Some(GamesMenuAction::OpenProfile),
        SLOT_OPEN_SHOP => Some(GamesMenuAction::OpenShop),
        SLOT_TP_SPAWN => Some(GamesMenuAction::TeleportPoi("spawn")),
        SLOT_TP_PORTAL => Some(GamesMenuAction::TeleportPoi("portal")),
        SLOT_TP_CAPITAINERIE => Some(GamesMenuAction::TeleportPoi("capitainerie")),
        SLOT_TP_CASTLE => Some(GamesMenuAction::TeleportPoi("castle")),
        SLOT_OPEN_OPTIONS => Some(GamesMenuAction::OpenOptions),
        _ => None,
    }
}

pub fn decode_options_click(slot: i32) -> Option<OptionsMenuAction> {
    let slot = u16::try_from(slot).ok()?;
    match slot {
        OPT_SLOT_VISIBILITY => Some(OptionsMenuAction::ToggleVisibility),
        OPT_SLOT_FLIGHT => Some(OptionsMenuAction::ToggleFlight),
        OPT_SLOT_SPEED => Some(OptionsMenuAction::CycleSpeed),
        OPT_SLOT_CHAT_SOUNDS => Some(OptionsMenuAction::ToggleChatSounds),
        OPT_SLOT_FERRISKEY => Some(OptionsMenuAction::FerrisKeyAction),
        OPT_SLOT_BACK => Some(OptionsMenuAction::BackToGames),
        _ => None,
    }
}

pub fn decode_gadgets_click(slot: i32) -> Option<GadgetsMenuAction> {
    let slot = u16::try_from(slot).ok()?;
    match slot {
        GAD_SLOT_FEATHER => Some(GadgetsMenuAction::Equip(LobbyGadget::FeatherDash)),
        GAD_SLOT_ROCKET => Some(GadgetsMenuAction::Equip(LobbyGadget::FireworkRocket)),
        GAD_SLOT_SHEEP => Some(GadgetsMenuAction::Equip(LobbyGadget::SheepCannon)),
        GAD_SLOT_VORTEX => Some(GadgetsMenuAction::Equip(LobbyGadget::VortexWand)),
        GAD_SLOT_WIND => Some(GadgetsMenuAction::Equip(LobbyGadget::WindCharge)),
        GAD_SLOT_ENDER => Some(GadgetsMenuAction::Equip(LobbyGadget::EnderGrapple)),
        GAD_SLOT_UNEQUIP => Some(GadgetsMenuAction::UnequipGadget),
        GAD_SLOT_FLIGHT => Some(GadgetsMenuAction::ToggleFlight),
        GAD_SLOT_TRAIL => Some(GadgetsMenuAction::CycleTrail),
        GAD_SLOT_SPEED => Some(GadgetsMenuAction::CycleSpeed),
        GAD_SLOT_BACK => Some(GadgetsMenuAction::BackToGames),
        _ => None,
    }
}

pub fn apply_lobby_hotbar(player: &Player, claims: Option<&UserClaims>, prefs: &PlayerPrefs) {
    let compass = ItemStack::new(Material::Compass, 1)
        .name("§6§lMenu des Jeux §7(Clic droit)")
        .lore([
            "§7Naviguez entre les mini-jeux",
            "§7et les zones du Hub FerrisCraft.",
        ]);

    let sheepwars = ItemStack::new(Material::WhiteWool, 1)
        .name("§d§lSheepWars §7(Accès direct)")
        .lore([
            "§7Rejoignez directement la bataille",
            "§7des moutons explosifs et galactiques !",
        ])
        .enchant(Enchantment::Unbreaking, 1);

    let account = match claims {
        Some(user) => ItemStack::new(Material::Emerald, 1)
            .name(format!("§a§lFerrisKey : {} §7(✔)", user.username))
            .lore([
                "§7Votre identité FerrisKey est active.",
                "§eClic droit §7pour voir votre profil.",
            ])
            .enchant(Enchantment::Unbreaking, 1),
        None => ItemStack::new(Material::AmethystShard, 1)
            .name("§e§lSe connecter avec FerrisKey §7(/login)")
            .lore([
                "§7Liez votre compte personnel pour accéder",
                "§7à vos grades et aux agents Capitainerie.",
            ]),
    };

    let gadgets_chest = ItemStack::new(Material::Chest, 1)
        .name("§b§lGadgets & Cosmétiques §7(Clic droit)")
        .lore([
            "§7Équipez propulseur, canon à moutons,",
            "§7fusées, grappin, traînées et /fly !",
        ]);

    let equipped_gadget = prefs.equipped_gadget.map(LobbyGadget::hotbar_item);

    let visibility = if prefs.players_visible {
        ItemStack::new(Material::LimeDye, 1)
            .name("§a§lJoueurs : Visibles §7(Clic droit)")
            .lore(["§7Cliquez pour masquer les joueurs du hub."])
    } else {
        ItemStack::new(Material::GrayDye, 1)
            .name("§7§lJoueurs : Cachés §8(Clic droit)")
            .lore(["§7Cliquez pour réafficher les joueurs du hub."])
    };

    let options = ItemStack::new(Material::Comparator, 1)
        .name("§e§lOptions & Préférences §7(Clic droit)")
        .lore([
            "§7Vol, vitesse de déplacement, notifications",
            "§7et gestion de votre session FerrisKey.",
        ]);

    player.inventory.set(SlotItem::new(
        InventorySlot::Hotbar(HOTBAR_GAMES_SLOT),
        Some(compass),
    ));
    player.inventory.set(SlotItem::new(
        InventorySlot::Hotbar(HOTBAR_SHEEPWARS_SLOT),
        Some(sheepwars),
    ));
    player.inventory.set(SlotItem::new(
        InventorySlot::Hotbar(HOTBAR_ACCOUNT_SLOT),
        Some(account),
    ));
    player.inventory.set(SlotItem::new(
        InventorySlot::Hotbar(HOTBAR_GADGETS_MENU_SLOT),
        Some(gadgets_chest),
    ));
    player.inventory.set(SlotItem::new(
        InventorySlot::Hotbar(HOTBAR_EQUIPPED_GADGET_SLOT),
        equipped_gadget,
    ));
    player.inventory.set(SlotItem::new(
        InventorySlot::Hotbar(HOTBAR_VISIBILITY_SLOT),
        Some(visibility),
    ));
    player.inventory.set(SlotItem::new(
        InventorySlot::Hotbar(HOTBAR_OPTIONS_SLOT),
        Some(options),
    ));
}

pub(crate) fn fill_border(inv: &CustomInventory, rows: u8, accent: Material, neutral: Material) {
    let total = u16::from(rows) * 9;
    let accent_pane = ItemStack::new(accent, 1).name("§8 ");
    let neutral_pane = ItemStack::new(neutral, 1).name("§8 ");
    for slot in 0..total {
        let row = slot / 9;
        let col = slot % 9;
        if row == 0 || row == u16::from(rows) - 1 || col == 0 || col == 8 {
            let pane =
                if (row == 0 || row == u16::from(rows) - 1) && (col == 0 || col == 4 || col == 8) {
                    accent_pane.clone()
                } else {
                    neutral_pane.clone()
                };
            inv.set(slot, Some(pane));
        }
    }
}

pub fn populate_games_menu(inv: &CustomInventory, queued_sheepwars: usize) {
    fill_border(
        inv,
        GAMES_ROWS,
        Material::PurpleStainedGlassPane,
        Material::GrayStainedGlassPane,
    );

    inv.set(
        SLOT_SHEEPWARS,
        Some(
            ItemStack::new(Material::WhiteWool, 1)
                .name("§d§l⚔ SHEEP WARS §8• §a§lOUVERT")
                .lore([
                    "§8Classique inspiré d'Epicube".to_owned(),
                    String::new(),
                    "§7Deux équipes s'affrontent d'île en île à coup".to_owned(),
                    "§7de moutons explosifs, chercheurs, glacés et".to_owned(),
                    "§7intergalactiques sur un décor destructible !".to_owned(),
                    String::new(),
                    format!("§7En file / prêts : §d{queued_sheepwars}"),
                    "§a▶ Cliquez pour rejoindre !".to_owned(),
                ])
                .enchant(Enchantment::Unbreaking, 1),
        ),
    );

    inv.set(
        SLOT_PITCHOUT,
        Some(
            ItemStack::new(Material::Bow, 1)
                .name("§b§lPitchout §8• §7Bientôt")
                .lore([
                    "§7Éjectez vos adversaires hors de l'arène",
                    "§7à l'arc et à la pelle recul MAX !",
                    "",
                    "§e◌ En cours de développement",
                ]),
        ),
    );

    inv.set(
        SLOT_FK,
        Some(
            ItemStack::new(Material::IronSword, 1)
                .name("§e§lFallen Kingdoms §8• §7Bientôt")
                .lore([
                    "§7Fortifiez votre base, préparez vos assauts",
                    "§7et détruisez la salle des coffres adverse.",
                    "",
                    "§e◌ En cours de développement",
                ]),
        ),
    );

    inv.set(
        SLOT_PVPSWAP,
        Some(
            ItemStack::new(Material::EnderPearl, 1)
                .name("§5§lPvP Swap §8• §7Bientôt")
                .lore([
                    "§7Téléportations imprévisibles entre arènes,",
                    "§7pièges et affrontement final !",
                    "",
                    "§e◌ En cours de développement",
                ]),
        ),
    );

    inv.set(
        SLOT_MOUTRON,
        Some(
            ItemStack::new(Material::LightBlueWool, 1)
                .name("§3§lMoutron §8• §7Bientôt")
                .lore([
                    "§7Course lumineuse à dos de mouton turbo :",
                    "§7ne coupez pas les traînées adverses !",
                    "",
                    "§e◌ En cours de développement",
                ]),
        ),
    );

    inv.set(
        SLOT_OPEN_PROFILE,
        Some(
            ItemStack::new(Material::Emerald, 1)
                .name("§6§l✦ Profil")
                .lore([
                    "§7Level réseau, level et statistiques",
                    "§7de chaque mini-jeu.",
                ]),
        ),
    );
    inv.set(
        SLOT_OPEN_SHOP,
        Some(
            ItemStack::new(Material::GoldIngot, 1)
                .name("§a§l✦ Boutique")
                .lore([
                    "§7Dépensez vos coins en gadgets, traînées",
                    "§7et effets de victoire.",
                ]),
        ),
    );
    inv.set(
        SLOT_OPEN_GADGETS,
        Some(
            ItemStack::new(Material::Chest, 1)
                .name("§b§l✦ Gadgets & Cosmétiques")
                .lore([
                    "§7Propulseur, Canon à Moutons, Fusée,",
                    "§7Grappin, Traînées et Vol (/fly) !",
                ]),
        ),
    );
    inv.set(
        SLOT_TP_SPAWN,
        Some(
            ItemStack::new(Material::GoldIngot, 1)
                .name("§6§l✦ Parvis & Galerie des Jeux")
                .lore(["§7Téléportation en haut du tapis rouge (Sud)."]),
        ),
    );
    inv.set(
        SLOT_TP_PORTAL,
        Some(
            ItemStack::new(Material::EnderPearl, 1)
                .name("§5§l✦ Place Centrale & Vortex")
                .lore(["§7Téléportation devant le Vortex Pourpre."]),
        ),
    );
    inv.set(
        SLOT_TP_CAPITAINERIE,
        Some(
            ItemStack::new(Material::Spyglass, 1)
                .name("§b§l✦ Esplanade de la Capitainerie")
                .lore([
                    "§7Rencontrez le Capitaine et les agents IA",
                    "§7avec votre compte FerrisKey.",
                ]),
        ),
    );
    inv.set(
        SLOT_TP_CASTLE,
        Some(
            ItemStack::new(Material::StoneBricks, 1)
                .name("§e§l✦ Remparts du Château")
                .lore(["§7Panorama sur la cité et les dirigeables."]),
        ),
    );
    inv.set(
        SLOT_OPEN_OPTIONS,
        Some(
            ItemStack::new(Material::Comparator, 1)
                .name("§e§l⚙ Mes Options & Compte FerrisKey")
                .lore(["§7Ouvrir vos préférences personnelles."]),
        ),
    );
}

pub fn populate_options_menu(
    inv: &CustomInventory,
    prefs: &PlayerPrefs,
    claims: Option<&UserClaims>,
) {
    fill_border(
        inv,
        OPTIONS_ROWS,
        Material::OrangeStainedGlassPane,
        Material::GrayStainedGlassPane,
    );

    let vis_item = if prefs.players_visible {
        ItemStack::new(Material::LimeDye, 1)
            .name("§a§lVisibilité des joueurs : ACTIVÉE")
            .lore([
                "§7Vous voyez tous les joueurs présents",
                "§7dans le Hub FerrisCraft.",
                "",
                "§e▶ Cliquez pour masquer",
            ])
    } else {
        ItemStack::new(Material::GrayDye, 1)
            .name("§c§lVisibilité des joueurs : DÉSACTIVÉE")
            .lore([
                "§7Les autres joueurs sont masqués",
                "§7pour une navigation plus fluide.",
                "",
                "§a▶ Cliquez pour afficher",
            ])
    };
    inv.set(OPT_SLOT_VISIBILITY, Some(vis_item));

    let flight_item = if prefs.flight_enabled {
        ItemStack::new(Material::Feather, 1)
            .name("§b§lVol dans le Lobby : ACTIVÉ")
            .lore([
                "§7Double-espace pour survoler la cité et les dirigeables !",
                "",
                "§e▶ Cliquez pour désactiver",
            ])
            .enchant(Enchantment::Unbreaking, 1)
    } else {
        ItemStack::new(Material::Feather, 1)
            .name("§7§lVol dans le Lobby : DÉSACTIVÉ")
            .lore([
                "§7Activez le vol pour explorer le château,",
                "§7les tours et la flotte de dirigeables.",
                "",
                "§a▶ Cliquez pour activer",
            ])
    };
    inv.set(OPT_SLOT_FLIGHT, Some(flight_item));

    let speed_mat = match prefs.speed_tier {
        crate::prefs::SpeedTier::Normal => Material::Sugar,
        crate::prefs::SpeedTier::Fast | crate::prefs::SpeedTier::Sprint => Material::RabbitFoot,
    };
    inv.set(
        OPT_SLOT_SPEED,
        Some(
            ItemStack::new(speed_mat, 1)
                .name(format!("§e§lVitesse : {}", prefs.speed_tier.label()))
                .lore([
                    "§7Ajuste votre vitesse de marche dans le Hub.",
                    "",
                    "§e▶ Cliquez pour changer de palier",
                ]),
        ),
    );

    let sound_item = if prefs.chat_sounds {
        ItemStack::new(Material::Bell, 1)
            .name("§a§lSons & Mentions Tchat : ACTIVÉS")
            .lore([
                "§7Joue une note lorsque quelqu'un",
                "§7mentionne @VotrePseudo dans le tchat.",
                "",
                "§e▶ Cliquez pour couper",
            ])
    } else {
        ItemStack::new(Material::NoteBlock, 1)
            .name("§7§lSons & Mentions Tchat : COUPÉS")
            .lore([
                "§7Aucun signal sonore sur les @mentions.",
                "",
                "§a▶ Cliquez pour réactiver",
            ])
    };
    inv.set(OPT_SLOT_CHAT_SOUNDS, Some(sound_item));

    let fk_item = match claims {
        Some(user) => {
            let roles = if user.roles.is_empty() {
                "membre".to_owned()
            } else {
                user.roles.join(", ")
            };
            ItemStack::new(Material::Emerald, 1)
                .name(format!("§a§l✔ Compte FerrisKey : {}", user.username))
                .lore([
                    format!(
                        "§7E-mail : §f{}",
                        user.email.as_deref().unwrap_or("non renseigné")
                    ),
                    format!("§7Rôles : §b{roles}"),
                    String::new(),
                    "§7Utilisé automatiquement pour vos".to_owned(),
                    "§7actions Capitainerie et votre grade.".to_owned(),
                    String::new(),
                    "§c▶ Cliquez pour délier (/logout)".to_owned(),
                ])
                .enchant(Enchantment::Unbreaking, 1)
        }
        None => ItemStack::new(Material::AmethystShard, 1)
            .name("§e§l🔐 Compte FerrisKey : Non lié")
            .lore([
                "§7Connectez votre compte FerrisKey sans",
                "§7saisir de mot de passe en jeu (Device Flow).",
                "",
                "§a▶ Cliquez pour générer un lien de connexion",
            ]),
    };
    inv.set(OPT_SLOT_FERRISKEY, Some(fk_item));

    inv.set(
        OPT_SLOT_BACK,
        Some(
            ItemStack::new(Material::Compass, 1)
                .name("§6§l← Retour au Menu des Jeux")
                .lore(["§7Revenir à la carte de navigation."]),
        ),
    );
}

pub fn populate_gadgets_menu(
    inv: &CustomInventory,
    prefs: &PlayerPrefs,
    profile: Option<&PlayerProfile>,
) {
    fill_border(
        inv,
        GADGETS_ROWS,
        Material::LightBlueStainedGlassPane,
        Material::GrayStainedGlassPane,
    );

    let gadget_slots = [
        (GAD_SLOT_FEATHER, LobbyGadget::FeatherDash),
        (GAD_SLOT_ROCKET, LobbyGadget::FireworkRocket),
        (GAD_SLOT_SHEEP, LobbyGadget::SheepCannon),
        (GAD_SLOT_VORTEX, LobbyGadget::VortexWand),
        (GAD_SLOT_WIND, LobbyGadget::WindCharge),
        (GAD_SLOT_ENDER, LobbyGadget::EnderGrapple),
    ];

    for (slot, gadget) in gadget_slots {
        let equipped = prefs.equipped_gadget == Some(gadget);
        let usable = gadget_usable(profile, gadget);
        let [line1, line2] = gadget.description();
        let cd_secs = gadget.cooldown().as_secs_f32();
        let action_line = if equipped {
            "§a✔ Actuellement dans votre main (slot 7)".to_owned()
        } else if usable {
            "§e▶ Cliquez pour équiper".to_owned()
        } else {
            let requirement = Catalog::entry(gadget.cosmetic())
                .map_or_else(|| "Verrouillé".to_owned(), |e| unlock_label(e.unlock));
            format!("§c✖ {requirement} §8(/boutique)")
        };
        let mut item = ItemStack::new(gadget.material(), 1)
            .name(if equipped {
                format!("{} §a§l[ÉQUIPÉ]", gadget.title())
            } else if usable {
                gadget.title().to_owned()
            } else {
                format!("{} §c§l[VERROUILLÉ]", gadget.title())
            })
            .lore([
                line1.to_owned(),
                line2.to_owned(),
                String::new(),
                format!("§7Recharge : §f{cd_secs:.1}s"),
                action_line,
            ]);
        if equipped {
            item = item.enchant(Enchantment::Unbreaking, 1);
        }
        inv.set(slot, Some(item));
    }

    inv.set(
        GAD_SLOT_UNEQUIP,
        Some(
            ItemStack::new(Material::Barrier, 1)
                .name("§c§lRanger le gadget")
                .lore([
                    "§7Libère le slot 7 de votre barre.",
                    "",
                    if prefs.equipped_gadget.is_none() {
                        "§7Aucun gadget équipé."
                    } else {
                        "§e▶ Cliquez pour déséquiper"
                    },
                ]),
        ),
    );

    let flight_item = if prefs.flight_enabled {
        ItemStack::new(Material::Elytra, 1)
            .name("§b§lVol dans le Lobby : ACTIVÉ")
            .lore([
                "§7Double-espace pour voler librement !",
                "",
                "§e▶ Cliquez pour désactiver (/fly)",
            ])
            .enchant(Enchantment::Unbreaking, 1)
    } else {
        ItemStack::new(Material::Elytra, 1)
            .name("§7§lVol dans le Lobby : DÉSACTIVÉ")
            .lore([
                "§7Survolez le château et la cité.",
                "",
                "§a▶ Cliquez pour activer (/fly)",
            ])
    };
    inv.set(GAD_SLOT_FLIGHT, Some(flight_item));

    let trail_item = ItemStack::new(prefs.aura_trail.material(), 1)
        .name(format!("§d§lTraînée : {}", prefs.aura_trail.label()))
        .lore([
            "§7Diffuse des particules derrière vous",
            "§7lorsque vous explorez le Hub.",
            "",
            "§e▶ Cliquez pour changer de traînée",
            "§8Les traînées verrouillées sont dans /boutique",
        ]);
    inv.set(GAD_SLOT_TRAIL, Some(trail_item));

    let speed_mat = match prefs.speed_tier {
        crate::prefs::SpeedTier::Normal => Material::Sugar,
        crate::prefs::SpeedTier::Fast | crate::prefs::SpeedTier::Sprint => Material::RabbitFoot,
    };
    inv.set(
        GAD_SLOT_SPEED,
        Some(
            ItemStack::new(speed_mat, 1)
                .name(format!("§e§lVitesse : {}", prefs.speed_tier.label()))
                .lore([
                    "§7Ajuste marche et vitesse de vol (/speed).",
                    "",
                    "§e▶ Cliquez pour changer de palier",
                ]),
        ),
    );

    inv.set(
        GAD_SLOT_BACK,
        Some(
            ItemStack::new(Material::Compass, 1)
                .name("§6§l← Retour au Menu des Jeux")
                .lore(["§7Revenir à la carte de navigation."]),
        ),
    );
}

type ServerMenuIds = Vec<Option<String>>;

#[derive(Default)]
pub struct MenuManager {
    games_menu: RwLock<Option<CustomInventory>>,
    options_menus: RwLock<HashMap<Uuid, CustomInventory>>,
    gadgets_menus: RwLock<HashMap<Uuid, CustomInventory>>,
    servers_menus: RwLock<HashMap<Uuid, (CustomInventory, ServerMenuIds)>>,
    progress_menus: RwLock<HashMap<Uuid, (CustomInventory, ProgressMenuKind)>>,
}

impl MenuManager {
    pub async fn init(&self, server: &Server) -> Result<(), Error> {
        let inv = server
            .create_inventory(GAMES_ROWS, "§5§lFerrisCraft §8• §fMini-Jeux")
            .await?;
        populate_games_menu(&inv, 0);
        *self.games_menu.write().unwrap_or_else(|e| e.into_inner()) = Some(inv);
        Ok(())
    }

    pub fn games_menu(&self) -> Option<CustomInventory> {
        self.games_menu
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn refresh_games_menu(&self, queued_sheepwars: usize) {
        if let Some(inv) = self.games_menu() {
            populate_games_menu(&inv, queued_sheepwars);
        }
    }

    pub async fn open_games(&self, player: &Player) -> Result<(), Error> {
        if let Some(inv) = self.games_menu() {
            player.inventory.open(&inv).await?;
        }
        Ok(())
    }

    pub async fn open_options(
        &self,
        server: &Server,
        player: &Player,
        prefs: &PlayerPrefs,
        claims: Option<&UserClaims>,
    ) -> Result<(), Error> {
        self.discard_options(player.uuid);
        let inv = server
            .create_inventory(OPTIONS_ROWS, "§6§lFerrisCraft §8• §fOptions")
            .await?;
        populate_options_menu(&inv, prefs, claims);
        self.options_menus
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(player.uuid, inv.clone());
        player.inventory.open(&inv).await
    }

    pub fn refresh_open_options(
        &self,
        player_uuid: Uuid,
        prefs: &PlayerPrefs,
        claims: Option<&UserClaims>,
    ) {
        let inv = self
            .options_menus
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(&player_uuid)
            .cloned();
        if let Some(inv) = inv {
            populate_options_menu(&inv, prefs, claims);
        }
    }

    pub async fn open_gadgets(
        &self,
        server: &Server,
        player: &Player,
        prefs: &PlayerPrefs,
        profile: Option<&PlayerProfile>,
    ) -> Result<(), Error> {
        self.discard_gadgets(player.uuid);
        let inv = server
            .create_inventory(GADGETS_ROWS, "§b§lFerrisCraft §8• §fGadgets")
            .await?;
        populate_gadgets_menu(&inv, prefs, profile);
        self.gadgets_menus
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(player.uuid, inv.clone());
        player.inventory.open(&inv).await
    }

    pub fn refresh_open_gadgets(
        &self,
        player_uuid: Uuid,
        prefs: &PlayerPrefs,
        profile: Option<&PlayerProfile>,
    ) {
        let inv = self
            .gadgets_menus
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(&player_uuid)
            .cloned();
        if let Some(inv) = inv {
            populate_gadgets_menu(&inv, prefs, profile);
        }
    }

    pub async fn open_progress(
        &self,
        server: &Server,
        player: &Player,
        kind: ProgressMenuKind,
        profile: Option<&PlayerProfile>,
    ) -> Result<(), Error> {
        self.discard_progress(player.uuid);
        let inv = server.create_inventory(kind.rows(), kind.title()).await?;
        populate_progress_menu(&inv, kind, profile);
        self.progress_menus
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(player.uuid, (inv.clone(), kind));
        player.inventory.open(&inv).await
    }

    pub fn progress_menu_kind(
        &self,
        player_uuid: Uuid,
        inv_ref: Option<&InventoryRef>,
    ) -> Option<ProgressMenuKind> {
        let custom = inv_ref.and_then(InventoryRef::custom)?;
        self.progress_menus
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(&player_uuid)
            .filter(|(known, _)| known == custom)
            .map(|(_, kind)| *kind)
    }

    pub fn show_progress_page(
        &self,
        player_uuid: Uuid,
        kind: ProgressMenuKind,
        profile: Option<&PlayerProfile>,
    ) {
        let mut guard = self
            .progress_menus
            .write()
            .unwrap_or_else(|e| e.into_inner());
        if let Some((inv, current)) = guard.get_mut(&player_uuid) {
            *current = kind;
            populate_progress_menu(inv, kind, profile);
        }
    }

    pub fn refresh_open_progress(&self, player_uuid: Uuid, profile: Option<&PlayerProfile>) {
        let guard = self
            .progress_menus
            .read()
            .unwrap_or_else(|e| e.into_inner());
        if let Some((inv, kind)) = guard.get(&player_uuid) {
            populate_progress_menu(inv, *kind, profile);
        }
    }

    pub fn discard_progress(&self, player_uuid: Uuid) {
        if let Some((removed, _)) = self
            .progress_menus
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&player_uuid)
        {
            removed.discard();
        }
    }

    pub async fn open_servers(
        &self,
        server: &Server,
        player: &Player,
        lobby_version: Option<&str>,
        snapshot: &FleetStatusSnapshot,
    ) -> Result<(), Error> {
        self.discard_servers(player.uuid);
        let inv = server
            .create_inventory(SERVERS_ROWS, "§d§lFerrisCraft §8• §fServeurs")
            .await?;
        populate_servers_menu(
            &inv,
            lobby_version,
            snapshot.orchestrator_version.as_deref(),
            snapshot.proxy_version.as_deref(),
            &snapshot.servers,
            &snapshot.lobbies,
        );
        self.servers_menus
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(
                player.uuid,
                (
                    inv.clone(),
                    sorted_server_ids(&snapshot.servers, &snapshot.lobbies),
                ),
            );
        player.inventory.open(&inv).await
    }

    pub fn servers_menu_server_ids(
        &self,
        player_uuid: Uuid,
        inv_ref: Option<&InventoryRef>,
    ) -> Option<ServerMenuIds> {
        let custom = inv_ref.and_then(InventoryRef::custom)?;
        self.servers_menus
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(&player_uuid)
            .filter(|(known, _)| known == custom)
            .map(|(_, ids)| ids.clone())
    }

    pub fn discard_servers(&self, player_uuid: Uuid) {
        if let Some((removed, _)) = self
            .servers_menus
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&player_uuid)
        {
            removed.discard();
        }
    }

    pub fn is_games_inventory(&self, inv_ref: Option<&InventoryRef>) -> bool {
        let Some(custom) = inv_ref.and_then(InventoryRef::custom) else {
            return false;
        };
        self.games_menu().as_ref() == Some(custom)
    }

    pub fn is_player_options_inventory(
        &self,
        player_uuid: Uuid,
        inv_ref: Option<&InventoryRef>,
    ) -> bool {
        let Some(custom) = inv_ref.and_then(InventoryRef::custom) else {
            return false;
        };
        self.options_menus
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(&player_uuid)
            .is_some_and(|known| known == custom)
    }

    pub fn is_player_gadgets_inventory(
        &self,
        player_uuid: Uuid,
        inv_ref: Option<&InventoryRef>,
    ) -> bool {
        let Some(custom) = inv_ref.and_then(InventoryRef::custom) else {
            return false;
        };
        self.gadgets_menus
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(&player_uuid)
            .is_some_and(|known| known == custom)
    }

    pub fn handle_close(&self, player_uuid: Uuid, closed: &InventoryRef) {
        let Some(custom) = closed.custom() else {
            return;
        };
        let mut opt_guard = self
            .options_menus
            .write()
            .unwrap_or_else(|e| e.into_inner());
        if opt_guard.get(&player_uuid) == Some(custom)
            && let Some(removed) = opt_guard.remove(&player_uuid)
        {
            removed.discard();
        }
        drop(opt_guard);

        let mut gad_guard = self
            .gadgets_menus
            .write()
            .unwrap_or_else(|e| e.into_inner());
        if gad_guard.get(&player_uuid) == Some(custom)
            && let Some(removed) = gad_guard.remove(&player_uuid)
        {
            removed.discard();
        }
        drop(gad_guard);

        let mut srv_guard = self
            .servers_menus
            .write()
            .unwrap_or_else(|e| e.into_inner());
        if srv_guard.get(&player_uuid).map(|(inv, _)| inv) == Some(custom)
            && let Some((removed, _)) = srv_guard.remove(&player_uuid)
        {
            removed.discard();
        }
        drop(srv_guard);

        let mut prg_guard = self
            .progress_menus
            .write()
            .unwrap_or_else(|e| e.into_inner());
        if prg_guard.get(&player_uuid).map(|(inv, _)| inv) == Some(custom)
            && let Some((removed, _)) = prg_guard.remove(&player_uuid)
        {
            removed.discard();
        }
    }

    pub fn discard_options(&self, player_uuid: Uuid) {
        if let Some(removed) = self
            .options_menus
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&player_uuid)
        {
            removed.discard();
        }
    }

    pub fn discard_gadgets(&self, player_uuid: Uuid) {
        if let Some(removed) = self
            .gadgets_menus
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&player_uuid)
        {
            removed.discard();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn games_options_and_gadgets_slots_decode_to_expected_actions() {
        assert_eq!(
            decode_games_click(i32::from(SLOT_SHEEPWARS)),
            Some(GamesMenuAction::JoinSheepWars)
        );
        assert_eq!(
            decode_games_click(i32::from(SLOT_OPEN_GADGETS)),
            Some(GamesMenuAction::OpenGadgets)
        );
        assert_eq!(
            decode_games_click(i32::from(SLOT_TP_PORTAL)),
            Some(GamesMenuAction::TeleportPoi("portal"))
        );
        assert_eq!(
            decode_games_click(i32::from(SLOT_OPEN_OPTIONS)),
            Some(GamesMenuAction::OpenOptions)
        );
        assert_eq!(
            decode_games_click(i32::from(SLOT_OPEN_PROFILE)),
            Some(GamesMenuAction::OpenProfile)
        );
        assert_eq!(
            decode_games_click(i32::from(SLOT_OPEN_SHOP)),
            Some(GamesMenuAction::OpenShop)
        );
        assert_eq!(decode_games_click(0), None);

        assert_eq!(
            decode_options_click(i32::from(OPT_SLOT_VISIBILITY)),
            Some(OptionsMenuAction::ToggleVisibility)
        );
        assert_eq!(
            decode_options_click(i32::from(OPT_SLOT_FLIGHT)),
            Some(OptionsMenuAction::ToggleFlight)
        );
        assert_eq!(
            decode_options_click(i32::from(OPT_SLOT_SPEED)),
            Some(OptionsMenuAction::CycleSpeed)
        );
        assert_eq!(
            decode_options_click(i32::from(OPT_SLOT_FERRISKEY)),
            Some(OptionsMenuAction::FerrisKeyAction)
        );
        assert_eq!(
            decode_options_click(i32::from(OPT_SLOT_BACK)),
            Some(OptionsMenuAction::BackToGames)
        );

        assert_eq!(
            decode_gadgets_click(i32::from(GAD_SLOT_FEATHER)),
            Some(GadgetsMenuAction::Equip(LobbyGadget::FeatherDash))
        );
        assert_eq!(
            decode_gadgets_click(i32::from(GAD_SLOT_SHEEP)),
            Some(GadgetsMenuAction::Equip(LobbyGadget::SheepCannon))
        );
        assert_eq!(
            decode_gadgets_click(i32::from(GAD_SLOT_TRAIL)),
            Some(GadgetsMenuAction::CycleTrail)
        );
        assert_eq!(
            decode_gadgets_click(i32::from(GAD_SLOT_UNEQUIP)),
            Some(GadgetsMenuAction::UnequipGadget)
        );
    }
}
