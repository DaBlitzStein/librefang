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

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use librefang_api::routes::{self, AppState};
use librefang_testing::{MockKernelBuilder, TestAppState};
use std::sync::Arc;
use tower::ServiceExt;

struct Harness {
    app: Router,
    _state: Arc<AppState>,
    _test: TestAppState,
}

async fn boot() -> Harness {
    let (state, test) = MockKernelBuilder::default()
        .build_app_state()
        .await
        .expect("boot kernel for agent-types test");
    let app = routes::agent_templates::router()
        .with_state(Arc::new(state.clone()))
        .into_make_service();
    let app = axum::Router::new().merge(axum::Router::from(app));
    Harness {
        app,
        _state: state.into(),
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

#[tokio::test]
async fn agent_types_list_returns_200() {
    let h = boot().await;
    let req = Request::builder()
        .method(Method::GET)
        .uri("/templates")
        .body(Body::empty())
        .unwrap();
    let resp = h.app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

// ---------------------------------------------------------------------------
// Create → read → update → delete lifecycle
// ---------------------------------------------------------------------------

#[tokio::test]
async fn agent_types_crud_lifecycle() {
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
    let resp = h.app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    // Read the one we just created
    let req = Request::builder()
        .method(Method::GET)
        .uri("/templates/test-agent-crud")
        .body(Body::empty())
        .unwrap();
    let resp = h.app.oneshot(req).await.unwrap();
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
    let resp = h.app.oneshot(req).await.unwrap();
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
    let resp = h.app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Confirm deleted — should 404
    let req = Request::builder()
        .method(Method::GET)
        .uri("/templates/test-agent-crud")
        .body(Body::empty())
        .unwrap();
    let resp = h.app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn agent_types_create_rejects_missing_name() {
    let h = boot().await;
    let body = serde_json::json!({"description": "no name"});
    let req = Request::builder()
        .method(Method::POST)
        .uri("/templates")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = h.app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn agent_types_rejects_path_traversal() {
    let h = boot().await;
    let req = Request::builder()
        .method(Method::GET)
        .uri("/templates/../../../etc/passwd")
        .body(Body::empty())
        .unwrap();
    let resp = h.app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn agent_types_create_rejects_invalid_name_chars() {
    let h = boot().await;
    let body = agent_type_json("bad name", "has spaces");
    let req = Request::builder()
        .method(Method::POST)
        .uri("/templates")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = h.app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn agent_types_create_rejects_duplicate() {
    let h = boot().await;
    let body = agent_type_json("duplicate-test", "first create");
    let req = Request::builder()
        .method(Method::POST)
        .uri("/templates")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = h.app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    // Same name again → 409
    let req = Request::builder()
        .method(Method::POST)
        .uri("/templates")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = h.app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);

    // Cleanup
    let req = Request::builder()
        .method(Method::DELETE)
        .uri("/templates/duplicate-test")
        .body(Body::empty())
        .unwrap();
    let _ = h.app.oneshot(req).await;
}

#[tokio::test]
async fn agent_types_get_nonexistent_returns_404() {
    let h = boot().await;
    let req = Request::builder()
        .method(Method::GET)
        .uri("/templates/nonexistent-type")
        .body(Body::empty())
        .unwrap();
    let resp = h.app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn agent_types_list_includes_created_items() {
    let h = boot().await;
    let body = agent_type_json("list-include-test", "for list test");
    // Create it
    let req = Request::builder()
        .method(Method::POST)
        .uri("/templates")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let _ = h.app.oneshot(req).await.unwrap();

    // List — should include it
    let req = Request::builder()
        .method(Method::GET)
        .uri("/templates")
        .body(Body::empty())
        .unwrap();
    let resp = h.app.oneshot(req).await.unwrap();
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
    let _ = h.app.oneshot(req).await;
}

// ---------------------------------------------------------------------------
// TOML injection — ensure special characters are escaped
// ---------------------------------------------------------------------------

#[tokio::test]
async fn agent_types_escapes_toml_special_chars() {
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
    let resp = h.app.oneshot(req).await.unwrap();
    // Must not 500 — the TOML serializer should escape everything
    assert_eq!(resp.status(), StatusCode::CREATED);

    // Read back — round-trip should preserve the content
    let req = Request::builder()
        .method(Method::GET)
        .uri("/templates/toml-inject-test")
        .body(Body::empty())
        .unwrap();
    let resp = h.app.oneshot(req).await.unwrap();
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
    let _ = h.app.oneshot(req).await;
}
