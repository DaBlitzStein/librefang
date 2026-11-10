//! Integration tests for the session artifact HTTP API and signed links.

use axum::body::Body;
use axum::http::{HeaderMap, Method, Request, StatusCode};
use axum::Router;
use librefang_api::routes::{
    self,
    artifacts::{sign_link_token, LinkClaims},
    AppState,
};
use librefang_api::server;
use librefang_memory::artifact_store::{ArtifactRecord, ArtifactStore};
use librefang_testing::{MockKernelBuilder, TestAppState};
use librefang_types::agent::{AgentId, AgentManifest, SessionId};
use serde_json::Value;
use std::sync::Arc;
use tower::ServiceExt;

struct Harness {
    app: Router,
    state: Arc<AppState>,
    _test: TestAppState,
}

async fn boot() -> Harness {
    let test = TestAppState::new();
    let state = test.state.clone();
    let app = Router::new()
        .nest("/api", routes::artifacts::router())
        .with_state(state.clone());
    Harness {
        app,
        state,
        _test: test,
    }
}

fn seed_agent_and_session(h: &Harness) -> (AgentId, SessionId) {
    let agent = h
        .state
        .kernel
        .spawn_agent_typed(AgentManifest {
            name: format!("artifact-test-{}", uuid::Uuid::new_v4()),
            ..AgentManifest::default()
        })
        .expect("spawn agent");
    let session = h
        .state
        .kernel
        .memory_substrate()
        .create_session(agent)
        .expect("create session");
    (agent, session.id)
}

fn store(h: &Harness) -> &ArtifactStore {
    h.state
        .kernel
        .memory_substrate()
        .artifacts()
        .expect("boot attaches the artifact store")
}

fn put(h: &Harness, session: SessionId, path: &str, bytes: &[u8]) -> ArtifactRecord {
    store(h)
        .put(
            &session.0.to_string(),
            None,
            path,
            bytes,
            Some("text/plain"),
        )
        .expect("record artifact")
}

struct Response {
    status: StatusCode,
    headers: HeaderMap,
    body: Vec<u8>,
}

impl Response {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).expect("JSON body")
    }
}

async fn request(
    app: &Router,
    method: Method,
    uri: &str,
    body: Option<Value>,
    bearer: Option<&str>,
) -> Response {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(token) = bearer {
        builder = builder.header(axum::http::header::AUTHORIZATION, format!("Bearer {token}"));
    }
    let bytes = match body {
        Some(value) => {
            builder = builder.header(axum::http::header::CONTENT_TYPE, "application/json");
            serde_json::to_vec(&value).expect("serialize body")
        }
        None => Vec::new(),
    };
    let response = app
        .clone()
        .oneshot(builder.body(Body::from(bytes)).expect("request"))
        .await
        .expect("oneshot");
    let status = response.status();
    let headers = response.headers().clone();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body")
        .to_vec();
    Response {
        status,
        headers,
        body,
    }
}

fn header(resp: &Response, name: &str) -> String {
    resp.headers
        .get(name)
        .unwrap_or_else(|| panic!("missing {name} header"))
        .to_str()
        .expect("ASCII header")
        .to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn list_versions_and_download_round_trip() {
    let h = boot().await;
    let (_, session) = seed_agent_and_session(&h);
    let sid = session.0.to_string();
    put(&h, session, "docs/report.txt", b"first");
    put(&h, session, "docs/report.txt", b"second");

    let resp = request(
        &h.app,
        Method::GET,
        &format!("/api/sessions/{sid}/artifacts"),
        None,
        None,
    )
    .await;
    assert_eq!(resp.status, StatusCode::OK);
    let body = resp.json();
    let artifacts = body["artifacts"].as_array().expect("artifacts array");
    assert_eq!(artifacts.len(), 1, "{body}");
    assert_eq!(artifacts[0]["path"], "docs/report.txt");
    assert_eq!(artifacts[0]["version"], 2);

    let resp = request(
        &h.app,
        Method::GET,
        &format!("/api/sessions/{sid}/artifacts/versions?path=docs/report.txt"),
        None,
        None,
    )
    .await;
    assert_eq!(resp.status, StatusCode::OK);
    let body = resp.json();
    let versions = body["versions"].as_array().expect("versions array");
    assert_eq!(versions.len(), 2, "{body}");
    assert_eq!(versions[0]["version"], 1);
    assert_eq!(versions[1]["version"], 2);

    let resp = request(
        &h.app,
        Method::GET,
        &format!("/api/sessions/{sid}/artifacts/download?path=docs/report.txt&version=1"),
        None,
        None,
    )
    .await;
    assert_eq!(resp.status, StatusCode::OK);
    assert_eq!(resp.body, b"first");
    assert_eq!(header(&resp, "content-type"), "text/plain");
    assert_eq!(
        header(&resp, "content-disposition"),
        "attachment; filename=\"report.txt\""
    );
    assert_eq!(header(&resp, "content-length"), "5");
}

#[tokio::test(flavor = "multi_thread")]
async fn unknown_session_path_and_version_are_404() {
    let h = boot().await;
    let (_, session) = seed_agent_and_session(&h);
    let sid = session.0.to_string();
    put(&h, session, "known.txt", b"known");

    let unknown_session = uuid::Uuid::new_v4();
    let resp = request(
        &h.app,
        Method::GET,
        &format!("/api/sessions/{unknown_session}/artifacts"),
        None,
        None,
    )
    .await;
    assert_eq!(resp.status, StatusCode::NOT_FOUND);

    let resp = request(
        &h.app,
        Method::GET,
        &format!("/api/sessions/{sid}/artifacts/versions?path=missing.txt"),
        None,
        None,
    )
    .await;
    assert_eq!(resp.status, StatusCode::NOT_FOUND);

    let resp = request(
        &h.app,
        Method::GET,
        &format!("/api/sessions/{sid}/artifacts/download?path=known.txt&version=99"),
        None,
        None,
    )
    .await;
    assert_eq!(resp.status, StatusCode::NOT_FOUND);

    let resp = request(
        &h.app,
        Method::GET,
        &format!("/api/sessions/{sid}/artifacts/download?path=missing.txt&version=1"),
        None,
        None,
    )
    .await;
    assert_eq!(resp.status, StatusCode::NOT_FOUND);

    let resp = request(
        &h.app,
        Method::GET,
        "/api/sessions/not-a-uuid/artifacts",
        None,
        None,
    )
    .await;
    assert_eq!(resp.status, StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread")]
async fn restore_writes_exact_bytes_without_recording_a_version() {
    let h = boot().await;
    let (agent, session) = seed_agent_and_session(&h);
    let sid = session.0.to_string();
    let workspace = h
        .state
        .kernel
        .agent_registry()
        .get(agent)
        .expect("agent registered")
        .manifest
        .workspace
        .expect("spawned agent has a workspace");
    put(&h, session, "nested/restored.txt", b"restored-bytes");

    let resp = request(
        &h.app,
        Method::POST,
        &format!("/api/sessions/{sid}/artifacts/restore"),
        Some(serde_json::json!({"path": "nested/restored.txt", "version": 1})),
        None,
    )
    .await;
    assert_eq!(resp.status, StatusCode::OK, "{:?}", resp.json());
    let body = resp.json();
    assert_eq!(body["restored"], true);
    assert_eq!(body["path"], "nested/restored.txt");
    assert_eq!(body["version"], 1);
    assert_eq!(
        std::fs::read(workspace.join("nested/restored.txt")).expect("restored file"),
        b"restored-bytes"
    );

    let versions = store(&h)
        .versions(&sid, "nested/restored.txt")
        .expect("versions");
    assert_eq!(versions.len(), 1, "restore must not record a new version");
}

#[tokio::test(flavor = "multi_thread")]
async fn restore_refuses_traversal_absolute_and_symlink_escape() {
    let h = boot().await;
    let (agent, session) = seed_agent_and_session(&h);
    let sid = session.0.to_string();
    let workspace = h
        .state
        .kernel
        .agent_registry()
        .get(agent)
        .expect("agent registered")
        .manifest
        .workspace
        .expect("spawned agent has a workspace");

    put(&h, session, "../escape.txt", b"escaped");
    let resp = request(
        &h.app,
        Method::POST,
        &format!("/api/sessions/{sid}/artifacts/restore"),
        Some(serde_json::json!({"path": "../escape.txt", "version": 1})),
        None,
    )
    .await;
    assert_eq!(resp.status, StatusCode::FORBIDDEN);
    assert!(!workspace
        .parent()
        .expect("parent")
        .join("escape.txt")
        .exists());

    let absolute = workspace.join("absolute.txt");
    let absolute = absolute.to_string_lossy().to_string();
    put(&h, session, &absolute, b"absolute");
    let resp = request(
        &h.app,
        Method::POST,
        &format!("/api/sessions/{sid}/artifacts/restore"),
        Some(serde_json::json!({"path": absolute, "version": 1})),
        None,
    )
    .await;
    assert_eq!(resp.status, StatusCode::FORBIDDEN);
    assert!(!workspace.join("absolute.txt").exists());

    #[cfg(unix)]
    {
        let outside = tempfile::tempdir().expect("outside dir");
        std::os::unix::fs::symlink(outside.path(), workspace.join("escape-link"))
            .expect("plant symlink");
        put(&h, session, "escape-link/secret.txt", b"secret");
        let resp = request(
            &h.app,
            Method::POST,
            &format!("/api/sessions/{sid}/artifacts/restore"),
            Some(serde_json::json!({"path": "escape-link/secret.txt", "version": 1})),
            None,
        )
        .await;
        assert_eq!(
            resp.status,
            StatusCode::FORBIDDEN,
            "a symlinked ancestor must not become a write path"
        );
        assert!(!outside.path().join("secret.txt").exists());
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn restore_refuses_a_missing_workspace() {
    let h = boot().await;
    let agent = AgentId::new();
    let session = h
        .state
        .kernel
        .memory_substrate()
        .create_session(agent)
        .expect("create session");
    let sid = session.id.0.to_string();
    put(&h, session.id, "orphan.txt", b"orphan");

    let resp = request(
        &h.app,
        Method::POST,
        &format!("/api/sessions/{sid}/artifacts/restore"),
        Some(serde_json::json!({"path": "orphan.txt", "version": 1})),
        None,
    )
    .await;
    assert_eq!(resp.status, StatusCode::CONFLICT);
}

#[tokio::test(flavor = "multi_thread")]
async fn link_downloads_the_linked_version_only() {
    let h = boot().await;
    let (_, session) = seed_agent_and_session(&h);
    let sid = session.0.to_string();
    put(&h, session, "shared.bin", b"version-one");
    put(&h, session, "shared.bin", b"version-two");

    let before = chrono::Utc::now().timestamp();
    let resp = request(
        &h.app,
        Method::POST,
        &format!("/api/sessions/{sid}/artifacts/link"),
        Some(serde_json::json!({"path": "shared.bin", "version": 1})),
        None,
    )
    .await;
    assert_eq!(resp.status, StatusCode::OK);
    let body = resp.json();
    let token_one = body["token"].as_str().expect("token").to_string();
    assert_eq!(
        body["url"].as_str().expect("url"),
        format!("/api/artifacts/{token_one}")
    );
    let expires = body["expires_at"].as_i64().expect("expires_at");
    assert!((before + 890..=before + 910).contains(&expires), "{body}");

    let resp = request(
        &h.app,
        Method::GET,
        &format!("/api/artifacts/{token_one}"),
        None,
        None,
    )
    .await;
    assert_eq!(resp.status, StatusCode::OK);
    assert_eq!(resp.body, b"version-one");
    assert_eq!(header(&resp, "content-type"), "text/plain");
    assert_eq!(
        header(&resp, "content-disposition"),
        "attachment; filename=\"shared.bin\""
    );
    assert_eq!(header(&resp, "content-length"), "11");

    let resp = request(
        &h.app,
        Method::POST,
        &format!("/api/sessions/{sid}/artifacts/link"),
        Some(serde_json::json!({"path": "shared.bin"})),
        None,
    )
    .await;
    assert_eq!(resp.status, StatusCode::OK);
    let token_latest = resp.json()["token"].as_str().expect("token").to_string();
    let resp = request(
        &h.app,
        Method::GET,
        &format!("/api/artifacts/{token_latest}"),
        None,
        None,
    )
    .await;
    assert_eq!(resp.body, b"version-two");
}

#[tokio::test(flavor = "multi_thread")]
async fn expired_tampered_and_malformed_tokens_are_403() {
    let h = boot().await;
    let (_, session) = seed_agent_and_session(&h);
    let sid = session.0.to_string();
    put(&h, session, "signed.txt", b"signed");

    let key_path = h.state.kernel.home_dir().join("artifacts").join("link.key");
    let resp = request(
        &h.app,
        Method::POST,
        &format!("/api/sessions/{sid}/artifacts/link"),
        Some(serde_json::json!({"path": "signed.txt"})),
        None,
    )
    .await;
    assert_eq!(resp.status, StatusCode::OK);
    let key = std::fs::read(&key_path).expect("link key exists after link");
    assert_eq!(key.len(), 32);

    let expired = LinkClaims {
        session_id: sid.clone(),
        path: "signed.txt".to_string(),
        version: 1,
        expires_at: chrono::Utc::now().timestamp() - 1,
    };
    let token = sign_link_token(&key, &expired);
    let resp = request(
        &h.app,
        Method::GET,
        &format!("/api/artifacts/{token}"),
        None,
        None,
    )
    .await;
    assert_eq!(resp.status, StatusCode::FORBIDDEN);

    let resp = request(
        &h.app,
        Method::POST,
        &format!("/api/sessions/{sid}/artifacts/link"),
        Some(serde_json::json!({"path": "signed.txt"})),
        None,
    )
    .await;
    let token = resp.json()["token"].as_str().expect("token").to_string();
    let mut tampered = token.clone().into_bytes();
    let last = tampered.len() - 1;
    tampered[last] = if tampered[last] == b'A' { b'B' } else { b'A' };
    let tampered = String::from_utf8(tampered).expect("ASCII");
    let resp = request(
        &h.app,
        Method::GET,
        &format!("/api/artifacts/{tampered}"),
        None,
        None,
    )
    .await;
    assert_eq!(resp.status, StatusCode::FORBIDDEN);

    let resp = request(
        &h.app,
        Method::GET,
        "/api/artifacts/not-a-valid-token",
        None,
        None,
    )
    .await;
    assert_eq!(resp.status, StatusCode::FORBIDDEN);

    let missing_row = LinkClaims {
        session_id: sid.clone(),
        path: "gone.txt".to_string(),
        version: 1,
        expires_at: chrono::Utc::now().timestamp() + 60,
    };
    let token = sign_link_token(&key, &missing_row);
    let resp = request(
        &h.app,
        Method::GET,
        &format!("/api/artifacts/{token}"),
        None,
        None,
    )
    .await;
    assert_eq!(resp.status, StatusCode::NOT_FOUND);

    let record = store(&h)
        .versions(&sid, "signed.txt")
        .expect("versions")
        .remove(0);
    let blob = h
        .state
        .kernel
        .home_dir()
        .join("artifacts")
        .join("blobs")
        .join(&record.content_hash[..2])
        .join(&record.content_hash);
    std::fs::remove_file(blob).expect("remove blob");
    let claims = LinkClaims {
        session_id: sid,
        path: "signed.txt".to_string(),
        version: 1,
        expires_at: chrono::Utc::now().timestamp() + 60,
    };
    let token = sign_link_token(&key, &claims);
    let resp = request(
        &h.app,
        Method::GET,
        &format!("/api/artifacts/{token}"),
        None,
        None,
    )
    .await;
    assert_eq!(
        resp.status,
        StatusCode::NOT_FOUND,
        "a signed link whose blob is gone must 404"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn link_ttl_is_clamped() {
    let h = boot().await;
    let (_, session) = seed_agent_and_session(&h);
    let sid = session.0.to_string();
    put(&h, session, "ttl.txt", b"ttl");

    let now = chrono::Utc::now().timestamp();
    let resp = request(
        &h.app,
        Method::POST,
        &format!("/api/sessions/{sid}/artifacts/link"),
        Some(serde_json::json!({"path": "ttl.txt", "ttl_secs": 1})),
        None,
    )
    .await;
    assert_eq!(resp.status, StatusCode::OK);
    let expires = resp.json()["expires_at"].as_i64().expect("expires_at");
    assert!((now + 55..=now + 65).contains(&expires), "{expires}");

    let now = chrono::Utc::now().timestamp();
    let resp = request(
        &h.app,
        Method::POST,
        &format!("/api/sessions/{sid}/artifacts/link"),
        Some(serde_json::json!({"path": "ttl.txt", "ttl_secs": 999_999})),
        None,
    )
    .await;
    let expires = resp.json()["expires_at"].as_i64().expect("expires_at");
    assert!(
        (now + 86_395..=now + 86_405).contains(&expires),
        "{expires}"
    );
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn link_key_is_private_and_stable_across_calls() {
    use std::os::unix::fs::PermissionsExt as _;

    let h = boot().await;
    let (_, session) = seed_agent_and_session(&h);
    let sid = session.0.to_string();
    put(&h, session, "key.txt", b"key");
    let key_path = h.state.kernel.home_dir().join("artifacts").join("link.key");

    let mut first_key = None;
    for call in 0..2 {
        let resp = request(
            &h.app,
            Method::POST,
            &format!("/api/sessions/{sid}/artifacts/link"),
            Some(serde_json::json!({"path": "key.txt"})),
            None,
        )
        .await;
        assert_eq!(resp.status, StatusCode::OK);
        if call == 0 {
            first_key = Some(std::fs::read(&key_path).expect("link key"));
        }
    }

    let metadata = std::fs::metadata(&key_path).expect("link key");
    assert_eq!(metadata.len(), 32);
    assert_eq!(
        metadata.permissions().mode() & 0o777,
        0o600,
        "link.key must not be group/world readable"
    );
    assert_eq!(
        std::fs::read(&key_path).expect("link key"),
        first_key.expect("read after first call"),
        "the key must not rotate between link calls"
    );
}

/// Boot the full server router with an operator key and `require_auth_for_reads`.
async fn boot_full_with_strict_reads() -> (Harness, String) {
    let test = TestAppState::with_builder(MockKernelBuilder::new().with_config(|cfg| {
        cfg.api_key = "test-secret-key".to_string();
        cfg.require_auth_for_reads = Some(true);
    }));
    let (app, state) = server::build_router(
        test.state.kernel.clone(),
        "127.0.0.1:0".parse().expect("addr"),
    )
    .await;
    (
        Harness {
            app,
            state,
            _test: test,
        },
        "test-secret-key".to_string(),
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn signed_url_is_public_while_management_routes_require_auth() {
    let (h, api_key) = boot_full_with_strict_reads().await;
    let (_, session) = seed_agent_and_session(&h);
    let sid = session.0.to_string();
    put(&h, session, "public.txt", b"public-bytes");

    let resp = request(
        &h.app,
        Method::GET,
        &format!("/api/sessions/{sid}/artifacts"),
        None,
        None,
    )
    .await;
    assert_eq!(resp.status, StatusCode::UNAUTHORIZED);

    let resp = request(
        &h.app,
        Method::POST,
        &format!("/api/sessions/{sid}/artifacts/link"),
        Some(serde_json::json!({"path": "public.txt"})),
        None,
    )
    .await;
    assert_eq!(resp.status, StatusCode::UNAUTHORIZED);

    let resp = request(
        &h.app,
        Method::GET,
        &format!("/api/sessions/{sid}/artifacts"),
        None,
        Some(&api_key),
    )
    .await;
    assert_eq!(resp.status, StatusCode::OK);

    let resp = request(
        &h.app,
        Method::POST,
        &format!("/api/sessions/{sid}/artifacts/link"),
        Some(serde_json::json!({"path": "public.txt"})),
        Some(&api_key),
    )
    .await;
    assert_eq!(resp.status, StatusCode::OK);
    let token = resp.json()["token"].as_str().expect("token").to_string();

    let resp = request(
        &h.app,
        Method::GET,
        &format!("/api/artifacts/{token}"),
        None,
        None,
    )
    .await;
    assert_eq!(
        resp.status,
        StatusCode::OK,
        "a signed link must stay reachable when require_auth_for_reads is on"
    );
    assert_eq!(resp.body, b"public-bytes");
}
