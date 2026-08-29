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
    assert_eq!(fetched["provider"], "test-provider");
    assert_eq!(fetched["model"], "test-model");
    assert!(fetched["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t == "file_read"));
    assert!(fetched["skills"]
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
    assert_eq!(fetched["provider"], "updated-provider");
    assert_eq!(fetched["model"], "updated-model");
    assert!(fetched["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t == "shell_exec"));
    assert!(fetched["skills"]
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
    assert!(list["items"]
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

    // Create with a channel allowlist and preferred-model tiers.
    let create_body = serde_json::json!({
        "name": "channels-routing-roundtrip",
        "description": "round trip test",
        "system_prompt": "You are a test agent.",
        "provider": "test-provider",
        "model": "test-model",
        "tools": ["file_read"],
        "skills": ["test-skill"],
        "channels": ["telegram", "discord"],
        "routing": {
            "simple_model": "cheap-1",
            "medium_model": "mid-1",
            "complex_model": "big-1",
            "simple_threshold": 100,
            "complex_threshold": 900
        }
    });
    let req = Request::builder()
        .method(Method::POST)
        .uri("/templates")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&create_body).unwrap()))
        .unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    // GET — channels + routing must come back (defect 1: manifest_to_agent_type
    // used to omit both entirely).
    let req = Request::builder()
        .method(Method::GET)
        .uri("/templates/channels-routing-roundtrip")
        .body(Body::empty())
        .unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let fetched: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        fetched["channels"]
            .as_array()
            .expect("channels must be present after create")
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["telegram", "discord"]
    );
    assert_eq!(fetched["routing"]["simple_model"], "cheap-1");
    assert_eq!(fetched["routing"]["medium_model"], "mid-1");
    assert_eq!(fetched["routing"]["complex_model"], "big-1");
    assert_eq!(fetched["routing"]["simple_threshold"], 100);
    assert_eq!(fetched["routing"]["complex_threshold"], 900);

    // PUT with exactly what the GET returned — mirrors the dashboard's edit
    // flow (toForm() reads the detail GET, toInput() sends the flat shape
    // straight back on save). Strip the extras the detail GET adds on top
    // of the flat AgentType shape (source/manifest/manifest_toml) since the
    // dashboard's AgentTypeInput never sends those back.
    let mut put_body = fetched.clone();
    put_body["description"] = serde_json::Value::String("round trip test, saved again".to_string());
    if let Some(o) = put_body.as_object_mut() {
        o.remove("source");
        o.remove("manifest");
        o.remove("manifest_toml");
    }
    let req = Request::builder()
        .method(Method::PUT)
        .uri("/templates/channels-routing-roundtrip")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&put_body).unwrap()))
        .unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // GET again — channels + routing must still be intact. Before the fix,
    // the round-trip through the front's toForm()/toInput() pair would have
    // wiped both back to []/None on this second save.
    let req = Request::builder()
        .method(Method::GET)
        .uri("/templates/channels-routing-roundtrip")
        .body(Body::empty())
        .unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let refetched: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(refetched["description"], "round trip test, saved again");
    assert_eq!(
        refetched["channels"]
            .as_array()
            .expect("channels must survive the round trip")
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["telegram", "discord"]
    );
    assert_eq!(refetched["routing"]["simple_model"], "cheap-1");
    assert_eq!(refetched["routing"]["medium_model"], "mid-1");
    assert_eq!(refetched["routing"]["complex_model"], "big-1");
    assert_eq!(refetched["routing"]["simple_threshold"], 100);
    assert_eq!(refetched["routing"]["complex_threshold"], 900);

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
        fetched["description"],
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
// Full-manifest `manifest_toml` path (#7742) — the dashboard's
// `AgentManifestForm`-backed agent-type editor sends the whole document
// instead of the flat 9-key JSON shape the other tests above exercise.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn agent_types_create_accepts_manifest_toml_with_extended_fields() {
    let _guard = crud_lock().lock().expect("crud lock");
    let h = boot().await;

    // `[resources]` is a field the flat 9-key JSON shape has never covered.
    // The TOML's own `name` deliberately differs from the top-level JSON
    // `name` — the server must pin the manifest to the latter, exactly like
    // the flat-JSON create path already does.
    let manifest_toml = r#"
name = "wrong-name-inside-toml"
description = "created from full manifest"

[model]
provider = "test-provider"
model = "test-model"
system_prompt = "You are a test agent."

[resources]
max_llm_tokens_per_hour = 5000
"#;
    let body = serde_json::json!({
        "name": "manifest-toml-create-test",
        "manifest_toml": manifest_toml,
    });
    let req = Request::builder()
        .method(Method::POST)
        .uri("/templates")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    let req = Request::builder()
        .method(Method::GET)
        .uri("/templates/manifest-toml-create-test")
        .body(Body::empty())
        .unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let fetched: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(fetched["description"], "created from full manifest");
    let manifest_toml_out = fetched["manifest_toml"]
        .as_str()
        .expect("manifest_toml must be present on the detail GET");
    let manifest: librefang_types::agent::AgentManifest =
        toml::from_str(manifest_toml_out).unwrap();
    assert_eq!(
        manifest.name, "manifest-toml-create-test",
        "the template id (path/JSON name) must win over the name embedded in manifest_toml"
    );
    assert_eq!(manifest.resources.max_llm_tokens_per_hour, Some(5000));

    // Cleanup
    let req = Request::builder()
        .method(Method::DELETE)
        .uri("/templates/manifest-toml-create-test")
        .body(Body::empty())
        .unwrap();
    let _ = h.app.clone().oneshot(req).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn agent_types_update_manifest_toml_replaces_whole_document() {
    let _guard = crud_lock().lock().expect("crud lock");
    let home = agent_types_home();
    let h = boot().await;

    // Seed a template with a field the replacement document below never
    // mentions.
    let templates_dir = home.path().join("agent-types");
    std::fs::create_dir_all(&templates_dir).unwrap();
    let seed_toml = r#"
name = "manifest-toml-update-test"
description = "original"

[model]
provider = "test-provider"
model = "test-model"
system_prompt = "original prompt"

[resources]
max_llm_tokens_per_hour = 1000
"#;
    std::fs::write(
        templates_dir.join("manifest-toml-update-test.toml"),
        seed_toml,
    )
    .unwrap();

    // PUT with a full `manifest_toml` document — the full-replace path
    // (#7742), distinct from the flat-JSON merge path exercised by
    // `agent_types_update_preserves_unmentioned_manifest_fields` above: a
    // field the new document doesn't mention (`resources`) is gone
    // afterwards, not preserved, because the whole document becomes the new
    // source of truth — exactly like `PATCH /agents/{id}` with
    // `manifest_toml` does for a live agent. This is safe for the
    // dashboard's editor because it always seeds the form (and its
    // `extras`) from this same template's current `manifest_toml`, so a
    // save that "doesn't mention" a field really means the user's form
    // never carried it forward, not that the server invented a merge gap.
    // The name embedded in the TOML deliberately differs from the path
    // segment to assert the server still pins it.
    let new_manifest_toml = r#"
name = "some-other-name"
description = "updated via full manifest"

[model]
provider = "test-provider"
model = "test-model"
system_prompt = "updated prompt"
"#;
    let body = serde_json::json!({ "manifest_toml": new_manifest_toml });
    let req = Request::builder()
        .method(Method::PUT)
        .uri("/templates/manifest-toml-update-test")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let content =
        std::fs::read_to_string(templates_dir.join("manifest-toml-update-test.toml")).unwrap();
    let manifest: librefang_types::agent::AgentManifest = toml::from_str(&content).unwrap();
    assert_eq!(
        manifest.name, "manifest-toml-update-test",
        "the path segment must win over the name embedded in manifest_toml"
    );
    assert_eq!(manifest.description, "updated via full manifest");
    assert_eq!(manifest.model.system_prompt, "updated prompt");
    assert_eq!(
        manifest.resources.max_llm_tokens_per_hour, None,
        "manifest_toml is a full replace, not a merge — fields the new document omits are gone"
    );

    // Cleanup
    let req = Request::builder()
        .method(Method::DELETE)
        .uri("/templates/manifest-toml-update-test")
        .body(Body::empty())
        .unwrap();
    let _ = h.app.clone().oneshot(req).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn agent_types_create_rejects_invalid_manifest_toml() {
    let _guard = crud_lock().lock().expect("crud lock");
    let h = boot().await;
    let body = serde_json::json!({
        "name": "bad-manifest-toml-create",
        "manifest_toml": "this is not [valid toml",
    });
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
async fn agent_types_update_rejects_invalid_manifest_toml() {
    let _guard = crud_lock().lock().expect("crud lock");
    let h = boot().await;
    let create_body = agent_type_json("bad-manifest-toml-update", "seed");
    let req = Request::builder()
        .method(Method::POST)
        .uri("/templates")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&create_body).unwrap()))
        .unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    let body = serde_json::json!({ "manifest_toml": "this is not [valid toml" });
    let req = Request::builder()
        .method(Method::PUT)
        .uri("/templates/bad-manifest-toml-update")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // Cleanup
    let req = Request::builder()
        .method(Method::DELETE)
        .uri("/templates/bad-manifest-toml-update")
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
