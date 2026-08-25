//! Cross-user authorization coverage for agent, session, observability, and trigger routes.
//! Requests run through the real domain routers and auth middleware so `AuthenticatedApiUser` extraction is exercised end to end.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use librefang_api::middleware;
use librefang_api::routes::{self, AppState};
use librefang_kernel::auth::UserRole as KernelUserRole;
use librefang_testing::{MockKernelBuilder, TestAppState};
use librefang_types::agent::{AgentId, AgentManifest, UserId};
use librefang_types::config::UserConfig;
use std::sync::Arc;
use tower::ServiceExt;

const ALICE_KEY: &str = "alice-owner-scope-key";
const BOB_KEY: &str = "bob-owner-scope-key";
const ADMIN_KEY: &str = "admin-owner-scope-key";

struct Harness {
    app: Router,
    state: Arc<AppState>,
    _tmp: tempfile::TempDir,
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.state.kernel.shutdown();
    }
}

async fn boot() -> Harness {
    let users = [
        ("Alice", "user", ALICE_KEY),
        ("Bob", "user", BOB_KEY),
        ("Admin", "admin", ADMIN_KEY),
    ];
    let mut configs = Vec::new();
    let mut auth_users = Vec::new();
    for (name, role, key) in users {
        let hash = librefang_api::password_hash::hash_password(key).expect("hash test key");
        configs.push(UserConfig {
            name: name.to_string(),
            role: role.to_string(),
            api_key_hash: Some(hash.clone()),
            ..Default::default()
        });
        auth_users.push(middleware::ApiUserAuth {
            name: name.to_string(),
            role: KernelUserRole::from_str_role(role),
            api_key_hash: hash,
            user_id: UserId::from_name(name),
        });
    }

    let test = TestAppState::with_builder(MockKernelBuilder::new().with_config(move |cfg| {
        cfg.api_key = "owner-scope-master-key".to_string();
        cfg.users = configs;
        // Alice is on the support rota, Bob is not (#7744 / #7745). Declared
        // on the shared harness so the group-ownership cases exercise the real
        // `AuthManager` membership resolution rather than a stub — membership
        // is derived from this config at boot, so a group nobody is in would
        // make those assertions pass for the wrong reason.
        cfg.user_groups = vec![librefang_types::config::UserGroup {
            id: "support".to_string(),
            name: "Support".to_string(),
            description: String::new(),
            members: ["Alice".to_string()].into_iter().collect(),
        }];
    }))
    .with_api_key("owner-scope-master-key")
    .with_user_api_keys(auth_users);
    let (state, tmp, _) = test.into_parts();
    state.kernel.clone().set_self_handle();

    let auth_state = middleware::AuthState {
        api_key_lock: state.api_key_lock.clone(),
        master_key: state.master_key.clone(),
        active_sessions: state.active_sessions.clone(),
        dashboard_auth_enabled: false,
        user_api_keys: state.user_api_keys.clone(),
        // `true`, not the test-harness-typical `false`: production's own `derive_require_auth_for_reads` (server.rs) auto-enables this whenever any authentication is configured, which this harness's `[[users]]` always does.
        // Every dashboard-read-public route (including bare `GET /api/agents`) must go through the real bearer check here, or `AuthenticatedApiUser` is never populated for those routes and the ownership assertions below would pass vacuously — see `non_admin_cannot_override_owner_filter_on_list_agents`.
        require_auth_for_reads: true,
        allow_no_auth: false,
        audit_log: Some(state.kernel.audit().clone()),
    };
    let app = Router::new()
        .nest("/api", routes::agents::router())
        .nest("/api", routes::workflows::router())
        .layer(axum::middleware::from_fn_with_state(
            auth_state,
            middleware::auth,
        ))
        .with_state(state.clone());

    Harness {
        app,
        state,
        _tmp: tmp,
    }
}

fn spawn_authored(state: &AppState, author: &str) -> AgentId {
    state
        .kernel
        .spawn_agent_typed(AgentManifest {
            name: format!("owner-scope-{}", uuid::Uuid::new_v4()),
            author: author.to_string(),
            ..AgentManifest::default()
        })
        .expect("spawn authored test agent")
}

async fn request_status(
    app: &Router,
    method: Method,
    path: &str,
    bearer: &str,
    body: Option<serde_json::Value>,
) -> StatusCode {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {bearer}"));
    let body = match body {
        Some(value) => {
            builder = builder.header("content-type", "application/json");
            Body::from(serde_json::to_vec(&value).expect("serialize request"))
        }
        None => Body::empty(),
    };
    app.clone()
        .oneshot(builder.body(body).expect("build request"))
        .await
        .expect("route response")
        .status()
}

#[tokio::test(flavor = "multi_thread")]
async fn non_owner_cannot_read_agent_scoped_resources() {
    let h = boot().await;
    let agent_id = spawn_authored(&h.state, "Alice");
    let created = h
        .state
        .kernel
        .create_agent_session(agent_id, Some("owner-scope-test"))
        .expect("create materialized session");
    let session_id: librefang_types::agent::SessionId = created["session_id"]
        .as_str()
        .expect("created session id")
        .parse()
        .expect("parse created session id");
    let aid = agent_id.to_string();
    let sid = session_id.to_string();
    let cases = vec![
        (Method::GET, format!("/api/agents/{aid}"), None),
        (Method::GET, format!("/api/agents/{aid}/runtime"), None),
        (Method::GET, format!("/api/agents/{aid}/tools"), None),
        (Method::GET, format!("/api/agents/{aid}/skills"), None),
        (Method::GET, format!("/api/agents/{aid}/mcp_servers"), None),
        (Method::GET, format!("/api/agents/{aid}/channels"), None),
        (Method::GET, format!("/api/agents/{aid}/files"), None),
        (
            Method::GET,
            format!("/api/agents/{aid}/files/AGENT.md"),
            None,
        ),
        (Method::GET, format!("/api/agents/{aid}/deliveries"), None),
        (Method::GET, format!("/api/agents/{aid}/traces"), None),
        (Method::GET, format!("/api/agents/{aid}/metrics"), None),
        (Method::GET, format!("/api/agents/{aid}/logs"), None),
        (Method::GET, format!("/api/agents/{aid}/session"), None),
        (
            Method::GET,
            format!("/api/agents/{aid}/session/context"),
            None,
        ),
        (Method::GET, format!("/api/agents/{aid}/sessions"), None),
        (
            Method::GET,
            format!("/api/agents/{aid}/sessions/{sid}/stream"),
            None,
        ),
        (
            Method::GET,
            format!("/api/agents/{aid}/sessions/{sid}/export"),
            None,
        ),
        (
            Method::GET,
            format!("/api/agents/{aid}/sessions/{sid}/trajectory"),
            None,
        ),
    ];

    let mut failures = Vec::new();
    for (method, path, body) in cases {
        let status = request_status(&h.app, method, &path, BOB_KEY, body).await;
        if status != StatusCode::NOT_FOUND {
            failures.push(format!("{path}: expected 404, got {status}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[tokio::test(flavor = "multi_thread")]
async fn non_admin_agent_session_mutations_are_blocked_by_rbac_middleware() {
    let h = boot().await;
    let agent_id = spawn_authored(&h.state, "Alice");
    let created = h
        .state
        .kernel
        .create_agent_session(agent_id, Some("mutation-rbac-test"))
        .expect("create materialized session");
    let session_id = created["session_id"].as_str().expect("session id");
    let export = h
        .state
        .kernel
        .export_session(agent_id, session_id.parse().expect("parse session id"))
        .expect("export initial session");
    let aid = agent_id.to_string();
    let cases = vec![
        (
            Method::POST,
            format!("/api/agents/{aid}/sessions"),
            Some(serde_json::json!({})),
        ),
        (
            Method::POST,
            format!("/api/agents/{aid}/sessions/{session_id}/switch"),
            None,
        ),
        (
            Method::POST,
            format!("/api/agents/{aid}/sessions/import"),
            Some(serde_json::to_value(export).expect("serialize export")),
        ),
        (
            Method::POST,
            format!("/api/agents/{aid}/session/reset"),
            None,
        ),
        (
            Method::POST,
            format!("/api/agents/{aid}/session/reboot"),
            None,
        ),
        (Method::DELETE, format!("/api/agents/{aid}/history"), None),
        (
            Method::POST,
            format!("/api/agents/{aid}/session/compact"),
            None,
        ),
        (
            Method::POST,
            format!("/api/agents/{aid}/sessions/{session_id}/stop"),
            None,
        ),
    ];
    for (method, path, body) in cases {
        let status = request_status(&h.app, method, &path, BOB_KEY, body).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{path}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn trigger_routes_enforce_owner_read_and_rbac_mutations() {
    use librefang_kernel::triggers::TriggerPattern;

    let h = boot().await;
    let agent_id = spawn_authored(&h.state, "Alice");
    let trigger_id = h
        .state
        .kernel
        .register_trigger_with_target(
            agent_id,
            TriggerPattern::ContentMatch {
                substring: "owner-scope".to_string(),
            },
            "{{event}}".to_string(),
            0,
            None,
            Some(0),
            None,
            None,
        )
        .expect("register trigger");
    let status = request_status(
        &h.app,
        Method::GET,
        &format!("/api/triggers/{trigger_id}"),
        BOB_KEY,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    for key in [ALICE_KEY, ADMIN_KEY] {
        let status = request_status(
            &h.app,
            Method::GET,
            &format!("/api/triggers/{trigger_id}"),
            key,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
    }

    let mutation_cases = [
        (
            Method::POST,
            "/api/triggers".to_string(),
            Some(serde_json::json!({
                "agent_id": agent_id.to_string(),
                "pattern": {"content_match": {"substring": "blocked"}}
            })),
        ),
        (
            Method::PATCH,
            format!("/api/triggers/{trigger_id}"),
            Some(serde_json::json!({"enabled": false})),
        ),
        (Method::DELETE, format!("/api/triggers/{trigger_id}"), None),
    ];
    for (method, path, body) in mutation_cases {
        let status = request_status(&h.app, method, &path, BOB_KEY, body).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{path}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn owner_and_admin_can_access_agent_observability() {
    let h = boot().await;
    let agent_id = spawn_authored(&h.state, "Alice");
    for key in [ALICE_KEY, ADMIN_KEY] {
        let status = request_status(
            &h.app,
            Method::GET,
            &format!("/api/agents/{agent_id}/metrics"),
            key,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
    }

    let missing = AgentId::new();
    for endpoint in ["traces", "logs"] {
        let status = request_status(
            &h.app,
            Method::GET,
            &format!("/api/agents/{missing}/{endpoint}"),
            ADMIN_KEY,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{endpoint}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn non_owner_cannot_clone_agent() {
    // `agent_clone` in `middleware::user_role_allows_request` deliberately lets any `User`-role caller POST `/clone` on an arbitrary agent id (unlike most mutations, which require Admin+), so the ownership boundary has to be enforced in the handler itself.
    // Bob cannot read the resulting clone back afterwards either way (it keeps Alice's `author`), but without this check he could still trigger unauthorized cloning of her agent by guessing/enumerating its UUID.
    let h = boot().await;
    let agent_id = spawn_authored(&h.state, "Alice");
    let aid = agent_id.to_string();

    let status = request_status(
        &h.app,
        Method::POST,
        &format!("/api/agents/{aid}/clone"),
        BOB_KEY,
        Some(serde_json::json!({"new_name": "bob-should-not-get-this"})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let status = request_status(
        &h.app,
        Method::POST,
        &format!("/api/agents/{aid}/clone"),
        ALICE_KEY,
        Some(serde_json::json!({"new_name": format!("alice-clone-{}", uuid::Uuid::new_v4())})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
}

#[tokio::test(flavor = "multi_thread")]
async fn non_owner_cannot_message_another_users_agent() {
    // `agent_message` in `middleware::user_role_allows_request` deliberately lets any `User`-role caller POST `/message` and `/message/stream` on an arbitrary agent id (unlike most mutations, which require Admin+), so the ownership boundary has to be enforced in the handler itself — the same shape as `non_owner_cannot_clone_agent`.
    // Without it a non-owner could drive a full LLM turn — tool execution and budget spend included — on another user's agent by guessing/enumerating its UUID.
    let h = boot().await;
    let agent_id = spawn_authored(&h.state, "Alice");
    let aid = agent_id.to_string();

    for path in [
        format!("/api/agents/{aid}/message"),
        format!("/api/agents/{aid}/message/stream"),
    ] {
        let status = request_status(
            &h.app,
            Method::POST,
            &path,
            BOB_KEY,
            Some(serde_json::json!({"message": "hello"})),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "{path} must 404 for a non-owner"
        );

        // Alice, the real owner, must clear the ownership check and reach the provider-auth check below it, never a 404.
        // The test harness has no provider configured, so this is deterministic without a real LLM call.
        let status = request_status(
            &h.app,
            Method::POST,
            &path,
            ALICE_KEY,
            Some(serde_json::json!({"message": "hello"})),
        )
        .await;
        assert_ne!(
            status,
            StatusCode::NOT_FOUND,
            "{path} must not 404 for the owner"
        );
    }
}

fn spawn_cron_job(
    state: &AppState,
    agent_id: AgentId,
    name: &str,
) -> librefang_types::scheduler::CronJobId {
    let job = librefang_types::scheduler::CronJob {
        id: librefang_types::scheduler::CronJobId::new(),
        agent_id,
        name: name.to_string(),
        enabled: true,
        schedule: librefang_types::scheduler::CronSchedule::Cron {
            expr: "* * * * *".to_string(),
            tz: None,
        },
        action: librefang_types::scheduler::CronAction::AgentTurn {
            message: "owner-scope cron probe".to_string(),
            model_override: None,
            timeout_secs: None,
            pre_check_script: None,
            pre_script: None,
            silent_marker: None,
        },
        delivery: librefang_types::scheduler::CronDelivery::None,
        delivery_targets: Vec::new(),
        peer_id: None,
        session_mode: None,
        created_at: chrono::Utc::now(),
        last_run: None,
        next_run: None,
    };
    state
        .kernel
        .cron()
        .add_job(job, false)
        .expect("register test cron job")
}

#[tokio::test(flavor = "multi_thread")]
async fn cron_and_schedule_routes_enforce_owner_read() {
    // #6753 follow-up: `/api/cron/jobs*` and `/api/schedules*` carry the same cross-owner disclosure class (user-authored `message`/`prompt_template` content) this PR closed for `/api/triggers/*`, but the GET handlers had no `can_access_agent` check at all.
    let h = boot().await;
    let agent_id = spawn_authored(&h.state, "Alice");
    let job_id = spawn_cron_job(&h.state, agent_id, "owner-scope-cron-job");
    let aid = agent_id.to_string();

    // Detail reads: non-owner gets 404, owner and admin get 200.
    for path in [
        format!("/api/cron/jobs/{job_id}"),
        format!("/api/cron/jobs/{job_id}/status"),
        format!("/api/schedules/{job_id}"),
    ] {
        let status = request_status(&h.app, Method::GET, &path, BOB_KEY, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
        for key in [ALICE_KEY, ADMIN_KEY] {
            let status = request_status(&h.app, Method::GET, &path, key, None).await;
            assert_eq!(status, StatusCode::OK, "{path} as {key}");
        }
    }

    // Filtered list (?agent_id=): non-owner gets an empty list, not the job.
    let status_and_body = |bearer: &'static str, path: String| {
        let app = h.app.clone();
        async move {
            let resp = app
                .oneshot(
                    Request::builder()
                        .method(Method::GET)
                        .uri(path)
                        .header("authorization", format!("Bearer {bearer}"))
                        .body(Body::empty())
                        .expect("build request"),
                )
                .await
                .expect("route response");
            let status = resp.status();
            let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
                .await
                .expect("read body");
            (
                status,
                serde_json::from_slice::<serde_json::Value>(&bytes).unwrap_or_default(),
            )
        }
    };
    let (status, body) = status_and_body(BOB_KEY, format!("/api/cron/jobs?agent_id={aid}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["total"],
        serde_json::json!(0),
        "Bob must not see Alice's cron job"
    );

    // Unfiltered list: non-owner's result must not contain the other user's job.
    let (status, body) = status_and_body(BOB_KEY, "/api/cron/jobs".to_string()).await;
    assert_eq!(status, StatusCode::OK);
    let jobs = body["jobs"].as_array().expect("jobs[]");
    assert!(
        jobs.iter()
            .all(|j| j["id"] != serde_json::json!(job_id.to_string())),
        "Bob's unfiltered /api/cron/jobs must not include Alice's job"
    );

    let (status, body) = status_and_body(BOB_KEY, "/api/schedules".to_string()).await;
    assert_eq!(status, StatusCode::OK);
    let items = body["items"].as_array().expect("items[]");
    assert!(
        items
            .iter()
            .all(|j| j["id"] != serde_json::json!(job_id.to_string())),
        "Bob's unfiltered /api/schedules must not include Alice's job"
    );

    // Owner and admin still see it in the unfiltered lists.
    for key in [ALICE_KEY, ADMIN_KEY] {
        let (status, body) = status_and_body(key, "/api/cron/jobs".to_string()).await;
        assert_eq!(status, StatusCode::OK);
        let jobs = body["jobs"].as_array().expect("jobs[]");
        assert!(
            jobs.iter()
                .any(|j| j["id"] == serde_json::json!(job_id.to_string())),
            "{key} should still see the job in /api/cron/jobs"
        );
    }
}

async fn get_json(app: &Router, path: &str, bearer: &str) -> (StatusCode, serde_json::Value) {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(path)
                .header("authorization", format!("Bearer {bearer}"))
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("route response");
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("read body");
    (
        status,
        serde_json::from_slice::<serde_json::Value>(&bytes).unwrap_or_default(),
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn non_admin_cannot_override_owner_filter_on_list_agents() {
    // `list_agents` only auto-injected `?owner=<caller>` when the query param was absent — a non-admin caller supplying `?owner=<someone-else>` explicitly was trusted as-is, defeating the ownership scoping this PR enforces on every other agent-scoped route.
    let h = boot().await;
    let alice_agent = spawn_authored(&h.state, "Alice");

    // Bob explicitly asks for Alice's agents — must still be scoped to Bob, not Alice.
    let (status, body) = get_json(&h.app, "/api/agents?owner=Alice", BOB_KEY).await;
    assert_eq!(status, StatusCode::OK);
    let items = body["items"].as_array().expect("items[]");
    assert!(
        items
            .iter()
            .all(|a| a["id"] != serde_json::json!(alice_agent.to_string())),
        "Bob must not see Alice's agent even when explicitly requesting ?owner=Alice: {body}"
    );

    // Alice herself, and an Admin explicitly filtering by her name, still see it.
    for key in [ALICE_KEY, ADMIN_KEY] {
        let (status, body) = get_json(&h.app, "/api/agents?owner=Alice", key).await;
        assert_eq!(status, StatusCode::OK);
        let items = body["items"].as_array().expect("items[]");
        assert!(
            items
                .iter()
                .any(|a| a["id"] == serde_json::json!(alice_agent.to_string())),
            "{key} should still see Alice's agent via ?owner=Alice: {body}"
        );
    }
}

// ---------------------------------------------------------------------------
// Ownership stamping (#7744)
// ---------------------------------------------------------------------------
//
// These live here, at the *stamping* site, rather than beside the `Principal`
// definition. A test next to the type can only prove the field round-trips;
// the bug this closes is that the server never wrote it from the credential,
// which is only observable through the route that spawns the agent.

async fn request_json(
    app: &Router,
    method: Method,
    path: &str,
    bearer: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {bearer}"));
    let body = match body {
        Some(value) => {
            builder = builder.header("content-type", "application/json");
            Body::from(serde_json::to_vec(&value).expect("serialize request"))
        }
        None => Body::empty(),
    };
    let resp = app
        .clone()
        .oneshot(builder.body(body).expect("build request"))
        .await
        .expect("route response");
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("read body");
    (
        status,
        serde_json::from_slice::<serde_json::Value>(&bytes).unwrap_or_default(),
    )
}

/// Spawn over HTTP as `bearer`, with `author` written into the manifest body.
/// Returns the new agent's id.
async fn spawn_over_http(app: &Router, bearer: &str, author: &str) -> String {
    let name = format!("stamped-{}", uuid::Uuid::new_v4());
    let (status, body) = request_json(
        app,
        Method::POST,
        "/api/agents",
        bearer,
        Some(serde_json::json!({
            "manifest_toml": format!(
                "name = \"{name}\"\nauthor = \"{author}\"\nmodule = \"builtin:chat\"\n"
            ),
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "spawn should succeed: {body}");
    body["agent_id"]
        .as_str()
        .expect("agent_id in spawn response")
        .to_string()
}

/// The agent belongs to whoever authenticated, not to whoever the request body
/// named.
///
/// This is the hole the field exists to close: `author` is free text on the
/// posted manifest, and `can_access_agent` gated on it. A caller could
/// therefore write somebody else's name into the manifest and the agent would
/// answer to that name — self-assertion doing the work of authentication.
///
/// The assertion is deliberately two-sided. Checking only that `owner` reads
/// back as `Admin` would still pass if `author` kept its old authority
/// somewhere; so Bob, whose name is on the manifest, must also be refused.
#[tokio::test(flavor = "multi_thread")]
async fn creating_an_agent_stamps_the_caller_not_the_author_in_the_body() {
    let h = boot().await;

    // Admin spawns, but writes Bob's name into the manifest.
    let agent_id = spawn_over_http(&h.app, ADMIN_KEY, "Bob").await;

    let (status, body) = get_json(&h.app, &format!("/api/agents/{agent_id}"), ADMIN_KEY).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["owner"],
        serde_json::json!({"kind": "user", "id": "Admin"}),
        "the owner must come from the bearer token, not from `author` in the body: {body}"
    );
    assert_eq!(
        body["author"], "Bob",
        "`author` is still recorded as provenance — it is only stripped of authority: {body}"
    );

    // And the name in the body grants Bob nothing.
    let status = request_status(
        &h.app,
        Method::GET,
        &format!("/api/agents/{agent_id}"),
        BOB_KEY,
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "writing `author = \"Bob\"` must not hand Bob the agent"
    );
}

/// An edit must not reassign ownership.
///
/// `PATCH {"manifest_toml": ...}` is a whole-manifest replacement, and `owner`
/// is a field on `AgentManifest` — so it is expressible in the TOML a client
/// posts. Without preservation, the right to edit an agent would silently
/// become the right to take it.
///
/// The fixture is spawned over HTTP by Admin so it is genuinely owned before
/// the edit. Seeding an unowned agent and watching it stay unowned would
/// compare null to null and pass with the preservation deleted.
#[tokio::test(flavor = "multi_thread")]
async fn editing_an_agent_cannot_change_who_owns_it() {
    let h = boot().await;
    let agent_id = spawn_over_http(&h.app, ADMIN_KEY, "Admin").await;

    let (_, before) = get_json(&h.app, &format!("/api/agents/{agent_id}"), ADMIN_KEY).await;
    assert_eq!(
        before["owner"],
        serde_json::json!({"kind": "user", "id": "Admin"}),
        "the fixture must actually be owned, or this test proves nothing: {before}"
    );

    // A full replacement that explicitly claims the agent for Bob.
    let name = before["name"].as_str().expect("name").to_string();
    let (status, body) = request_json(
        &h.app,
        Method::PATCH,
        &format!("/api/agents/{agent_id}"),
        ADMIN_KEY,
        Some(serde_json::json!({
            "manifest_toml": format!(
                "name = \"{name}\"\ndescription = \"after\"\nmodule = \"builtin:chat\"\n\n[owner]\nkind = \"user\"\nid = \"Bob\"\n"
            ),
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (_, after) = get_json(&h.app, &format!("/api/agents/{agent_id}"), ADMIN_KEY).await;
    assert_eq!(
        after["description"], "after",
        "the edit must actually apply, or this test proves nothing: {after}"
    );
    assert_eq!(
        after["owner"], before["owner"],
        "an edit must leave ownership exactly as it was: {after}"
    );

    // The claim in the body must not have granted Bob anything either.
    let status = request_status(
        &h.app,
        Method::GET,
        &format!("/api/agents/{agent_id}"),
        BOB_KEY,
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "an `[owner]` block in the edit body must not transfer the agent"
    );
}

/// An agent stored before ownership existed keeps working.
///
/// Every `agent.toml` on disk today has no `[owner]` block. If the absent
/// field were read as "owned by nobody", `is_owned_by` would deny its author
/// and the upgrade would quietly lock operators out of their own agents. The
/// fallback to `author` is what makes this a widening change rather than a
/// breaking one — it is exactly the rule that was there before.
#[tokio::test(flavor = "multi_thread")]
async fn an_agent_with_no_owner_stays_reachable_by_its_author() {
    let h = boot().await;
    // `spawn_authored` goes through the kernel, not the route, so nothing
    // stamps it — the same shape as a manifest already on disk.
    let agent_id = spawn_authored(&h.state, "Alice");

    let (status, body) = get_json(&h.app, &format!("/api/agents/{agent_id}"), ALICE_KEY).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a pre-ownership agent must stay reachable by its author: {body}"
    );
    assert_eq!(
        body["owner"],
        serde_json::Value::Null,
        "unowned must be reported as null, not invented: {body}"
    );
    assert_eq!(
        body["owner_label"],
        serde_json::Value::Null,
        "an unowned agent has no label to show: {body}"
    );

    // It appears in her list, and not in Bob's.
    let (_, alice_list) = get_json(&h.app, "/api/agents", ALICE_KEY).await;
    assert!(
        alice_list["items"]
            .as_array()
            .expect("items[]")
            .iter()
            .any(|a| a["id"] == serde_json::json!(agent_id.to_string())),
        "Alice must still see her pre-ownership agent: {alice_list}"
    );
    let (_, bob_list) = get_json(&h.app, "/api/agents", BOB_KEY).await;
    assert!(
        bob_list["items"]
            .as_array()
            .expect("items[]")
            .iter()
            .all(|a| a["id"] != serde_json::json!(agent_id.to_string())),
        "the author fallback must not widen access to other users: {bob_list}"
    );
}

/// A group-owned agent belongs to the group's members, not to nobody.
///
/// `Principal::Group` is the reason ownership is a principal rather than a
/// user id: work done on a support shift belongs to the team. If access
/// resolved a group owner by string-comparing it against the caller's name,
/// nobody below Admin would ever match, and recording a group as the owner
/// would be a way to lose an agent rather than to share one.
///
/// Both directions are asserted. That Alice can reach it shows membership is
/// consulted at all; that Bob cannot shows the check is membership and not
/// merely "has an owner we could not parse, allow it".
#[tokio::test(flavor = "multi_thread")]
async fn a_group_owned_agent_is_reachable_by_the_groups_members() {
    let h = boot().await;
    let agent_id = h
        .state
        .kernel
        .spawn_agent_typed(AgentManifest {
            name: format!("group-owned-{}", uuid::Uuid::new_v4()),
            // Nobody's name, so a stray `author` match cannot be what makes
            // this pass.
            author: "nobody".to_string(),
            owner: Some(librefang_types::principal::Principal::Group(
                "support".to_string(),
            )),
            ..AgentManifest::default()
        })
        .expect("spawn group-owned agent");

    let (status, body) = get_json(&h.app, &format!("/api/agents/{agent_id}"), ALICE_KEY).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "Alice is in `support`, so the agent is hers to reach: {body}"
    );
    assert_eq!(
        body["owner_label"], "group:support",
        "the surfaces need the qualified label to render an owner: {body}"
    );

    let status = request_status(
        &h.app,
        Method::GET,
        &format!("/api/agents/{agent_id}"),
        BOB_KEY,
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "Bob is not in `support` and must not reach a support-owned agent"
    );

    // The list must agree with the detail route, or a group-owned agent
    // becomes one you can only open if you already know its id.
    let (_, alice_list) = get_json(&h.app, "/api/agents", ALICE_KEY).await;
    assert!(
        alice_list["items"]
            .as_array()
            .expect("items[]")
            .iter()
            .any(|a| a["id"] == serde_json::json!(agent_id.to_string())),
        "the list must show what the detail route lets Alice open: {alice_list}"
    );
    let (_, bob_list) = get_json(&h.app, "/api/agents", BOB_KEY).await;
    assert!(
        bob_list["items"]
            .as_array()
            .expect("items[]")
            .iter()
            .all(|a| a["id"] != serde_json::json!(agent_id.to_string())),
        "Bob must not see a support-owned agent in his list: {bob_list}"
    );
}

/// `?owner=` addresses the real owner, including a group.
///
/// The filter previously compared against `manifest.author`, so it could only
/// ever name a user. With ownership stamped, an admin auditing "what does the
/// support team own" had no way to ask — the qualified `group:<id>` form is
/// what makes that question expressible.
#[tokio::test(flavor = "multi_thread")]
async fn the_owner_filter_can_address_a_group() {
    let h = boot().await;
    let group_agent = h
        .state
        .kernel
        .spawn_agent_typed(AgentManifest {
            name: format!("group-filter-{}", uuid::Uuid::new_v4()),
            author: "nobody".to_string(),
            owner: Some(librefang_types::principal::Principal::Group(
                "support".to_string(),
            )),
            ..AgentManifest::default()
        })
        .expect("spawn group-owned agent");
    let user_agent = spawn_authored(&h.state, "Alice");

    let (status, body) = get_json(&h.app, "/api/agents?owner=group:support", ADMIN_KEY).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let ids: Vec<String> = body["items"]
        .as_array()
        .expect("items[]")
        .iter()
        .filter_map(|a| a["id"].as_str().map(str::to_string))
        .collect();
    assert!(
        ids.contains(&group_agent.to_string()),
        "`?owner=group:support` must find the support-owned agent: {body}"
    );
    assert!(
        !ids.contains(&user_agent.to_string()),
        "a user-authored agent must not answer to a group filter: {body}"
    );
}
