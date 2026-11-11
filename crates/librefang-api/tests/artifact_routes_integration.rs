//! Integration tests for the session artifact HTTP API and signed links.

use axum::body::Body;
use axum::http::{HeaderMap, Method, Request, StatusCode};
use axum::Router;
use librefang_api::middleware::{AuthenticatedApiUser, UserRole};
use librefang_api::routes::{
    self,
    artifacts::{sign_link_token, LinkClaims},
    AppState,
};
use librefang_api::server;
use librefang_memory::artifact_store::{ArtifactRecord, ArtifactStore};
use librefang_testing::{MockKernelBuilder, TestAppState};
use librefang_types::agent::{AgentId, AgentManifest, SessionId, UserId};
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
    let agent = spawn_authored(&h.state, &format!("artifact-test-{}", uuid::Uuid::new_v4()));
    let session = h
        .state
        .kernel
        .memory_substrate()
        .create_session(agent)
        .expect("create session");
    (agent, session.id)
}

fn spawn_authored(state: &AppState, author: &str) -> AgentId {
    state
        .kernel
        .spawn_agent_typed(AgentManifest {
            name: format!("artifact-test-{}", uuid::Uuid::new_v4()),
            author: author.to_string(),
            ..AgentManifest::default()
        })
        .expect("spawn agent")
}

fn store_for(state: &AppState) -> &ArtifactStore {
    state
        .kernel
        .memory_substrate()
        .artifacts()
        .expect("boot attaches the artifact store")
}

fn store(h: &Harness) -> &ArtifactStore {
    store_for(&h.state)
}

fn put_for(state: &AppState, session: SessionId, path: &str, bytes: &[u8]) -> ArtifactRecord {
    store_for(state)
        .put(
            &session.0.to_string(),
            None,
            path,
            bytes,
            Some("text/plain"),
        )
        .expect("record artifact")
}

fn put(h: &Harness, session: SessionId, path: &str, bytes: &[u8]) -> ArtifactRecord {
    put_for(&h.state, session, path, bytes)
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

fn build_request(
    method: Method,
    uri: &str,
    body: Option<Value>,
    bearer: Option<&str>,
) -> Request<Body> {
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
    builder.body(Body::from(bytes)).expect("request")
}

async fn send(app: &Router, request: Request<Body>) -> Response {
    let response = app.clone().oneshot(request).await.expect("oneshot");
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

async fn request(
    app: &Router,
    method: Method,
    uri: &str,
    body: Option<Value>,
    bearer: Option<&str>,
) -> Response {
    send(app, build_request(method, uri, body, bearer)).await
}

async fn request_as_user(
    app: &Router,
    method: Method,
    uri: &str,
    body: Option<Value>,
    user: AuthenticatedApiUser,
) -> Response {
    let mut request = build_request(method, uri, body, None);
    request.extensions_mut().insert(user);
    send(app, request).await
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

    // The path segment is caller-supplied; store lookups must use the parsed
    // canonical UUID on every management route, or an uppercase spelling
    // returns an empty list / 404 and cannot restore or link.
    let upper = sid.to_uppercase();
    let resp = request(
        &h.app,
        Method::GET,
        &format!("/api/sessions/{upper}/artifacts"),
        None,
        None,
    )
    .await;
    assert_eq!(resp.status, StatusCode::OK);
    assert_eq!(
        resp.json()["artifacts"]
            .as_array()
            .expect("artifacts")
            .len(),
        1
    );

    let resp = request(
        &h.app,
        Method::GET,
        &format!("/api/sessions/{upper}/artifacts/versions?path=docs/report.txt"),
        None,
        None,
    )
    .await;
    assert_eq!(resp.status, StatusCode::OK);
    assert_eq!(
        resp.json()["versions"].as_array().expect("versions").len(),
        2
    );

    let resp = request(
        &h.app,
        Method::GET,
        &format!("/api/sessions/{upper}/artifacts/download?path=docs/report.txt&version=1"),
        None,
        None,
    )
    .await;
    assert_eq!(resp.status, StatusCode::OK);
    assert_eq!(resp.body, b"first");

    let resp = request(
        &h.app,
        Method::POST,
        &format!("/api/sessions/{upper}/artifacts/restore"),
        Some(serde_json::json!({"path": "docs/report.txt", "version": 1})),
        None,
    )
    .await;
    assert_eq!(resp.status, StatusCode::OK, "{:?}", resp.json());

    let resp = request(
        &h.app,
        Method::POST,
        &format!("/api/sessions/{upper}/artifacts/link"),
        Some(serde_json::json!({"path": "docs/report.txt"})),
        None,
    )
    .await;
    assert_eq!(resp.status, StatusCode::OK);

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
async fn cross_owner_session_artifacts_are_404_and_own_session_works() {
    let h = boot().await;
    let bob_agent = spawn_authored(&h.state, "Bob");
    let bob_session = h
        .state
        .kernel
        .memory_substrate()
        .create_session(bob_agent)
        .expect("create Bob session")
        .id;
    put_for(&h.state, bob_session, "bob.txt", b"bob-bytes");

    let alice_agent = spawn_authored(&h.state, "Alice");
    let alice_session = h
        .state
        .kernel
        .memory_substrate()
        .create_session(alice_agent)
        .expect("create Alice session")
        .id;
    put_for(&h.state, alice_session, "alice.txt", b"alice-bytes");

    let alice = AuthenticatedApiUser {
        name: "Alice".to_string(),
        role: UserRole::User,
        user_id: UserId::from_name("Alice"),
    };
    let bob_sid = bob_session.0.to_string();
    let cases = [
        (
            Method::GET,
            format!("/api/sessions/{bob_sid}/artifacts"),
            None,
        ),
        (
            Method::GET,
            format!("/api/sessions/{bob_sid}/artifacts/versions?path=bob.txt"),
            None,
        ),
        (
            Method::GET,
            format!("/api/sessions/{bob_sid}/artifacts/download?path=bob.txt&version=1"),
            None,
        ),
        (
            Method::POST,
            format!("/api/sessions/{bob_sid}/artifacts/restore"),
            Some(serde_json::json!({"path": "bob.txt", "version": 1})),
        ),
        (
            Method::POST,
            format!("/api/sessions/{bob_sid}/artifacts/link"),
            Some(serde_json::json!({"path": "bob.txt"})),
        ),
    ];
    for (method, uri, body) in cases {
        let resp = request_as_user(&h.app, method.clone(), &uri, body, alice.clone()).await;
        assert_eq!(
            resp.status,
            StatusCode::NOT_FOUND,
            "{method} {uri} must not disclose another owner's session"
        );
    }

    let alice_sid = alice_session.0.to_string();
    let resp = request_as_user(
        &h.app,
        Method::GET,
        &format!("/api/sessions/{alice_sid}/artifacts"),
        None,
        alice.clone(),
    )
    .await;
    assert_eq!(resp.status, StatusCode::OK);
    assert_eq!(
        resp.json()["artifacts"]
            .as_array()
            .expect("artifacts")
            .len(),
        1
    );

    let resp = request_as_user(
        &h.app,
        Method::GET,
        &format!("/api/sessions/{alice_sid}/artifacts/download?path=alice.txt&version=1"),
        None,
        alice,
    )
    .await;
    assert_eq!(resp.status, StatusCode::OK);
    assert_eq!(resp.body, b"alice-bytes");
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
async fn restore_refuses_the_agent_manifest() {
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
    put(&h, session, "agent.toml", b"pwned = true\n");

    let resp = request(
        &h.app,
        Method::POST,
        &format!("/api/sessions/{sid}/artifacts/restore"),
        Some(serde_json::json!({"path": "agent.toml", "version": 1})),
        None,
    )
    .await;
    assert_eq!(resp.status, StatusCode::FORBIDDEN);
    assert_ne!(
        std::fs::read(workspace.join("agent.toml")).unwrap_or_default(),
        b"pwned = true\n",
        "an artifact must not overwrite the agent's own manifest"
    );
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

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn link_key_symlink_and_loose_mode_fail_closed() {
    use std::os::unix::fs::PermissionsExt as _;

    let h = boot().await;
    let (_, session) = seed_agent_and_session(&h);
    let sid = session.0.to_string();
    put(&h, session, "key.txt", b"key");
    let blob_root = h.state.kernel.home_dir().join("artifacts");
    std::fs::create_dir_all(&blob_root).expect("blob root");
    let key_path = blob_root.join("link.key");

    let outside = tempfile::tempdir().expect("outside dir");
    let target = outside.path().join("planted.key");
    std::fs::write(&target, [7u8; 32]).expect("planted key");
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600))
        .expect("planted key mode");
    std::os::unix::fs::symlink(&target, &key_path).expect("plant symlink");
    let resp = request(
        &h.app,
        Method::POST,
        &format!("/api/sessions/{sid}/artifacts/link"),
        Some(serde_json::json!({"path": "key.txt"})),
        None,
    )
    .await;
    assert_eq!(
        resp.status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "a symlinked link.key must fail closed: {:?}",
        resp.json()
    );

    std::fs::remove_file(&key_path).expect("remove symlink");
    std::fs::write(&key_path, [9u8; 32]).expect("loose key");
    std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o644))
        .expect("loose key mode");
    let resp = request(
        &h.app,
        Method::POST,
        &format!("/api/sessions/{sid}/artifacts/link"),
        Some(serde_json::json!({"path": "key.txt"})),
        None,
    )
    .await;
    assert_eq!(
        resp.status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "a 0644 link.key must fail closed: {:?}",
        resp.json()
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
        Method::POST,
        &format!("/api/sessions/{sid}/artifacts/restore"),
        Some(serde_json::json!({"path": "public.txt", "version": 1})),
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

    // Only GET is public. A POST to the token path must hit the auth layer,
    // not the public GET allowlist.
    let resp = request(
        &h.app,
        Method::POST,
        &format!("/api/artifacts/{token}"),
        None,
        None,
    )
    .await;
    assert_eq!(resp.status, StatusCode::UNAUTHORIZED);
}
