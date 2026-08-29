//! Integration tests for agent-types CRUD and ephemeral-agent spawn routes.
//!
//! Covered routes (refs #6699):
//!   * `GET    /api/templates`          — list all agent types
//!   * `GET    /api/templates/{name}`   — get one agent type
//!   * `POST   /api/templates`          — create an agent type
//!   * `PUT    /api/templates/{name}`   — update an agent type
//!   * `DELETE /api/templates/{name}`   — delete an agent type
//!
//! These tests boot a real `LibreFangKernel` via `MockKernelBuilder` (no
//! networking, no LLM credentials) and drive the agent-types router via
//! `tower::ServiceExt::oneshot`.

// The process-wide `crud_lock` is intentionally held across the whole test
// body (including awaits): it serializes CRUD tests against each other so
// they do not interleave kernel state. A test-only std mutex, held for the
// test's duration, is exactly its purpose.
#![allow(clippy::await_holding_lock)]

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use librefang_api::routes::{self, AppState};
use librefang_testing::{MockKernelBuilder, TestAppState};
use std::sync::{Arc, Mutex, OnceLock};
use tower::ServiceExt;

struct Harness {
    app: Router,
    _state: Arc<AppState>,
    _test: TestAppState,
}

/// One tempdir for the whole test binary, set once via OnceLock — the
/// same pattern as `profiles_templates_routes_integration.rs`. Setting
/// `LIBREFANG_HOME` per-test races with every other concurrent test
/// that calls `librefang_home()` (#6931 review).
fn agent_types_home() -> &'static tempfile::TempDir {
    static HOME: OnceLock<tempfile::TempDir> = OnceLock::new();
    HOME.get_or_init(|| {
        let tmp = tempfile::tempdir().expect("tempdir for agent-types test");
        // Safety: set once, before any concurrent reader — the standard
        // workspace pattern for env-var-driven tests (Rust 2024 edition
        // requires the unsafe block for env mutation).
        std::env::set_var("LIBREFANG_HOME", tmp.path());
        tmp
    })
}

/// Serialise CRUD tests so concurrent creates don't observe each other's
/// fixtures when listing (list walks the whole templates dir).
fn crud_lock() -> &'static Mutex<()> {
    static LOCK: Mutex<()> = Mutex::new(());
    &LOCK
}

async fn boot() -> Harness {
    let _home = agent_types_home();
    let test = TestAppState::with_builder(MockKernelBuilder::new().with_config(|cfg| {
        cfg.default_model = librefang_types::config::DefaultModelConfig {
            provider: "ollama".to_string(),
            model: "test-model".to_string(),
            api_key_env: "OLLAMA_API_KEY".to_string(),
            base_url: None,
            message_timeout_secs: 300,
            extra_params: std::collections::BTreeMap::new(),
            cli_profile_dirs: Vec::new(),
        };
    }));
    let state = test.app_state();
    let app = routes::agent_templates::router().with_state(state.clone());
    Harness {
        app,
        _state: state,
        _test: test,
    }
}

fn agent_type_json(name: &str, desc: &str) -> serde_json::Value {
    serde_json::json!({
        "name": name,
        "description": desc,
        "system_prompt": "You are a test agent.",
        "provider": "test-provider",
        "model": "test-model",
        "tools": ["file_read", "web_fetch"],
        "skills": ["test-skill"]
    })
}

// ---------------------------------------------------------------------------
// List
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn agent_types_list_returns_200() {
    let h = boot().await;
    let req = Request::builder()
        .method(Method::GET)
        .uri("/templates")
        .body(Body::empty())
        .unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

// ---------------------------------------------------------------------------
// Create → read → update → delete lifecycle
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn agent_types_crud_lifecycle() {
    let _guard = crud_lock().lock().expect("crud lock");
    let h = boot().await;
    let body = agent_type_json("test-agent-crud", "A test agent for CRUD.");
    let body_bytes = serde_json::to_vec(&body).unwrap();

    // Create
    let req = Request::builder()
        .method(Method::POST)
        .uri("/templates")
        .header("content-type", "application/json")
        .body(Body::from(body_bytes.clone()))
        .unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    // Read the one we just created
    let req = Request::builder()
        .method(Method::GET)
        .uri("/templates/test-agent-crud")
        .body(Body::empty())
        .unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let fetched: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(fetched["name"], "test-agent-crud");
    assert_eq!(fetched["spec"]["provider"], "test-provider");
    assert_eq!(fetched["spec"]["model"], "test-model");
    assert!(fetched["spec"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t == "file_read"));
    assert!(fetched["spec"]["skills"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s == "test-skill"));

    // Update — change description and tools
    let updated = serde_json::json!({
        "name": "test-agent-crud",
        "description": "Updated description.",
        "system_prompt": "You are an updated test agent.",
        "provider": "updated-provider",
        "model": "updated-model",
        "tools": ["shell_exec"],
        "skills": ["updated-skill"]
    });
    let req = Request::builder()
        .method(Method::PUT)
        .uri("/templates/test-agent-crud")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&updated).unwrap()))
        .unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let fetched: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(fetched["name"], "test-agent-crud");
    assert_eq!(fetched["spec"]["provider"], "updated-provider");
    assert_eq!(fetched["spec"]["model"], "updated-model");
    assert!(fetched["spec"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t == "shell_exec"));
    assert!(fetched["spec"]["skills"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s == "updated-skill"));

    // Delete
    let req = Request::builder()
        .method(Method::DELETE)
        .uri("/templates/test-agent-crud")
        .body(Body::empty())
        .unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Confirm deleted — should 404
    let req = Request::builder()
        .method(Method::GET)
        .uri("/templates/test-agent-crud")
        .body(Body::empty())
        .unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn agent_types_create_rejects_missing_name() {
    let h = boot().await;
    let body = serde_json::json!({"description": "no name"});
    let req = Request::builder()
        .method(Method::POST)
        .uri("/templates")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread")]
async fn agent_types_rejects_path_traversal() {
    let h = boot().await;
    let req = Request::builder()
        .method(Method::GET)
        .uri("/templates/../../../etc/passwd")
        .body(Body::empty())
        .unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn agent_types_create_rejects_invalid_name_chars() {
    let h = boot().await;
    let body = agent_type_json("bad name", "has spaces");
    let req = Request::builder()
        .method(Method::POST)
        .uri("/templates")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread")]
async fn agent_types_create_rejects_duplicate() {
    let _guard = crud_lock().lock().expect("crud lock");
    let h = boot().await;
    let body = agent_type_json("duplicate-test", "first create");
    let req = Request::builder()
        .method(Method::POST)
        .uri("/templates")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    // Same name again → 409
    let req = Request::builder()
        .method(Method::POST)
        .uri("/templates")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);

    // Cleanup
    let req = Request::builder()
        .method(Method::DELETE)
        .uri("/templates/duplicate-test")
        .body(Body::empty())
        .unwrap();
    let _ = h.app.clone().oneshot(req).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn agent_types_get_nonexistent_returns_404() {
    let h = boot().await;
    let req = Request::builder()
        .method(Method::GET)
        .uri("/templates/nonexistent-type")
        .body(Body::empty())
        .unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn agent_types_list_includes_created_items() {
    let _guard = crud_lock().lock().expect("crud lock");
    let h = boot().await;
    let body = agent_type_json("list-include-test", "for list test");
    // Create it
    let req = Request::builder()
        .method(Method::POST)
        .uri("/templates")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let _ = h.app.clone().oneshot(req).await.unwrap();

    // List — should include it
    let req = Request::builder()
        .method(Method::GET)
        .uri("/templates")
        .body(Body::empty())
        .unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let list: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(list["templates"]
        .as_array()
        .unwrap()
        .iter()
        .any(|i| i["name"] == "list-include-test"));

    // Cleanup
    let req = Request::builder()
        .method(Method::DELETE)
        .uri("/templates/list-include-test")
        .body(Body::empty())
        .unwrap();
    let _ = h.app.clone().oneshot(req).await;
}

// ---------------------------------------------------------------------------
// #7740 — PUT must not destroy manifest fields outside the flat JSON shape
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn agent_types_update_preserves_unmentioned_manifest_fields() {
    let _guard = crud_lock().lock().expect("crud lock");
    let home = agent_types_home();
    let h = boot().await;

    // Seed a template file directly on disk with fields the flat "agent
    // type" JSON shape has never covered: [compaction] and
    // max_history_messages. `#[serde(default)]` on `AgentManifest` means
    // every other field can be left out of the fixture.
    let templates_dir = home.path().join("agent-types");
    std::fs::create_dir_all(&templates_dir).unwrap();
    // `max_history_messages` must sit before the `[model]` table header —
    // once a TOML table opens, every bare `key = value` line belongs to
    // that table until the next header, so placing it after `[model]`
    // would silently fold it into `ModelConfig`'s `#[serde(flatten)]
    // extra_params` bag instead of the top-level `AgentManifest` field.
    let seed_toml = r#"
name = "compaction-preserve-test"
description = "original description"
max_history_messages = 42

[model]
provider = "test-provider"
model = "test-model"
system_prompt = "You are a test agent."

[compaction]
threshold_messages = 30
keep_recent = 10
"#;
    std::fs::write(
        templates_dir.join("compaction-preserve-test.toml"),
        seed_toml,
    )
    .unwrap();

    // PUT with the plain flat body the dashboard sends — it says nothing
    // about compaction or max_history_messages at all.
    let body = serde_json::json!({
        "name": "compaction-preserve-test",
        "description": "updated description",
        "system_prompt": "You are an updated test agent.",
        "provider": "test-provider",
        "model": "test-model",
        "tools": [],
        "skills": []
    });
    let req = Request::builder()
        .method(Method::PUT)
        .uri("/templates/compaction-preserve-test")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Read the file back off disk — the edited field must have changed,
    // and the fields the request never mentioned must still be there.
    let content =
        std::fs::read_to_string(templates_dir.join("compaction-preserve-test.toml")).unwrap();
    let manifest: librefang_types::agent::AgentManifest = toml::from_str(&content).unwrap();
    assert_eq!(manifest.description, "updated description");
    assert_eq!(
        manifest.max_history_messages,
        Some(42),
        "max_history_messages must survive an update that never mentions it: {content}"
    );
    let compaction = manifest
        .compaction
        .unwrap_or_else(|| panic!("[compaction] must survive the update: {content}"));
    assert_eq!(compaction.threshold_messages, Some(30));
    assert_eq!(compaction.keep_recent, Some(10));

    // Cleanup
    let req = Request::builder()
        .method(Method::DELETE)
        .uri("/templates/compaction-preserve-test")
        .body(Body::empty())
        .unwrap();
    let _ = h.app.clone().oneshot(req).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn agent_types_channels_and_routing_round_trip_through_update() {
    let _guard = crud_lock().lock().expect("crud lock");
    let h = boot().await;

    // Seed via filesystem — channels live outside the flat AgentTypeSpec
    // shape, so they can only enter through the manifest file on disk.
    let home = agent_types_home();
    let templates_dir = home.path().join("agent-types");
    std::fs::create_dir_all(&templates_dir).unwrap();
    let seed_toml = r#"
name = "channels-routing-roundtrip"
description = "round trip test"
channels = ["telegram", "discord"]

[model]
provider = "test-provider"
model = "test-model"
system_prompt = "You are a test agent."

[capabilities]
tools = ["file_read"]
"#;
    std::fs::write(
        templates_dir.join("channels-routing-roundtrip.toml"),
        seed_toml,
    )
    .unwrap();

    // PUT with the flat spec shape — it touches only the 7 spec fields,
    // so channels must survive untouched (the whole point of #7740).
    let put_body = serde_json::json!({
        "description": "round trip test, saved again",
    });
    let req = Request::builder()
        .method(Method::PUT)
        .uri("/templates/channels-routing-roundtrip")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&put_body).unwrap()))
        .unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Read the file back off disk — channels must still be there.
    let content =
        std::fs::read_to_string(templates_dir.join("channels-routing-roundtrip.toml")).unwrap();
    let manifest: librefang_types::agent::AgentManifest = toml::from_str(&content).unwrap();
    assert_eq!(manifest.description, "round trip test, saved again");
    assert_eq!(
        manifest.channels,
        vec!["telegram", "discord"],
        "channels must survive a flat-spec PUT that never mentions them: {content}"
    );

    // Cleanup
    let req = Request::builder()
        .method(Method::DELETE)
        .uri("/templates/channels-routing-roundtrip")
        .body(Body::empty())
        .unwrap();
    let _ = h.app.clone().oneshot(req).await;
}

// ---------------------------------------------------------------------------
// TOML injection — ensure special characters are escaped
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn agent_types_escapes_toml_special_chars() {
    let _guard = crud_lock().lock().expect("crud lock");
    let h = boot().await;
    let body = serde_json::json!({
        "name": "toml-inject-test",
        "description": "desc with \"quotes\" and \\ backslashes and \n newlines",
        "system_prompt": "prompt with \"quotes\"",
        "provider": "test",
        "model": "test",
        "tools": ["file_read"],
        "skills": []
    });
    let req = Request::builder()
        .method(Method::POST)
        .uri("/templates")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    // Must not 500 — the TOML serializer should escape everything
    assert_eq!(resp.status(), StatusCode::CREATED);

    // Read back — round-trip should preserve the content
    let req = Request::builder()
        .method(Method::GET)
        .uri("/templates/toml-inject-test")
        .body(Body::empty())
        .unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let fetched: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        fetched["spec"]["description"],
        "desc with \"quotes\" and \\ backslashes and \n newlines"
    );

    // Cleanup
    let req = Request::builder()
        .method(Method::DELETE)
        .uri("/templates/toml-inject-test")
        .body(Body::empty())
        .unwrap();
    let _ = h.app.clone().oneshot(req).await;
}

// ---------------------------------------------------------------------------
// #6931 review: PUT with a non-object JSON body must not panic
// ---------------------------------------------------------------------------
//
// `serde_json::Value`'s `IndexMut<&str>` only handles `Null` and `Object`;
// every other variant (array, string, number, bool) panics on
// `body["name"] = ...`. `Json<serde_json::Value>` happily deserializes any
// of those shapes, so an existing agent-type plus a non-object `PUT` body
// used to bring the whole process down.

#[tokio::test(flavor = "multi_thread")]
async fn agent_types_update_rejects_array_body() {
    let _guard = crud_lock().lock().expect("crud lock");
    let h = boot().await;
    let create_body = agent_type_json("non-object-body-array", "seed");
    let req = Request::builder()
        .method(Method::POST)
        .uri("/templates")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&create_body).unwrap()))
        .unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    let req = Request::builder()
        .method(Method::PUT)
        .uri("/templates/non-object-body-array")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&serde_json::json!([])).unwrap(),
        ))
        .unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // Cleanup
    let req = Request::builder()
        .method(Method::DELETE)
        .uri("/templates/non-object-body-array")
        .body(Body::empty())
        .unwrap();
    let _ = h.app.clone().oneshot(req).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn agent_types_update_rejects_number_body() {
    let _guard = crud_lock().lock().expect("crud lock");
    let h = boot().await;
    let create_body = agent_type_json("non-object-body-number", "seed");
    let req = Request::builder()
        .method(Method::POST)
        .uri("/templates")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&create_body).unwrap()))
        .unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    let req = Request::builder()
        .method(Method::PUT)
        .uri("/templates/non-object-body-number")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&serde_json::json!(42)).unwrap(),
        ))
        .unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // Cleanup
    let req = Request::builder()
        .method(Method::DELETE)
        .uri("/templates/non-object-body-number")
        .body(Body::empty())
        .unwrap();
    let _ = h.app.clone().oneshot(req).await;
}
