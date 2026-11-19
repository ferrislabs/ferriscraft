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
use uuid::Uuid;

use crate::api::AppState;
use crate::config::OrchestratorConfig;

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

pub fn build_sheepwars_pod(
    pod_name: &str,
    config: &OrchestratorConfig,
    template: Option<&PodTemplateSpec>,
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

    tracing::info!(
        namespace = %config.k8s_namespace,
        min_replicas = config.min_replicas,
        max_replicas = config.max_replicas,
        queue_min_players = config.queue_min_players,
        "Réconciliateur Kubernetes SheepWars démarré"
    );

    let mut ticker = tokio::time::interval(config.reconcile_interval);
    loop {
        ticker.tick().await;
        if let Err(err) = reconcile_once(&state, &config, &pods_api, template.as_ref()).await {
            tracing::warn!(%err, "Erreur lors de la réconciliation Kubernetes");
        }
    }
}

async fn reconcile_once(
    state: &AppState,
    config: &OrchestratorConfig,
    pods_api: &Api<Pod>,
    template: Option<&PodTemplateSpec>,
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
        let server_id = pod
            .metadata
            .labels
            .as_ref()
            .and_then(|l| l.get("ferriscraft.io/server-id"))
            .cloned()
            .unwrap_or_else(|| pod_name.clone());
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
            server_id,
            Some(pod_name),
            address,
            ready,
            terminating,
        ));
    }

    let actions = {
        let mut engine = state.engine.lock().unwrap_or_else(|e| e.into_inner());
        events.extend(engine.retain_infra_servers(&live_ids));
        let actions = engine.tick(now);
        events.extend(actions.proxy_events.clone());
        actions
    };
    state.emit_proxy_events(events);

    for server_id in actions.servers_to_delete {
        tracing::info!(%server_id, "Suppression du Pod SheepWars");
        if let Err(err) = pods_api.delete(&server_id, &DeleteParams::default()).await {
            tracing::warn!(%server_id, %err, "Échec suppression Pod SheepWars");
        }
    }

    for _ in 0..actions.servers_to_create {
        let short = Uuid::new_v4().simple().to_string();
        let pod_name = format!("sheepwars-{}", &short[..6]);
        let pod = build_sheepwars_pod(&pod_name, config, template);
        tracing::info!(%pod_name, "Création d'un nouveau Pod SheepWars");
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
