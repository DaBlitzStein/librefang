//! Versioned session artifact API and signed download links.
//!
//! Management routes are scoped to one session and inherit the same auth and
//! owner checks as `/api/sessions/*`: listing, version history, download and
//! restore. `GET /api/artifacts/{token}` is public — the HMAC-signed token
//! carries the only credential, exactly like a presigned object-store URL.

use super::AppState;
use crate::middleware::AuthenticatedApiUser;
use crate::types::ApiErrorResponse;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::header::{CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_TYPE};
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use base64::Engine as _;
use hmac::{Hmac, Mac};
use librefang_memory::artifact_store::{ArtifactRecord, ArtifactStore};
use librefang_types::agent::{AgentId, SessionId};
use rand::Rng as _;
use serde::Deserialize;
use sha2::Sha256;
use std::io::Write as _;
use std::path::Path as FsPath;
use std::sync::Arc;
use subtle::ConstantTimeEq;
use uuid::Uuid;

/// Default lifetime of a signed artifact link.
const LINK_TTL_DEFAULT_SECS: i64 = 900;
/// Minimum accepted lifetime of a signed artifact link.
const LINK_TTL_MIN_SECS: i64 = 60;
/// Maximum accepted lifetime of a signed artifact link.
const LINK_TTL_MAX_SECS: i64 = 86_400;
/// Raw HMAC key length persisted in `<blob_root>/link.key`.
const LINK_KEY_BYTES: usize = 32;
/// Raw HMAC-SHA256 signature length.
const LINK_SIG_BYTES: usize = 32;

type HmacSha256 = Hmac<Sha256>;

/// Build the artifact routes. Mounted under `/api` by `server.rs`.
pub fn router() -> axum::Router<Arc<AppState>> {
    use axum::routing::{get, post};
    axum::Router::new()
        .route("/sessions/{session_id}/artifacts", get(list_artifacts))
        .route(
            "/sessions/{session_id}/artifacts/versions",
            get(list_artifact_versions),
        )
        .route(
            "/sessions/{session_id}/artifacts/download",
            get(download_artifact),
        )
        .route(
            "/sessions/{session_id}/artifacts/restore",
            post(restore_artifact),
        )
        .route("/sessions/{session_id}/artifacts/link", post(link_artifact))
        .route("/artifacts/{token}", get(download_linked_artifact))
}

/// Claims carried by a signed artifact link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkClaims {
    /// Session the artifact belongs to.
    pub session_id: String,
    /// Artifact path exactly as recorded.
    pub path: String,
    /// Version the token may read.
    pub version: i64,
    /// Unix expiry in seconds.
    pub expires_at: i64,
}

/// Sign one artifact link with `key`.
///
/// The token is URL-safe base64 of `session_id\npath\nversion\nexpiry`
/// followed by the raw HMAC-SHA256 over those bytes.
pub fn sign_link_token(key: &[u8], claims: &LinkClaims) -> String {
    let payload = format!(
        "{}\n{}\n{}\n{}",
        claims.session_id, claims.path, claims.version, claims.expires_at
    );
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(payload.as_bytes());
    let mut token = payload.into_bytes();
    token.extend_from_slice(&mac.finalize().into_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(token)
}

/// Verify a token and return its claims, or `None` for a malformed, tampered
/// or expired token.
pub fn verify_link_token(key: &[u8], token: &str, now: i64) -> Option<LinkClaims> {
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(token)
        .ok()?;
    if decoded.len() <= LINK_SIG_BYTES {
        return None;
    }
    let (payload, signature) = decoded.split_at(decoded.len() - LINK_SIG_BYTES);

    let mut mac = HmacSha256::new_from_slice(key).ok()?;
    mac.update(payload);
    let expected = mac.finalize().into_bytes();
    if !bool::from(expected.as_slice().ct_eq(signature)) {
        return None;
    }

    let payload = std::str::from_utf8(payload).ok()?;
    let mut fields = payload.split('\n');
    let session_id = fields.next()?.to_string();
    let path = fields.next()?.to_string();
    let version = fields.next()?.parse::<i64>().ok()?;
    let expires_at = fields.next()?.parse::<i64>().ok()?;
    if fields.next().is_some() || expires_at <= now {
        return None;
    }
    Some(LinkClaims {
        session_id,
        path,
        version,
        expires_at,
    })
}

/// Read the link key under `blob_root`, creating it on first use.
///
/// The key is written to a temp file in the same directory and renamed into
/// place, so a reader never sees a partial key. A lost creation race re-reads
/// the winner instead of overwriting it.
fn load_or_create_link_key(blob_root: &FsPath) -> std::io::Result<Vec<u8>> {
    let key_path = blob_root.join("link.key");
    match std::fs::read(&key_path) {
        Ok(key) if key.len() == LINK_KEY_BYTES => return Ok(key),
        Ok(_) => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "artifact link key has the wrong length",
            ))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }

    std::fs::create_dir_all(blob_root)?;
    let mut key = [0u8; LINK_KEY_BYTES];
    rand::rng().fill_bytes(&mut key);

    let tmp_path = blob_root.join(format!(".link.key.{}.tmp", Uuid::new_v4()));
    {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let mut tmp = options.open(&tmp_path)?;
        tmp.write_all(&key)?;
        tmp.sync_all()?;
    }

    if key_path.exists() {
        let _ = std::fs::remove_file(&tmp_path);
        return std::fs::read(&key_path);
    }
    if let Err(e) = std::fs::rename(&tmp_path, &key_path) {
        let _ = std::fs::remove_file(&tmp_path);
        if key_path.exists() {
            return std::fs::read(&key_path);
        }
        return Err(e);
    }
    // A concurrent creator may have renamed between the check and ours; the
    // file is the authority, never the bytes this call generated.
    std::fs::read(&key_path)
}

// `axum::Response` is 128 bytes and trips clippy's `result_large_err` lint;
// the same suppression every other route module uses for handler helpers.
#[allow(clippy::result_large_err)]
fn link_key(state: &AppState) -> Result<Vec<u8>, Response> {
    let config = state.kernel.config_ref();
    let blob_root = config.artifacts.resolved_blob_dir(&config.home_dir);
    drop(config);
    load_or_create_link_key(&blob_root).map_err(|e| {
        tracing::error!(error = %e, "failed to load artifact link key");
        ApiErrorResponse::internal("artifact link signing is unavailable").into_response()
    })
}

#[allow(clippy::result_large_err)]
fn artifact_store(state: &AppState) -> Result<&ArtifactStore, Response> {
    state.kernel.memory_substrate().artifacts().ok_or_else(|| {
        tracing::error!("artifact store is not attached to the memory substrate");
        ApiErrorResponse::internal("artifact store is unavailable").into_response()
    })
}

/// Resolve the session's agent, applying the same existence and owner-scope
/// contract as `GET /api/sessions/{id}`.
#[allow(clippy::result_large_err)]
fn session_agent(
    state: &AppState,
    session_id: &str,
    api_user: Option<&axum::Extension<AuthenticatedApiUser>>,
) -> Result<AgentId, Response> {
    let uuid = session_id
        .parse::<Uuid>()
        .map_err(|_| ApiErrorResponse::bad_request("invalid session id").into_response())?;
    match state
        .kernel
        .memory_substrate()
        .get_session_with_created_at(SessionId(uuid))
    {
        Ok(Some((session, _)))
            if super::caller_may_access_agent_data(state, session.agent_id, api_user) =>
        {
            Ok(session.agent_id)
        }
        Ok(_) => Err(ApiErrorResponse::not_found("session not found").into_response()),
        Err(e) => {
            tracing::error!(error = %e, "failed to load session for artifact route");
            Err(ApiErrorResponse::internal("failed to load session").into_response())
        }
    }
}

fn download_response(record: &ArtifactRecord, bytes: Vec<u8>) -> Response {
    let filename: String = record
        .path
        .rsplit('/')
        .next()
        .unwrap_or("artifact")
        .chars()
        .map(|c| {
            if c.is_control() || c == '"' || c == '\\' {
                '_'
            } else {
                c
            }
        })
        .collect();
    let filename = if filename.is_empty() {
        "artifact".to_string()
    } else {
        filename
    };

    let content_type = record
        .mime_type
        .as_deref()
        .and_then(|mime| HeaderValue::from_str(mime).ok())
        .unwrap_or_else(|| HeaderValue::from_static("application/octet-stream"));
    let disposition = HeaderValue::from_str(&format!("attachment; filename=\"{filename}\""))
        .unwrap_or_else(|_| HeaderValue::from_static("attachment"));

    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, content_type)
        .header(CONTENT_DISPOSITION, disposition)
        .header(CONTENT_LENGTH, bytes.len().to_string())
        .body(Body::from(bytes))
        .expect("artifact download header values were validated above")
}

/// `GET /api/sessions/{session_id}/artifacts`
pub async fn list_artifacts(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    api_user: Option<axum::Extension<AuthenticatedApiUser>>,
) -> Response {
    if let Err(resp) = session_agent(&state, &session_id, api_user.as_ref()) {
        return resp;
    }
    let store = match artifact_store(&state) {
        Ok(store) => store,
        Err(resp) => return resp,
    };
    match store.list(&session_id) {
        Ok(artifacts) => Json(serde_json::json!({ "artifacts": artifacts })).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "failed to list session artifacts");
            ApiErrorResponse::internal("failed to list artifacts").into_response()
        }
    }
}

#[derive(Deserialize)]
/// Query for the version-history route.
pub struct VersionsQuery {
    /// Artifact path exactly as recorded.
    pub path: String,
}

/// `GET /api/sessions/{session_id}/artifacts/versions?path=...`
pub async fn list_artifact_versions(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Query(query): Query<VersionsQuery>,
    api_user: Option<axum::Extension<AuthenticatedApiUser>>,
) -> Response {
    if let Err(resp) = session_agent(&state, &session_id, api_user.as_ref()) {
        return resp;
    }
    let store = match artifact_store(&state) {
        Ok(store) => store,
        Err(resp) => return resp,
    };
    match store.versions(&session_id, &query.path) {
        Ok(versions) if versions.is_empty() => {
            ApiErrorResponse::not_found("artifact path not found").into_response()
        }
        Ok(versions) => Json(serde_json::json!({ "versions": versions })).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "failed to list artifact versions");
            ApiErrorResponse::internal("failed to list artifact versions").into_response()
        }
    }
}

#[derive(Deserialize)]
/// Query for the download route.
pub struct DownloadQuery {
    /// Artifact path exactly as recorded.
    pub path: String,
    /// Version to download.
    pub version: i64,
}

/// `GET /api/sessions/{session_id}/artifacts/download?path=...&version=N`
pub async fn download_artifact(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Query(query): Query<DownloadQuery>,
    api_user: Option<axum::Extension<AuthenticatedApiUser>>,
) -> Response {
    if let Err(resp) = session_agent(&state, &session_id, api_user.as_ref()) {
        return resp;
    }
    let store = match artifact_store(&state) {
        Ok(store) => store,
        Err(resp) => return resp,
    };
    match store.read(&session_id, &query.path, query.version) {
        Ok(Some((record, bytes))) => download_response(&record, bytes),
        Ok(None) => ApiErrorResponse::not_found("artifact version not found").into_response(),
        Err(e) => {
            tracing::error!(error = %e, "failed to read artifact version");
            ApiErrorResponse::not_found("artifact version not found").into_response()
        }
    }
}

#[derive(Deserialize)]
/// Body for the restore route.
pub struct RestoreRequest {
    /// Artifact path exactly as recorded; restored relative to the workspace.
    pub path: String,
    /// Version whose bytes are written back.
    pub version: i64,
}

/// `POST /api/sessions/{session_id}/artifacts/restore`
pub async fn restore_artifact(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    api_user: Option<axum::Extension<AuthenticatedApiUser>>,
    Json(request): Json<RestoreRequest>,
) -> Response {
    let agent_id = match session_agent(&state, &session_id, api_user.as_ref()) {
        Ok(agent_id) => agent_id,
        Err(resp) => return resp,
    };
    let store = match artifact_store(&state) {
        Ok(store) => store,
        Err(resp) => return resp,
    };
    let (record, bytes) = match store.read(&session_id, &request.path, request.version) {
        Ok(Some(row)) => row,
        Ok(None) => {
            return ApiErrorResponse::not_found("artifact version not found").into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "failed to read artifact version for restore");
            return ApiErrorResponse::not_found("artifact version not found").into_response();
        }
    };

    if request.path.is_empty() || FsPath::new(&request.path).is_absolute() {
        return ApiErrorResponse::forbidden("artifact path must be workspace-relative")
            .into_response();
    }

    let workspace = match state.kernel.agent_registry().get(agent_id) {
        Some(entry) => entry.manifest.workspace.clone(),
        None => None,
    };
    let Some(workspace) = workspace.filter(|path| path.is_dir()) else {
        return ApiErrorResponse::conflict("agent workspace is missing").into_response();
    };

    let resolved = match librefang_kernel::workspace_sandbox::resolve_sandbox_path(
        &request.path,
        &workspace,
    ) {
        Ok(path) => path,
        Err(e) => {
            tracing::warn!(path = %request.path, error = %e, "refused unsafe artifact restore path");
            return ApiErrorResponse::forbidden("artifact path is not allowed in the workspace")
                .into_response();
        }
    };
    if let Err(e) =
        librefang_kernel::workspace_sandbox::deny_own_manifest_write(&resolved, &workspace)
    {
        tracing::warn!(path = %request.path, error = %e, "refused artifact restore over agent manifest");
        return ApiErrorResponse::forbidden("artifact path is not allowed in the workspace")
            .into_response();
    }

    if let Err(e) = write_atomically(&resolved, &bytes) {
        tracing::error!(path = %resolved.display(), error = %e, "failed to restore artifact");
        return ApiErrorResponse::internal("failed to write artifact into the workspace")
            .into_response();
    }

    Json(serde_json::json!({
        "restored": true,
        "path": record.path,
        "version": record.version,
    }))
    .into_response()
}

/// Write `bytes` through a sibling temp file and rename, so a reader sees the
/// old file or the complete new one.
fn write_atomically(destination: &FsPath, bytes: &[u8]) -> std::io::Result<()> {
    let parent = destination
        .parent()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "no parent"))?;
    std::fs::create_dir_all(parent)?;
    let filename = destination
        .file_name()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "no filename"))?;
    let tmp = parent.join(format!(
        ".{}.{}.tmp",
        filename.to_string_lossy(),
        Uuid::new_v4()
    ));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, destination)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

#[derive(Deserialize)]
/// Body for the link route.
pub struct LinkRequest {
    /// Artifact path exactly as recorded.
    pub path: String,
    /// Version to link; defaults to the latest.
    pub version: Option<i64>,
    /// Requested lifetime; clamped to 60..=86400 seconds.
    pub ttl_secs: Option<i64>,
}

/// `POST /api/sessions/{session_id}/artifacts/link`
pub async fn link_artifact(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    api_user: Option<axum::Extension<AuthenticatedApiUser>>,
    Json(request): Json<LinkRequest>,
) -> Response {
    if let Err(resp) = session_agent(&state, &session_id, api_user.as_ref()) {
        return resp;
    }
    let store = match artifact_store(&state) {
        Ok(store) => store,
        Err(resp) => return resp,
    };
    let versions = match store.versions(&session_id, &request.path) {
        Ok(versions) if versions.is_empty() => {
            return ApiErrorResponse::not_found("artifact path not found").into_response()
        }
        Ok(versions) => versions,
        Err(e) => {
            tracing::error!(error = %e, "failed to read artifact versions for link");
            return ApiErrorResponse::internal("failed to read artifact versions").into_response();
        }
    };
    let version = match request.version {
        Some(version) if versions.iter().any(|row| row.version == version) => version,
        Some(_) => {
            return ApiErrorResponse::not_found("artifact version not found").into_response()
        }
        None => versions.last().expect("non-empty versions").version,
    };

    let key = match link_key(&state) {
        Ok(key) => key,
        Err(resp) => return resp,
    };
    let ttl = request
        .ttl_secs
        .unwrap_or(LINK_TTL_DEFAULT_SECS)
        .clamp(LINK_TTL_MIN_SECS, LINK_TTL_MAX_SECS);
    let claims = LinkClaims {
        session_id: session_id.clone(),
        path: request.path,
        version,
        expires_at: chrono::Utc::now().timestamp() + ttl,
    };
    let token = sign_link_token(&key, &claims);
    Json(serde_json::json!({
        "url": format!("/api/artifacts/{token}"),
        "token": token,
        "expires_at": claims.expires_at,
    }))
    .into_response()
}

/// `GET /api/artifacts/{token}` — public; the signed token is the credential.
pub async fn download_linked_artifact(
    State(state): State<Arc<AppState>>,
    Path(token): Path<String>,
) -> Response {
    let key = match link_key(&state) {
        Ok(key) => key,
        Err(resp) => return resp,
    };
    let Some(claims) = verify_link_token(&key, &token, chrono::Utc::now().timestamp()) else {
        return ApiErrorResponse::forbidden("invalid or expired artifact link").into_response();
    };
    let store = match artifact_store(&state) {
        Ok(store) => store,
        Err(resp) => return resp,
    };
    match store.read(&claims.session_id, &claims.path, claims.version) {
        Ok(Some((record, bytes))) => download_response(&record, bytes),
        Ok(None) => ApiErrorResponse::not_found("artifact version not found").into_response(),
        Err(e) => {
            tracing::error!(error = %e, "failed to read linked artifact version");
            ApiErrorResponse::not_found("artifact version not found").into_response()
        }
    }
}
