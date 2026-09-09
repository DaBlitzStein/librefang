//! Integration tests for the credential-vault HTTP routes (#8164).
//!
//! Routes covered (handlers in `src/routes/vault.rs`):
//!   - `GET    /api/vault/keys`
//!   - `PUT    /api/vault/keys/{key}`
//!   - `DELETE /api/vault/keys/{key}`
//!
//! The harness mirrors `audit_routes_integration.rs`: a real `Router` behind the production auth middleware, driven with `tower::oneshot`.
//!
//! The load-bearing assertions are the ones about what a write is observable through afterwards.
//! `vault_put_is_visible_to_the_running_kernel_without_a_restart` reads the value back through `KernelApi::vault_get` — the accessor `routes::skills::resolve_github_token` calls, and the one that reads the kernel's cached in-memory map rather than the file — because a write that only reached disk would leave a live daemon serving the old value.
//! `vault_put_persists_to_the_vault_file` opens a fresh `CredentialVault` over the same home directory to prove the write is durable and not merely cached.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use librefang_api::middleware;
use librefang_api::routes;
use librefang_kernel::auth::UserRole as KernelUserRole;
use librefang_testing::{MockKernelBuilder, TestAppState};
use librefang_types::agent::UserId;
use librefang_types::config::UserConfig;
use std::sync::Arc;
use tower::ServiceExt;

const MASTER_KEY: &str = "vault-master-key";
const ADMIN_KEY: &str = "alice-vault-admin-key";
const VIEWER_KEY: &str = "bob-vault-viewer-key";

struct VaultHarness {
    app: Router,
    state: Arc<routes::AppState>,
    _tmp: tempfile::TempDir,
}

impl Drop for VaultHarness {
    fn drop(&mut self) {
        self.state.kernel.shutdown();
    }
}

fn build_harness() -> VaultHarness {
    let users = [("Alice", "admin", ADMIN_KEY), ("Bob", "viewer", VIEWER_KEY)];
    let mut user_configs: Vec<UserConfig> = Vec::with_capacity(users.len());
    let mut api_user_records: Vec<middleware::ApiUserAuth> = Vec::with_capacity(users.len());
    for (name, role_str, key) in users {
        let hash =
            librefang_api::password_hash::hash_password(key).expect("password hash should succeed");
        user_configs.push(UserConfig {
            name: name.to_string(),
            role: role_str.to_string(),
            channel_bindings: std::collections::HashMap::new(),
            api_key_hash: Some(hash.clone()),
            ..Default::default()
        });
        api_user_records.push(middleware::ApiUserAuth {
            name: name.to_string(),
            role: KernelUserRole::from_str_role(role_str),
            api_key_hash: hash,
            user_id: UserId::from_name(name),
        });
    }

    let test = TestAppState::with_builder(MockKernelBuilder::new().with_config(move |cfg| {
        cfg.api_key = MASTER_KEY.to_string();
        cfg.users = user_configs;
    }))
    .with_api_key(MASTER_KEY)
    .with_user_api_keys(api_user_records);

    let (state, tmp, _cfg_path) = test.into_parts();

    let auth_state = middleware::AuthState {
        api_key_lock: state.api_key_lock.clone(),
        master_key: state.master_key.clone(),
        active_sessions: state.active_sessions.clone(),
        dashboard_auth_enabled: false,
        user_api_keys: state.user_api_keys.clone(),
        require_auth_for_reads: false,
        allow_no_auth: true,
        audit_log: Some(state.kernel.audit().clone()),
    };

    let app = Router::new()
        .nest("/api", routes::vault::router())
        .layer(axum::middleware::from_fn_with_state(
            auth_state,
            middleware::auth,
        ))
        .with_state(state.clone());

    VaultHarness {
        app,
        state,
        _tmp: tmp,
    }
}

async fn send(
    app: Router,
    method: Method,
    path: &str,
    bearer: Option<&str>,
    body: Option<serde_json::Value>,
) -> (StatusCode, Vec<u8>) {
    let mut builder = Request::builder().method(method).uri(path);
    if let Some(token) = bearer {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    let req = match body {
        Some(json) => builder
            .header("content-type", "application/json")
            .body(Body::from(json.to_string())),
        None => builder.body(Body::empty()),
    }
    .expect("build request");
    let resp = app.oneshot(req).await.expect("oneshot");
    let status = resp.status();
    let bytes = resp
        .into_body()
        .collect()
        .await
        .expect("collect body")
        .to_bytes()
        .to_vec();
    (status, bytes)
}

fn body_json(bytes: &[u8]) -> serde_json::Value {
    serde_json::from_slice(bytes).expect("response body must be valid JSON")
}

/// The entry for `key` in a `GET /api/vault/keys` response body.
fn key_entry<'a>(body: &'a serde_json::Value, key: &str) -> &'a serde_json::Value {
    body["keys"]
        .as_array()
        .expect("`keys` must be an array")
        .iter()
        .find(|entry| entry["key"] == key)
        .unwrap_or_else(|| panic!("`keys` must list {key}"))
}

/// The vault-presence flag for `key`. Says nothing about what the daemon resolves — see [`key_source`].
fn key_is_set(body: &serde_json::Value, key: &str) -> bool {
    key_entry(body, key)["set"]
        .as_bool()
        .expect("`set` must be a boolean")
}

/// The effective source reported for `key`: `unset`, `vault` or `environment`.
fn key_source(body: &serde_json::Value, key: &str) -> String {
    key_entry(body, key)["source"]
        .as_str()
        .expect("`source` must be a string")
        .to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn vault_put_is_visible_to_the_running_kernel_without_a_restart() {
    let h = build_harness();
    assert_eq!(
        h.state.kernel.vault_get("GITHUB_TOKEN"),
        None,
        "precondition: nothing stored yet"
    );

    let (status, _) = send(
        h.app.clone(),
        Method::PUT,
        "/api/vault/keys/GITHUB_TOKEN",
        Some(ADMIN_KEY),
        Some(serde_json::json!({ "value": "ghp_hot_reload" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Separate path: the kernel accessor `resolve_github_token` consults,
    // reading the cached in-memory map the daemon serves from.
    assert_eq!(
        h.state.kernel.vault_get("GITHUB_TOKEN").as_deref(),
        Some("ghp_hot_reload"),
        "the write must be visible to the live kernel with no restart"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn vault_put_persists_to_the_vault_file() {
    let h = build_harness();
    let (status, _) = send(
        h.app.clone(),
        Method::PUT,
        "/api/vault/keys/GITHUB_TOKEN",
        Some(ADMIN_KEY),
        Some(serde_json::json!({ "value": "ghp_durable" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let mut fresh = librefang_extensions::vault::CredentialVault::new(
        h.state.kernel.home_dir().join("vault.enc"),
    );
    fresh.unlock().expect("freshly opened vault must unlock");
    assert_eq!(
        fresh.get("GITHUB_TOKEN").map(|v| v.to_string()),
        Some("ghp_durable".to_string()),
        "the write must reach vault.enc, not just the in-memory cache"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn vault_put_trims_surrounding_whitespace() {
    let h = build_harness();
    let (status, _) = send(
        h.app.clone(),
        Method::PUT,
        "/api/vault/keys/GITHUB_TOKEN",
        Some(ADMIN_KEY),
        Some(serde_json::json!({ "value": "  ghp_pasted\n" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        h.state.kernel.vault_get("GITHUB_TOKEN").as_deref(),
        Some("ghp_pasted"),
        "a token pasted with a trailing newline must not be stored verbatim"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn vault_listing_reports_presence_and_never_the_value() {
    let h = build_harness();
    let secret = "ghp_must_never_be_echoed";

    let (_, before) = send(
        h.app.clone(),
        Method::GET,
        "/api/vault/keys",
        Some(ADMIN_KEY),
        None,
    )
    .await;
    assert!(!key_is_set(&body_json(&before), "GITHUB_TOKEN"));

    let (status, put_body) = send(
        h.app.clone(),
        Method::PUT,
        "/api/vault/keys/GITHUB_TOKEN",
        Some(ADMIN_KEY),
        Some(serde_json::json!({ "value": secret })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        !String::from_utf8_lossy(&put_body).contains(secret),
        "the write response must not echo the secret back"
    );

    let (status, after) = send(
        h.app.clone(),
        Method::GET,
        "/api/vault/keys",
        Some(ADMIN_KEY),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        !String::from_utf8_lossy(&after).contains(secret),
        "the listing must never carry the secret value: {}",
        String::from_utf8_lossy(&after)
    );
    assert!(key_is_set(&body_json(&after), "GITHUB_TOKEN"));
}

#[tokio::test(flavor = "multi_thread")]
async fn vault_delete_clears_the_secret_for_the_running_kernel() {
    let h = build_harness();
    send(
        h.app.clone(),
        Method::PUT,
        "/api/vault/keys/GITHUB_TOKEN",
        Some(ADMIN_KEY),
        Some(serde_json::json!({ "value": "ghp_temporary" })),
    )
    .await;

    let (status, body) = send(
        h.app.clone(),
        Method::DELETE,
        "/api/vault/keys/GITHUB_TOKEN",
        Some(ADMIN_KEY),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body_json(&body)["removed"], serde_json::json!(true));
    assert_eq!(
        h.state.kernel.vault_get("GITHUB_TOKEN"),
        None,
        "the delete must be visible to the live kernel with no restart"
    );

    // Deleting an absent key is a successful no-op, not a 404.
    let (status, body) = send(
        h.app.clone(),
        Method::DELETE,
        "/api/vault/keys/GITHUB_TOKEN",
        Some(ADMIN_KEY),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body_json(&body)["removed"], serde_json::json!(false));
}

#[tokio::test(flavor = "multi_thread")]
async fn vault_rejects_keys_outside_the_allowlist() {
    let h = build_harness();
    let (status, _) = send(
        h.app.clone(),
        Method::PUT,
        "/api/vault/keys/SOME_OTHER_SECRET",
        Some(ADMIN_KEY),
        Some(serde_json::json!({ "value": "nope" })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        h.state.kernel.vault_get("SOME_OTHER_SECRET"),
        None,
        "a rejected key must not be written"
    );

    let (status, _) = send(
        h.app.clone(),
        Method::DELETE,
        "/api/vault/keys/SOME_OTHER_SECRET",
        Some(ADMIN_KEY),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn vault_rejects_an_empty_value() {
    let h = build_harness();
    let (status, _) = send(
        h.app.clone(),
        Method::PUT,
        "/api/vault/keys/GITHUB_TOKEN",
        Some(ADMIN_KEY),
        Some(serde_json::json!({ "value": "   " })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(h.state.kernel.vault_get("GITHUB_TOKEN"), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn vault_writes_require_an_admin_credential() {
    let h = build_harness();

    let (status, _) = send(
        h.app.clone(),
        Method::PUT,
        "/api/vault/keys/GITHUB_TOKEN",
        Some(VIEWER_KEY),
        Some(serde_json::json!({ "value": "ghp_viewer" })),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "viewer must be refused");

    let (status, _) = send(
        h.app.clone(),
        Method::GET,
        "/api/vault/keys",
        Some(VIEWER_KEY),
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "even presence is admin-gated — it reveals which credentials an operator configured"
    );

    let (status, _) = send(
        h.app.clone(),
        Method::PUT,
        "/api/vault/keys/GITHUB_TOKEN",
        None,
        Some(serde_json::json!({ "value": "ghp_anon" })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "anonymous must be refused by the auth middleware"
    );

    assert_eq!(
        h.state.kernel.vault_get("GITHUB_TOKEN"),
        None,
        "no refused request may have written"
    );
}

/// Serializes the tests that mutate `GITHUB_TOKEN`. An environment variable is process-global, so two of them running on different test threads would see each other's writes.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// A guard that takes [`ENV_LOCK`], clears `GITHUB_TOKEN` for the duration of a test, and restores whatever the process started with.
///
/// The listing's `source` field is the only thing in this binary that reads the ambient environment, and a CI runner that happens to export `GITHUB_TOKEN` would otherwise turn the `unset` case into a false failure.
struct GithubTokenEnvGuard {
    prior: Option<String>,
    _lock: std::sync::MutexGuard<'static, ()>,
}

impl GithubTokenEnvGuard {
    fn take() -> Self {
        // A sibling test that panicked mid-guard poisons the mutex; the exclusion it provides is still what we want.
        let lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prior = std::env::var("GITHUB_TOKEN").ok();
        std::env::remove_var("GITHUB_TOKEN");
        Self { prior, _lock: lock }
    }

    fn set(value: &str) {
        std::env::set_var("GITHUB_TOKEN", value);
    }

    fn clear() {
        std::env::remove_var("GITHUB_TOKEN");
    }
}

impl Drop for GithubTokenEnvGuard {
    fn drop(&mut self) {
        match self.prior.take() {
            Some(prior) => std::env::set_var("GITHUB_TOKEN", prior),
            None => std::env::remove_var("GITHUB_TOKEN"),
        }
    }
}

/// The listing reports where the daemon actually resolves each key from, not merely whether the vault holds a copy.
///
/// `resolve_github_token` reads the process environment before it touches the vault, so a `set` flag computed from the vault alone reports `false` on a host where promotion works, and reports `false` again after a delete that revoked nothing because the environment still supplies the token. All three cases live in one test so the environment mutation is never visible to a concurrently running sibling.
#[tokio::test(flavor = "multi_thread")]
async fn vault_listing_reports_the_effective_source_of_each_key() {
    let _env = GithubTokenEnvGuard::take();
    let h = build_harness();

    // 1. Neither environment nor vault.
    let (status, body) = send(
        h.app.clone(),
        Method::GET,
        "/api/vault/keys",
        Some(ADMIN_KEY),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let body = body_json(&body);
    assert_eq!(key_source(&body, "GITHUB_TOKEN"), "unset");
    assert!(!key_is_set(&body, "GITHUB_TOKEN"));

    // 2. Vault only — the value the daemon resolves comes from the vault.
    let (status, _) = send(
        h.app.clone(),
        Method::PUT,
        "/api/vault/keys/GITHUB_TOKEN",
        Some(ADMIN_KEY),
        Some(serde_json::json!({ "value": "ghp_from_the_vault" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, body) = send(
        h.app.clone(),
        Method::GET,
        "/api/vault/keys",
        Some(ADMIN_KEY),
        None,
    )
    .await;
    let body = body_json(&body);
    assert_eq!(key_source(&body, "GITHUB_TOKEN"), "vault");
    assert!(key_is_set(&body, "GITHUB_TOKEN"));

    // 3. Environment set on top of the vault copy — the environment wins, and the listing must say so rather than reporting the vault copy as the effective credential.
    GithubTokenEnvGuard::set("ghp_from_the_environment");
    let (_, body) = send(
        h.app.clone(),
        Method::GET,
        "/api/vault/keys",
        Some(ADMIN_KEY),
        None,
    )
    .await;
    let raw = String::from_utf8_lossy(&body).to_string();
    let body = body_json(&body);
    assert_eq!(key_source(&body, "GITHUB_TOKEN"), "environment");
    assert!(
        key_is_set(&body, "GITHUB_TOKEN"),
        "`set` keeps its narrow meaning: the vault still holds a copy"
    );
    assert!(
        !raw.contains("ghp_from_the_environment") && !raw.contains("ghp_from_the_vault"),
        "reporting the source must not leak either value: {raw}"
    );

    // 4. The delete that revokes nothing. The vault copy goes, the environment still supplies the token, and both the write response and the listing have to keep saying so.
    let (status, delete_body) = send(
        h.app.clone(),
        Method::DELETE,
        "/api/vault/keys/GITHUB_TOKEN",
        Some(ADMIN_KEY),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body_json(&delete_body)["source"],
        "environment",
        "a delete that leaves the environment in charge must not answer `unset`"
    );
    let (_, body) = send(
        h.app.clone(),
        Method::GET,
        "/api/vault/keys",
        Some(ADMIN_KEY),
        None,
    )
    .await;
    let body = body_json(&body);
    assert_eq!(key_source(&body, "GITHUB_TOKEN"), "environment");
    assert!(
        !key_is_set(&body, "GITHUB_TOKEN"),
        "the vault copy is gone even though the environment still resolves"
    );

    // 5. Environment gone too — back to genuinely unset.
    GithubTokenEnvGuard::clear();
    let (_, body) = send(
        h.app.clone(),
        Method::GET,
        "/api/vault/keys",
        Some(ADMIN_KEY),
        None,
    )
    .await;
    assert_eq!(key_source(&body_json(&body), "GITHUB_TOKEN"), "unset");
}

/// A write while the environment overrides the key still reports `environment`, so the operator is told the value they just stored is inert rather than being shown a bare success.
#[tokio::test(flavor = "multi_thread")]
async fn vault_put_reports_an_environment_override_rather_than_a_bare_success() {
    let _env = GithubTokenEnvGuard::take();
    GithubTokenEnvGuard::set("ghp_env_wins");
    let h = build_harness();

    let (status, body) = send(
        h.app.clone(),
        Method::PUT,
        "/api/vault/keys/GITHUB_TOKEN",
        Some(ADMIN_KEY),
        Some(serde_json::json!({ "value": "ghp_stored_but_inert" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let parsed = body_json(&body);
    assert_eq!(parsed["set"], true, "the vault write did land");
    assert_eq!(
        parsed["source"], "environment",
        "but the daemon still resolves the environment's value"
    );
}
