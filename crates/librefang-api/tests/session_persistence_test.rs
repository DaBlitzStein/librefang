//! A dashboard session must survive a daemon restart.
//!
//! `save_sessions` writes `sessions.json` on every login, logout and GC sweep,
//! but `load_sessions` used to drop every row it had written — the on-disk key
//! is a `$sha256$` digest and the in-memory map was keyed by the cleartext
//! token, so nothing restored could ever match a presented token. The file was
//! write-only and every restart logged every operator out.
//!
//! This boots a router, logs in, then boots a second router over the *same*
//! home directory — the restart — and presents the token the first one issued.

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Method, Request, StatusCode};
use librefang_api::server;
use librefang_kernel::LibreFangKernel;
use librefang_types::config::{DefaultModelConfig, KernelConfig};
use std::net::SocketAddr;
use std::sync::Arc;
use tower::ServiceExt;

const USER: &str = "operator";
const PASS: &str = "correct-horse-battery-staple";

/// An endpoint that requires auth unconditionally, so a 200 proves the
/// presented token authenticated rather than falling through a public route.
const AUTHED_PATH: &str = "/api/status";

fn config_for(home: &std::path::Path) -> KernelConfig {
    KernelConfig {
        home_dir: home.to_path_buf(),
        data_dir: home.join("data"),
        dashboard_user: USER.to_string(),
        dashboard_pass: PASS.to_string(),
        default_model: DefaultModelConfig {
            provider: "ollama".to_string(),
            model: "test-model".to_string(),
            api_key_env: "OLLAMA_API_KEY".to_string(),
            base_url: None,
            message_timeout_secs: 300,
            extra_params: std::collections::BTreeMap::new(),
            cli_profile_dirs: Vec::new(),
        },
        ..KernelConfig::default()
    }
}

/// Boot a router over `home`. Returns the router plus the state, whose kernel
/// must be shut down before the next boot over the same directory.
async fn boot(home: &std::path::Path) -> (axum::Router, Arc<librefang_api::routes::AppState>) {
    let kernel =
        Arc::new(LibreFangKernel::boot_with_config(config_for(home)).expect("kernel boot"));
    kernel.set_self_handle();
    server::build_router(kernel, "127.0.0.1:0".parse().expect("addr")).await
}

fn with_peer(builder: axum::http::request::Builder, body: Body) -> Request<Body> {
    let mut req = builder.body(body).expect("request");
    // `dashboard_login` extracts ConnectInfo; `oneshot` does not populate it.
    req.extensions_mut().insert(ConnectInfo(
        "127.0.0.1:54321".parse::<SocketAddr>().expect("peer"),
    ));
    req
}

/// Pull the session token out of the `Set-Cookie` the login response carries.
fn session_token_from(headers: &axum::http::HeaderMap) -> String {
    headers
        .get_all(axum::http::header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find_map(|c| c.split(';').next()?.strip_prefix("librefang_session="))
        .map(str::to_string)
        .expect("login must issue a librefang_session cookie")
}

async fn get_with_token(app: axum::Router, path: &str, token: Option<&str>) -> StatusCode {
    let mut builder = Request::builder().method(Method::GET).uri(path);
    if let Some(t) = token {
        builder = builder.header(axum::http::header::AUTHORIZATION, format!("Bearer {t}"));
    }
    app.oneshot(with_peer(builder, Body::empty()))
        .await
        .expect("response")
        .status()
}

/// A restored row that carries no identity must still face the RBAC gate.
///
/// The gate (`user_role_allows_request`) denies owner-only writes, privileged
/// GETs, and every non-GET below `User`. It used to live inside the `if let`
/// that unpacks `user_name` / `user_role`, so a row with neither skipped it
/// entirely and fell through to the handler — a fail-open that only stayed
/// harmless while such rows were discarded on load.
#[tokio::test(flavor = "multi_thread")]
async fn a_restored_session_without_attribution_still_faces_the_rbac_gate() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let home = tmp.path();
    librefang_kernel::registry_sync::seed_registry_fixture_for_tests(home);
    std::fs::create_dir_all(home.join("data")).expect("data dir");

    // A pre-attribution row: hashed key, no user_name, no user_role. Written
    // by hand because no current code path can produce one.
    let token = "e".repeat(64);
    let key = librefang_api::password_hash::hash_device_token(&token);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    std::fs::write(
        home.join("data").join("sessions.json"),
        format!(r#"{{"{key}":{{"token":"","created_at":{now}}}}}"#),
    )
    .expect("write sessions.json");

    let (app, state) = boot(home).await;

    // POST /api/agents is an agent-creation write: Viewer must not reach it.
    let status = app
        .clone()
        .oneshot(with_peer(
            Request::builder()
                .method(Method::POST)
                .uri("/api/agents")
                .header(axum::http::header::AUTHORIZATION, format!("Bearer {token}"))
                .header(axum::http::header::CONTENT_TYPE, "application/json"),
            Body::from(serde_json::json!({ "name": "probe" }).to_string()),
        ))
        .await
        .expect("response")
        .status();
    state.kernel.shutdown();

    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "an unattributed session must be evaluated at the Viewer floor, not waved past the gate"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn dashboard_session_survives_a_daemon_restart() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let home = tmp.path();
    librefang_kernel::registry_sync::seed_registry_fixture_for_tests(home);

    // --- first boot: log in and capture the issued token ---
    let (app, state) = boot(home).await;

    let login = app
        .clone()
        .oneshot(with_peer(
            Request::builder()
                .method(Method::POST)
                .uri("/api/auth/dashboard-login")
                .header(axum::http::header::CONTENT_TYPE, "application/json"),
            Body::from(serde_json::json!({ "username": USER, "password": PASS }).to_string()),
        ))
        .await
        .expect("login response");
    assert_eq!(
        login.status(),
        StatusCode::OK,
        "login with the configured dashboard credentials must succeed"
    );
    let token = session_token_from(login.headers());

    assert_eq!(
        get_with_token(app.clone(), AUTHED_PATH, Some(&token)).await,
        StatusCode::OK,
        "sanity: the freshly issued token authenticates before any restart"
    );
    assert_eq!(
        get_with_token(app.clone(), AUTHED_PATH, None).await,
        StatusCode::UNAUTHORIZED,
        "sanity: {AUTHED_PATH} must require auth, or the test proves nothing"
    );

    state.kernel.shutdown();
    drop(app);

    // --- restart: a brand-new kernel and router over the same home ---
    let (restarted, restarted_state) = boot(home).await;

    let status = get_with_token(restarted.clone(), AUTHED_PATH, Some(&token)).await;
    restarted_state.kernel.shutdown();

    assert_eq!(
        status,
        StatusCode::OK,
        "the session issued before the restart must still authenticate afterwards"
    );
}
