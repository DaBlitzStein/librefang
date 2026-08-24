//! User group read endpoints (#7745).
//!
//! Groups are declared in `config.toml` under `[[user_groups]]` and resolved
//! in memory by `AuthManager`; there is no membership table. These handlers
//! therefore read the resolved `AuthManager` snapshot rather than the raw
//! config, so what the surfaces show is what authorization actually uses —
//! deduplicated, and with members who name no configured user already
//! reported as such rather than silently listed as real.
//!
//! **Read-only, deliberately.** `POST /api/user-groups/{id}/members` is only
//! meaningful under a stored-membership model, which #7745 does not take: once
//! an identity provider is authoritative on every login (#7746), a written
//! membership would be silently discarded at the next sign-in. Adding somebody
//! to a group is a `config.toml` edit plus `POST /api/config/reload`, which is
//! hot — no restart.
//!
//! Auth: not in the `is_public` allowlist, so every request goes through the
//! authenticated middleware path, matching `/api/users`.
//!
//! The detail route addresses a group by its stable `id`, not its display
//! `name`. The two are separate fields precisely because a group is renamed
//! far more often than it is dissolved, and `Principal::Group` records the id;
//! keying the URL on the mutable name would change a group's address every
//! time somebody retitled it.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use librefang_types::config::UserGroup;
use serde::Serialize;

use super::AppState;

pub fn router() -> axum::Router<Arc<AppState>> {
    axum::Router::new()
        .route("/user-groups", axum::routing::get(list_user_groups))
        .route("/user-groups/{id}", axum::routing::get(get_user_group))
}

// ---------------------------------------------------------------------------
// View models
// ---------------------------------------------------------------------------

/// A user group as the read surfaces see it.
///
/// `members` is a sorted `Vec` on the wire because JSON has no set type; the
/// ordering is the `BTreeSet`'s and is stable across processes, so a client
/// diffing two responses sees a change only when membership actually changed.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct UserGroupView {
    /// Stable identifier — what ownership records point at.
    pub id: String,
    /// Human-readable name, safe to change.
    pub name: String,
    /// What the group is for; may be empty.
    pub description: String,
    /// Member user names, matching `UserConfig::name`.
    pub members: Vec<String>,
    /// Number of declared members, so a list view need not count client-side.
    pub member_count: usize,
}

impl From<UserGroup> for UserGroupView {
    fn from(group: UserGroup) -> Self {
        let members: Vec<String> = group.members.into_iter().collect();
        Self {
            id: group.id,
            name: group.name,
            description: group.description,
            member_count: members.len(),
            members,
        }
    }
}

fn err_response(status: StatusCode, msg: impl Into<String>) -> axum::response::Response {
    (
        status,
        Json(serde_json::json!({ "status": "error", "error": msg.into() })),
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/api/user-groups",
    tag = "user-groups",
    responses(
        (status = 200, description = "Declared user groups, ordered by id", body = [UserGroupView])
    )
)]
pub async fn list_user_groups(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let groups: Vec<UserGroupView> = state
        .kernel
        .auth_manager()
        .user_groups()
        .into_iter()
        .map(UserGroupView::from)
        .collect();
    Json(groups).into_response()
}

#[utoipa::path(
    get,
    path = "/api/user-groups/{id}",
    tag = "user-groups",
    params(("id" = String, Path, description = "Stable group id (not the display name)")),
    responses(
        (status = 200, description = "User group detail", body = UserGroupView),
        (status = 404, description = "Not found"),
    )
)]
pub async fn get_user_group(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.kernel.auth_manager().user_group(&id) {
        Some(group) => Json(UserGroupView::from(group)).into_response(),
        None => err_response(
            StatusCode::NOT_FOUND,
            format!("user group '{id}' not found"),
        ),
    }
}
