//! Shared knowledge bases: documents uploaded once, readable by chosen agents (#8327).
//!
//! * `GET /api/knowledge` — every base, with its document count and the agents that hold it.
//! * `POST /api/knowledge` — create one.
//! * `DELETE /api/knowledge/{name}` — remove it, and revoke it from every agent that held it.
//! * `GET /api/knowledge/{name}/documents` — list its documents.
//! * `PUT /api/knowledge/{name}/documents/{filename}` — write one, raw body.
//! * `DELETE /api/knowledge/{name}/documents/{filename}` — remove one.
//! * `PUT /api/knowledge/{name}/agents` — set which agents hold it, and in which mode.
//!
//! # Why this stores nothing of its own
//!
//! A knowledge base here *is* a named workspace, the mechanism `agent.toml`'s `[workspaces]` already describes and that the agent side already implements end to end.
//! `ensure_named_workspaces` creates the directory and refuses to follow a symlink out of the tree; `resolve_file_path_ext` takes the resolved paths as additional sandbox roots; `ALIAS_PATH_KEYS` expands a leading `@name/` for `file_read`, `file_write`, `file_list`, `code_search` and the media tools (#8051); `build_tools_content` writes `- **@name** → /abs/path (read-only)` into the agent's `TOOLS.md` so the model is told the alias exists.
//! Agents sharing one path do not collide, because identity files live in each agent's private `.identity/` rather than the workspace root.
//!
//! What was missing was never storage or retrieval — it was that all of it required hand-editing TOML and copying files onto the host.
//! These routes are that surface and nothing more, which is why there is no index to fall out of sync with the files, no second retrieval path for an operator to confuse with the wiki, and no new scoping concept: an agent holds a base or it does not, and the answer lives in the manifest beside every other per-agent capability.
//!
//! # Why a fixed `knowledge/` prefix
//!
//! Bases live at `{workspaces_dir}/knowledge/{name}` and nowhere else.
//! A workspace declaration can point at any relative path, so accepting one from a request would mean validating an operator-supplied path against traversal, symlinked ancestors and collisions with an agent's private workspace — all of which [`ensure_named_workspaces`](librefang_kernel) already handles for the *declaration*, but none of which help when the request is the thing choosing the path.
//! Anchoring to one segment under one prefix makes the whole class unreachable: a name that matches [`is_valid_segment`] cannot contain a separator, cannot be `.` or `..`, and joins to exactly one directory.
//! An operator who wants a shared workspace somewhere else still writes it in `agent.toml`, which is unchanged.
//!
//! # Why deleting a base also edits manifests
//!
//! `ensure_named_workspaces` runs on every spawn and *creates* the directory for any `path` declaration.
//! Deleting the directory alone would therefore delete the documents and leave the declaration behind, and the next respawn would recreate an empty base that the dashboard lists as real and the agent's `TOOLS.md` still advertises.
//! Revoking is part of deleting, or the delete is a lie the next restart tells.

use std::collections::HashMap;
use std::path::{Path as FsPath, PathBuf};
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use librefang_types::agent::{AgentId, WorkspaceDecl, WorkspaceMode};
use serde::{Deserialize, Serialize};

use super::AppState;

/// Directory under `workspaces_dir` that holds every base.
const KNOWLEDGE_PREFIX: &str = "knowledge";

/// Largest document accepted, in bytes.
///
/// These are read into an agent's context by `file_read`, so the ceiling that
/// matters is a prompt rather than a disk. Four megabytes of text is already
/// far past any context window; past that the operator wants a different tool,
/// and finding that out at upload time beats finding it out when a turn fails.
const MAX_DOCUMENT_BYTES: usize = 4 * 1024 * 1024;

pub fn router() -> axum::Router<Arc<AppState>> {
    axum::Router::new()
        .route(
            "/knowledge",
            axum::routing::get(list_bases).post(create_base),
        )
        .route("/knowledge/{name}", axum::routing::delete(delete_base))
        .route(
            "/knowledge/{name}/documents",
            axum::routing::get(list_documents),
        )
        .route(
            "/knowledge/{name}/documents/{filename}",
            axum::routing::put(put_document).delete(delete_document),
        )
        .route("/knowledge/{name}/agents", axum::routing::put(set_holders))
}

/// One path segment that is safe to join, checked rather than sanitised.
///
/// Rejecting is the whole point: silently rewriting `../../etc` into something
/// legal would accept a request the caller meant differently, and the caller
/// cannot tell which name it ended up with. The charset excludes every
/// separator on both platform families, and the leading-character rule keeps
/// out `.`, `..` and dotfiles in one condition.
fn is_valid_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment.len() <= 64
        && segment
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric())
        && segment
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// Absolute directory of a base, or `None` when the name is not a safe segment.
fn base_dir(state: &AppState, name: &str) -> Option<PathBuf> {
    is_valid_segment(name).then(|| {
        state
            .kernel
            .config_snapshot()
            .effective_workspaces_dir()
            .join(KNOWLEDGE_PREFIX)
            .join(name)
    })
}

/// The `path` a manifest declaration carries for a base, as written in `agent.toml`.
fn decl_path(name: &str) -> PathBuf {
    FsPath::new(KNOWLEDGE_PREFIX).join(name)
}

fn bad_request(message: &str) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({ "error": message })),
    )
}

fn not_found(message: &str) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({ "error": message })),
    )
}

/// An agent that holds a base, as seen from the base's side.
///
/// Both directions are listed on purpose. #8321 was the same shape of defect:
/// a binding that existed and worked, visible from one side only, which read
/// as broken from the other.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct BaseHolder {
    pub agent_id: String,
    pub agent_name: String,
    /// Alias the agent reaches it by, i.e. the `@name` in its `TOOLS.md`.
    pub alias: String,
    /// `"rw"` or `"r"`.
    pub mode: String,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct KnowledgeBase {
    pub name: String,
    /// Path as it appears in `agent.toml`, relative to `workspaces_dir`.
    pub path: String,
    pub document_count: usize,
    pub total_bytes: u64,
    pub agents: Vec<BaseHolder>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct KnowledgeDocument {
    pub filename: String,
    pub bytes: u64,
    /// RFC 3339, or absent when the filesystem does not report one.
    pub modified: Option<String>,
}

fn mode_str(mode: WorkspaceMode) -> &'static str {
    match mode {
        WorkspaceMode::ReadWrite => "rw",
        WorkspaceMode::ReadOnly => "r",
    }
}

/// Every agent holding `name`, sorted by agent name (#3298).
fn holders_of(state: &AppState, name: &str) -> Vec<BaseHolder> {
    let wanted = decl_path(name);
    let mut out: Vec<BaseHolder> = state
        .kernel
        .agent_registry()
        .list()
        .into_iter()
        .flat_map(|entry| {
            let agent_id = entry.id.to_string();
            let agent_name = entry.name.clone();
            entry
                .manifest
                .workspaces
                .iter()
                .filter(|(_, decl)| decl.path.as_deref() == Some(wanted.as_path()))
                .map(|(alias, decl)| BaseHolder {
                    agent_id: agent_id.clone(),
                    agent_name: agent_name.clone(),
                    alias: alias.clone(),
                    mode: mode_str(decl.mode).to_string(),
                })
                .collect::<Vec<_>>()
        })
        .collect();
    out.sort_by(|a, b| (&a.agent_name, &a.alias).cmp(&(&b.agent_name, &b.alias)));
    out
}

/// Documents in `dir`, sorted by filename. Sub-directories are skipped.
fn read_documents(dir: &FsPath) -> Vec<KnowledgeDocument> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<KnowledgeDocument> = entries
        .flatten()
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            if !metadata.is_file() {
                return None;
            }
            let filename = entry.file_name().to_string_lossy().into_owned();
            if filename.starts_with('.') {
                return None;
            }
            Some(KnowledgeDocument {
                filename,
                bytes: metadata.len(),
                modified: metadata
                    .modified()
                    .ok()
                    .map(|time| chrono::DateTime::<chrono::Utc>::from(time).to_rfc3339()),
            })
        })
        .collect();
    out.sort_by(|a, b| a.filename.cmp(&b.filename));
    out
}

/// GET /api/knowledge — every base with its documents' totals and its holders.
#[utoipa::path(
    get,
    path = "/api/knowledge",
    tag = "knowledge",
    responses((status = 200, description = "Shared knowledge bases", body = crate::types::JsonObject))
)]
pub async fn list_bases(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let root = state
        .kernel
        .config_snapshot()
        .effective_workspaces_dir()
        .join(KNOWLEDGE_PREFIX);

    let mut bases: Vec<KnowledgeBase> = std::fs::read_dir(&root)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            // A directory whose name this API would refuse to create is not
            // one it should report either — listing it would offer an
            // operator a base whose every other route answers 400.
            if !is_valid_segment(&name) {
                return None;
            }
            let documents = read_documents(&entry.path());
            Some(KnowledgeBase {
                path: decl_path(&name).to_string_lossy().into_owned(),
                document_count: documents.len(),
                total_bytes: documents.iter().map(|d| d.bytes).sum(),
                agents: holders_of(&state, &name),
                name,
            })
        })
        .collect();
    bases.sort_by(|a, b| a.name.cmp(&b.name));

    (StatusCode::OK, Json(serde_json::json!({ "bases": bases })))
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateBaseRequest {
    pub name: String,
}

/// POST /api/knowledge — create an empty base.
#[utoipa::path(
    post,
    path = "/api/knowledge",
    tag = "knowledge",
    request_body = CreateBaseRequest,
    responses(
        (status = 201, description = "Created", body = crate::types::JsonObject),
        (status = 400, description = "Invalid name", body = crate::types::JsonObject),
        (status = 409, description = "A base of that name already exists", body = crate::types::JsonObject)
    )
)]
pub async fn create_base(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateBaseRequest>,
) -> impl IntoResponse {
    let Some(dir) = base_dir(&state, &body.name) else {
        return bad_request(
            "A knowledge base name must start with a letter or digit and may then contain letters, digits, '.', '_' and '-', up to 64 characters.",
        );
    };
    if dir.exists() {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({ "error": "A knowledge base of that name already exists." })),
        );
    }
    if let Err(error) = std::fs::create_dir_all(&dir) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(
                serde_json::json!({ "error": format!("Could not create the knowledge base: {error}") }),
            ),
        );
    }
    (
        StatusCode::CREATED,
        Json(serde_json::json!({
            "name": body.name,
            "path": decl_path(&body.name).to_string_lossy(),
        })),
    )
}

/// DELETE /api/knowledge/{name} — remove the base and revoke it everywhere.
#[utoipa::path(
    delete,
    path = "/api/knowledge/{name}",
    tag = "knowledge",
    params(("name" = String, Path, description = "Knowledge base name")),
    responses(
        (status = 200, description = "Deleted", body = crate::types::JsonObject),
        (status = 404, description = "No such base", body = crate::types::JsonObject)
    )
)]
pub async fn delete_base(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let Some(dir) = base_dir(&state, &name) else {
        return bad_request("Invalid knowledge base name.");
    };
    if !dir.is_dir() {
        return not_found("No such knowledge base.");
    }

    // Revoke first. If the directory removal then fails, the operator is left
    // with an unreferenced directory rather than with agents holding an alias
    // whose target the next respawn silently recreates.
    let mut revoked = 0usize;
    for holder in holders_of(&state, &name) {
        let Ok(agent_id) = holder.agent_id.parse::<AgentId>() else {
            continue;
        };
        let Some(entry) = state.kernel.agent_registry().get(agent_id) else {
            continue;
        };
        let wanted = decl_path(&name);
        let remaining: HashMap<String, WorkspaceDecl> = entry
            .manifest
            .workspaces
            .iter()
            .filter(|(_, decl)| decl.path.as_deref() != Some(wanted.as_path()))
            .map(|(alias, decl)| (alias.clone(), decl.clone()))
            .collect();
        if state
            .kernel
            .set_agent_workspaces(agent_id, remaining)
            .is_ok()
        {
            revoked += 1;
        }
    }

    if let Err(error) = std::fs::remove_dir_all(&dir) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": format!("Revoked the base from {revoked} agent(s) but could not remove its directory: {error}")
            })),
        );
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({ "status": "ok", "revoked_from": revoked })),
    )
}

/// GET /api/knowledge/{name}/documents — list the documents in a base.
#[utoipa::path(
    get,
    path = "/api/knowledge/{name}/documents",
    tag = "knowledge",
    params(("name" = String, Path, description = "Knowledge base name")),
    responses(
        (status = 200, description = "Documents", body = crate::types::JsonObject),
        (status = 404, description = "No such base", body = crate::types::JsonObject)
    )
)]
pub async fn list_documents(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let Some(dir) = base_dir(&state, &name) else {
        return bad_request("Invalid knowledge base name.");
    };
    if !dir.is_dir() {
        return not_found("No such knowledge base.");
    }
    let documents = read_documents(&dir);
    (
        StatusCode::OK,
        Json(serde_json::json!({ "documents": documents })),
    )
}

/// PUT /api/knowledge/{name}/documents/{filename} — write a document.
///
/// The body is the file, raw. That is the convention `POST /api/agents/{id}/upload`
/// already set in this codebase, and it keeps `multipart` off the dependency list
/// for a payload that is one file.
#[utoipa::path(
    put,
    path = "/api/knowledge/{name}/documents/{filename}",
    tag = "knowledge",
    params(
        ("name" = String, Path, description = "Knowledge base name"),
        ("filename" = String, Path, description = "Document filename")
    ),
    request_body(content = String, content_type = "application/octet-stream"),
    responses(
        (status = 200, description = "Written", body = crate::types::JsonObject),
        (status = 404, description = "No such base", body = crate::types::JsonObject),
        (status = 413, description = "Document too large", body = crate::types::JsonObject)
    )
)]
pub async fn put_document(
    State(state): State<Arc<AppState>>,
    Path((name, filename)): Path<(String, String)>,
    body: Bytes,
) -> impl IntoResponse {
    let Some(dir) = base_dir(&state, &name) else {
        return bad_request("Invalid knowledge base name.");
    };
    if !dir.is_dir() {
        return not_found("No such knowledge base.");
    }
    if !is_valid_segment(&filename) {
        return bad_request(
            "A document filename must start with a letter or digit and may then contain letters, digits, '.', '_' and '-', up to 64 characters.",
        );
    }
    if body.len() > MAX_DOCUMENT_BYTES {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(serde_json::json!({
                "error": format!("A document may be at most {MAX_DOCUMENT_BYTES} bytes; this one is {}.", body.len())
            })),
        );
    }

    let path = dir.join(&filename);
    if let Err(error) = std::fs::write(&path, &body) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Could not write the document: {error}") })),
        );
    }
    (
        StatusCode::OK,
        Json(serde_json::json!({ "status": "ok", "filename": filename, "bytes": body.len() })),
    )
}

/// DELETE /api/knowledge/{name}/documents/{filename} — remove a document.
#[utoipa::path(
    delete,
    path = "/api/knowledge/{name}/documents/{filename}",
    tag = "knowledge",
    params(
        ("name" = String, Path, description = "Knowledge base name"),
        ("filename" = String, Path, description = "Document filename")
    ),
    responses(
        (status = 200, description = "Deleted", body = crate::types::JsonObject),
        (status = 404, description = "No such base or document", body = crate::types::JsonObject)
    )
)]
pub async fn delete_document(
    State(state): State<Arc<AppState>>,
    Path((name, filename)): Path<(String, String)>,
) -> impl IntoResponse {
    let Some(dir) = base_dir(&state, &name) else {
        return bad_request("Invalid knowledge base name.");
    };
    if !is_valid_segment(&filename) {
        return bad_request("Invalid document filename.");
    }
    let path = dir.join(&filename);
    if !path.is_file() {
        return not_found("No such document.");
    }
    if let Err(error) = std::fs::remove_file(&path) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Could not remove the document: {error}") })),
        );
    }
    (StatusCode::OK, Json(serde_json::json!({ "status": "ok" })))
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct BaseHolderRequest {
    pub agent_id: String,
    /// `"rw"` or `"r"`. Defaults to read-only: a knowledge base is a thing to
    /// read, and an agent that can rewrite the documents every other agent
    /// reads is a decision the operator should have to make explicitly.
    #[serde(default)]
    pub mode: Option<String>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SetHoldersRequest {
    /// The complete set of agents that should hold this base. Agents absent
    /// from the list have it revoked, which is what makes "share with nobody"
    /// expressible as `[]` rather than needing a separate route.
    pub agents: Vec<BaseHolderRequest>,
}

/// PUT /api/knowledge/{name}/agents — set exactly which agents hold this base.
#[utoipa::path(
    put,
    path = "/api/knowledge/{name}/agents",
    tag = "knowledge",
    params(("name" = String, Path, description = "Knowledge base name")),
    request_body = SetHoldersRequest,
    responses(
        (status = 200, description = "Holders updated", body = crate::types::JsonObject),
        (status = 404, description = "No such base", body = crate::types::JsonObject)
    )
)]
pub async fn set_holders(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(body): Json<SetHoldersRequest>,
) -> impl IntoResponse {
    let Some(dir) = base_dir(&state, &name) else {
        return bad_request("Invalid knowledge base name.");
    };
    if !dir.is_dir() {
        return not_found("No such knowledge base.");
    }

    let wanted_path = decl_path(&name);
    let mut requested: HashMap<AgentId, WorkspaceMode> = HashMap::new();
    for holder in &body.agents {
        let Ok(agent_id) = holder.agent_id.parse::<AgentId>() else {
            return bad_request(&format!("Not an agent id: {}.", holder.agent_id));
        };
        if state.kernel.agent_registry().get(agent_id).is_none() {
            return not_found("No such agent.");
        }
        let mode = match holder.mode.as_deref() {
            None | Some("r") | Some("read") | Some("read-only") => WorkspaceMode::ReadOnly,
            Some("rw") | Some("read-write") => WorkspaceMode::ReadWrite,
            Some(other) => return bad_request(&format!("Not a workspace mode: {other}.")),
        };
        requested.insert(agent_id, mode);
    }

    // Refuse the whole request before changing anything if any target is
    // provisioned: half-applying a sharing decision leaves the operator with a
    // base whose holder list is neither what it was nor what they asked for.
    for agent_id in requested.keys() {
        if let Some(refusal) = super::agents::guard_provisioned_agent(&state, *agent_id) {
            return refusal;
        }
    }
    let current: Vec<AgentId> = holders_of(&state, &name)
        .iter()
        .filter_map(|holder| holder.agent_id.parse::<AgentId>().ok())
        .collect();
    for agent_id in &current {
        if !requested.contains_key(agent_id) {
            if let Some(refusal) = super::agents::guard_provisioned_agent(&state, *agent_id) {
                return refusal;
            }
        }
    }

    let mut changed = 0usize;
    for agent_id in current.iter().copied().chain(requested.keys().copied()) {
        let Some(entry) = state.kernel.agent_registry().get(agent_id) else {
            continue;
        };
        // Drop any existing declaration of this base, whatever alias it used,
        // then re-add it under the canonical alias when the agent should keep
        // it. Rewriting rather than merging is what lets a mode change and an
        // alias rename both land as one edit.
        let mut next: HashMap<String, WorkspaceDecl> = entry
            .manifest
            .workspaces
            .iter()
            .filter(|(_, decl)| decl.path.as_deref() != Some(wanted_path.as_path()))
            .map(|(alias, decl)| (alias.clone(), decl.clone()))
            .collect();
        if let Some(mode) = requested.get(&agent_id) {
            next.insert(
                name.clone(),
                WorkspaceDecl {
                    path: Some(wanted_path.clone()),
                    mount: None,
                    mode: *mode,
                },
            );
        }
        if next == entry.manifest.workspaces {
            continue;
        }
        if let Err(error) = state.kernel.set_agent_workspaces(agent_id, next) {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": format!("Updated {changed} agent(s), then failed on {agent_id}: {error}")
                })),
            );
        }
        changed += 1;
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "agents": holders_of(&state, &name),
        })),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_that_could_escape_the_prefix_is_refused() {
        // The whole traversal class, rather than one example of it: a segment
        // that passes this cannot contain a separator or be a relative marker,
        // so `root.join(KNOWLEDGE_PREFIX).join(name)` has exactly one meaning.
        for hostile in [
            "..",
            ".",
            "../etc",
            "a/b",
            "a\\b",
            "/abs",
            ".hidden",
            "-leading",
            "",
            "with space",
            "nul\0byte",
        ] {
            assert!(!is_valid_segment(hostile), "accepted {hostile:?}");
        }
    }

    #[test]
    fn ordinary_names_are_accepted() {
        for ok in ["handbook", "team-notes", "v2.1_specs", "a", "A9"] {
            assert!(is_valid_segment(ok), "refused {ok:?}");
        }
        // 64 is the ceiling, 65 is not.
        assert!(is_valid_segment(&"a".repeat(64)));
        assert!(!is_valid_segment(&"a".repeat(65)));
    }

    #[test]
    fn the_declaration_path_is_the_prefix_plus_the_name() {
        // What lands in `agent.toml`, and what `holders_of` matches against.
        // If these two ever disagree, sharing silently stops being visible.
        assert_eq!(decl_path("handbook"), FsPath::new("knowledge/handbook"));
    }
}
