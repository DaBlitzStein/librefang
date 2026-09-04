//! Integration tests for the body-size limits on `POST /api/agents/{id}/upload` (#8181).
//!
//! The upload router carries its own, larger `RequestBodyLimitLayer` sized to `max_upload_size_bytes`, and the comment above it has claimed since it was written that uploads therefore bypass the global `max_request_body_bytes` cap.
//! They did not: the global layer was applied to the finished `app`, which wraps every route already registered, merged sub-routers included, and the smaller of two nested limits is the one that cuts.
//! An operator who raised `max_upload_size_bytes` to 100 MB still had every upload over the 1 MB global cap killed mid-stream, which reaches a browser as `NetworkError` rather than a status.
//!
//! These tests run against the production router (`server::build_router`), because that is where the ordering lives — a hand-rolled router in the test would assert on a layer stack nobody serves.
//!
//! Run: cargo test -p librefang-api --test upload_body_limit_test

use axum::body::Body;
use axum::http::{Request, StatusCode};
use librefang_api::routes::AppState;
use librefang_api::server;
use librefang_kernel::LibreFangKernel;
use librefang_types::config::{DefaultModelConfig, KernelConfig};
use std::sync::Arc;
use tower::ServiceExt;

const TEST_TOKEN: &str = "test-secret";

/// `AgentId` is a UUID and the handler parses it before anything else, so a name-shaped id gets a 400 that never reaches the body.
/// The agent need not exist: the upload handler writes the file and does not look the agent up.
const TEST_AGENT_ID: &str = "00000000-0000-4000-8000-000000000001";

/// 1 KiB global cap against a 64 KiB upload cap: small enough that an 8 KiB body sits unambiguously between them, so a test body cannot accidentally satisfy both.
const GLOBAL_BODY_CAP: usize = 1024;
const UPLOAD_BODY_CAP: usize = 64 * 1024;
const BETWEEN_THE_CAPS: usize = 8 * 1024;

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

async fn boot() -> Harness {
    let tmp = tempfile::tempdir().expect("tempdir");

    let config = KernelConfig {
        home_dir: tmp.path().to_path_buf(),
        data_dir: tmp.path().join("data"),
        api_key: TEST_TOKEN.to_string(),
        max_request_body_bytes: GLOBAL_BODY_CAP,
        max_upload_size_bytes: UPLOAD_BODY_CAP,
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

    let kernel = Arc::new(LibreFangKernel::boot_with_config(config).expect("kernel boot"));
    kernel.clone().set_self_handle();
    let (app, state) = server::build_router(kernel, "127.0.0.1:0".parse().unwrap()).await;

    Harness {
        app,
        state,
        _tmp: tmp,
    }
}

fn upload_request(len: usize) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(format!("/api/agents/{TEST_AGENT_ID}/upload"))
        .header("authorization", format!("Bearer {TEST_TOKEN}"))
        .header("content-type", "text/plain")
        .header("x-filename", "attachment.txt")
        .header("content-length", len.to_string())
        .body(Body::from(vec![b'a'; len]))
        .expect("request")
}

/// The regression itself: a body the operator's `max_upload_size_bytes` allows and the global cap does not must reach the handler.
#[tokio::test(flavor = "multi_thread")]
async fn upload_between_the_global_cap_and_the_upload_cap_is_accepted() {
    let harness = boot().await;

    let response = harness
        .app
        .clone()
        .oneshot(upload_request(BETWEEN_THE_CAPS))
        .await
        .expect("router responds");

    assert_eq!(
        response.status(),
        StatusCode::CREATED,
        "an {BETWEEN_THE_CAPS}-byte upload is under max_upload_size_bytes ({UPLOAD_BODY_CAP}) and must be accepted even though it exceeds max_request_body_bytes ({GLOBAL_BODY_CAP})"
    );
}

/// A body over the upload cap gets an answer, not a dropped connection, and the answer names the cap.
#[tokio::test(flavor = "multi_thread")]
async fn upload_over_the_upload_cap_gets_a_413_naming_the_cap() {
    let harness = boot().await;

    let response = harness
        .app
        .clone()
        .oneshot(upload_request(UPLOAD_BODY_CAP + 1))
        .await
        .expect("router responds");

    assert_eq!(
        response.status(),
        StatusCode::PAYLOAD_TOO_LARGE,
        "a body over max_upload_size_bytes must be refused with a status the client can render"
    );

    let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("error body");
    let json: serde_json::Value = serde_json::from_slice(&bytes).expect("error body is JSON");
    assert_eq!(
        json["max_upload_size_bytes"], UPLOAD_BODY_CAP,
        "the refusal must tell the client which cap it hit, so 'too large' is distinguishable from 'daemon unreachable': {json}"
    );
}

/// The control for the fix: moving the global limit off `app` must not remove it from the routes it was protecting.
#[tokio::test(flavor = "multi_thread")]
async fn the_global_cap_still_applies_off_the_upload_path() {
    let harness = boot().await;

    let response = harness
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/agents/{TEST_AGENT_ID}/message"))
                .header("authorization", format!("Bearer {TEST_TOKEN}"))
                // Not `application/json`, so the JSON depth guard forwards it untouched and the body limit is the only thing that can reject it.
                .header("content-type", "text/plain")
                .header("content-length", BETWEEN_THE_CAPS.to_string())
                .body(Body::from(vec![b'a'; BETWEEN_THE_CAPS]))
                .expect("request"),
        )
        .await
        .expect("router responds");

    assert_eq!(
        response.status(),
        StatusCode::PAYLOAD_TOO_LARGE,
        "a non-upload route must still be capped at max_request_body_bytes ({GLOBAL_BODY_CAP})"
    );
}
