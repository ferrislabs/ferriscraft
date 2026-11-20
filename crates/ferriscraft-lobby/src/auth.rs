use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rspigot_plugin::{Click, Part, RichMessage, TeamColor, Uuid};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::watch;

use crate::config::EnvConfig;

const REFRESH_MARGIN_SECS: u64 = 30;
const DEFAULT_TOKEN_LIFETIME_SECS: u64 = 300;
const DEFAULT_DEVICE_EXPIRES_SECS: u64 = 600;
const DEFAULT_POLL_INTERVAL_SECS: u64 = 5;

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("FerrisKey est injoignable : {0}")]
    Http(#[from] reqwest::Error),
    #[error("FerrisKey a refusé la requête ({status}) : {message}")]
    Rejected { status: u16, message: String },
    #[error("le code de connexion a expiré")]
    Expired,
    #[error("la demande de connexion a été refusée")]
    Denied,
    #[error("la connexion en cours a été remplacée ou annulée")]
    Cancelled,
    #[error("aucun compte FerrisKey lié — tapez /login")]
    NotAuthenticated,
    #[error("la session FerrisKey a expiré, reconnectez-vous avec /login")]
    SessionExpired,
    #[error("aucun compte Capitainerie lié — tapez /caplogin")]
    CapitainerieNotAuthenticated,
    #[error("la session Capitainerie a expiré, reconnectez-vous avec /caplogin")]
    CapitainerieSessionExpired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Rank {
    Admin,
    Mod,
    Dev,
    Vip,
    Member,
    Guest,
}

impl Rank {
    pub const ALL: [Self; 6] = [
        Self::Admin,
        Self::Mod,
        Self::Dev,
        Self::Vip,
        Self::Member,
        Self::Guest,
    ];

    pub fn resolve(claims: Option<&UserClaims>, is_op: bool, has_admin_perm: bool) -> Self {
        if is_op || has_admin_perm {
            return Self::Admin;
        }
        let Some(claims) = claims else {
            return Self::Guest;
        };
        let has_any = |targets: &[&str]| {
            claims.roles.iter().any(|role| {
                let lower = role.to_ascii_lowercase();
                targets.iter().any(|t| lower == *t)
            })
        };
        if has_any(&[
            "admin",
            "administrator",
            "owner",
            "fondateur",
            "ferriscraft-admin",
        ]) {
            Self::Admin
        } else if has_any(&["mod", "moderator", "modo", "staff", "ferriscraft-mod"]) {
            Self::Mod
        } else if has_any(&[
            "dev",
            "developer",
            "builder",
            "architect",
            "ferriscraft-dev",
        ]) {
            Self::Dev
        } else if has_any(&["vip", "mvp", "epic", "supporter"]) {
            Self::Vip
        } else {
            Self::Member
        }
    }

    pub const fn team_name(self, verified: bool) -> &'static str {
        match (self, verified) {
            (Self::Admin, true) => "00_admin_fk",
            (Self::Admin, false) => "01_admin",
            (Self::Mod, _) => "10_mod",
            (Self::Dev, _) => "20_dev",
            (Self::Vip, _) => "30_vip",
            (Self::Member, _) => "40_member",
            (Self::Guest, _) => "90_guest",
        }
    }

    pub const fn team_color(self) -> TeamColor {
        match self {
            Self::Admin => TeamColor::Red,
            Self::Mod => TeamColor::Blue,
            Self::Dev => TeamColor::Aqua,
            Self::Vip => TeamColor::Gold,
            Self::Member => TeamColor::Green,
            Self::Guest => TeamColor::Gray,
        }
    }

    pub const fn tab_prefix(self) -> &'static str {
        match self {
            Self::Admin => "§c§lADMIN §f",
            Self::Mod => "§9§lMOD §f",
            Self::Dev => "§b§lDEV §f",
            Self::Vip => "§6§lVIP §f",
            Self::Member => "§a§lFK §f",
            Self::Guest => "§7",
        }
    }

    pub const fn chat_badge(self) -> &'static str {
        match self {
            Self::Admin => "§c§l[ADMIN]",
            Self::Mod => "§9§l[MOD]",
            Self::Dev => "§b§l[DEV]",
            Self::Vip => "§6§l[VIP]",
            Self::Member => "§a[Membre]",
            Self::Guest => "§7[Joueur]",
        }
    }

    pub const fn sidebar_label(self) -> &'static str {
        match self {
            Self::Admin => "§c§lADMIN",
            Self::Mod => "§9§lMOD",
            Self::Dev => "§b§lDEV",
            Self::Vip => "§6§lVIP",
            Self::Member => "§aMembre",
            Self::Guest => "§7Invité",
        }
    }

    pub const fn bypasses_chat_cooldown(self) -> bool {
        matches!(self, Self::Admin | Self::Mod | Self::Dev)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserClaims {
    pub sub: String,
    pub username: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub roles: Vec<String>,
    #[serde(default)]
    pub exp: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerSession {
    pub player_uuid: Uuid,
    pub claims: UserClaims,
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    pub expires_at: u64,
}

impl PlayerSession {
    pub fn is_fresh(&self, now_secs: u64) -> bool {
        self.expires_at > now_secs.saturating_add(REFRESH_MARGIN_SECS)
    }
}

const fn default_expires() -> u64 {
    DEFAULT_DEVICE_EXPIRES_SECS
}

const fn default_interval() -> u64 {
    DEFAULT_POLL_INTERVAL_SECS
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeviceAuthorization {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    #[serde(default)]
    pub verification_uri_complete: Option<String>,
    #[serde(default = "default_expires")]
    pub expires_in: u64,
    #[serde(default = "default_interval")]
    pub interval: u64,
}

impl DeviceAuthorization {
    pub fn browser_url(&self) -> &str {
        self.verification_uri_complete
            .as_deref()
            .unwrap_or(&self.verification_uri)
    }
}

#[derive(Debug, Deserialize)]
struct TokenSuccess {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<u64>,
    #[serde(default)]
    id_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OAuthErrorPayload {
    error: String,
    #[serde(default)]
    error_description: Option<String>,
}

fn now_epoch_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

pub fn decode_jwt_claims(jwt: &str) -> Option<UserClaims> {
    let mut parts = jwt.split('.');
    let _header = parts.next()?;
    let payload_b64 = parts.next()?.trim_end_matches('=');
    let bytes = URL_SAFE_NO_PAD.decode(payload_b64).ok()?;
    let json: serde_json::Value = serde_json::from_slice(&bytes).ok()?;

    let str_field = |key: &str| {
        json.get(key)
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };

    let sub = str_field("sub").unwrap_or_else(|| "unknown".to_owned());
    let username = str_field("preferred_username")
        .or_else(|| str_field("username"))
        .or_else(|| str_field("name"))
        .unwrap_or_else(|| sub.clone());
    let display_name = str_field("name");
    let email = str_field("email");
    let exp = json.get("exp").and_then(serde_json::Value::as_u64);

    let mut role_set = BTreeSet::new();
    let mut collect_array = |arr: Option<&Vec<serde_json::Value>>| {
        if let Some(items) = arr {
            for item in items {
                if let Some(role) = item.as_str().map(str::trim).filter(|s| !s.is_empty()) {
                    role_set.insert(role.to_owned());
                }
            }
        }
    };

    collect_array(json.get("roles").and_then(serde_json::Value::as_array));
    collect_array(
        json.get("realm_access")
            .and_then(|ra| ra.get("roles"))
            .and_then(serde_json::Value::as_array),
    );
    if let Some(resources) = json
        .get("resource_access")
        .and_then(serde_json::Value::as_object)
    {
        for client_obj in resources.values() {
            collect_array(
                client_obj
                    .get("roles")
                    .and_then(serde_json::Value::as_array),
            );
        }
    }

    Some(UserClaims {
        sub,
        username,
        display_name,
        email,
        roles: role_set.into_iter().collect(),
        exp,
    })
}

pub fn build_device_prompt(auth: &DeviceAuthorization) -> RichMessage {
    build_device_prompt_for(auth, None)
}

pub fn build_device_prompt_for(
    auth: &DeviceAuthorization,
    context_label: Option<&str>,
) -> RichMessage {
    let url = auth.browser_url().to_owned();
    let code = auth.user_code.clone();
    let heading = match context_label {
        Some(ctx) => format!("§6§l🔐 Connexion FerrisKey — {ctx}\n"),
        None => "§6§l🔐 Connexion FerrisKey\n".to_owned(),
    };
    RichMessage::new()
        .text("§8§m                                                  \n")
        .text(heading)
        .text("§7Validez votre identité dans le navigateur :\n  ")
        .part(
            Part::new("§b§l[🌐 Ouvrir FerrisKey]")
                .hover(format!("§eCliquez pour ouvrir :\n§f{url}"))
                .click(Click::OpenUrl(url)),
        )
        .text("  ")
        .part(
            Part::new(format!("§e§l[📋 Code : {code}]"))
                .hover("§7Cliquez pour copier le code dans le presse-papier")
                .click(Click::CopyToClipboard(code)),
        )
        .text(format!(
            "\n§8Expire dans {} min — détection automatique dès validation.\n",
            (auth.expires_in / 60).max(1)
        ))
        .text("§8§m                                                  ")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum AuthScope {
    Lobby,
    Capitainerie,
}

struct RealmTarget<'a> {
    internal_base_url: &'a str,
    public_base_url: Option<&'a str>,
    device_auth_url: String,
    token_url: String,
    client_id: &'a str,
    client_secret: Option<&'a str>,
    scope: &'a str,
    resource: Option<&'a str>,
    dedicated_capitainerie: bool,
}

fn authority_of_url(url: &str) -> Option<&str> {
    let after_scheme = url.split_once("://").map_or(url, |(_, rest)| rest);
    let authority = after_scheme.split('/').next()?.trim();
    (!authority.is_empty()).then_some(authority)
}

fn rewrite_public_url(url: &str, internal_base: &str, public_base: &str) -> String {
    let internal = internal_base.trim_end_matches('/');
    let public = public_base.trim_end_matches('/');
    if let Some(rest) = url.strip_prefix(internal)
        && (rest.is_empty() || rest.starts_with('/'))
    {
        return format!("{public}{rest}");
    }
    url.to_owned()
}

struct SessionStore {
    path: PathBuf,
    sessions: RwLock<HashMap<Uuid, PlayerSession>>,
    pending_cancels: RwLock<HashMap<Uuid, watch::Sender<bool>>>,
}

impl SessionStore {
    fn new(path: PathBuf) -> Self {
        let loaded = Self::load_sessions(&path);
        Self {
            path,
            sessions: RwLock::new(loaded),
            pending_cancels: RwLock::new(HashMap::new()),
        }
    }

    fn session_of(&self, uuid: Uuid) -> Option<PlayerSession> {
        self.sessions
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(&uuid)
            .cloned()
    }

    fn is_authenticated(&self, uuid: Uuid) -> bool {
        self.sessions
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(&uuid)
    }

    fn uuids(&self) -> Vec<Uuid> {
        self.sessions
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .keys()
            .copied()
            .collect()
    }

    fn cancel_pending(&self, uuid: Uuid) {
        if let Some(tx) = self
            .pending_cancels
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&uuid)
        {
            let _ = tx.send(true);
        }
    }

    fn register_pending(&self, uuid: Uuid) -> watch::Receiver<bool> {
        let (tx, rx) = watch::channel(false);
        self.pending_cancels
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(uuid, tx);
        rx
    }

    fn logout(&self, uuid: Uuid) -> bool {
        self.cancel_pending(uuid);
        let removed = self
            .sessions
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&uuid)
            .is_some();
        if removed {
            self.persist_sessions();
        }
        removed
    }

    fn store_tokens(
        &self,
        player_uuid: Uuid,
        granted: TokenSuccess,
        previous: Option<PlayerSession>,
    ) -> PlayerSession {
        let claims = decode_jwt_claims(&granted.access_token)
            .or_else(|| granted.id_token.as_deref().and_then(decode_jwt_claims))
            .or_else(|| previous.as_ref().map(|prev| prev.claims.clone()))
            .unwrap_or_else(|| UserClaims {
                sub: player_uuid.to_string(),
                username: "Membre".to_owned(),
                display_name: None,
                email: None,
                roles: Vec::new(),
                exp: None,
            });

        let now = now_epoch_secs();
        let expires_at = granted
            .expires_in
            .map(|secs| now.saturating_add(secs))
            .or(claims.exp)
            .unwrap_or_else(|| now.saturating_add(DEFAULT_TOKEN_LIFETIME_SECS));

        let refresh_token = granted
            .refresh_token
            .or_else(|| previous.and_then(|prev| prev.refresh_token));

        let session = PlayerSession {
            player_uuid,
            claims,
            access_token: granted.access_token,
            refresh_token,
            expires_at,
        };

        self.sessions
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(player_uuid, session.clone());
        self.persist_sessions();
        session
    }

    fn load_sessions(path: &Path) -> HashMap<Uuid, PlayerSession> {
        if path.as_os_str().is_empty() || !path.exists() {
            return HashMap::new();
        }
        let Ok(raw) = std::fs::read_to_string(path) else {
            return HashMap::new();
        };
        serde_json::from_str::<Vec<PlayerSession>>(&raw)
            .map(|list| list.into_iter().map(|s| (s.player_uuid, s)).collect())
            .unwrap_or_default()
    }

    fn persist_sessions(&self) {
        if self.path.as_os_str().is_empty() {
            return;
        }
        let list: Vec<PlayerSession> = self
            .sessions
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .cloned()
            .collect();
        let Ok(raw) = serde_json::to_string_pretty(&list) else {
            return;
        };
        if std::fs::write(&self.path, format!("{raw}\n")).is_ok() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ =
                    std::fs::set_permissions(&self.path, std::fs::Permissions::from_mode(0o600));
            }
        }
    }
}

pub struct AuthManager {
    config: EnvConfig,
    http: reqwest::Client,
    lobby_store: SessionStore,
    capitainerie_store: SessionStore,
}

impl AuthManager {
    pub fn new(config: EnvConfig) -> Self {
        let lobby_store = SessionStore::new(config.sessions_path.clone());
        let capitainerie_store = if config.has_dedicated_capitainerie_auth() {
            SessionStore::new(config.capitainerie_sessions_path.clone())
        } else {
            SessionStore::new(PathBuf::new())
        };
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            config,
            http,
            lobby_store,
            capitainerie_store,
        }
    }

    pub fn uses_separate_capitainerie_auth(&self) -> bool {
        self.config.has_dedicated_capitainerie_auth()
    }

    fn effective_scope(&self, requested: AuthScope) -> AuthScope {
        match requested {
            AuthScope::Lobby => AuthScope::Lobby,
            AuthScope::Capitainerie if self.config.has_dedicated_capitainerie_auth() => {
                AuthScope::Capitainerie
            }
            AuthScope::Capitainerie => AuthScope::Lobby,
        }
    }

    fn store_for(&self, scope: AuthScope) -> &SessionStore {
        match self.effective_scope(scope) {
            AuthScope::Lobby => &self.lobby_store,
            AuthScope::Capitainerie => &self.capitainerie_store,
        }
    }

    fn target_for(&self, scope: AuthScope) -> RealmTarget<'_> {
        match self.effective_scope(scope) {
            AuthScope::Lobby => RealmTarget {
                internal_base_url: self.config.ferriskey_base_url.as_str(),
                public_base_url: self.config.ferriskey_public_url.as_deref(),
                device_auth_url: self.config.device_auth_url(),
                token_url: self.config.token_url(),
                client_id: self.config.ferriskey_client_id.as_str(),
                client_secret: self.config.ferriskey_client_secret.as_deref(),
                scope: self.config.ferriskey_scope.as_str(),
                resource: if self.config.has_dedicated_capitainerie_auth() {
                    None
                } else {
                    self.config.capitainerie_resource.as_deref()
                },
                dedicated_capitainerie: false,
            },
            AuthScope::Capitainerie => RealmTarget {
                internal_base_url: self.config.capitainerie_ferriskey_url.as_str(),
                public_base_url: self.config.capitainerie_ferriskey_public_url.as_deref(),
                device_auth_url: self.config.capitainerie_device_auth_url(),
                token_url: self.config.capitainerie_token_url(),
                client_id: self.config.capitainerie_client_id.as_str(),
                client_secret: self.config.capitainerie_client_secret.as_deref(),
                scope: self.config.capitainerie_scope.as_str(),
                resource: self.config.capitainerie_resource.as_deref(),
                dedicated_capitainerie: true,
            },
        }
    }

    fn post_oauth_form(
        &self,
        url: &str,
        public_base_url: Option<&str>,
        form: &[(&str, &str)],
    ) -> reqwest::RequestBuilder {
        let mut req = self.http.post(url).form(form);
        if let Some(authority) = public_base_url.and_then(authority_of_url) {
            req = req.header(reqwest::header::HOST, authority);
        }
        req
    }

    pub fn session_of(&self, uuid: Uuid) -> Option<PlayerSession> {
        self.lobby_store.session_of(uuid)
    }

    pub fn claims_of(&self, uuid: Uuid) -> Option<UserClaims> {
        self.session_of(uuid).map(|s| s.claims)
    }

    pub fn is_authenticated(&self, uuid: Uuid) -> bool {
        self.lobby_store.is_authenticated(uuid)
    }

    pub fn logout(&self, uuid: Uuid) -> bool {
        self.lobby_store.logout(uuid)
    }

    pub fn capitainerie_session_of(&self, uuid: Uuid) -> Option<PlayerSession> {
        self.store_for(AuthScope::Capitainerie).session_of(uuid)
    }

    pub fn capitainerie_claims_of(&self, uuid: Uuid) -> Option<UserClaims> {
        self.capitainerie_session_of(uuid).map(|s| s.claims)
    }

    pub fn is_capitainerie_authenticated(&self, uuid: Uuid) -> bool {
        self.store_for(AuthScope::Capitainerie)
            .is_authenticated(uuid)
    }

    pub fn capitainerie_logout(&self, uuid: Uuid) -> bool {
        self.store_for(AuthScope::Capitainerie).logout(uuid)
    }

    pub fn cancel_pending(&self, uuid: Uuid) {
        self.lobby_store.cancel_pending(uuid);
        self.capitainerie_store.cancel_pending(uuid);
    }

    pub async fn start_device_flow(
        &self,
        player_uuid: Uuid,
    ) -> Result<(DeviceAuthorization, watch::Receiver<bool>), AuthError> {
        self.start_device_flow_scoped(AuthScope::Lobby, player_uuid)
            .await
    }

    pub async fn start_capitainerie_device_flow(
        &self,
        player_uuid: Uuid,
    ) -> Result<(DeviceAuthorization, watch::Receiver<bool>), AuthError> {
        self.start_device_flow_scoped(AuthScope::Capitainerie, player_uuid)
            .await
    }

    async fn start_device_flow_scoped(
        &self,
        scope: AuthScope,
        player_uuid: Uuid,
    ) -> Result<(DeviceAuthorization, watch::Receiver<bool>), AuthError> {
        let store = self.store_for(scope);
        let target = self.target_for(scope);
        store.cancel_pending(player_uuid);

        let mut form = vec![("client_id", target.client_id), ("scope", target.scope)];
        if let Some(secret) = target.client_secret {
            form.push(("client_secret", secret));
        }
        if let Some(resource) = target.resource {
            form.push(("resource", resource));
        }

        let response = self
            .post_oauth_form(&target.device_auth_url, target.public_base_url, &form)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let message = response.text().await.unwrap_or_default();
            return Err(AuthError::Rejected { status, message });
        }

        let mut device: DeviceAuthorization = response.json().await?;
        if let Some(public_base) = target.public_base_url {
            device.verification_uri =
                rewrite_public_url(&device.verification_uri, target.internal_base_url, public_base);
            device.verification_uri_complete = device
                .verification_uri_complete
                .map(|uri| rewrite_public_url(&uri, target.internal_base_url, public_base));
        }
        let rx = store.register_pending(player_uuid);
        Ok((device, rx))
    }

    pub async fn poll_device_flow(
        &self,
        player_uuid: Uuid,
        device: &DeviceAuthorization,
        cancel_rx: watch::Receiver<bool>,
    ) -> Result<PlayerSession, AuthError> {
        self.poll_device_flow_scoped(AuthScope::Lobby, player_uuid, device, cancel_rx)
            .await
    }

    pub async fn poll_capitainerie_device_flow(
        &self,
        player_uuid: Uuid,
        device: &DeviceAuthorization,
        cancel_rx: watch::Receiver<bool>,
    ) -> Result<PlayerSession, AuthError> {
        self.poll_device_flow_scoped(AuthScope::Capitainerie, player_uuid, device, cancel_rx)
            .await
    }

    async fn poll_device_flow_scoped(
        &self,
        scope: AuthScope,
        player_uuid: Uuid,
        device: &DeviceAuthorization,
        mut cancel_rx: watch::Receiver<bool>,
    ) -> Result<PlayerSession, AuthError> {
        let store = self.store_for(scope);
        let target = self.target_for(scope);
        let mut interval_secs = device.interval.max(1);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(device.expires_in.max(10));

        loop {
            tokio::select! {
                _ = cancel_rx.changed() => {
                    if *cancel_rx.borrow() {
                        return Err(AuthError::Cancelled);
                    }
                }
                _ = tokio::time::sleep(Duration::from_secs(interval_secs)) => {}
            }

            if tokio::time::Instant::now() >= deadline {
                return Err(AuthError::Expired);
            }

            let mut form = vec![
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("device_code", device.device_code.as_str()),
                ("client_id", target.client_id),
            ];
            if let Some(secret) = target.client_secret {
                form.push(("client_secret", secret));
            }
            if let Some(resource) = target.resource {
                form.push(("resource", resource));
            }

            let response = self
                .post_oauth_form(&target.token_url, target.public_base_url, &form)
                .send()
                .await?;

            if response.status().is_success() {
                let granted: TokenSuccess = response.json().await?;
                let session = store.store_tokens(player_uuid, granted, None);
                return Ok(session);
            }

            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            if let Ok(err) = serde_json::from_str::<OAuthErrorPayload>(&body) {
                match err.error.as_str() {
                    "authorization_pending" => continue,
                    "slow_down" => {
                        interval_secs = interval_secs.saturating_add(5);
                        continue;
                    }
                    "expired_token" => return Err(AuthError::Expired),
                    "access_denied" => return Err(AuthError::Denied),
                    other => {
                        return Err(AuthError::Rejected {
                            status,
                            message: err.error_description.unwrap_or_else(|| other.to_owned()),
                        });
                    }
                }
            }

            return Err(AuthError::Rejected {
                status,
                message: body.chars().take(200).collect(),
            });
        }
    }

    pub async fn valid_access_token(&self, player_uuid: Uuid) -> Result<String, AuthError> {
        self.valid_access_token_scoped(AuthScope::Lobby, player_uuid)
            .await
    }

    pub async fn valid_capitainerie_token(&self, player_uuid: Uuid) -> Result<String, AuthError> {
        self.valid_access_token_scoped(AuthScope::Capitainerie, player_uuid)
            .await
    }

    async fn valid_access_token_scoped(
        &self,
        scope: AuthScope,
        player_uuid: Uuid,
    ) -> Result<String, AuthError> {
        let store = self.store_for(scope);
        let target = self.target_for(scope);
        let not_auth_err = || {
            if target.dedicated_capitainerie {
                AuthError::CapitainerieNotAuthenticated
            } else {
                AuthError::NotAuthenticated
            }
        };
        let expired_err = || {
            if target.dedicated_capitainerie {
                AuthError::CapitainerieSessionExpired
            } else {
                AuthError::SessionExpired
            }
        };

        let current = store.session_of(player_uuid).ok_or_else(not_auth_err)?;
        let now = now_epoch_secs();
        if current.is_fresh(now) {
            return Ok(current.access_token);
        }

        let Some(refresh_token) = current.refresh_token.clone() else {
            store.logout(player_uuid);
            return Err(expired_err());
        };

        let mut form = vec![
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token.as_str()),
            ("client_id", target.client_id),
        ];
        if let Some(secret) = target.client_secret {
            form.push(("client_secret", secret));
        }
        if let Some(resource) = target.resource {
            form.push(("resource", resource));
        }

        let response = self
            .post_oauth_form(&target.token_url, target.public_base_url, &form)
            .send()
            .await?;

        if !response.status().is_success() {
            store.logout(player_uuid);
            return Err(expired_err());
        }

        let granted: TokenSuccess = response.json().await?;
        let refreshed = store.store_tokens(player_uuid, granted, Some(current));
        Ok(refreshed.access_token)
    }

    pub async fn observer_token(&self) -> Option<String> {
        if let Some(token) = &self.config.capitainerie_observer_token {
            return Some(token.clone());
        }
        let uuids = self.store_for(AuthScope::Capitainerie).uuids();
        for uuid in uuids {
            if let Ok(token) = self.valid_capitainerie_token(uuid).await {
                return Some(token);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::{Form, State};
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    use axum::routing::post;
    use axum::{Json, Router};
    use serde_json::json;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::net::TcpListener;

    fn make_jwt(payload: &serde_json::Value) -> String {
        let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"RS256","typ":"JWT"}"#);
        let body = URL_SAFE_NO_PAD.encode(serde_json::to_vec(payload).expect("json"));
        format!("{header}.{body}.sig")
    }

    #[test]
    fn decodes_ferriskey_jwt_claims_and_resolves_ranks() {
        let token = make_jwt(&json!({
            "sub": "user-42",
            "preferred_username": "nathael",
            "name": "Nathaël",
            "email": "nathael@ferrislabs.io",
            "exp": 1999999999u64,
            "realm_access": {
                "roles": ["member", "admin"]
            }
        }));

        let claims = decode_jwt_claims(&token).expect("valid jwt");
        assert_eq!(claims.sub, "user-42");
        assert_eq!(claims.username, "nathael");
        assert_eq!(claims.email.as_deref(), Some("nathael@ferrislabs.io"));
        assert_eq!(Rank::resolve(Some(&claims), false, false), Rank::Admin);
        assert_eq!(Rank::resolve(None, false, false), Rank::Guest);
    }

    #[tokio::test]
    async fn device_flow_polls_pending_then_authenticates_and_refreshes() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let app = Router::new()
            .route(
                "/realms/ferriscraft/protocol/openid-connect/auth/device",
                post(|| async {
                    Json(json!({
                        "device_code": "dev-123",
                        "user_code": "ABCD-1234",
                        "verification_uri": "http://localhost:3333/device",
                        "verification_uri_complete": "http://localhost:3333/device?user_code=ABCD-1234",
                        "expires_in": 300,
                        "interval": 1
                    }))
                }),
            )
            .route(
                "/realms/ferriscraft/protocol/openid-connect/token",
                post(
                    |State(counter): State<Arc<AtomicUsize>>,
                     Form(form): Form<HashMap<String, String>>| async move {
                        match form.get("grant_type").map(String::as_str) {
                            Some("urn:ietf:params:oauth:grant-type:device_code") => {
                                let step = counter.fetch_add(1, Ordering::SeqCst);
                                if step == 0 {
                                    (
                                        StatusCode::BAD_REQUEST,
                                        Json(json!({"error": "authorization_pending"})),
                                    )
                                        .into_response()
                                } else {
                                    let jwt = make_jwt(&json!({
                                        "sub": "u-1",
                                        "preferred_username": "ferris_player",
                                        "realm_access": {"roles": ["dev"]}
                                    }));
                                    Json(json!({
                                        "access_token": jwt,
                                        "refresh_token": "ref-1",
                                        "expires_in": 10
                                    }))
                                    .into_response()
                                }
                            }
                            Some("refresh_token") => {
                                let jwt = make_jwt(&json!({
                                    "sub": "u-1",
                                    "preferred_username": "ferris_player",
                                    "realm_access": {"roles": ["dev"]}
                                }));
                                Json(json!({
                                    "access_token": jwt,
                                    "refresh_token": "ref-2",
                                    "expires_in": 600
                                }))
                                .into_response()
                            }
                            _ => StatusCode::BAD_REQUEST.into_response(),
                        }
                    },
                ),
            )
            .with_state(Arc::clone(&attempts));

        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        let cfg = EnvConfig {
            ferriskey_base_url: format!("http://{addr}"),
            ferriskey_public_url: None,
            ferriskey_realm: "ferriscraft".to_owned(),
            ferriskey_client_id: "ferriscraft-lobby".to_owned(),
            ferriskey_client_secret: None,
            ferriskey_scope: "openid profile email".to_owned(),
            capitainerie_url: None,
            capitainerie_console_url: None,
            capitainerie_resource: None,
            capitainerie_observer_token: None,
            capitainerie_ferriskey_url: format!("http://{addr}"),
            capitainerie_ferriskey_public_url: None,
            capitainerie_realm: "ferriscraft".to_owned(),
            capitainerie_client_id: "ferriscraft-lobby".to_owned(),
            capitainerie_client_secret: None,
            capitainerie_scope: "openid profile email".to_owned(),
            capitainerie_sessions_path: PathBuf::new(),
            sheepwars_transfer: None,
            sheepwars_backend: None,
            sheepwars_world: None,
            orchestrator_url: None,
            orchestrator_token: None,
            layout_path: PathBuf::new(),
            sessions_path: PathBuf::new(),
        };

        let manager = AuthManager::new(cfg);
        assert!(!manager.uses_separate_capitainerie_auth());
        let player_id = Uuid::new_v4();
        let (device, cancel_rx) = manager.start_device_flow(player_id).await.expect("start");
        assert_eq!(device.user_code, "ABCD-1234");

        let session = manager
            .poll_device_flow(player_id, &device, cancel_rx)
            .await
            .expect("poll succeeds");
        assert_eq!(session.claims.username, "ferris_player");
        assert_eq!(
            Rank::resolve(Some(&session.claims), false, false),
            Rank::Dev
        );

        // expires_in was 10s (< REFRESH_MARGIN_SECS = 30s), so valid_access_token triggers refresh!
        let _fresh = manager
            .valid_access_token(player_id)
            .await
            .expect("refreshed");
        assert_eq!(
            manager
                .session_of(player_id)
                .unwrap()
                .refresh_token
                .as_deref(),
            Some("ref-2")
        );
    }

    #[tokio::test]
    async fn dual_realm_isolates_lobby_and_capitainerie_sessions() {
        let app = Router::new()
            .route(
                "/realms/ferriscraft/protocol/openid-connect/auth/device",
                post(|Form(form): Form<HashMap<String, String>>| async move {
                    assert_eq!(
                        form.get("client_id").map(String::as_str),
                        Some("ferriscraft-lobby")
                    );
                    assert!(
                        !form.contains_key("resource"),
                        "lobby device flow must not send capitainerie resource"
                    );
                    Json(json!({
                        "device_code": "dc-lobby",
                        "user_code": "LOBBY-11",
                        "verification_uri": "http://localhost:3333/device",
                        "expires_in": 300,
                        "interval": 1
                    }))
                }),
            )
            .route(
                "/realms/ferriscraft/protocol/openid-connect/token",
                post(|Form(form): Form<HashMap<String, String>>| async move {
                    assert_eq!(
                        form.get("client_id").map(String::as_str),
                        Some("ferriscraft-lobby")
                    );
                    assert!(!form.contains_key("resource"));
                    let jwt = make_jwt(&json!({
                        "sub": "u-lobby",
                        "preferred_username": "lobby_user",
                        "iss": "http://localhost:3333/realms/ferriscraft"
                    }));
                    Json(json!({
                        "access_token": jwt,
                        "refresh_token": "rt-lobby",
                        "expires_in": 600
                    }))
                }),
            )
            .route(
                "/realms/capitainerie/protocol/openid-connect/auth/device",
                post(|Form(form): Form<HashMap<String, String>>| async move {
                    assert_eq!(form.get("client_id").map(String::as_str), Some("minecraft"));
                    assert_eq!(
                        form.get("resource").map(String::as_str),
                        Some("http://localhost:4100")
                    );
                    Json(json!({
                        "device_code": "dc-cap",
                        "user_code": "CAPI-22",
                        "verification_uri": "http://localhost:3333/device",
                        "expires_in": 300,
                        "interval": 1
                    }))
                }),
            )
            .route(
                "/realms/capitainerie/protocol/openid-connect/token",
                post(|Form(form): Form<HashMap<String, String>>| async move {
                    assert_eq!(form.get("client_id").map(String::as_str), Some("minecraft"));
                    assert_eq!(
                        form.get("resource").map(String::as_str),
                        Some("http://localhost:4100")
                    );
                    let jwt = make_jwt(&json!({
                        "sub": "u-cap",
                        "preferred_username": "cap_user",
                        "iss": "http://localhost:3333/realms/capitainerie",
                        "aud": "http://localhost:4100",
                        "azp": "minecraft"
                    }));
                    Json(json!({
                        "access_token": jwt,
                        "refresh_token": "rt-cap",
                        "expires_in": 600
                    }))
                }),
            );

        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        let cfg = EnvConfig {
            ferriskey_base_url: format!("http://{addr}"),
            ferriskey_public_url: None,
            ferriskey_realm: "ferriscraft".to_owned(),
            ferriskey_client_id: "ferriscraft-lobby".to_owned(),
            ferriskey_client_secret: Some("lobby-secret".to_owned()),
            ferriskey_scope: "openid profile email roles".to_owned(),
            capitainerie_url: Some("http://localhost:4100".to_owned()),
            capitainerie_console_url: None,
            capitainerie_resource: Some("http://localhost:4100".to_owned()),
            capitainerie_observer_token: None,
            capitainerie_ferriskey_url: format!("http://{addr}"),
            capitainerie_ferriskey_public_url: None,
            capitainerie_realm: "capitainerie".to_owned(),
            capitainerie_client_id: "minecraft".to_owned(),
            capitainerie_client_secret: None,
            capitainerie_scope: "openid profile email".to_owned(),
            capitainerie_sessions_path: PathBuf::new(),
            sheepwars_transfer: None,
            sheepwars_backend: None,
            sheepwars_world: None,
            orchestrator_url: None,
            orchestrator_token: None,
            layout_path: PathBuf::new(),
            sessions_path: PathBuf::new(),
        };

        let manager = AuthManager::new(cfg);
        assert!(manager.uses_separate_capitainerie_auth());

        let player_id = Uuid::new_v4();
        let (lobby_dev, lobby_cancel) = manager.start_device_flow(player_id).await.unwrap();
        assert_eq!(lobby_dev.user_code, "LOBBY-11");
        let lobby_sess = manager
            .poll_device_flow(player_id, &lobby_dev, lobby_cancel)
            .await
            .unwrap();
        assert_eq!(lobby_sess.claims.username, "lobby_user");
        assert!(manager.is_authenticated(player_id));
        assert!(!manager.is_capitainerie_authenticated(player_id));
        assert!(manager.observer_token().await.is_none());

        let (cap_dev, cap_cancel) = manager
            .start_capitainerie_device_flow(player_id)
            .await
            .unwrap();
        assert_eq!(cap_dev.user_code, "CAPI-22");
        let cap_sess = manager
            .poll_capitainerie_device_flow(player_id, &cap_dev, cap_cancel)
            .await
            .unwrap();
        assert_eq!(cap_sess.claims.username, "cap_user");
        assert!(manager.is_capitainerie_authenticated(player_id));

        let lobby_tok = manager.valid_access_token(player_id).await.unwrap();
        let cap_tok = manager.valid_capitainerie_token(player_id).await.unwrap();
        assert_ne!(lobby_tok, cap_tok);
        assert_eq!(
            manager.observer_token().await.as_deref(),
            Some(cap_tok.as_str())
        );

        assert!(manager.capitainerie_logout(player_id));
        assert!(!manager.is_capitainerie_authenticated(player_id));
        assert!(manager.is_authenticated(player_id));
    }

    #[tokio::test]
    async fn device_flow_uses_public_url_host_header_and_rewrites_verification_uris() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        let internal_url = format!("http://{addr}");
        let internal_for_route = internal_url.clone();

        let app = Router::new()
            .route(
                "/realms/capitainerie/protocol/openid-connect/auth/device",
                post(move |headers: axum::http::HeaderMap| {
                    let internal_resp = internal_for_route.clone();
                    async move {
                        let host = headers
                            .get(axum::http::header::HOST)
                            .and_then(|v| v.to_str().ok())
                            .map(str::to_owned);
                        assert_eq!(host.as_deref(), Some("localhost:3333"));
                        Json(json!({
                            "device_code": "dc-pub",
                            "user_code": "PUB-99",
                            "verification_uri": format!("{internal_resp}/realms/capitainerie/device"),
                            "verification_uri_complete": format!("{internal_resp}/realms/capitainerie/device?user_code=PUB-99"),
                            "expires_in": 300,
                            "interval": 1
                        }))
                    }
                }),
            )
            .route(
                "/realms/capitainerie/protocol/openid-connect/token",
                post(|headers: axum::http::HeaderMap| async move {
                    let host = headers
                        .get(axum::http::header::HOST)
                        .and_then(|v| v.to_str().ok())
                        .map(str::to_owned);
                    assert_eq!(host.as_deref(), Some("localhost:3333"));
                    let jwt = make_jwt(&json!({
                        "sub": "u-pub",
                        "preferred_username": "pub_user",
                        "iss": "http://localhost:3333/realms/capitainerie"
                    }));
                    Json(json!({
                        "access_token": jwt,
                        "refresh_token": "rt-pub",
                        "expires_in": 600
                    }))
                }),
            );

        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        let cfg = EnvConfig {
            ferriskey_base_url: internal_url.clone(),
            ferriskey_public_url: Some("http://localhost:3333".to_owned()),
            ferriskey_realm: "capitainerie".to_owned(),
            ferriskey_client_id: "minecraft".to_owned(),
            ferriskey_client_secret: None,
            ferriskey_scope: "openid profile email".to_owned(),
            capitainerie_url: None,
            capitainerie_console_url: None,
            capitainerie_resource: None,
            capitainerie_observer_token: None,
            capitainerie_ferriskey_url: internal_url,
            capitainerie_ferriskey_public_url: Some("http://localhost:3333".to_owned()),
            capitainerie_realm: "capitainerie".to_owned(),
            capitainerie_client_id: "minecraft".to_owned(),
            capitainerie_client_secret: None,
            capitainerie_scope: "openid profile email".to_owned(),
            capitainerie_sessions_path: PathBuf::new(),
            sheepwars_transfer: None,
            sheepwars_backend: None,
            sheepwars_world: None,
            orchestrator_url: None,
            orchestrator_token: None,
            layout_path: PathBuf::new(),
            sessions_path: PathBuf::new(),
        };

        let manager = AuthManager::new(cfg);
        let player_id = Uuid::new_v4();
        let (device, cancel_rx) = manager.start_device_flow(player_id).await.expect("start");
        assert_eq!(
            device.verification_uri,
            "http://localhost:3333/realms/capitainerie/device"
        );
        assert_eq!(
            device.browser_url(),
            "http://localhost:3333/realms/capitainerie/device?user_code=PUB-99"
        );
        let session = manager
            .poll_device_flow(player_id, &device, cancel_rx)
            .await
            .expect("poll succeeds");
        assert_eq!(session.claims.username, "pub_user");
    }
}
