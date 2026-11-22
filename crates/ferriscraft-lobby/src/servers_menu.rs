use ferriscraft_protocol::{ServerPhase, ServerSummary};
use rspigot_plugin::{CustomInventory, ItemStack, Material};

pub const SERVERS_ROWS: u8 = 6;
pub const SERVERS_SLOT_HEADER: u16 = 4;
pub const SERVERS_FIRST_SLOT: u16 = 9;
pub const SERVERS_MAX: usize = 36;

pub fn short_version(version: &str) -> String {
    if is_full_sha(version) {
        return version[..7].to_owned();
    }
    if let Some(rest) = version.strip_prefix("pr-")
        && let Some((number, sha)) = rest.split_once('-')
        && !number.is_empty()
        && number.bytes().all(|b| b.is_ascii_digit())
        && is_full_sha(sha)
    {
        return format!("pr-{number}-{}", &sha[..7]);
    }
    version.to_owned()
}

fn is_full_sha(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|b| b.is_ascii_hexdigit())
}

pub fn short_version_or_unknown(version: Option<&str>) -> String {
    version.map_or_else(|| "?".to_owned(), short_version)
}

pub fn format_age(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m {:02}s", secs / 60, secs % 60)
    } else {
        format!("{}h {:02}m", secs / 3600, (secs % 3600) / 60)
    }
}

pub fn phase_style(phase: ServerPhase) -> (Material, &'static str) {
    match phase {
        ServerPhase::Starting => (Material::LightBlueWool, "Démarrage"),
        ServerPhase::Waiting => (Material::LimeWool, "En attente"),
        ServerPhase::Countdown => (Material::YellowWool, "Compte à rebours"),
        ServerPhase::InGame => (Material::OrangeWool, "En jeu"),
        ServerPhase::Ending => (Material::GrayWool, "Fin de partie"),
    }
}

pub fn decode_servers_slot(slot: i32, server_count: usize) -> Option<usize> {
    let slot = u16::try_from(slot).ok()?;
    let index = usize::from(slot.checked_sub(SERVERS_FIRST_SLOT)?);
    (index < server_count.min(SERVERS_MAX)).then_some(index)
}

pub fn sorted_server_ids(servers: &[ServerSummary]) -> Vec<String> {
    let mut ids: Vec<String> = servers.iter().map(|s| s.server_id.clone()).collect();
    ids.sort();
    ids.truncate(SERVERS_MAX);
    ids
}

fn server_item(server: &ServerSummary) -> ItemStack {
    let (phase_material, label) = phase_style(server.phase);
    let material = if server.outdated {
        Material::RedWool
    } else {
        phase_material
    };
    let name = if server.outdated {
        format!("§c{}", server.server_id)
    } else {
        format!("§f{}", server.server_id)
    };
    let mut lore = vec![
        format!("§7{label}"),
        format!("§7Joueurs §f{}/{}", server.player_count, server.max_players),
        format!(
            "§7Version §f{}",
            short_version_or_unknown(server.version.as_deref())
        ),
        format!("§7Âge §f{}", format_age(server.age_secs)),
    ];
    if server.outdated {
        lore.push("§cAncienne version".to_owned());
    }
    if server.ready {
        lore.push("§eClic : observer".to_owned());
    }
    ItemStack::new(material, 1).name(name).lore(lore)
}

pub fn populate_servers_menu(
    inv: &CustomInventory,
    lobby_version: Option<&str>,
    orchestrator_version: Option<&str>,
    servers: &[ServerSummary],
) {
    inv.set(
        SERVERS_SLOT_HEADER,
        Some(
            ItemStack::new(Material::Book, 1)
                .name("§dVersions déployées")
                .lore([
                    format!("§7Lobby §f{}", short_version_or_unknown(lobby_version)),
                    format!(
                        "§7Orchestrateur §f{}",
                        short_version_or_unknown(orchestrator_version)
                    ),
                ]),
        ),
    );
    let mut sorted: Vec<&ServerSummary> = servers.iter().collect();
    sorted.sort_by(|a, b| a.server_id.cmp(&b.server_id));
    for (index, server) in sorted.into_iter().take(SERVERS_MAX).enumerate() {
        let slot = SERVERS_FIRST_SLOT + u16::try_from(index).unwrap_or(0);
        inv.set(slot, Some(server_item(server)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA: &str = "221bc38da1571c58215910983e4ab060145106f9";

    #[test]
    fn short_version_handles_sha_pr_and_other() {
        assert_eq!(short_version(SHA), "221bc38");
        assert_eq!(short_version(&format!("pr-12-{SHA}")), "pr-12-221bc38");
        assert_eq!(short_version("dev"), "dev");
        assert_eq!(short_version("pr-x-abc"), "pr-x-abc");
        assert_eq!(short_version(&SHA[..39]), &SHA[..39]);
    }

    #[test]
    fn unknown_version_is_question_mark() {
        assert_eq!(short_version_or_unknown(None), "?");
        assert_eq!(short_version_or_unknown(Some(SHA)), "221bc38");
    }

    #[test]
    fn age_formats() {
        assert_eq!(format_age(42), "42s");
        assert_eq!(format_age(185), "3m 05s");
        assert_eq!(format_age(2 * 3600 + 7 * 60 + 30), "2h 07m");
        assert_eq!(format_age(0), "0s");
        assert_eq!(format_age(3600), "1h 00m");
    }

    #[test]
    fn phase_styles() {
        assert_eq!(
            phase_style(ServerPhase::Starting),
            (Material::LightBlueWool, "Démarrage")
        );
        assert_eq!(
            phase_style(ServerPhase::Waiting),
            (Material::LimeWool, "En attente")
        );
        assert_eq!(
            phase_style(ServerPhase::Countdown),
            (Material::YellowWool, "Compte à rebours")
        );
        assert_eq!(
            phase_style(ServerPhase::InGame),
            (Material::OrangeWool, "En jeu")
        );
        assert_eq!(
            phase_style(ServerPhase::Ending),
            (Material::GrayWool, "Fin de partie")
        );
    }

    #[test]
    fn slot_decoding() {
        assert_eq!(decode_servers_slot(9, 3), Some(0));
        assert_eq!(decode_servers_slot(11, 3), Some(2));
        assert_eq!(decode_servers_slot(12, 3), None);
        assert_eq!(decode_servers_slot(4, 3), None);
        assert_eq!(decode_servers_slot(-1, 3), None);
        assert_eq!(decode_servers_slot(45, 100), None);
        assert_eq!(decode_servers_slot(44, 100), Some(35));
    }

    fn summary(id: &str) -> ServerSummary {
        ServerSummary {
            server_id: id.to_owned(),
            pod_name: None,
            address: String::new(),
            phase: ServerPhase::Waiting,
            player_count: 0,
            max_players: 10,
            ready: true,
            heartbeat_age_secs: 0,
            age_secs: 0,
            version: None,
            outdated: false,
            kind: Default::default(),
            draining: false,
        }
    }

    #[test]
    fn ids_sorted_and_capped() {
        let servers: Vec<ServerSummary> = (0..50)
            .rev()
            .map(|i| summary(&format!("s{i:02}")))
            .collect();
        let ids = sorted_server_ids(&servers);
        assert_eq!(ids.len(), SERVERS_MAX);
        assert_eq!(ids[0], "s00");
        assert_eq!(ids[35], "s35");
    }
}
