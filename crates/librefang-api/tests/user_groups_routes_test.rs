//! Integration coverage for the read-only user-group routes (#7745).
//!
//! The point of these tests is the whole path, not the handler: a
//! `[[user_groups]]` declaration in the kernel config has to survive boot,
//! reach the `AuthManager` snapshot that authorization actually consults, and
//! come back out of `GET /api/user-groups` with its members intact. A handler
//! unit test would pass with the config never wired into the kernel at all,
//! which is the failure mode this suite exists to catch.

use axum::body::{to_bytes, Body};
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use librefang_api::routes::{self, AppState};
use librefang_testing::{MockKernelBuilder, TestAppState};
use librefang_types::config::{UserConfig, UserGroup};
use std::collections::HashMap;
use std::sync::Arc;
use tower::ServiceExt;

struct Harness {
    app: Router,
    #[allow(dead_code)]
    state: Arc<AppState>,
    #[allow(dead_code)]
    test: TestAppState,
}

fn user(name: &str) -> UserConfig {
    UserConfig {
        name: name.to_string(),
        role: "user".to_string(),
        channel_bindings: HashMap::new(),
        api_key_hash: None,
        budget: None,
        tool_policy: None,
        tool_categories: None,
        memory_access: None,
        channel_tool_rules: HashMap::new(),
    }
}

fn group(id: &str, name: &str, members: &[&str]) -> UserGroup {
    UserGroup {
        id: id.to_string(),
        name: name.to_string(),
        description: String::new(),
        members: members.iter().map(|m| m.to_string()).collect(),
    }
}

/// Boots a kernel whose config declares `users` and `user_groups`, exactly as
/// an operator's `config.toml` would.
async fn boot_with(users: Vec<UserConfig>, groups: Vec<UserGroup>) -> Harness {
    let test = TestAppState::with_builder(MockKernelBuilder::new().with_config(move |cfg| {
        cfg.users = users.clone();
        cfg.user_groups = groups.clone();
    }));
    let state = test.state.clone();
    let app = Router::new()
        .nest("/api", routes::user_groups::router())
        .with_state(state.clone());
    Harness { app, state, test }
}

async fn get_json(app: &Router, path: &str) -> (StatusCode, serde_json::Value) {
    let request = Request::builder()
        .method(Method::GET)
        .uri(path)
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    (status, serde_json::from_slice(&body).unwrap())
}

/// The declaration-to-route path: what the operator wrote must be what the
/// endpoint serves, members included.
#[tokio::test(flavor = "multi_thread")]
async fn a_declared_group_resolves_to_its_members_over_the_route() {
    let harness = boot_with(
        vec![user("paco"), user("mia")],
        vec![group("support", "Support", &["paco", "mia"])],
    )
    .await;

    let (status, body) = get_json(&harness.app, "/api/user-groups").await;
    assert_eq!(status, StatusCode::OK);

    let groups = body.as_array().expect("the list endpoint returns an array");
    assert_eq!(groups.len(), 1, "served: {body}");
    assert_eq!(groups[0]["id"], "support");
    assert_eq!(groups[0]["name"], "Support");
    assert_eq!(groups[0]["member_count"], 2);
    assert_eq!(
        groups[0]["members"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|m| m.as_str())
            .collect::<Vec<_>>(),
        vec!["mia", "paco"],
        "members are served in the BTreeSet's stable order"
    );
}

/// A user in two groups has to appear in both. This is the membership case the
/// feature exists for, asserted through the surface rather than only in the
/// kernel.
#[tokio::test(flavor = "multi_thread")]
async fn a_user_in_two_groups_appears_in_both_over_the_route() {
    let harness = boot_with(
        vec![user("paco"), user("mia")],
        vec![
            group("support", "Support", &["paco", "mia"]),
            group("platform", "Platform", &["paco"]),
        ],
    )
    .await;

    let (status, body) = get_json(&harness.app, "/api/user-groups").await;
    assert_eq!(status, StatusCode::OK);

    let membership: Vec<(String, Vec<String>)> = body
        .as_array()
        .unwrap()
        .iter()
        .map(|g| {
            (
                g["id"].as_str().unwrap().to_string(),
                g["members"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|m| m.as_str().unwrap().to_string())
                    .collect(),
            )
        })
        .collect();

    assert_eq!(
        membership,
        vec![
            ("platform".to_string(), vec!["paco".to_string()]),
            (
                "support".to_string(),
                vec!["mia".to_string(), "paco".to_string()]
            ),
        ],
        "groups list in a stable order and paco is in both"
    );
}

/// The detail route is keyed on the stable id, and a group with no members is
/// a legitimate thing to serve rather than a 404.
#[tokio::test(flavor = "multi_thread")]
async fn the_detail_route_serves_one_group_by_its_stable_id() {
    let harness = boot_with(
        vec![user("paco")],
        vec![
            group("support", "Support", &["paco"]),
            group("audit", "Audit", &[]),
        ],
    )
    .await;

    let (status, body) = get_json(&harness.app, "/api/user-groups/support").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["id"], "support");
    assert_eq!(body["members"].as_array().unwrap().len(), 1);

    let (status, body) = get_json(&harness.app, "/api/user-groups/audit").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["member_count"], 0);
    assert!(
        body["members"].as_array().unwrap().is_empty(),
        "an empty group is served, not treated as missing"
    );
}

/// Fail closed at the surface: a group nobody declared is a 404, and a
/// misspelling must not be resolved to the group it nearly matches.
#[tokio::test(flavor = "multi_thread")]
async fn an_unknown_group_is_not_found() {
    let harness = boot_with(
        vec![user("paco")],
        vec![group("support", "Support", &["paco"])],
    )
    .await;

    let (status, _) = get_json(&harness.app, "/api/user-groups/suport").await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a misspelled id must not resolve to the real group"
    );
}

/// The display name is mutable and the id is not, so the detail route must not
/// answer to the name — otherwise a group's address changes on every rename,
/// which is the exact failure the separate id field prevents.
#[tokio::test(flavor = "multi_thread")]
async fn the_detail_route_does_not_answer_to_the_display_name() {
    let harness = boot_with(
        vec![user("paco")],
        vec![group("support", "Customer Success", &["paco"])],
    )
    .await;

    let (status, _) = get_json(&harness.app, "/api/user-groups/Customer%20Success").await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, body) = get_json(&harness.app, "/api/user-groups/support").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["name"], "Customer Success",
        "the rename is visible in the payload without moving the URL"
    );
}

/// Every existing deployment declares no groups at all, so the list has to be
/// an empty array rather than an error or a null.
#[tokio::test(flavor = "multi_thread")]
async fn a_deployment_with_no_groups_serves_an_empty_list() {
    let harness = boot_with(vec![user("paco")], vec![]).await;

    let (status, body) = get_json(&harness.app, "/api/user-groups").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, serde_json::json!([]));
}

/// A group listing somebody who is not a configured user keeps the
/// declaration visible — the operator needs to see their typo — but the kernel
/// does not index it as a real membership. Asserting the served payload keeps
/// the two behaviours from being conflated.
#[tokio::test(flavor = "multi_thread")]
async fn a_member_who_is_not_a_configured_user_is_still_shown_in_the_declaration() {
    let harness = boot_with(
        vec![user("paco")],
        vec![group("support", "Support", &["paco", "ghost"])],
    )
    .await;

    let (status, body) = get_json(&harness.app, "/api/user-groups/support").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["members"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|m| m.as_str())
            .collect::<Vec<_>>(),
        vec!["ghost", "paco"]
    );
}

/// No write surface ships in this increment. Membership is derived from
/// config, so a POST would have nothing durable to write to and would be
/// silently discarded once an IdP is authoritative (#7746).
#[tokio::test(flavor = "multi_thread")]
async fn there_is_no_write_endpoint() {
    let harness = boot_with(
        vec![user("paco")],
        vec![group("support", "Support", &["paco"])],
    )
    .await;

    for (method, uri) in [
        (Method::POST, "/api/user-groups"),
        (Method::POST, "/api/user-groups/support/members"),
        (Method::PUT, "/api/user-groups/support"),
        (Method::DELETE, "/api/user-groups/support"),
    ] {
        let request = Request::builder()
            .method(method.clone())
            .uri(uri)
            .body(Body::empty())
            .unwrap();
        let response = harness.app.clone().oneshot(request).await.unwrap();
        assert!(
            matches!(
                response.status(),
                StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED
            ),
            "{method} {uri} must not be routed, got {}",
            response.status()
        );
    }
}
