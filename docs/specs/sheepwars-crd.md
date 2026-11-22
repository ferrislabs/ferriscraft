# Serveurs SheepWars en ressource Kubernetes

Statut : proposé
Date : 2026-11-22
Id : `swc`

## Goal

Voir et piloter la flotte SheepWars avec `kubectl get sws`. Chaque serveur est un objet `SheepWarsServer` qui possède son Pod.

## Context

- L'orchestrateur crée des Pods nus (`k8s.rs`, `build_sheepwars_pod`) et garde l'état de la flotte en mémoire (`FleetEngine`).
- Le hash du template est déjà posé en label `ferriscraft.io/template-hash` (#15). La version vient des heartbeats (#18).
- ArgoCD déploie le chart. L'AppProject `ferriscraft` (dépôt infra) n'autorise aujourd'hui que le kind cluster-scoped `Namespace`.

Décisions : [ADR 0008](../adr/0008-crd-sheepwarsserver-maison.md) (CRD maison, pas Agones), [ADR 0009](../adr/0009-politique-de-flotte-dans-les-values.md) (politique dans les values Helm, pas de CRD `SheepWarsFleet`).

## Invariants

1. `FleetEngine` ne connaît pas Kubernetes. Seul l'adapter `k8s.rs` lit et écrit les objets. [revue]
2. En mode CRD, chaque Pod SheepWars créé a une `ownerReference` `controller: true` vers son `SheepWarsServer`. [test unitaire sur la construction du Pod]
3. Le `status` n'est écrit que s'il diffère de celui lu dans le cluster. [test unitaire sur `desired_status`]
4. Avec `orchestrator.sheepwars.crd.enabled: false` (défaut), le comportement est celui d'avant : Pods nus, aucun appel à l'API `ferriscraft.io`. [test : rendu Helm sans CRD ni règle RBAC ; code : branche unique sur le flag]
5. Supprimer un serveur en mode CRD supprime l'objet. Le Pod part par le garbage collector. Un Pod sans objet (créé avant le mode CRD) est supprimé directement. [code]

## Out of scope

- CRD de politique de flotte (ADR 0009).
- Watch sur les objets : la boucle de réconciliation (1 s) liste Pods et objets.
- Actions pilotées par l'utilisateur via l'objet (`kubectl delete sws x` fonctionne par le GC, aucun autre champ n'est lu).
- Changement de l'AppProject ArgoCD (dépôt infra, PR à part).

## Design

- Nouveau module `crates/ferriscraft-orchestrator/src/crd.rs` : types du CRD (contrat figé ci-dessous) et fonctions pures `desired_status`, `owner_reference`.
- `OrchestratorConfig.use_crd: bool`, lu depuis `SHEEPWARS_CRD_ENABLED` (défaut `false`).
- Boucle en mode CRD :
  1. Liste les Pods (sélecteur actuel) et les `SheepWarsServer` du namespace.
  2. Alimente l'engine depuis les Pods, comme aujourd'hui.
  3. `tick`.
  4. Suppressions : objet présent → `delete` de l'objet (propagation `Background`). Sinon `delete` du Pod.
  5. Créations : crée l'objet (`spec.templateHash`, `spec.map`, mêmes labels que le Pod), puis le Pod avec l'`ownerReference`. Si le Pod échoue, supprime l'objet.
  6. Objet sans Pod depuis plus de `starting_timeout` : supprimé (Pod évincé ou jamais créé).
  7. Statut : pour chaque serveur de l'engine qui a un objet, `desired_status(summary, status actuel, now)`. Si différent, `patch` merge sur la sous-ressource `status`.
- `phaseSince` : `now` si la phase change, sinon la valeur actuelle.
- Chart : `templates/sheepwarsservers-crd.yaml` (rendu seulement si `orchestrator.sheepwars.crd.enabled`, annotations `helm.sh/resource-policy: keep` et sync-wave `-1`), règles RBAC `sheepwarsservers` et `sheepwarsservers/status` sous le même flag, variable `SHEEPWARS_CRD_ENABLED`.
- Le YAML du CRD dans le chart est généré depuis `SheepWarsServer::crd()`. Un test vérifie qu'ils sont égaux.

## Errors

Une erreur de l'API `ferriscraft.io` est journalisée en `warn` et la boucle continue, comme pour les Pods aujourd'hui. Un CRD absent alors que le flag est actif donne un `warn` à chaque boucle. Aucun Pod nu n'est créé à sa place.

## Acceptance

```gherkin
@spec-swc-1  Scenario: un serveur créé en mode CRD est visible
  Given orchestrator.sheepwars.crd.enabled=true
  When l'orchestrateur crée un serveur
  Then `kubectl get sws` liste l'objet avec sa phase, ses joueurs, sa version et outdated
  And son Pod a une ownerReference vers l'objet

@spec-swc-2  Scenario: le statut suit la phase
  Given un serveur en waiting
  When une partie démarre
  Then status.phase passe à in_game et status.phaseSince change
  And aucun patch n'est envoyé tant que rien ne change

@spec-swc-3  Scenario: supprimer l'objet arrête le serveur
  When un opérateur fait `kubectl delete sws <id>`
  Then le Pod est supprimé par le garbage collector

@spec-swc-4  Scenario: mode désactivé
  Given orchestrator.sheepwars.crd.enabled=false
  Then le chart ne rend ni CRD ni règle RBAC ferriscraft.io
  And l'orchestrateur crée des Pods nus comme avant
```

`swc-2` (partie pure) et la construction du Pod (`swc-1`) sont des tests unitaires. `swc-4` est vérifié par `helm template`. `swc-1`, `swc-2` et `swc-3` sont vérifiés à la main sur k3d.

## Simulation

Non. Pas de concurrence nouvelle : une seule boucle écrit.

## Frozen contracts

```rust
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
pub enum SheepWarsServerPhase { Starting, Waiting, Countdown, InGame, Ending }

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
```

## Verify (exit)

```bash
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p ferriscraft-orchestrator
helm lint deploy/charts/ferriscraft
helm template t deploy/charts/ferriscraft | grep -c sheepwarsservers        # 0
helm template t deploy/charts/ferriscraft --set orchestrator.sheepwars.crd.enabled=true | grep -c sheepwarsservers   # > 0
```
