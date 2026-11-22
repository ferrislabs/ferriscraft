use std::collections::BTreeMap;

use ferriscraft_protocol::{ServerPhase, ServerSummary};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::OwnerReference;
use kube::CustomResource;
use kube::api::ObjectMeta;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(CustomResource, Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[kube(
    group = "ferriscraft.io",
    version = "v1alpha1",
    kind = "SheepWarsServer",
    plural = "sheepwarsservers",
    shortname = "sws",
    namespaced,
    status = "SheepWarsServerStatus",
    printcolumn = r#"{"name":"Phase","type":"string","jsonPath":".status.phase"}"#,
    printcolumn = r#"{"name":"Players","type":"integer","jsonPath":".status.players"}"#,
    printcolumn = r#"{"name":"Version","type":"string","jsonPath":".status.version"}"#,
    printcolumn = r#"{"name":"Outdated","type":"boolean","jsonPath":".status.outdated"}"#,
    printcolumn = r#"{"name":"Age","type":"date","jsonPath":".metadata.creationTimestamp"}"#
)]
#[serde(rename_all = "camelCase")]
pub struct SheepWarsServerSpec {
    pub template_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub map: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SheepWarsServerPhase {
    Starting,
    Waiting,
    Countdown,
    InGame,
    Ending,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SheepWarsServerStatus {
    pub phase: SheepWarsServerPhase,
    pub players: u32,
    pub max_players: u32,
    pub ready: bool,
    pub outdated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub phase_since: String,
}

impl From<ServerPhase> for SheepWarsServerPhase {
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
    current: Option<&SheepWarsServerStatus>,
    now: &str,
) -> SheepWarsServerStatus {
    let phase = SheepWarsServerPhase::from(summary.phase);
    let phase_since = match current {
        Some(status) if status.phase == phase => status.phase_since.clone(),
        _ => now.to_owned(),
    };
    SheepWarsServerStatus {
        phase,
        players: summary.player_count,
        max_players: summary.max_players,
        ready: summary.ready,
        outdated: summary.outdated,
        version: summary.version.clone(),
        phase_since,
    }
}

pub fn needs_patch(
    desired: &SheepWarsServerStatus,
    current: Option<&SheepWarsServerStatus>,
) -> bool {
    current != Some(desired)
}

pub fn owner_reference(server: &SheepWarsServer) -> Option<OwnerReference> {
    Some(OwnerReference {
        api_version: "ferriscraft.io/v1alpha1".to_owned(),
        kind: "SheepWarsServer".to_owned(),
        name: server.metadata.name.clone()?,
        uid: server.metadata.uid.clone()?,
        controller: Some(true),
        block_owner_deletion: None,
    })
}

pub fn new_server(
    name: &str,
    namespace: &str,
    labels: BTreeMap<String, String>,
    template_hash: &str,
    map: Option<&str>,
) -> SheepWarsServer {
    let mut server = SheepWarsServer::new(
        name,
        SheepWarsServerSpec {
            template_hash: template_hash.to_owned(),
            map: map.map(str::to_owned),
        },
    );
    server.metadata = ObjectMeta {
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
        }
    }

    #[test]
    fn desired_status_without_current_uses_now() {
        let status = desired_status(&summary(ServerPhase::Waiting), None, "t1");
        assert_eq!(status.phase, SheepWarsServerPhase::Waiting);
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
        assert_eq!(second.phase, SheepWarsServerPhase::InGame);
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
        let mut server = new_server("sheepwars-abc123", "ns", BTreeMap::new(), "h", None);
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
        let server = new_server("sheepwars-abc123", "ns", BTreeMap::new(), "h", None);
        assert!(owner_reference(&server).is_none());
    }

    #[test]
    fn new_server_carries_labels_and_spec() {
        let labels = BTreeMap::from([("a".to_owned(), "b".to_owned())]);
        let server = new_server("n", "ns", labels.clone(), "hash", Some("steampunk"));
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
}
