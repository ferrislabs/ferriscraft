use std::collections::{BTreeMap, HashSet};
use std::time::{Duration, Instant};

use k8s_openapi::api::core::v1::{
    Container, ContainerPort, EmptyDirVolumeSource, EnvVar, Pod, PodSpec, PodTemplateSpec, Probe,
    SecretVolumeSource, TCPSocketAction, Volume, VolumeMount,
};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;
use kube::Client;
use kube::api::{Api, DeleteParams, ListParams, PostParams};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::api::AppState;
use crate::config::OrchestratorConfig;
use crate::engine::ServerDeletion;

use crate::s3::{MapCatalog, S3Client, hex_lower, sanitize_k8s_label};

pub const TEMPLATE_HASH_LABEL: &str = "ferriscraft.io/template-hash";
const TEMPLATE_HASH_LEN: usize = 12;

const SHEEPWARS_SELECTOR: &str =
    "app.kubernetes.io/component=sheepwars,app.kubernetes.io/managed-by=ferriscraft-orchestrator";

pub fn load_pod_template(config: &OrchestratorConfig) -> Option<PodTemplateSpec> {
    let path = config.pod_template_path.as_ref()?;
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(err) => {
            tracing::warn!(?path, %err, "Impossible de lire le template de Pod SheepWars");
            return None;
        }
    };
    match serde_yaml::from_str::<PodTemplateSpec>(&raw) {
        Ok(tpl) => Some(tpl),
        Err(err) => {
            tracing::warn!(?path, %err, "YAML/JSON PodTemplateSpec SheepWars invalide");
            None
        }
    }
}

pub fn template_hash(config: &OrchestratorConfig, template: Option<&PodTemplateSpec>) -> String {
    let rendered = match template {
        Some(tpl) => serde_json::to_vec(tpl),
        None => serde_json::to_vec(&default_pod_spec(config)),
    }
    .unwrap_or_default();
    let mut hash = hex_lower(&Sha256::digest(&rendered));
    hash.truncate(TEMPLATE_HASH_LEN);
    hash
}

pub fn build_sheepwars_pod(
    pod_name: &str,
    config: &OrchestratorConfig,
    template: Option<&PodTemplateSpec>,
    selected_map: Option<&str>,
) -> Pod {
    let mut labels = BTreeMap::new();
    labels.insert(
        "app.kubernetes.io/name".to_owned(),
        "ferriscraft".to_owned(),
    );
    labels.insert(
        "app.kubernetes.io/component".to_owned(),
        "sheepwars".to_owned(),
    );
    labels.insert(
        "app.kubernetes.io/managed-by".to_owned(),
        "ferriscraft-orchestrator".to_owned(),
    );
    labels.insert("ferriscraft.io/server-id".to_owned(), pod_name.to_owned());
    labels.insert(
        TEMPLATE_HASH_LABEL.to_owned(),
        template_hash(config, template),
    );
    if let Some(map_name) = selected_map {
        let label = sanitize_k8s_label(map_name);
        if !label.is_empty() {
            labels.insert("ferriscraft.io/map".to_owned(), label);
        }
    }

    let mut annotations = BTreeMap::new();
    if let Some(tpl) = template
        && let Some(meta) = &tpl.metadata
    {
        if let Some(tpl_labels) = &meta.labels {
            for (k, v) in tpl_labels {
                labels.entry(k.clone()).or_insert_with(|| v.clone());
            }
        }
        if let Some(tpl_ann) = &meta.annotations {
            for (k, v) in tpl_ann {
                annotations.insert(k.clone(), v.clone());
            }
        }
    }

    let mut spec = template
        .and_then(|t| t.spec.clone())
        .unwrap_or_else(|| default_pod_spec(config));

    spec.restart_policy = Some("Never".to_owned());

    if let Some(init_containers) = &mut spec.init_containers {
        for container in init_containers {
            if let Some(map_name) = selected_map {
                upsert_env(&mut container.env, "SHEEPWARS_MAP_NAME", map_name);
            }
        }
    }

    for container in &mut spec.containers {
        upsert_env(&mut container.env, "SERVER_ID", pod_name);
        upsert_env(
            &mut container.env,
            "ORCHESTRATOR_URL",
            &config.orchestrator_public_url,
        );
        upsert_env(
            &mut container.env,
            "SHEEPWARS_MAX_PLAYERS",
            &config.max_players_per_server.to_string(),
        );
        upsert_env(
            &mut container.env,
            "SERVER_PORT",
            &config.sheepwars_port.to_string(),
        );
        if let Some(map_name) = selected_map {
            upsert_env(&mut container.env, "SHEEPWARS_MAP_NAME", map_name);
        }
    }

    Pod {
        metadata: ObjectMeta {
            name: Some(pod_name.to_owned()),
            namespace: Some(config.k8s_namespace.clone()),
            labels: Some(labels),
            annotations: if annotations.is_empty() {
                None
            } else {
                Some(annotations)
            },
            ..Default::default()
        },
        spec: Some(spec),
        status: None,
    }
}

fn upsert_env(envs: &mut Option<Vec<EnvVar>>, name: &str, value: &str) {
    let list = envs.get_or_insert_with(Vec::new);
    if let Some(existing) = list.iter_mut().find(|e| e.name == name) {
        existing.value = Some(value.to_owned());
        existing.value_from = None;
    } else {
        list.push(EnvVar {
            name: name.to_owned(),
            value: Some(value.to_owned()),
            ..Default::default()
        });
    }
}

fn default_pod_spec(config: &OrchestratorConfig) -> PodSpec {
    PodSpec {
        restart_policy: Some("Never".to_owned()),
        containers: vec![Container {
            name: "rspigot".to_owned(),
            image: Some(config.sheepwars_image.clone()),
            ports: Some(vec![
                ContainerPort {
                    name: Some("minecraft".to_owned()),
                    container_port: i32::from(config.sheepwars_port),
                    ..Default::default()
                },
                ContainerPort {
                    name: Some("grpc-plugin".to_owned()),
                    container_port: 25581,
                    ..Default::default()
                },
            ]),
            readiness_probe: Some(Probe {
                tcp_socket: Some(TCPSocketAction {
                    port: IntOrString::Int(i32::from(config.sheepwars_port)),
                    host: None,
                }),
                initial_delay_seconds: Some(2),
                period_seconds: Some(2),
                ..Default::default()
            }),
            volume_mounts: Some(vec![
                VolumeMount {
                    name: "forwarding-secret".to_owned(),
                    mount_path: "/etc/ferriscraft/forwarding".to_owned(),
                    read_only: Some(true),
                    ..Default::default()
                },
                VolumeMount {
                    name: "world-data".to_owned(),
                    mount_path: "/data".to_owned(),
                    ..Default::default()
                },
            ]),
            ..Default::default()
        }],
        volumes: Some(vec![
            Volume {
                name: "forwarding-secret".to_owned(),
                secret: Some(SecretVolumeSource {
                    secret_name: Some(config.forwarding_secret_name.clone()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            Volume {
                name: "world-data".to_owned(),
                empty_dir: Some(EmptyDirVolumeSource::default()),
                ..Default::default()
            },
        ]),
        ..Default::default()
    }
}

fn is_pod_ready_condition(pod: &Pod) -> bool {
    pod.status
        .as_ref()
        .and_then(|s| s.conditions.as_ref())
        .is_some_and(|conds| {
            conds
                .iter()
                .any(|c| c.type_ == "Ready" && c.status == "True")
        })
}

fn is_pod_terminating_or_done(pod: &Pod) -> bool {
    if pod.metadata.deletion_timestamp.is_some() {
        return true;
    }
    matches!(
        pod.status.as_ref().and_then(|s| s.phase.as_deref()),
        Some("Succeeded" | "Failed")
    )
}

struct MapDiscoveryState {
    client: S3Client,
    catalog: MapCatalog,
    last_refresh: Option<Instant>,
}

impl MapDiscoveryState {
    fn new(client: S3Client) -> Self {
        Self {
            client,
            catalog: MapCatalog::default(),
            last_refresh: None,
        }
    }

    async fn refresh_if_due(&mut self, ttl: Duration) {
        let now = Instant::now();
        let retry_interval = if self.catalog.maps().is_empty() {
            ttl.min(Duration::from_secs(3))
        } else {
            ttl
        };
        if let Some(last) = self.last_refresh
            && now.duration_since(last) < retry_interval
        {
            return;
        }
        self.last_refresh = Some(now);
        match self.client.list_maps().await {
            Ok(maps) => {
                if self.catalog.update(maps) {
                    tracing::info!(
                        maps = ?self.catalog.maps(),
                        bucket = %self.client.config().bucket,
                        "Catalogue des maps SheepWars mis à jour depuis S3"
                    );
                }
            }
            Err(err) => {
                tracing::warn!(%err, "Impossible de lister les maps S3");
            }
        }
    }
}

pub async fn run_k8s_reconcile_loop(state: AppState, config: OrchestratorConfig) {
    let client = match Client::try_default().await {
        Ok(c) => c,
        Err(err) => {
            tracing::error!(%err, "Impossible d'initialiser le client Kubernetes");
            return;
        }
    };
    let pods_api: Api<Pod> = Api::namespaced(client, &config.k8s_namespace);
    let template = load_pod_template(&config);
    let current_hash = template_hash(&config, template.as_ref());

    let mut map_discovery = config
        .s3
        .as_ref()
        .and_then(|s3_cfg| match S3Client::new(s3_cfg.clone()) {
            Ok(client) => Some(MapDiscoveryState::new(client)),
            Err(err) => {
                tracing::error!(%err, "Configuration endpoint S3 invalide");
                None
            }
        });

    tracing::info!(
        namespace = %config.k8s_namespace,
        min_replicas = config.min_replicas,
        max_replicas = config.max_replicas,
        queue_min_players = config.queue_min_players,
        s3_enabled = map_discovery.is_some(),
        fixed_map = ?config.fixed_map,
        template_hash = %current_hash,
        "Réconciliateur Kubernetes SheepWars démarré"
    );

    let mut ticker = tokio::time::interval(config.reconcile_interval);
    loop {
        ticker.tick().await;
        if let Some(discovery) = map_discovery.as_mut() {
            discovery.refresh_if_due(config.s3_cache_ttl).await;
        }
        if let Err(err) = reconcile_once(
            &state,
            &config,
            &pods_api,
            template.as_ref(),
            &current_hash,
            map_discovery.as_mut(),
        )
        .await
        {
            tracing::warn!(%err, "Erreur lors de la réconciliation Kubernetes");
        }
    }
}

async fn reconcile_once(
    state: &AppState,
    config: &OrchestratorConfig,
    pods_api: &Api<Pod>,
    template: Option<&PodTemplateSpec>,
    current_hash: &str,
    mut map_discovery: Option<&mut MapDiscoveryState>,
) -> Result<(), kube::Error> {
    let lp = ListParams::default().labels(SHEEPWARS_SELECTOR);
    let list = pods_api.list(&lp).await?;

    let mut live_ids = HashSet::new();
    let mut events = Vec::new();
    let now = Instant::now();

    for pod in list.items {
        let Some(pod_name) = pod.metadata.name.clone() else {
            continue;
        };
        let labels = pod.metadata.labels.as_ref();
        let server_id = labels
            .and_then(|l| l.get("ferriscraft.io/server-id"))
            .cloned()
            .unwrap_or_else(|| pod_name.clone());
        let outdated = labels
            .and_then(|l| l.get(TEMPLATE_HASH_LABEL))
            .is_none_or(|hash| hash != current_hash);
        live_ids.insert(server_id.clone());

        let address = pod
            .status
            .as_ref()
            .and_then(|s| s.pod_ip.as_ref())
            .map(|ip| format!("{ip}:{}", config.sheepwars_port));
        let ready = is_pod_ready_condition(&pod);
        let terminating = is_pod_terminating_or_done(&pod);

        let mut engine = state.engine.lock().unwrap_or_else(|e| e.into_inner());
        events.extend(engine.upsert_infra_server(
            now,
            server_id.clone(),
            Some(pod_name),
            address,
            ready,
            terminating,
        ));
        if outdated {
            engine.mark_outdated(&server_id);
        }
    }

    let actions = {
        let mut engine = state.engine.lock().unwrap_or_else(|e| e.into_inner());
        events.extend(engine.retain_infra_servers(&live_ids));
        let actions = engine.tick(now);
        events.extend(actions.proxy_events.clone());
        actions
    };
    state.emit_proxy_events(events);

    for ServerDeletion { server_id, reason } in actions.servers_to_delete {
        if reason.is_abnormal() {
            tracing::warn!(%server_id, ?reason, "Suppression du Pod SheepWars");
        } else {
            tracing::info!(%server_id, ?reason, "Suppression du Pod SheepWars");
        }
        if let Err(err) = pods_api.delete(&server_id, &DeleteParams::default()).await {
            tracing::warn!(%server_id, %err, "Échec suppression Pod SheepWars");
        }
    }

    for _ in 0..actions.servers_to_create {
        let selected_map: Option<String> = if let Some(fixed) = &config.fixed_map {
            Some(fixed.clone())
        } else if let Some(discovery) = map_discovery.as_deref_mut() {
            match discovery.catalog.next_map() {
                Some(map) => Some(map.to_owned()),
                None => {
                    tracing::warn!(
                        "Création de Pod SheepWars différée : aucune map disponible dans S3"
                    );
                    break;
                }
            }
        } else {
            None
        };

        let short = Uuid::new_v4().simple().to_string();
        let pod_name = format!("sheepwars-{}", &short[..6]);
        let pod = build_sheepwars_pod(&pod_name, config, template, selected_map.as_deref());
        tracing::info!(%pod_name, map = ?selected_map, "Création d'un nouveau Pod SheepWars");
        match pods_api.create(&PostParams::default(), &pod).await {
            Ok(_) => {
                let mut engine = state.engine.lock().unwrap_or_else(|e| e.into_inner());
                engine.register_creating_server(Instant::now(), pod_name.clone(), Some(pod_name));
            }
            Err(err) => {
                tracing::error!(%pod_name, %err, "Échec création Pod SheepWars");
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::BackendMode;

    fn test_config() -> OrchestratorConfig {
        OrchestratorConfig {
            bind_addr: "127.0.0.1:8080".parse().unwrap(),
            backend: BackendMode::Kubernetes,
            api_token: None,
            lobby_server_name: "lobby".to_owned(),
            queue_min_players: 1,
            max_players_per_server: 16,
            min_replicas: 1,
            max_replicas: 5,
            standby_servers: 1,
            idle_ttl: Duration::from_secs(60),
            starting_timeout: Duration::from_secs(120),
            transfer_timeout: Duration::from_secs(15),
            empty_grace: Duration::from_secs(30),
            ending_timeout: Duration::from_secs(60),
            heartbeat_timeout: Duration::from_secs(30),
            reconcile_interval: Duration::from_secs(1),
            k8s_namespace: "ferriscraft".to_owned(),
            pod_template_path: None,
            sheepwars_image: "ghcr.io/ferrislabs/ferriscraft-rspigot:dev".to_owned(),
            sheepwars_port: 25565,
            forwarding_secret_name: "ferriscraft-forwarding".to_owned(),
            orchestrator_public_url: "http://ferriscraft-orchestrator:8080".to_owned(),
            static_servers: Vec::new(),
            s3: None,
            s3_cache_ttl: Duration::from_secs(30),
            fixed_map: None,
        }
    }

    #[test]
    fn build_sheepwars_pod_injects_map_into_init_containers_and_labels() {
        let template = PodTemplateSpec {
            metadata: None,
            spec: Some(PodSpec {
                init_containers: Some(vec![Container {
                    name: "map-sync".to_owned(),
                    ..Default::default()
                }]),
                containers: vec![
                    Container {
                        name: "rspigot".to_owned(),
                        ..Default::default()
                    },
                    Container {
                        name: "sheepwars-plugin".to_owned(),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }),
        };

        let pod = build_sheepwars_pod(
            "sheepwars-abc123",
            &test_config(),
            Some(&template),
            Some("steampunk"),
        );

        let labels = pod.metadata.labels.as_ref().unwrap();
        assert_eq!(labels.get("ferriscraft.io/map").unwrap(), "steampunk");

        let spec = pod.spec.as_ref().unwrap();
        let init_env = spec.init_containers.as_ref().unwrap()[0]
            .env
            .as_ref()
            .unwrap();
        assert!(
            init_env
                .iter()
                .any(|e| e.name == "SHEEPWARS_MAP_NAME" && e.value.as_deref() == Some("steampunk"))
        );

        for container in &spec.containers {
            let envs = container.env.as_ref().unwrap();
            assert!(
                envs.iter().any(
                    |e| e.name == "SHEEPWARS_MAP_NAME" && e.value.as_deref() == Some("steampunk")
                )
            );
        }
    }

    fn plugin_template(image: &str) -> PodTemplateSpec {
        PodTemplateSpec {
            metadata: None,
            spec: Some(PodSpec {
                containers: vec![Container {
                    name: "sheepwars-plugin".to_owned(),
                    image: Some(image.to_owned()),
                    ..Default::default()
                }],
                ..Default::default()
            }),
        }
    }

    #[test]
    fn pod_is_labelled_with_the_template_hash() {
        let config = test_config();
        let template = plugin_template("plugin:v1");
        let pod = build_sheepwars_pod("sheepwars-abc123", &config, Some(&template), None);
        let labels = pod.metadata.labels.unwrap();
        assert_eq!(
            labels.get(TEMPLATE_HASH_LABEL),
            Some(&template_hash(&config, Some(&template)))
        );
    }

    #[test]
    fn template_hash_changes_with_the_image_and_is_stable() {
        let config = test_config();
        let v1 = template_hash(&config, Some(&plugin_template("plugin:v1")));
        let v2 = template_hash(&config, Some(&plugin_template("plugin:v2")));
        assert_eq!(v1.len(), TEMPLATE_HASH_LEN);
        assert_ne!(v1, v2);
        assert_eq!(
            v1,
            template_hash(&config, Some(&plugin_template("plugin:v1")))
        );
        assert_ne!(template_hash(&config, None), v1);
    }
}
