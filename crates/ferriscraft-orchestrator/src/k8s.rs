use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::{Duration, Instant};

use ferriscraft_protocol::GameKind;
use k8s_openapi::api::core::v1::{
    Container, ContainerPort, EmptyDirVolumeSource, EnvVar, Pod, PodSpec, PodTemplateSpec, Probe,
    SecretVolumeSource, TCPSocketAction, Volume, VolumeMount,
};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{ObjectMeta, OwnerReference};
use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;
use kube::Client;
use kube::api::{Api, DeleteParams, ListParams, Patch, PatchParams, PostParams, PropagationPolicy};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::api::AppState;
use crate::config::{GameConfig, OrchestratorConfig};
use crate::crd::{
    GameServerObject, GameServerStatus, SheepWarsServer, TowerWarsServer, desired_status,
    needs_patch, new_server, owner_reference,
};
use crate::engine::{FleetEngine, ServerDeletion};

use crate::s3::{MapCatalog, S3Client, hex_lower, sanitize_k8s_label};

pub const TEMPLATE_HASH_LABEL: &str = "ferriscraft.io/template-hash";
const TEMPLATE_HASH_LEN: usize = 12;

const DELETE_RETRY_AFTER: Duration = Duration::from_secs(30);

pub fn load_pod_template(game: &GameConfig) -> Option<PodTemplateSpec> {
    let path = game.pod_template_path.as_ref()?;
    let name = game.spec().display_name;
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(err) => {
            tracing::warn!(?path, %err, game = name, "Impossible de lire le template de Pod");
            return None;
        }
    };
    match serde_yaml::from_str::<PodTemplateSpec>(&raw) {
        Ok(tpl) => Some(tpl),
        Err(err) => {
            tracing::warn!(?path, %err, game = name, "YAML/JSON PodTemplateSpec invalide");
            None
        }
    }
}

pub fn template_hash(game: &GameConfig, template: Option<&PodTemplateSpec>) -> String {
    let rendered = match template {
        Some(tpl) => serde_json::to_vec(tpl),
        None => serde_json::to_vec(&default_pod_spec(game)),
    }
    .unwrap_or_default();
    let mut hash = hex_lower(&Sha256::digest(&rendered));
    hash.truncate(TEMPLATE_HASH_LEN);
    hash
}

pub fn build_pod(
    pod_name: &str,
    config: &OrchestratorConfig,
    game: &GameConfig,
    template: Option<&PodTemplateSpec>,
    selected_map: Option<&str>,
) -> Pod {
    let spec_of_game = game.spec();
    let mut labels = BTreeMap::new();
    labels.insert(
        "app.kubernetes.io/name".to_owned(),
        "ferriscraft".to_owned(),
    );
    labels.insert(
        "app.kubernetes.io/component".to_owned(),
        spec_of_game.component.to_owned(),
    );
    labels.insert(
        "app.kubernetes.io/managed-by".to_owned(),
        "ferriscraft-orchestrator".to_owned(),
    );
    labels.insert("ferriscraft.io/server-id".to_owned(), pod_name.to_owned());
    labels.insert(
        TEMPLATE_HASH_LABEL.to_owned(),
        template_hash(game, template),
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
        .unwrap_or_else(|| default_pod_spec(game));

    spec.restart_policy = Some("Never".to_owned());

    let map_env = spec_of_game.env_name("MAP_NAME");
    let max_players_env = spec_of_game.env_name("MAX_PLAYERS");

    if let Some(init_containers) = &mut spec.init_containers {
        for container in init_containers {
            if let Some(map_name) = selected_map {
                upsert_env(&mut container.env, &map_env, map_name);
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
            &max_players_env,
            &game.max_players_per_server.to_string(),
        );
        upsert_env(&mut container.env, "SERVER_PORT", &game.port.to_string());
        if let Some(map_name) = selected_map {
            upsert_env(&mut container.env, &map_env, map_name);
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

fn default_pod_spec(game: &GameConfig) -> PodSpec {
    PodSpec {
        restart_policy: Some("Never".to_owned()),
        containers: vec![Container {
            name: "rspigot".to_owned(),
            image: Some(game.image.clone()),
            ports: Some(vec![
                ContainerPort {
                    name: Some("minecraft".to_owned()),
                    container_port: i32::from(game.port),
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
                    port: IntOrString::Int(i32::from(game.port)),
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
                    secret_name: Some(game.forwarding_secret_name.clone()),
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

    async fn refresh_if_due(&mut self, ttl: Duration, game: &str) {
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
                        game,
                        "Catalogue des maps mis à jour depuis S3"
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
    let mut tasks = tokio::task::JoinSet::new();
    for game in config.games() {
        let (state, config, client, game_config) =
            (state.clone(), config.clone(), client.clone(), game.clone());
        match game.game {
            GameKind::SheepWars => {
                tasks.spawn(run_game_reconcile_loop::<SheepWarsServer>(
                    state,
                    config,
                    game_config,
                    client,
                ));
            }
            GameKind::TowerWars => {
                tasks.spawn(run_game_reconcile_loop::<TowerWarsServer>(
                    state,
                    config,
                    game_config,
                    client,
                ));
            }
        }
    }
    while tasks.join_next().await.is_some() {}
}

async fn run_game_reconcile_loop<K: GameServerObject>(
    state: AppState,
    config: OrchestratorConfig,
    game: GameConfig,
    client: Client,
) {
    let spec = game.spec();
    let pods_api: Api<Pod> = Api::namespaced(client.clone(), &config.k8s_namespace);
    let servers_api: Option<Api<K>> = game
        .use_crd
        .then(|| Api::namespaced(client, &config.k8s_namespace));
    let template = load_pod_template(&game);
    let current_hash = template_hash(&game, template.as_ref());

    let mut map_discovery = config
        .s3
        .as_ref()
        .filter(|_| spec.s3_maps)
        .and_then(|s3_cfg| match S3Client::new(s3_cfg.clone()) {
            Ok(client) => Some(MapDiscoveryState::new(client)),
            Err(err) => {
                tracing::error!(%err, "Configuration endpoint S3 invalide");
                None
            }
        });

    tracing::info!(
        game = spec.display_name,
        namespace = %config.k8s_namespace,
        min_replicas = game.min_replicas,
        max_replicas = game.max_replicas,
        queue_min_players = game.queue_min_players,
        s3_enabled = map_discovery.is_some(),
        fixed_map = ?game.fixed_map,
        template_hash = %current_hash,
        "Réconciliateur Kubernetes démarré"
    );

    let mut ticker = tokio::time::interval(config.reconcile_interval);
    loop {
        ticker.tick().await;
        if let Some(discovery) = map_discovery.as_mut() {
            discovery
                .refresh_if_due(config.s3_cache_ttl, spec.display_name)
                .await;
        }
        let ctx = ReconcileCtx {
            state: &state,
            config: &config,
            game: &game,
            pods_api: &pods_api,
            servers_api: servers_api.as_ref(),
            template: template.as_ref(),
            current_hash: &current_hash,
        };
        if let Err(err) = reconcile_once(&ctx, map_discovery.as_mut()).await {
            tracing::warn!(%err, game = spec.display_name, "Erreur lors de la réconciliation Kubernetes");
        }
    }
}

struct ReconcileCtx<'a, K> {
    state: &'a AppState,
    config: &'a OrchestratorConfig,
    game: &'a GameConfig,
    pods_api: &'a Api<Pod>,
    servers_api: Option<&'a Api<K>>,
    template: Option<&'a PodTemplateSpec>,
    current_hash: &'a str,
}

impl<K> ReconcileCtx<'_, K> {
    fn with_engine<R>(&self, f: impl FnOnce(&mut FleetEngine) -> R) -> Option<R> {
        let mut games = self.state.games.lock().unwrap_or_else(|e| e.into_inner());
        games.engine_mut(self.game.game).ok().map(f)
    }
}

fn attach_owner(pod: &mut Pod, owner: Option<OwnerReference>) {
    if let Some(owner) = owner {
        pod.metadata
            .owner_references
            .get_or_insert_with(Vec::new)
            .push(owner);
    }
}

async fn list_servers<K: GameServerObject>(api: &Api<K>) -> HashMap<String, K> {
    match api.list(&ListParams::default()).await {
        Ok(list) => list
            .items
            .into_iter()
            .filter_map(|server| Some((server.meta().name.clone()?, server)))
            .collect(),
        Err(err) => {
            tracing::warn!(%err, kind = %K::kind(&()), "Impossible de lister les objets");
            HashMap::new()
        }
    }
}

async fn delete_server_object<K: GameServerObject>(api: &Api<K>, name: &str) {
    let dp = DeleteParams {
        propagation_policy: Some(PropagationPolicy::Background),
        ..Default::default()
    };
    if let Err(err) = api.delete(name, &dp).await {
        tracing::warn!(%name, %err, kind = %K::kind(&()), "Échec suppression de l'objet");
    }
}

async fn delete_server<K: GameServerObject>(
    ctx: &ReconcileCtx<'_, K>,
    server_id: &str,
    objects: &mut HashMap<String, K>,
) {
    if let Some(api) = ctx.servers_api
        && objects.remove(server_id).is_some()
    {
        delete_server_object(api, server_id).await;
        return;
    }
    if let Err(err) = ctx
        .pods_api
        .delete(server_id, &DeleteParams::default())
        .await
    {
        tracing::warn!(%server_id, %err, game = ctx.game.spec().display_name, "Échec suppression du Pod");
    }
}

async fn create_bare_pod(pods_api: &Api<Pod>, pod_name: &str, pod: &Pod) -> bool {
    match pods_api.create(&PostParams::default(), pod).await {
        Ok(_) => true,
        Err(err) => {
            tracing::error!(%pod_name, %err, "Échec création du Pod");
            false
        }
    }
}

async fn create_owned_pod<K: GameServerObject>(
    ctx: &ReconcileCtx<'_, K>,
    api: &Api<K>,
    pod_name: &str,
    mut pod: Pod,
    map: Option<&str>,
) -> bool {
    let server: K = new_server(
        pod_name,
        &ctx.config.k8s_namespace,
        pod.metadata.labels.clone().unwrap_or_default(),
        ctx.current_hash,
        map,
    );
    let created = match api.create(&PostParams::default(), &server).await {
        Ok(created) => created,
        Err(err) => {
            tracing::warn!(%pod_name, %err, kind = %K::kind(&()), "Échec création de l'objet");
            return false;
        }
    };
    attach_owner(&mut pod, owner_reference(&created));
    if create_bare_pod(ctx.pods_api, pod_name, &pod).await {
        return true;
    }
    delete_server_object(api, pod_name).await;
    false
}

async fn delete_orphan_objects<K: GameServerObject>(
    ctx: &ReconcileCtx<'_, K>,
    api: &Api<K>,
    pod_names: &HashSet<String>,
    objects: &mut HashMap<String, K>,
) {
    let Ok(timeout) = chrono::Duration::from_std(ctx.game.starting_timeout) else {
        return;
    };
    let now = chrono::Utc::now();
    let orphans: Vec<String> = objects
        .iter()
        .filter(|(name, server)| {
            let meta = server.meta();
            !pod_names.contains(*name)
                && meta.deletion_timestamp.is_none()
                && meta
                    .creation_timestamp
                    .as_ref()
                    .is_some_and(|created| now - created.0 > timeout)
        })
        .map(|(name, _)| name.clone())
        .collect();
    for name in orphans {
        tracing::info!(%name, kind = %K::kind(&()), "Suppression d'un objet sans Pod");
        delete_server_object(api, &name).await;
        objects.remove(&name);
    }
}

async fn sync_statuses<K: GameServerObject>(
    ctx: &ReconcileCtx<'_, K>,
    api: &Api<K>,
    objects: &HashMap<String, K>,
) {
    let Some(summaries) = ctx.with_engine(|engine| engine.snapshot(Instant::now()).servers) else {
        return;
    };
    let now = chrono::Utc::now().to_rfc3339();
    for summary in &summaries {
        let Some(object) = objects.get(&summary.server_id) else {
            continue;
        };
        let current: Option<&GameServerStatus> = object.current_status();
        let desired = desired_status(summary, current, &now);
        if !needs_patch(&desired, current) {
            continue;
        }
        let patch = Patch::Merge(serde_json::json!({ "status": desired }));
        if let Err(err) = api
            .patch_status(&summary.server_id, &PatchParams::default(), &patch)
            .await
        {
            tracing::warn!(server_id = %summary.server_id, %err, kind = %K::kind(&()), "Échec mise à jour du statut");
        }
    }
}

async fn reconcile_once<K: GameServerObject>(
    ctx: &ReconcileCtx<'_, K>,
    mut map_discovery: Option<&mut MapDiscoveryState>,
) -> Result<(), kube::Error> {
    let ReconcileCtx {
        state,
        config,
        game,
        pods_api,
        servers_api,
        template,
        ..
    } = *ctx;
    let spec = game.spec();
    let current_hash = ctx.current_hash;
    let lp = ListParams::default().labels(&spec.pod_selector());
    let list = pods_api.list(&lp).await?;
    let pod_names: HashSet<String> = if servers_api.is_some() {
        list.items
            .iter()
            .filter_map(|pod| pod.metadata.name.clone())
            .collect()
    } else {
        HashSet::new()
    };
    let mut objects = match servers_api {
        Some(api) => list_servers(api).await,
        None => HashMap::new(),
    };

    let mut live_ids = HashSet::new();
    let mut terminating_ids = HashSet::new();
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
        if pod.metadata.deletion_timestamp.is_some() {
            terminating_ids.insert(server_id.clone());
        }

        let address = pod
            .status
            .as_ref()
            .and_then(|s| s.pod_ip.as_ref())
            .map(|ip| format!("{ip}:{}", game.port));
        let ready = is_pod_ready_condition(&pod);
        let terminating = is_pod_terminating_or_done(&pod);

        ctx.with_engine(|engine| {
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
        });
    }

    let planned = ctx.with_engine(|engine| {
        events.extend(engine.retain_infra_servers(&live_ids));
        let actions = engine.tick(now);
        events.extend(actions.proxy_events.clone());
        let retries = engine.deletions_to_retry(now, DELETE_RETRY_AFTER);
        (actions, retries)
    });
    state.emit_proxy_events(events);
    state.flush_queue_logged().await;
    let Some((actions, retries)) = planned else {
        return Ok(());
    };

    for ServerDeletion { server_id, reason } in actions.servers_to_delete {
        if reason.is_abnormal() {
            tracing::warn!(%server_id, ?reason, game = spec.display_name, "Suppression du Pod");
        } else {
            tracing::info!(%server_id, ?reason, game = spec.display_name, "Suppression du Pod");
        }
        delete_server(ctx, &server_id, &mut objects).await;
    }

    for server_id in retries {
        if terminating_ids.contains(&server_id) {
            continue;
        }
        tracing::warn!(%server_id, game = spec.display_name, "Pod toujours présent, nouvelle suppression");
        delete_server(ctx, &server_id, &mut objects).await;
    }

    for _ in 0..actions.servers_to_create {
        let selected_map: Option<String> = if let Some(fixed) = &game.fixed_map {
            Some(fixed.clone())
        } else if let Some(discovery) = map_discovery.as_deref_mut() {
            match discovery.catalog.next_map() {
                Some(map) => Some(map.to_owned()),
                None => {
                    tracing::warn!(
                        game = spec.display_name,
                        "Création de Pod différée : aucune map disponible dans S3"
                    );
                    break;
                }
            }
        } else {
            None
        };

        let short = Uuid::new_v4().simple().to_string();
        let pod_name = format!("{}-{}", spec.pod_prefix, &short[..6]);
        let pod = build_pod(&pod_name, config, game, template, selected_map.as_deref());
        tracing::info!(%pod_name, map = ?selected_map, game = spec.display_name, "Création d'un nouveau Pod");
        let created = match servers_api {
            Some(api) => create_owned_pod(ctx, api, &pod_name, pod, selected_map.as_deref()).await,
            None => create_bare_pod(pods_api, &pod_name, &pod).await,
        };
        if created {
            ctx.with_engine(|engine| {
                engine.register_creating_server(Instant::now(), pod_name.clone(), Some(pod_name));
            });
        } else {
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }

    if let Some(api) = servers_api {
        delete_orphan_objects(ctx, api, &pod_names, &mut objects).await;
        sync_statuses(ctx, api, &objects).await;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> OrchestratorConfig {
        let vars: HashMap<&str, &str> = HashMap::from([
            ("ORCHESTRATOR_BACKEND", "kubernetes"),
            ("K8S_NAMESPACE", "ferriscraft"),
            (
                "SHEEPWARS_IMAGE",
                "ghcr.io/ferrislabs/ferriscraft-rspigot:dev",
            ),
            ("SHEEPWARS_MAX_PLAYERS", "16"),
            ("TOWERWARS_ENABLED", "true"),
            (
                "TOWERWARS_IMAGE",
                "ghcr.io/ferrislabs/ferriscraft-towerwars:dev",
            ),
            ("TOWERWARS_PORT", "25566"),
            ("TOWERWARS_MAX_PLAYERS", "12"),
        ]);
        OrchestratorConfig::from_lookup(|name| vars.get(name).map(|v| (*v).to_owned()), None)
            .unwrap()
    }

    fn sheepwars_pod(name: &str, template: Option<&PodTemplateSpec>, map: Option<&str>) -> Pod {
        let config = test_config();
        build_pod(name, &config, &config.sheepwars, template, map)
    }

    fn towerwars_pod(name: &str, template: Option<&PodTemplateSpec>, map: Option<&str>) -> Pod {
        let config = test_config();
        let tower = config.towerwars.as_ref().unwrap();
        build_pod(name, &config, tower, template, map)
    }

    fn env_of<'a>(container: &'a Container, name: &str) -> Option<&'a str> {
        container
            .env
            .as_ref()?
            .iter()
            .find(|e| e.name == name)
            .and_then(|e| e.value.as_deref())
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

        let pod = sheepwars_pod("sheepwars-abc123", Some(&template), Some("steampunk"));

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
                envs.iter()
                    .any(|e| e.name == "SHEEPWARS_MAP_NAME"
                        && e.value.as_deref() == Some("steampunk"))
            );
        }
    }

    #[test]
    fn crd_mode_pod_carries_the_owner_reference() {
        let mut server = new_server::<SheepWarsServer>(
            "sheepwars-abc123",
            "ferriscraft",
            BTreeMap::new(),
            "h",
            None,
        );
        server.metadata.uid = Some("uid-1".to_owned());
        let mut pod = sheepwars_pod("sheepwars-abc123", None, None);
        attach_owner(&mut pod, owner_reference(&server));
        let owners = pod.metadata.owner_references.unwrap();
        assert_eq!(owners.len(), 1);
        assert_eq!(owners[0].kind, "SheepWarsServer");
        assert_eq!(owners[0].name, "sheepwars-abc123");
        assert_eq!(owners[0].uid, "uid-1");
        assert_eq!(owners[0].controller, Some(true));
    }

    #[test]
    fn attach_owner_without_reference_leaves_pod_untouched() {
        let mut pod = sheepwars_pod("sheepwars-abc123", None, None);
        attach_owner(&mut pod, None);
        assert!(pod.metadata.owner_references.is_none());
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
        let pod = sheepwars_pod("sheepwars-abc123", Some(&template), None);
        let labels = pod.metadata.labels.unwrap();
        assert_eq!(
            labels.get(TEMPLATE_HASH_LABEL),
            Some(&template_hash(&config.sheepwars, Some(&template)))
        );
    }

    #[test]
    fn template_hash_changes_with_the_image_and_is_stable() {
        let config = test_config();
        let game = &config.sheepwars;
        let v1 = template_hash(game, Some(&plugin_template("plugin:v1")));
        let v2 = template_hash(game, Some(&plugin_template("plugin:v2")));
        assert_eq!(v1.len(), TEMPLATE_HASH_LEN);
        assert_ne!(v1, v2);
        assert_eq!(v1, template_hash(game, Some(&plugin_template("plugin:v1"))));
        assert_ne!(template_hash(game, None), v1);
    }

    #[test]
    fn spec_tw_26_towerwars_pod_has_its_own_labels_and_name() {
        let pod = towerwars_pod("towerwars-abc123", None, None);
        assert_eq!(pod.metadata.name.as_deref(), Some("towerwars-abc123"));
        let labels = pod.metadata.labels.as_ref().unwrap();
        assert_eq!(
            labels
                .get("app.kubernetes.io/component")
                .map(String::as_str),
            Some("towerwars")
        );
        assert_eq!(
            labels.get("ferriscraft.io/server-id").map(String::as_str),
            Some("towerwars-abc123")
        );
        assert_eq!(
            labels
                .get("app.kubernetes.io/managed-by")
                .map(String::as_str),
            Some("ferriscraft-orchestrator")
        );
        let sheep = sheepwars_pod("sheepwars-abc123", None, None);
        assert_eq!(
            sheep
                .metadata
                .labels
                .as_ref()
                .and_then(|l| l.get("app.kubernetes.io/component"))
                .map(String::as_str),
            Some("sheepwars")
        );
    }

    #[test]
    fn spec_tw_26_towerwars_pod_uses_its_image_port_and_env() {
        let pod = towerwars_pod("towerwars-abc123", None, Some("test"));
        let spec = pod.spec.as_ref().unwrap();
        let container = &spec.containers[0];
        assert_eq!(
            container.image.as_deref(),
            Some("ghcr.io/ferrislabs/ferriscraft-towerwars:dev")
        );
        assert_eq!(container.ports.as_ref().unwrap()[0].container_port, 25566);
        assert_eq!(env_of(container, "SERVER_PORT"), Some("25566"));
        assert_eq!(env_of(container, "SERVER_ID"), Some("towerwars-abc123"));
        assert_eq!(env_of(container, "TOWERWARS_MAX_PLAYERS"), Some("12"));
        assert_eq!(env_of(container, "TOWERWARS_MAP_NAME"), Some("test"));
        assert_eq!(env_of(container, "SHEEPWARS_MAX_PLAYERS"), None);
        assert_eq!(env_of(container, "SHEEPWARS_MAP_NAME"), None);
    }

    #[test]
    fn towerwars_pod_without_a_map_sets_no_map_env() {
        let pod = towerwars_pod("towerwars-abc123", None, None);
        let container = &pod.spec.as_ref().unwrap().containers[0];
        assert_eq!(env_of(container, "TOWERWARS_MAP_NAME"), None);
        assert!(
            !pod.metadata
                .labels
                .as_ref()
                .unwrap()
                .contains_key("ferriscraft.io/map")
        );
    }

    #[test]
    fn spec_tw_26_towerwars_pod_is_owned_by_a_towerwarsserver() {
        let mut server = new_server::<TowerWarsServer>(
            "towerwars-abc123",
            "ferriscraft",
            BTreeMap::new(),
            "h",
            None,
        );
        server.metadata.uid = Some("uid-2".to_owned());
        let mut pod = towerwars_pod("towerwars-abc123", None, None);
        attach_owner(&mut pod, owner_reference(&server));
        let owners = pod.metadata.owner_references.unwrap();
        assert_eq!(owners.len(), 1);
        assert_eq!(owners[0].kind, "TowerWarsServer");
        assert_eq!(owners[0].api_version, "ferriscraft.io/v1alpha1");
        assert_eq!(owners[0].name, "towerwars-abc123");
        assert_eq!(owners[0].uid, "uid-2");
        assert_eq!(owners[0].controller, Some(true));
    }

    #[test]
    fn towerwars_template_hash_differs_from_sheepwars_default() {
        let config = test_config();
        let tower = config.towerwars.as_ref().unwrap();
        assert_ne!(
            template_hash(tower, None),
            template_hash(&config.sheepwars, None)
        );
    }

    #[test]
    fn each_game_lists_only_its_own_pods() {
        let config = test_config();
        let selectors: Vec<String> = config.games().map(|g| g.spec().pod_selector()).collect();
        assert_eq!(selectors.len(), 2);
        assert!(selectors[0].contains("component=sheepwars"));
        assert!(selectors[1].contains("component=towerwars"));
    }
}
