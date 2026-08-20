//! Integration tests for the `/api/agents/{id}/model_routing` route family.
//!
//! Refs #7741 — the TUI's model-routing editor (`r` on the agent detail
//! screen) silently discarded the edit on Enter because no `AgentAction`
//! was ever emitted, so nothing on the wire could persist it. There was
//! also no dedicated route to persist `model.mode` /
//! `model.router_override` for an *existing* agent at all — those fields
//! could previously only be set at creation time via the custom-agent
//! wizard's `manifest_toml`. These tests exercise the production router
//! (`server::build_router`) with `tower::ServiceExt::oneshot`, mirroring
//! `agent_channels_routes_test.rs`. No real LLM calls — every test is
//! hermetic.
//!
//! Routes covered:
//!   GET /api/agents/{id}/model_routing   (default shape, flexible shape)
//!   PUT /api/agents/{id}/model_routing   (set + read-back, clear back to
//!                                         fixed, bad id 400, unknown agent)
//!
//! Run: cargo test -p librefang-api --test agent_model_routing_routes_test

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use librefang_api::routes::AppState;
use librefang_api::server;
use librefang_kernel::LibreFangKernel;
use librefang_types::agent::{AgentId, AgentManifest};
use librefang_types::config::{DefaultModelConfig, KernelConfig};
use std::sync::Arc;
use tower::ServiceExt;

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

struct Harness {
    app: axum::Router,
    state: Arc<AppState>,
    _tmp: tempfile::TempDir,
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.state.kernel.shutdown();
    }
}

const TEST_TOKEN: &str = "test-secret";

async fn boot() -> Harness {
    let tmp = tempfile::tempdir().expect("tempdir");

    librefang_kernel::registry_sync::seed_registry_fixture_for_tests(tmp.path());

    let config = KernelConfig {
        home_dir: tmp.path().to_path_buf(),
        data_dir: tmp.path().join("data"),
        api_key: TEST_TOKEN.to_string(),
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
    };

    let kernel = LibreFangKernel::boot_with_config(config).expect("kernel boot");
    let kernel = Arc::new(kernel);
    kernel.set_self_handle();

    let (app, state) = server::build_router(kernel, "127.0.0.1:0".parse().expect("addr")).await;

    Harness {
        app,
        state,
        _tmp: tmp,
    }
}

fn spawn_named(state: &Arc<AppState>, name: &str) -> AgentId {
    let manifest = AgentManifest {
        name: name.to_string(),
        ..AgentManifest::default()
    };
    state
        .kernel
        .spawn_agent_typed(manifest)
        .expect("spawn_agent")
}

async fn send(app: axum::Router, req: Request<Body>) -> (StatusCode, serde_json::Value) {
    let resp = app.oneshot(req).await.expect("oneshot");
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("body");
    let json = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    };
    (status, json)
}

fn get(path: &str) -> Request<Body> {
    Request::builder()
        .method(Method::GET)
        .uri(path)
        .header("authorization", format!("Bearer {TEST_TOKEN}"))
        .body(Body::empty())
        .unwrap()
}

fn put_json(path: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method(Method::PUT)
        .uri(path)
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {TEST_TOKEN}"))
        .body(Body::from(body.to_string()))
        .unwrap()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// GET /api/agents/{id}/model_routing on a freshly spawned agent must
/// return the backward-compatible default: fixed mode, no profiles, no cap.
#[tokio::test(flavor = "multi_thread")]
async fn get_model_routing_default_shape() {
    let h = boot().await;
    let id = spawn_named(&h.state, "routing-default");

    let (status, body) = send(
        h.app.clone(),
        get(&format!("/api/agents/{id}/model_routing")),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body={body:?}");
    assert_eq!(body["mode"], "fixed");
    assert_eq!(body["allowed_profiles"], serde_json::json!([]));
    assert!(body["cost_budget"].is_null());
}

/// PUT then GET round-trip: switching to flexible mode with a profile
/// allowlist and a cost budget is reflected on read-back. This is the
/// literal regression scenario from #7741 — the TUI's Enter handler used to
/// go straight back to the detail screen without ever calling this route.
#[tokio::test(flavor = "multi_thread")]
async fn put_model_routing_flexible_roundtrip() {
    let h = boot().await;
    let id = spawn_named(&h.state, "routing-roundtrip");

    let (put_status, put_body) = send(
        h.app.clone(),
        put_json(
            &format!("/api/agents/{id}/model_routing"),
            serde_json::json!({
                "mode": "flexible",
                "allowed_profiles": ["coder", "architect"],
                "cost_budget": "medium",
            }),
        ),
    )
    .await;
    assert_eq!(put_status, StatusCode::OK, "PUT body={put_body:?}");
    assert_eq!(put_body["status"], "ok");

    let (get_status, get_body) = send(
        h.app.clone(),
        get(&format!("/api/agents/{id}/model_routing")),
    )
    .await;
    assert_eq!(get_status, StatusCode::OK, "GET body={get_body:?}");
    assert_eq!(get_body["mode"], "flexible");
    let profiles = get_body["allowed_profiles"]
        .as_array()
        .expect("allowed_profiles array");
    let mut names: Vec<&str> = profiles.iter().filter_map(|v| v.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, vec!["architect", "coder"]);
    assert_eq!(get_body["cost_budget"], "medium");
}

/// PUT back to fixed mode must clear the router override entirely, not
/// just leave stale profile/budget data hanging off the manifest.
#[tokio::test(flavor = "multi_thread")]
async fn put_model_routing_fixed_clears_override() {
    let h = boot().await;
    let id = spawn_named(&h.state, "routing-clear");

    // First switch to flexible with some override state.
    send(
        h.app.clone(),
        put_json(
            &format!("/api/agents/{id}/model_routing"),
            serde_json::json!({
                "mode": "flexible",
                "allowed_profiles": ["writer"],
                "cost_budget": "expensive",
            }),
        ),
    )
    .await;

    // Then switch back to fixed.
    let (put_status, _) = send(
        h.app.clone(),
        put_json(
            &format!("/api/agents/{id}/model_routing"),
            serde_json::json!({"mode": "fixed"}),
        ),
    )
    .await;
    assert_eq!(put_status, StatusCode::OK);

    let (get_status, get_body) = send(
        h.app.clone(),
        get(&format!("/api/agents/{id}/model_routing")),
    )
    .await;
    assert_eq!(get_status, StatusCode::OK, "GET body={get_body:?}");
    assert_eq!(get_body["mode"], "fixed");
    assert_eq!(
        get_body["allowed_profiles"],
        serde_json::json!([]),
        "switching back to fixed must clear the stale flexible-mode allowlist"
    );
    assert!(get_body["cost_budget"].is_null());
}

/// `cost_budget: "default"` (the TUI's own placeholder for "no cap") must
/// not be written to the manifest as a literal `CostTier` value — it isn't
/// a valid variant and would otherwise corrupt the agent's manifest.
#[tokio::test(flavor = "multi_thread")]
async fn put_model_routing_default_budget_means_no_cap() {
    let h = boot().await;
    let id = spawn_named(&h.state, "routing-default-budget");

    let (put_status, put_body) = send(
        h.app.clone(),
        put_json(
            &format!("/api/agents/{id}/model_routing"),
            serde_json::json!({
                "mode": "flexible",
                "allowed_profiles": ["coder"],
                "cost_budget": "default",
            }),
        ),
    )
    .await;
    assert_eq!(put_status, StatusCode::OK, "PUT body={put_body:?}");

    let (get_status, get_body) = send(
        h.app.clone(),
        get(&format!("/api/agents/{id}/model_routing")),
    )
    .await;
    assert_eq!(get_status, StatusCode::OK, "GET body={get_body:?}");
    assert!(
        get_body["cost_budget"].is_null(),
        "\"default\" must resolve to no cap, got {:?}",
        get_body["cost_budget"]
    );
}

/// GET or PUT with a non-UUID agent ID must return 400.
#[tokio::test(flavor = "multi_thread")]
async fn model_routing_bad_agent_id_returns_400() {
    let h = boot().await;

    let (get_status, _) = send(h.app.clone(), get("/api/agents/not-a-uuid/model_routing")).await;
    assert_eq!(get_status, StatusCode::BAD_REQUEST, "GET must be 400");

    let (put_status, _) = send(
        h.app.clone(),
        put_json(
            "/api/agents/not-a-uuid/model_routing",
            serde_json::json!({"mode": "fixed"}),
        ),
    )
    .await;
    assert_eq!(put_status, StatusCode::BAD_REQUEST, "PUT must be 400");
}

/// GET with a valid UUID that doesn't exist must return 404. PUT with a
/// valid UUID that doesn't exist returns 400 (agent-not-found error
/// propagated through the kernel, consistent with set_agent_skills /
/// set_agent_mcp_servers / set_agent_channels which also return 400 for
/// all kernel errors).
#[tokio::test(flavor = "multi_thread")]
async fn model_routing_unknown_agent_returns_error() {
    let h = boot().await;
    let unknown = AgentId::new();

    let (get_status, _) = send(
        h.app.clone(),
        get(&format!("/api/agents/{unknown}/model_routing")),
    )
    .await;
    assert_eq!(get_status, StatusCode::NOT_FOUND, "GET must be 404");

    let (put_status, _) = send(
        h.app.clone(),
        put_json(
            &format!("/api/agents/{unknown}/model_routing"),
            serde_json::json!({"mode": "fixed"}),
        ),
    )
    .await;
    assert_eq!(
        put_status,
        StatusCode::BAD_REQUEST,
        "PUT must be 400 (kernel error)"
    );
}
