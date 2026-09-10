//! Dead-route audit (refs #3721, Phase 1).
//!
//! Catches a recurring class of bugs where a handler is added in
//! `crates/librefang-api/src/routes/*.rs` (and annotated with
//! `#[utoipa::path(...)]`, so it appears in the OpenAPI surface) but
//! the corresponding `.route(...)` registration in
//! `crates/librefang-api/src/server.rs` (or one of its sub-routers) is
//! forgotten. The handler is then unreachable at runtime even though
//! the spec advertises it.
//!
//! Strategy:
//! 1. Boot the real production router via `server::build_router()`.
//! 2. Iterate every path in the spec **as served** by
//!    `/api/openapi.json` — which includes the `/api/v1/*` copies the
//!    handler injects, and which `ApiDoc::openapi()` alone does not.
//! 3. For each path, iterate every declared HTTP method (GET, POST, PUT,
//!    DELETE, PATCH, …) and dispatch one request per (path, method) pair.
//!    Non-GET methods send an empty JSON body (`{}`) with
//!    `Content-Type: application/json` — enough to reach the handler
//!    without triggering deserialization failures at the router level.
//! 4. Distinguish *router-level* 404 (path not registered) from
//!    *handler-level* 404 (handler ran and decided "agent not found")
//!    by inspecting the response. Every real handler in the codebase
//!    returns JSON when it produces a 404 (`ApiErrorResponse` and
//!    `application/json`), so **a 404 that is not JSON is a dead
//!    route**. That invariant is the test, rather than a match on one
//!    fallback's wire shape: axum's top-level fallback answers
//!    `text/plain` + `"Not Found"`, but a `.nest()`ed router answers a
//!    bare 404 with no content-type and an empty body, and the earlier
//!    `text/plain` match saw the second one as a live route.
//!    A `405 Method Not Allowed` means the *path* is wired
//!    even if this specific method is not — that still proves the route
//!    registration exists. Anything else — `200`, `204`, `400`, `401`,
//!    `403`, `405`, JSON `404`, `415`, `422`, `5xx`, … — means the route
//!    is wired and the handler ran.
//!
//! This is the automated replacement for Steps 4-6 of the legacy
//! "Live Integration Testing" curl checklist that lived in CLAUDE.md.
//! Phase 2 (payload smoke against TestServer for hot-path endpoints)
//! and Phase 3 (live LLM metering side-effect verification) are
//! tracked as follow-ups under #3721.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::response::IntoResponse;
use axum::Router;
use librefang_api::routes::AppState;
use librefang_api::server;
use librefang_kernel::LibreFangKernel;
use librefang_types::config::{DefaultModelConfig, KernelConfig};
use std::collections::BTreeSet;
use std::sync::Arc;
use tempfile::TempDir;
use tower::ServiceExt;

/// Substitute for `{param}` segments in OpenAPI path templates. Chosen to
/// be a simple ASCII identifier so it satisfies axum's path matchers
/// (which accept any non-`/` segment by default) and is unlikely to be
/// confused with a real entity ID.
const PATH_PLACEHOLDER: &str = "_audit_placeholder";

/// Paths the audit must skip. Each entry needs an explicit reason — we
/// do **not** want this list to absorb genuine bugs. Keep it tiny.
fn skip_paths() -> BTreeSet<&'static str> {
    BTreeSet::from([
        // OpenAPI spec endpoint itself is mounted by `build_router` via a
        // dedicated `.merge(SwaggerUi::...)` call rather than a single
        // `.route()`. Hitting it would still return 200, so it is harmless
        // either way; listed here purely for documentation completeness.
        // (No actual skip needed, but keep the set machinery in place so
        // future additions have a clear precedent.)
    ])
}

/// Boot a full production router on top of an in-memory tempdir-backed
/// kernel. Mirrors the `start_full_router` helper used elsewhere in
/// `tests/api_integration_test.rs` but kept self-contained so this file
/// can compile independently.
async fn boot_full_router() -> (Router, Arc<AppState>, TempDir) {
    let tmp = tempfile::tempdir().expect("Failed to create temp dir");

    // Seed the pinned registry fixture so the kernel boots without warnings.
    librefang_kernel::registry_sync::seed_registry_fixture_for_tests(tmp.path());

    let config = KernelConfig {
        home_dir: tmp.path().to_path_buf(),
        data_dir: tmp.path().join("data"),
        // Empty api_key disables auth (`is_public` allowlist still
        // applies, but most routes accept the request without auth at
        // all when no key is configured). This keeps the audit focused
        // on routing, not authentication — a 401 from a configured-key
        // run would still pass the "not 404" assertion, but skipping
        // auth here makes the failure mode singular and obvious.
        api_key: String::new(),
        // Without this the audit rubber-stamps most of the surface.
        //
        // It dispatches one request per (path, method) pair as fast as the
        // router will take them, and `api_requests_per_minute` defaults to
        // 500. Measured on `main` before this change: of 416 dispatches, 334
        // came back `429`, the first at dispatch #82 — and a 429 is not a 404,
        // so every one of them counted as "route is wired". Four fifths of the
        // audit was inert, and would have stayed green with four fifths of the
        // routes deleted.
        rate_limit: librefang_types::config::RateLimitConfig {
            api_requests_per_minute: u32::MAX,
            ..Default::default()
        },
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

    let kernel = LibreFangKernel::boot_with_config(config).expect("Kernel should boot");
    let kernel = Arc::new(kernel);
    kernel.set_self_handle();

    let (app, state) = server::build_router(
        kernel,
        "127.0.0.1:0".parse().expect("listen addr should parse"),
    )
    .await;

    (app, state, tmp)
}

/// Replace every `{name}` segment in a path template with the audit
/// placeholder. The placeholder is the same for every parameter — we
/// only care that the segment matches axum's matcher, not that the
/// downstream handler can find a real entity.
fn substitute_path_params(template: &str) -> String {
    let mut out = String::with_capacity(template.len());
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '{' {
            // Skip until matching '}'
            for inner in chars.by_ref() {
                if inner == '}' {
                    break;
                }
            }
            out.push_str(PATH_PLACEHOLDER);
        } else {
            out.push(c);
        }
    }
    out
}

#[tokio::test(flavor = "multi_thread")]
async fn dead_route_audit_every_openapi_path_is_registered_in_router() {
    let (app, state, _tmp) = boot_full_router().await;

    // Audit the spec **as served**, not `ApiDoc::openapi()`.
    //
    // The `/api/openapi.json` handler injects an `/api/v1/*` copy of every
    // `/api/*` path before sending it (`openapi::openapi_spec`), so the static
    // document this audit used to read carries only half of what clients
    // receive. That is how `/api/v1/versions` reached production advertised
    // and unserved: no guard had ever dispatched a single `/api/v1/*` path.
    // Calls the handler that serves `/api/openapi.json` rather than dispatching
    // through the router: this harness boots without an API key and the spec
    // route answers 401 there, which would tell us nothing about routing.
    let spec_resp = librefang_api::openapi::openapi_spec().await.into_response();
    assert_eq!(
        spec_resp.status(),
        StatusCode::OK,
        "the audit reads the served spec, so the spec handler must produce one"
    );
    let spec_bytes = axum::body::to_bytes(spec_resp.into_body(), usize::MAX)
        .await
        .expect("served spec body must read");
    let parsed: serde_json::Value =
        serde_json::from_slice(&spec_bytes).expect("served OpenAPI spec must be valid JSON");

    let paths = parsed["paths"]
        .as_object()
        .expect("OpenAPI spec must declare a `paths` object");

    let skip = skip_paths();
    let mut missing: Vec<String> = Vec::new();
    let mut audited: usize = 0;
    let total_paths = paths.len();

    for (template, ops) in paths {
        if skip.contains(template.as_str()) {
            continue;
        }

        let request_path = substitute_path_params(template);

        // Collect every HTTP method declared for this path in OpenAPI.
        // The operations object has keys like "get", "post", "put", etc.
        let methods: Vec<String> = ops
            .as_object()
            .map(|obj| {
                obj.keys()
                    .filter(|k| {
                        matches!(
                            k.to_lowercase().as_str(),
                            "get" | "post" | "put" | "delete" | "patch" | "head" | "options"
                        )
                    })
                    .map(|k| k.to_uppercase())
                    .collect()
            })
            .unwrap_or_default();

        // Fall back to GET if OpenAPI has no recognised method keys
        // (should not happen in a well-formed spec, but be defensive).
        let methods = if methods.is_empty() {
            vec!["GET".to_string()]
        } else {
            methods
        };

        for method in &methods {
            let body = if method == "GET" || method == "HEAD" || method == "DELETE" {
                Body::empty()
            } else {
                Body::from("{}")
            };

            let mut builder = Request::builder()
                .method(method.as_str())
                .uri(&request_path);
            if method != "GET" && method != "HEAD" && method != "DELETE" {
                builder = builder.header("content-type", "application/json");
            }

            let response = app
                .clone()
                .oneshot(
                    builder
                        .body(body)
                        .expect("synthetic audit request should build"),
                )
                .await
                .expect("router oneshot must not panic");

            audited += 1;
            let status = response.status();

            // 405 Method Not Allowed means the *path* is registered in axum —
            // the route wiring exists even though this specific verb isn't
            // handled. That is not a dead route.
            if status != StatusCode::NOT_FOUND {
                continue;
            }

            // Distinguish router-fallback 404 from handler 404. Axum's
            // default `not_found` service returns `text/plain` + the
            // literal body "Not Found". Real handlers that return 404
            // (e.g. "agent not found") use `ApiErrorResponse` which is
            // serialized as `application/json`.
            let content_type = response
                .headers()
                .get(axum::http::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string();
            // A 404 that is not JSON did not come from a handler.
            //
            // The previous rule matched one specific shape — `text/plain` with
            // the body `Not Found` — which is what axum's *top-level* fallback
            // produces. A `.nest()`ed router answers an unmatched path with a
            // bare 404: no content-type, empty body. `/api/v1/versions` was
            // exactly that, so the audit read it as a live route.
            //
            // Asserting the invariant the module header already states — every
            // real 404 in this codebase is an `ApiErrorResponse`, serialized as
            // `application/json` — covers both fallbacks and any third shape a
            // future axum release invents.
            // `status` is already known to be 404 here.
            let looks_like_router_fallback = !content_type.starts_with("application/json");

            if looks_like_router_fallback {
                missing.push(format!("{method} {template}"));
            }
        }
    }

    // Cleanup before assertion so a failure does not leak the kernel.
    state.kernel.shutdown();

    assert!(
        audited >= total_paths,
        "expected to audit at least {total_paths} (path, method) pairs \
         (one per OpenAPI path), only saw {audited} — \
         either the spec regressed or the audit logic broke"
    );

    assert!(
        missing.is_empty(),
        "Dead-route audit found {} OpenAPI (method, path) pair(s) that returned \
         a router-level 404 (a 404 whose body is not `application/json`, so no \
         handler produced it). Each entry \
         below is declared via `#[utoipa::path]` on a handler in \
         `crates/librefang-api/src/routes/` but is missing a matching \
         `.route(...)` registration in `crates/librefang-api/src/server.rs` \
         (or one of the sub-routers it merges). Add the registration or, \
         if the path was retired, remove the `#[utoipa::path]` annotation.\n\n{:#?}",
        missing.len(),
        missing,
    );
}

#[test]
fn substitute_path_params_replaces_every_placeholder() {
    assert_eq!(
        substitute_path_params("/api/agents/{id}/sessions/{session_id}/trajectory"),
        format!(
            "/api/agents/{p}/sessions/{p}/trajectory",
            p = PATH_PLACEHOLDER
        ),
    );
    assert_eq!(substitute_path_params("/api/health"), "/api/health");
    assert_eq!(
        substitute_path_params("/api/tools/{name}"),
        format!("/api/tools/{}", PATH_PLACEHOLDER),
    );
}

/// Version discovery is advertised unversioned, and only unversioned.
///
/// The audit above would also pass if someone deleted `/api/versions` from the
/// spec altogether, so pin the shape that is actually intended: the endpoint a
/// client reaches *before* it knows which version to ask for exists at `/api`
/// and is not advertised under `/api/v1`, where the router does not serve it.
#[tokio::test(flavor = "multi_thread")]
async fn version_discovery_is_advertised_unversioned_only() {
    let spec_resp = librefang_api::openapi::openapi_spec().await.into_response();
    let spec_bytes = axum::body::to_bytes(spec_resp.into_body(), usize::MAX)
        .await
        .expect("served spec body must read");
    let spec: serde_json::Value = serde_json::from_slice(&spec_bytes).expect("valid JSON");
    let paths = spec["paths"].as_object().expect("paths object");

    assert!(
        paths.contains_key("/api/versions"),
        "version discovery must stay in the spec"
    );
    assert!(
        !paths.contains_key("/api/v1/versions"),
        "`/api/versions` is registered outside `api_v1_routes()`, so a \
         `/api/v1` copy advertises a route the router answers with a 404"
    );
}
