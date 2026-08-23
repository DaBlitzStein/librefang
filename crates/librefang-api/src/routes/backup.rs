//! Backup / restore endpoints, shared implementation with the CLI.
//!
//! - `POST /api/backup` — create a backup (global, or `{ "agent": name }`).
//! - `GET /api/backup/list` — list existing backups.
//! - `GET /api/backup/download/{name}` — stream one.
//! - `POST /api/backup/restore` — restore `{ "path": host-path, "keep_config": bool }`.
//!   The restore runs through a detached helper that stops the daemon,
//!   extracts, and starts it again — a restore over a live SQLite would
//!   corrupt it, so the daemon gets out of its own way.

use crate::routes::AppState;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use std::sync::Arc;

fn backups_dir(state: &Arc<AppState>) -> std::path::PathBuf {
    state.kernel.home_dir().join("backups")
}

fn valid_backup_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
}

pub fn router() -> axum::Router<Arc<AppState>> {
    axum::Router::new()
        .route("/backup", axum::routing::post(create_backup))
        .route("/backup/list", axum::routing::get(list_backups))
        .route(
            "/backup/download/{name}",
            axum::routing::get(download_backup),
        )
        .route("/backup/{name}", axum::routing::delete(delete_backup))
        .route("/backup/restore", axum::routing::post(restore_backup))
}

/// POST /api/backup — create a backup tarball.
#[utoipa::path(post, path = "/api/backup", tag = "backup", responses((status = 201, description = "Backup created"), (status = 400, description = "Invalid agent or write failure")))]
pub async fn create_backup(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let agent = body["agent"].as_str().map(str::to_string);
    let home = state.kernel.home_dir();
    let dir = backups_dir(&state);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("create backups dir: {e}")})),
        )
            .into_response();
    }
    let name = format!(
        "backup-{}.tar.gz",
        chrono::Utc::now().format("%Y%m%d-%H%M%S")
    );
    let path = dir.join(&name);
    match librefang_kernel::backup::write_tarball(home, agent.as_deref(), &path) {
        Ok(bytes) => (
            StatusCode::CREATED,
            Json(serde_json::json!({"file": name, "bytes": bytes})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": e})),
        )
            .into_response(),
    }
}

/// GET /api/backup/list — existing backups, newest first.
#[utoipa::path(get, path = "/api/backup/list", tag = "backup", responses((status = 200, description = "Backup list")))]
pub async fn list_backups(State(state): State<Arc<AppState>>) -> Response {
    let dir = backups_dir(&state);
    let mut items = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if !valid_backup_name(&name) {
                continue;
            }
            let bytes = e.metadata().map(|m| m.len()).unwrap_or(0);
            items.push(serde_json::json!({
                "file": name,
                "bytes": bytes,
            }));
        }
    }
    items.sort_by(|a, b| b["file"].as_str().cmp(&a["file"].as_str()));
    Json(serde_json::json!({"backups": items})).into_response()
}

/// GET /api/backup/download/{name} — stream one backup.
#[utoipa::path(get, path = "/api/backup/download/{name}", tag = "backup", responses((status = 200, description = "Backup archive"), (status = 404, description = "Not found")))]
pub async fn download_backup(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Response {
    if !valid_backup_name(&name) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "invalid name"})),
        )
            .into_response();
    }
    let path = backups_dir(&state).join(&name);
    match tokio::fs::read(&path).await {
        Ok(bytes) => (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, "application/gzip"),
                (
                    header::CONTENT_DISPOSITION,
                    "attachment; filename=backup.tar.gz",
                ),
            ],
            Body::from(bytes),
        )
            .into_response(),
        Err(_) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "backup not found"})),
        )
            .into_response(),
    }
}

/// POST /api/backup/restore — restore from a host-local tarball.
///
/// The daemon cannot overwrite its own live SQLite, so the actual extraction
/// happens in a detached helper after this request returns: stop the service,
/// restore, start it again. `keep_config: true` keeps the target's own
/// config.toml (API key, listen address) — the mode for cloning another
/// host's agents and data onto this one.
#[utoipa::path(post, path = "/api/backup/restore", tag = "backup", responses((status = 202, description = "Restore scheduled"), (status = 400, description = "Invalid path")))]
pub async fn restore_backup(
    State(_state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let path_str = match body["path"].as_str() {
        Some(p) if !p.is_empty() => p.to_string(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "path is required"})),
            )
                .into_response();
        }
    };
    let keep_config = body["keep_config"].as_bool().unwrap_or(false);
    let path = std::path::PathBuf::from(&path_str);
    if !path.is_file() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": format!("{} is not a file", path.display())})),
        )
            .into_response();
    }

    // The helper must survive this process stopping, so spawn detached with
    // the same binary (which also implements `restore`).
    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": format!("no exe: {e}")})),
            )
                .into_response();
        }
    };
    let restore_flag = if keep_config { " --keep-config" } else { "" };
    let script = format!(
        "sleep 2; systemctl --user stop librefang.service; '{}' restore '{}'{}; systemctl --user start librefang.service",
        exe.display(),
        path.display(),
        restore_flag
    );
    match std::process::Command::new("sh")
        .arg("-c")
        .arg(&script)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(_) => (
            StatusCode::ACCEPTED,
            Json(serde_json::json!({
                "scheduled": true,
                "message": "Restore scheduled: the daemon will stop, restore, and start again.",
            })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("spawn helper: {e}")})),
        )
            .into_response(),
    }
}

/// DELETE /api/backup/{name} — remove one backup.
#[utoipa::path(delete, path = "/api/backup/{name}", tag = "backup", responses((status = 200, description = "Deleted"), (status = 404, description = "Not found")))]
pub async fn delete_backup(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Response {
    if !valid_backup_name(&name) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "invalid name"})),
        )
            .into_response();
    }
    let path = backups_dir(&state).join(&name);
    match tokio::fs::remove_file(&path).await {
        Ok(()) => Json(serde_json::json!({"deleted": name})).into_response(),
        Err(_) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "backup not found"})),
        )
            .into_response(),
    }
}
