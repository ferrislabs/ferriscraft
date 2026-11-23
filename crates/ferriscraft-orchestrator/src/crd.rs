use std::collections::BTreeMap;
use std::fmt::Debug;

use ferriscraft_protocol::{ServerPhase, ServerSummary};
use k8s_openapi::NamespaceResourceScope;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::OwnerReference;
use kube::api::ObjectMeta;
use kube::{CustomResource, Resource};
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

pub trait GameServerObject:
    Resource<DynamicType = (), Scope = NamespaceResourceScope>
    + Clone
    + Debug
    + Serialize
    + DeserializeOwned
    + Send
    + Sync
    + 'static
{
    fn create(name: &str, template_hash: &str, map: Option<&str>) -> Self;

    fn current_status(&self) -> Option<&GameServerStatus>;

    fn template_hash(&self) -> &str;
}

macro_rules! game_server_crd {
    ($spec:ident, $kind:ident, $kind_name:literal, $plural:literal, $short:literal) => {
        #[derive(CustomResource, Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
        #[kube(
            group = "ferriscraft.io",
            version = "v1alpha1",
            kind = $kind_name,
            plural = $plural,
            shortname = $short,
            namespaced,
            status = "GameServerStatus",
            printcolumn = r#"{"name":"Phase","type":"string","jsonPath":".status.phase"}"#,
            printcolumn = r#"{"name":"Players","type":"integer","jsonPath":".status.players"}"#,
            printcolumn = r#"{"name":"Version","type":"string","jsonPath":".status.version"}"#,
            printcolumn = r#"{"name":"Outdated","type":"boolean","jsonPath":".status.outdated"}"#,
            printcolumn = r#"{"name":"Age","type":"date","jsonPath":".metadata.creationTimestamp"}"#
        )]
        #[serde(rename_all = "camelCase")]
        pub struct $spec {
            pub template_hash: String,
            #[serde(default, skip_serializing_if = "Option::is_none")]
            pub map: Option<String>,
        }

        impl GameServerObject for $kind {
            fn create(name: &str, template_hash: &str, map: Option<&str>) -> Self {
                Self::new(
                    name,
                    $spec {
                        template_hash: template_hash.to_owned(),
                        map: map.map(str::to_owned),
                    },
                )
            }

            fn current_status(&self) -> Option<&GameServerStatus> {
                self.status.as_ref()
            }

            fn template_hash(&self) -> &str {
                &self.spec.template_hash
            }
        }
    };
}

game_server_crd!(
    SheepWarsServerSpec,
    SheepWarsServer,
    "SheepWarsServer",
    "sheepwarsservers",
    "sws"
);
game_server_crd!(
    TowerWarsServerSpec,
    TowerWarsServer,
    "TowerWarsServer",
    "towerwarsservers",
    "tws"
);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GameServerPhase {
    Starting,
    Waiting,
    Countdown,
    InGame,
    Ending,
}

pub type SheepWarsServerPhase = GameServerPhase;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GameServerStatus {
    pub phase: GameServerPhase,
    pub players: u32,
    pub max_players: u32,
    pub ready: bool,
    pub outdated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub phase_since: String,
}

pub type SheepWarsServerStatus = GameServerStatus;

impl From<ServerPhase> for GameServerPhase {
    fn from(phase: ServerPhase) -> Self {
        match phase {
            ServerPhase::Starting => Self::Starting,
            ServerPhase::Waiting => Self::Waiting,
            ServerPhase::Countdown => Self::Countdown,
            ServerPhase::InGame => Self::InGame,
            ServerPhase::Ending => Self::Ending,
        }
    }
}

pub fn desired_status(
    summary: &ServerSummary,
    current: Option<&GameServerStatus>,
    now: &str,
) -> GameServerStatus {
    let phase = GameServerPhase::from(summary.phase);
    let phase_since = match current {
        Some(status) if status.phase == phase => status.phase_since.clone(),
        _ => now.to_owned(),
    };
    GameServerStatus {
        phase,
        players: summary.player_count,
        max_players: summary.max_players,
        ready: summary.ready,
        outdated: summary.outdated,
        version: summary.version.clone(),
        phase_since,
    }
}

pub fn needs_patch(desired: &GameServerStatus, current: Option<&GameServerStatus>) -> bool {
    current != Some(desired)
}

pub fn owner_reference<K: GameServerObject>(server: &K) -> Option<OwnerReference> {
    let meta = server.meta();
    Some(OwnerReference {
        api_version: K::api_version(&()).into_owned(),
        kind: K::kind(&()).into_owned(),
        name: meta.name.clone()?,
        uid: meta.uid.clone()?,
        controller: Some(true),
        block_owner_deletion: None,
    })
}

pub fn new_server<K: GameServerObject>(
    name: &str,
    namespace: &str,
    labels: BTreeMap<String, String>,
    template_hash: &str,
    map: Option<&str>,
) -> K {
    let mut server = K::create(name, template_hash, map);
    *server.meta_mut() = ObjectMeta {
        name: Some(name.to_owned()),
        namespace: Some(namespace.to_owned()),
        labels: Some(labels),
        ..Default::default()
    };
    server
}

#[cfg(test)]
mod tests {
    use super::*;
    use kube::CustomResourceExt;

    fn summary(phase: ServerPhase) -> ServerSummary {
        ServerSummary {
            server_id: "sheepwars-abc123".to_owned(),
            pod_name: Some("sheepwars-abc123".to_owned()),
            address: "10.0.0.1:25565".to_owned(),
            phase,
            player_count: 3,
            max_players: 16,
            ready: true,
            heartbeat_age_secs: 0,
            age_secs: 10,
            version: Some("1.2.3".to_owned()),
            outdated: false,
            kind: Default::default(),
            draining: false,
        }
    }

    #[test]
    fn desired_status_without_current_uses_now() {
        let status = desired_status(&summary(ServerPhase::Waiting), None, "t1");
        assert_eq!(status.phase, GameServerPhase::Waiting);
        assert_eq!(status.players, 3);
        assert_eq!(status.max_players, 16);
        assert!(status.ready);
        assert_eq!(status.version.as_deref(), Some("1.2.3"));
        assert_eq!(status.phase_since, "t1");
    }

    #[test]
    fn desired_status_keeps_phase_since_when_phase_unchanged() {
        let first = desired_status(&summary(ServerPhase::Waiting), None, "t1");
        let second = desired_status(&summary(ServerPhase::Waiting), Some(&first), "t2");
        assert_eq!(second.phase_since, "t1");
        assert!(!needs_patch(&second, Some(&first)));
    }

    #[test]
    fn desired_status_resets_phase_since_on_phase_change() {
        let first = desired_status(&summary(ServerPhase::Waiting), None, "t1");
        let second = desired_status(&summary(ServerPhase::InGame), Some(&first), "t2");
        assert_eq!(second.phase, GameServerPhase::InGame);
        assert_eq!(second.phase_since, "t2");
        assert!(needs_patch(&second, Some(&first)));
    }

    #[test]
    fn needs_patch_when_status_absent_or_field_differs() {
        let desired = desired_status(&summary(ServerPhase::Waiting), None, "t1");
        assert!(needs_patch(&desired, None));
        let mut other = desired.clone();
        other.players = 4;
        assert!(needs_patch(&other, Some(&desired)));
    }

    #[test]
    fn owner_reference_points_to_the_server_as_controller() {
        let mut server =
            new_server::<SheepWarsServer>("sheepwars-abc123", "ns", BTreeMap::new(), "h", None);
        server.metadata.uid = Some("uid-1".to_owned());
        let owner = owner_reference(&server).unwrap();
        assert_eq!(owner.api_version, "ferriscraft.io/v1alpha1");
        assert_eq!(owner.kind, "SheepWarsServer");
        assert_eq!(owner.name, "sheepwars-abc123");
        assert_eq!(owner.uid, "uid-1");
        assert_eq!(owner.controller, Some(true));
        assert_eq!(owner.block_owner_deletion, None);
    }

    #[test]
    fn owner_reference_requires_uid() {
        let server =
            new_server::<SheepWarsServer>("sheepwars-abc123", "ns", BTreeMap::new(), "h", None);
        assert!(owner_reference(&server).is_none());
    }

    #[test]
    fn new_server_carries_labels_and_spec() {
        let labels = BTreeMap::from([("a".to_owned(), "b".to_owned())]);
        let server =
            new_server::<SheepWarsServer>("n", "ns", labels.clone(), "hash", Some("steampunk"));
        assert_eq!(server.metadata.name.as_deref(), Some("n"));
        assert_eq!(server.metadata.namespace.as_deref(), Some("ns"));
        assert_eq!(server.metadata.labels, Some(labels));
        assert_eq!(server.spec.template_hash, "hash");
        assert_eq!(server.spec.map.as_deref(), Some("steampunk"));
    }

    #[test]
    fn chart_crd_matches_generated_crd() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../deploy/charts/ferriscraft/templates/sheepwarsservers-crd.yaml"
        );
        let raw = std::fs::read_to_string(path).unwrap();
        let lines: Vec<&str> = raw.lines().collect();
        let body = lines[1..lines.len() - 1].join("\n");
        let mut chart: serde_json::Value = serde_yaml::from_str(&body).unwrap();
        chart["metadata"]
            .as_object_mut()
            .unwrap()
            .remove("annotations");
        assert_eq!(chart, serde_json::to_value(SheepWarsServer::crd()).unwrap());
    }

    #[test]
    fn chart_towerwars_crd_matches_generated_crd() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../deploy/charts/ferriscraft/templates/towerwarsservers-crd.yaml"
        );
        let raw = std::fs::read_to_string(path).unwrap();
        let lines: Vec<&str> = raw.lines().collect();
        let body = lines[1..lines.len() - 1].join("\n");
        let mut chart: serde_json::Value = serde_yaml::from_str(&body).unwrap();
        chart["metadata"]
            .as_object_mut()
            .unwrap()
            .remove("annotations");
        assert_eq!(chart, serde_json::to_value(TowerWarsServer::crd()).unwrap());
    }

    #[test]
    fn towerwars_crd_has_its_own_names() {
        let crd = TowerWarsServer::crd();
        assert_eq!(
            crd.metadata.name.as_deref(),
            Some("towerwarsservers.ferriscraft.io")
        );
        assert_eq!(crd.spec.group, "ferriscraft.io");
        assert_eq!(crd.spec.names.kind, "TowerWarsServer");
        assert_eq!(crd.spec.names.plural, "towerwarsservers");
        assert_eq!(crd.spec.names.short_names, Some(vec!["tws".to_owned()]));
        assert_eq!(crd.spec.versions[0].name, "v1alpha1");
    }

    #[test]
    fn both_crds_share_the_same_schema_and_columns() {
        let sheep = serde_json::to_value(SheepWarsServer::crd().spec.versions.remove(0)).unwrap();
        let tower = serde_json::to_value(TowerWarsServer::crd().spec.versions.remove(0)).unwrap();
        let body = |version: &serde_json::Value| {
            let schema = &version["schema"]["openAPIV3Schema"];
            (
                schema["properties"].clone(),
                schema["required"].clone(),
                version["additionalPrinterColumns"].clone(),
                version["subresources"].clone(),
            )
        };
        assert_eq!(body(&sheep), body(&tower));
        assert!(body(&tower).0["status"]["properties"]["phase"].is_object());
    }

    #[test]
    fn towerwars_owner_reference_points_to_a_towerwarsserver() {
        let mut server = new_server::<TowerWarsServer>(
            "towerwars-abc123",
            "ns",
            BTreeMap::new(),
            "h",
            Some("test"),
        );
        server.metadata.uid = Some("uid-2".to_owned());
        let owner = owner_reference(&server).unwrap();
        assert_eq!(owner.kind, "TowerWarsServer");
        assert_eq!(owner.api_version, "ferriscraft.io/v1alpha1");
        assert_eq!(owner.controller, Some(true));
        assert_eq!(server.spec.map.as_deref(), Some("test"));
        assert_eq!(server.template_hash(), "h");
    }

    #[test]
    fn status_helpers_are_shared_between_both_kinds() {
        let mut tower = new_server::<TowerWarsServer>("t", "ns", BTreeMap::new(), "h", None);
        assert!(tower.current_status().is_none());
        let status = desired_status(&summary(ServerPhase::Countdown), None, "t1");
        tower.status = Some(status.clone());
        assert_eq!(tower.current_status(), Some(&status));
    }
}
