//! Tool profile + agent-type (agent template) endpoints — extracted from
//! `system.rs` per #3749.
//!
//! Mounts `/profiles`, `/profiles/{name}`, `/agent-types`,
//! `/agent-types/{name}`, and `/agent-types/{name}/toml` as the canonical
//! routes. `/templates`, `/templates/{name}`, and `/templates/{name}/toml`
//! are kept mounted too, as a deprecated-but-functional alias (#7722,
//! homogenized further here) — an agent type IS a template, so both
//! spellings resolve to equivalent handlers and no client has to migrate.
//! This module is a sibling under `routes::` and is mounted via
//! `.merge(crate::routes::agent_templates::router())` from `system::router()`.

use super::AppState;
use crate::middleware::RequestLanguage;
use crate::types::ApiErrorResponse;
use axum::extract::Path;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use librefang_types::agent::AgentManifest;
use librefang_types::i18n::ErrorTranslator;
use std::sync::Arc;

/// Build routes for the tool-profile + agent-type (agent template) domain.
pub fn router() -> axum::Router<Arc<AppState>> {
    axum::Router::new()
        .route("/profiles", axum::routing::get(list_profiles))
        .route("/profiles/{name}", axum::routing::get(get_profile))
        // Canonical agent-type routes.
        .route(
            "/agent-types",
            axum::routing::get(list_agent_types).post(create_agent_type),
        )
        .route(
            "/agent-types/{name}",
            axum::routing::get(get_agent_type)
                .put(update_agent_type)
                .delete(delete_agent_type),
        )
        .route(
            "/agent-types/{name}/toml",
            axum::routing::get(get_agent_type_toml),
        )
        // Deprecated alias (#7722, canonicalized further here): agent
        // templates ARE agent types, so `/templates` keeps working for
        // existing clients — routed through thin `_deprecated` wrappers.
        // These are their own separate `#[utoipa::path(...)]`-annotated
        // handlers (rather than reusing the canonical ones' path items)
        // purely so their `operation_id`s stay distinct in the generated
        // OpenAPI spec; `utoipa::path`'s attribute-macro grammar in this
        // crate's utoipa version (5.x) has no `deprecated = true` argument
        // — marking an operation deprecated there means the item itself
        // carries the standard Rust `#[deprecated]` attribute, which is not
        // done here because axum's `.get(list_agent_types_deprecated)` etc.
        // wiring below is itself a "use" of the item and would then trip
        // `-D warnings`. The doc comments on each wrapper below carry the
        // "deprecated alias of ..." note instead.
        .route(
            "/templates",
            axum::routing::get(list_agent_types_deprecated)
                .post(create_agent_type_deprecated),
        )
        .route(
            "/templates/{name}",
            axum::routing::get(get_agent_type_deprecated)
                .put(update_agent_type_deprecated)
                .delete(delete_agent_type_deprecated),
        )
        .route(
            "/templates/{name}/toml",
            axum::routing::get(get_agent_type_toml_deprecated),
        )
}

// ---------------------------------------------------------------------------
// Profile + Mode endpoints
// ---------------------------------------------------------------------------

/// GET /api/profiles — List all tool profiles and their tool lists.
#[utoipa::path(
    get,
    path = "/api/profiles",
    tag = "system",
    responses(
        (status = 200, description = "List tool profiles", body = Vec<serde_json::Value>)
    )
)]
pub async fn list_profiles() -> impl IntoResponse {
    use librefang_types::agent::ToolProfile;

    let profiles = [
        ("minimal", ToolProfile::Minimal),
        ("coding", ToolProfile::Coding),
        ("research", ToolProfile::Research),
        ("messaging", ToolProfile::Messaging),
        ("automation", ToolProfile::Automation),
        ("full", ToolProfile::Full),
    ];

    let result: Vec<serde_json::Value> = profiles
        .iter()
        .map(|(name, profile)| {
            serde_json::json!({
                "name": name,
                "tools": profile.tools(),
            })
        })
        .collect();

    Json(result)
}

/// GET /api/profiles/:name — Get a single profile by name.
#[utoipa::path(get, path = "/api/profiles/{name}", tag = "system", params(("name" = String, Path, description = "Profile name")), responses((status = 200, description = "Profile details", body = crate::types::JsonObject)))]
pub async fn get_profile(
    Path(name): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    use librefang_types::agent::ToolProfile;

    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));

    let profiles: &[(&str, ToolProfile)] = &[
        ("minimal", ToolProfile::Minimal),
        ("coding", ToolProfile::Coding),
        ("research", ToolProfile::Research),
        ("messaging", ToolProfile::Messaging),
        ("automation", ToolProfile::Automation),
        ("full", ToolProfile::Full),
    ];

    match profiles.iter().find(|(n, _)| *n == name) {
        Some((n, profile)) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "name": n,
                "tools": profile.tools(),
            })),
        ),
        None => {
            ApiErrorResponse::not_found(t.t_args("api-error-profile-not-found", &[("name", &name)]))
                .into_json_tuple()
        }
    }
}

// ---------------------------------------------------------------------------
// Template endpoints
// ---------------------------------------------------------------------------

/// Validate a template name supplied via URL path before joining it onto the
/// templates directory. Only permits `[A-Za-z0-9_-]` to guarantee the result
/// cannot escape the base directory through `..`, absolute paths, or platform
/// separators (`/`, `\`). Rejects empty names and anything longer than 64
/// chars to cap log noise.
pub(crate) fn validate_agent_type_name(name: &str) -> Result<(), &'static str> {
    if name.is_empty() || name.len() > 64 {
        return Err("invalid template name");
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err("invalid template name");
    }
    Ok(())
}

#[cfg(test)]
mod agent_type_name_validation_tests {
    use super::validate_agent_type_name;

    #[test]
    fn accepts_simple_names() {
        assert!(validate_agent_type_name("assistant").is_ok());
        assert!(validate_agent_type_name("customer-support").is_ok());
        assert!(validate_agent_type_name("coder_v2").is_ok());
        assert!(validate_agent_type_name("a1").is_ok());
    }

    #[test]
    fn rejects_path_traversal() {
        assert!(validate_agent_type_name("..").is_err());
        assert!(validate_agent_type_name("../../etc").is_err());
        assert!(validate_agent_type_name("foo/../bar").is_err());
        assert!(validate_agent_type_name("..\\..\\tmp").is_err());
    }

    #[test]
    fn rejects_separators_and_absolute_paths() {
        assert!(validate_agent_type_name("foo/bar").is_err());
        assert!(validate_agent_type_name("foo\\bar").is_err());
        assert!(validate_agent_type_name("/etc/passwd").is_err());
        assert!(validate_agent_type_name("C:\\Windows").is_err());
    }

    #[test]
    fn rejects_empty_and_oversized() {
        assert!(validate_agent_type_name("").is_err());
        assert!(validate_agent_type_name(&"a".repeat(65)).is_err());
    }

    #[test]
    fn rejects_null_and_special_chars() {
        assert!(validate_agent_type_name("foo\0bar").is_err());
        assert!(validate_agent_type_name("foo bar").is_err());
        assert!(validate_agent_type_name("foo.bar").is_err());
        assert!(validate_agent_type_name("foo%2fbar").is_err());
    }
}

/// GET /api/agent-types — List available agent types from
/// `~/.librefang/agent-types/` (source = "template").
///
/// Used to also merge in `~/.librefang/workspaces/agents/` (source =
/// "agent") — dropped in the fix for the "Agent Types page is uneditable"
/// bug reported against an install with 42 agents and 0 templates. That
/// merge meant every live agent showed up here with no way to edit or
/// delete it (the dashboard's `AgentTypesPage` can only manage files under
/// `agent-types/`), and the confusing "Managed via Agents" label was the
/// symptom, not the cause. The underlying capability — spawning a worker
/// from an existing agent's manifest by name — still exists at the
/// resolution layer (`resolve_ephemeral_manifest`, `resolve_manifest`,
/// `load_agent_manifest_from_template_dirs` all fall back to
/// `workspaces/agents/<name>/agent.toml` when no template matches); only the
/// *listing* stops conflating the two concepts. The bridge from "I have
/// agents, not templates" to a populated, editable list is
/// `POST /api/agents/{id}/save-as-agent-type`, which extracts a live agent's
/// manifest into a real, editable `agent-types/<name>.toml` file.
#[utoipa::path(get, path = "/api/agent-types", tag = "system", operation_id = "list_agent_types", responses((status = 200, description = "List agent types", body = Vec<serde_json::Value>)))]
pub async fn list_agent_types() -> impl IntoResponse {
    let mut templates = Vec::new();

    // Template files (user-created via POST /api/agent-types, the
    // agent_type_create tool, or POST /api/agents/{id}/save-as-agent-type)
    let templates_dir = agent_types_dir();
    if let Ok(entries) = std::fs::read_dir(&templates_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() || path.extension().and_then(|e| e.to_str()) != Some("toml") {
                continue;
            }
            // The file stem is the identity every other agent-type route
            // resolves by (`/api/agent-types/{name}`, `…/{name}/toml`), so a
            // row must carry it rather than the manifest's own `name` field.
            let name = path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            // Do not advertise a type whose name `/api/agent-types/{name}` and `…/{name}/toml` will reject — a listed row a client cannot fetch or spawn from is a dead end (#7760).
            if validate_agent_type_name(&name).is_err() {
                tracing::warn!("skipping agent type with unusable name: {name:?}");
                continue;
            }
            let Ok(content) = std::fs::read_to_string(&path) else {
                continue;
            };
            let manifest = match toml::from_str::<AgentManifest>(&content) {
                Ok(manifest) => manifest,
                Err(e) => {
                    // One operator typo must not blank the whole catalog for every client.
                    // Skip the entry and name the file so the mistake is diagnosable instead of arriving as an entry with an empty description (#7760).
                    tracing::warn!(
                        "skipping agent type {}: invalid manifest: {e}",
                        path.display()
                    );
                    continue;
                }
            };
            templates.push(serde_json::json!({
                "name": name,
                "description": manifest.description,
                // `provider` / `model` let a client show and gate on what the type actually declares rather than assuming a default (#7760).
                "provider": manifest.model.provider,
                "model": manifest.model.model,
                "source": "template",
            }));
        }
    }

    // `read_dir` order is filesystem-defined; sort so the catalog renders in a stable order across calls and hosts.
    templates.sort_by(|a, b| {
        a["name"]
            .as_str()
            .unwrap_or_default()
            .cmp(b["name"].as_str().unwrap_or_default())
    });

    Json(serde_json::json!({
        "templates": templates,
        "items": templates,
        "total": templates.len(),
    }))
}

/// GET /api/templates — Deprecated alias of `GET /api/agent-types`. Kept
/// working for existing clients; new integrations should use the canonical
/// `/api/agent-types` path.
#[utoipa::path(get, path = "/api/templates", tag = "system", operation_id = "list_agent_templates", responses((status = 200, description = "List agent types (deprecated alias of GET /api/agent-types)", body = Vec<serde_json::Value>)))]
pub async fn list_agent_types_deprecated() -> impl IntoResponse {
    list_agent_types().await
}

/// GET /api/agent-types/:name — Get agent type details.
#[utoipa::path(get, path = "/api/agent-types/{name}", tag = "system", operation_id = "get_agent_type", params(("name" = String, Path, description = "Agent type name")), responses((status = 200, description = "Agent type details", body = crate::types::JsonObject)))]
pub async fn get_agent_type(
    Path(name): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    if validate_agent_type_name(&name).is_err() {
        return ApiErrorResponse::not_found(t.t("api-error-template-not-found")).into_json_tuple();
    }

    // Check templates dir first (user-created), then workspaces agents
    let template_path = agent_types_dir().join(format!("{name}.toml"));
    let agents_dir = super::system::librefang_home()
        .join("workspaces")
        .join("agents");
    let agent_path = agents_dir.join(&name).join("agent.toml");

    let (manifest_path, source) = if template_path.exists() {
        (template_path, "template")
    } else if agent_path.exists() {
        (agent_path, "agent")
    } else {
        return ApiErrorResponse::not_found(t.t("api-error-template-not-found")).into_json_tuple();
    };

    match std::fs::read_to_string(&manifest_path) {
        Ok(content) => match toml::from_str::<AgentManifest>(&content) {
            Ok(manifest) => (
                StatusCode::OK,
                Json({
                    let mut v = manifest_to_agent_type(&name, &manifest);
                    if let Some(o) = v.as_object_mut() {
                        o.insert(
                            "source".to_string(),
                            serde_json::Value::String(source.to_string()),
                        );
                        // The flat fields above are the list-row shape the
                        // dashboard's AgentTypes page reads, and `name` there is
                        // the *template id* (the `.toml` filename). They collapse
                        // that id together with the agent name the manifest
                        // itself declares, and drop `module` / `version` /
                        // `author` entirely — so a detail response built only
                        // from them cannot answer "what does this template
                        // actually declare?".
                        //
                        // Expose the parsed manifest under its own key
                        // alongside them: `name` stays the template id,
                        // `manifest.name` is the declared agent name, and the
                        // two are allowed to differ. Additive on purpose — the
                        // flat fields keep working unchanged.
                        match serde_json::to_value(&manifest) {
                            Ok(m) => {
                                o.insert("manifest".to_string(), m);
                            }
                            Err(e) => {
                                // Serializing a manifest that already parsed
                                // from TOML should not fail; surface it in the
                                // log rather than silently shipping a response
                                // with the key missing.
                                tracing::warn!(
                                    "Failed to serialize manifest for template '{name}': {e}"
                                );
                            }
                        }
                        o.insert(
                            "manifest_toml".to_string(),
                            serde_json::Value::String(content),
                        );
                    }
                    v
                }),
            ),
            Err(e) => {
                tracing::warn!("Invalid template manifest for '{name}': {e}");
                ApiErrorResponse::internal(t.t("api-error-template-invalid-manifest"))
                    .into_json_tuple()
            }
        },
        Err(e) => {
            tracing::warn!("Failed to read template '{name}': {e}");
            ApiErrorResponse::internal(t.t("api-error-template-read-failed")).into_json_tuple()
        }
    }
}

/// GET /api/templates/:name — Deprecated alias of `GET /api/agent-types/:name`.
#[utoipa::path(get, path = "/api/templates/{name}", tag = "system", operation_id = "get_agent_template", params(("name" = String, Path, description = "Agent type name")), responses((status = 200, description = "Agent type details (deprecated alias of GET /api/agent-types/:name)", body = crate::types::JsonObject)))]
pub async fn get_agent_type_deprecated(
    path: Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    get_agent_type(path, lang).await
}

/// GET /api/agent-types/:name/toml — Get the raw TOML content of an agent type.
#[utoipa::path(get, path = "/api/agent-types/{name}/toml", tag = "system", operation_id = "get_agent_type_toml", params(("name" = String, Path, description = "Agent type name")), responses((status = 200, description = "Agent type TOML content as plain text", body = String)))]
pub async fn get_agent_type_toml(
    Path(name): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    if validate_agent_type_name(&name).is_err() {
        return (
            StatusCode::NOT_FOUND,
            [(axum::http::header::CONTENT_TYPE, "text/plain")],
            t.t("api-error-template-not-found"),
        )
            .into_response();
    }
    let template_path = agent_types_dir().join(format!("{name}.toml"));
    let agents_dir = super::system::librefang_home()
        .join("workspaces")
        .join("agents");
    let agent_path = agents_dir.join(&name).join("agent.toml");

    let manifest_path = if template_path.exists() {
        template_path
    } else if agent_path.exists() {
        agent_path
    } else {
        return (
            StatusCode::NOT_FOUND,
            [(axum::http::header::CONTENT_TYPE, "text/plain")],
            t.t("api-error-template-not-found"),
        )
            .into_response();
    };

    match std::fs::read_to_string(&manifest_path) {
        Ok(content) => (
            StatusCode::OK,
            [(
                axum::http::header::CONTENT_TYPE,
                "text/plain; charset=utf-8",
            )],
            content,
        )
            .into_response(),
        Err(e) => {
            tracing::warn!("Failed to read template '{name}': {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                [(axum::http::header::CONTENT_TYPE, "text/plain")],
                t.t("api-error-template-read-failed"),
            )
                .into_response()
        }
    }
}

/// GET /api/templates/:name/toml — Deprecated alias of
/// `GET /api/agent-types/:name/toml`.
#[utoipa::path(get, path = "/api/templates/{name}/toml", tag = "system", operation_id = "get_agent_template_toml", params(("name" = String, Path, description = "Agent type name")), responses((status = 200, description = "Agent type TOML content as plain text (deprecated alias of GET /api/agent-types/:name/toml)", body = String)))]
pub async fn get_agent_type_toml_deprecated(
    path: Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    get_agent_type_toml(path, lang).await
}

// ---------------------------------------------------------------------------
// Agent-type CRUD endpoints
// ---------------------------------------------------------------------------
//
// Agent types are named manifests consumed by the ephemeral-worker spawn
// path (`EphemeralSpawnRequest.agent_type`). They live as `<name>.toml`
// under `~/.librefang/agent-types/`. Write operations are wired into
// `/api/agent-types` (and the deprecated `/api/templates` alias) in the
// unified `router()` above; the read endpoints serve both sources.

/// Directory holding agent-type manifests (`~/.librefang/agent-types/`).
pub(crate) fn agent_types_dir() -> std::path::PathBuf {
    librefang_types::registry_paths::installed_agent_types_dir(&super::system::librefang_home())
}

/// Flatten a manifest into the JSON shape the dashboard expects.
///
/// Must emit every field `interface AgentType` in the dashboard's `api.ts`
/// declares, including `channels` and `routing` — omitting either one here
/// silently defeats the WebUI's channel-allowlist and model-tier editor: the
/// form reads `undefined`, renders empty, and the next save (which reuses
/// this same flat shape) writes the empty value back out, erasing whatever
/// was actually configured (#7740).
pub(crate) fn manifest_to_agent_type(name: &str, m: &AgentManifest) -> serde_json::Value {
    serde_json::json!({
        "name": name,
        "description": m.description,
        "system_prompt": m.model.system_prompt,
        "provider": m.model.provider,
        "model": m.model.model,
        "tools": m.capabilities.tools,
        "skills": m.skills,
        "channels": m.channels,
        "routing": m.routing,
    })
}

/// POST /api/agent-types — Create a new agent type from JSON.
#[utoipa::path(post, path = "/api/agent-types", tag = "system", operation_id = "create_agent_type", request_body = crate::types::JsonObject, responses((status = 201, description = "Agent type created", body = crate::types::JsonObject), (status = 400, description = "Invalid input"), (status = 409, description = "Agent type already exists")))]
pub async fn create_agent_type(
    lang: Option<axum::Extension<RequestLanguage>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));

    let name = match body["name"].as_str() {
        Some(n) if !n.is_empty() => n.to_string(),
        _ => {
            return ApiErrorResponse::bad_request("name is required").into_json_tuple();
        }
    };
    if validate_agent_type_name(&name).is_err() {
        return ApiErrorResponse::bad_request("invalid agent type name").into_json_tuple();
    }

    // Full-manifest path (#7742): mirrors `PATCH /api/agents/{id}`'s
    // `manifest_toml` handling. When the caller (the dashboard's
    // `AgentManifestForm`-backed editor) supplies raw TOML, parse it
    // directly instead of rebuilding a manifest from the flat 9-key JSON
    // shape below — that shape only round-trips the fields it knows about,
    // which is fine for the old quick-create form but would silently drop
    // everything else (resources, autonomous, triggers, …) the visual
    // editor now exposes. The name is still pinned to the validated path
    // value so the template file id and the manifest's own `name` field
    // can never disagree.
    let toml_content =
        if let Some(manifest_toml) = body.get("manifest_toml").and_then(|v| v.as_str()) {
            let mut manifest: AgentManifest = match toml::from_str(manifest_toml) {
                Ok(m) => m,
                Err(e) => {
                    return ApiErrorResponse::bad_request(format!("invalid manifest_toml: {e}"))
                        .into_json_tuple();
                }
            };
            manifest.name = name.clone();
            match toml::to_string_pretty(&manifest) {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!("Failed to serialize manifest for agent-type '{name}': {e}");
                    return ApiErrorResponse::internal(t.t("api-error-internal")).into_json_tuple();
                }
            }
        } else {
            librefang_types::agent::agent_type_json_to_toml(&body)
        };

    let dir = agent_types_dir();
    let path = dir.join(format!("{name}.toml"));
    if path.exists() {
        return ApiErrorResponse::conflict(format!("Agent type '{name}' already exists"))
            .into_json_tuple();
    }

    // Cross-source collision (#6931 review): a workspace agent with the same
    // name is also resolvable as a template (dual-source listing), so
    // creating a template that shadows a live agent's name is a 409 too.
    let workspace_agent_path = super::system::librefang_home()
        .join("workspaces")
        .join("agents")
        .join(&name);
    if workspace_agent_path.exists() {
        return ApiErrorResponse::conflict(format!(
            "A workspace agent named '{name}' already exists — creating a template with the same name would shadow it"
        ))
        .into_json_tuple();
    }

    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::warn!("Failed to create templates dir: {e}");
        return ApiErrorResponse::internal(t.t("api-error-internal")).into_json_tuple();
    }
    if let Err(e) = std::fs::write(&path, &toml_content) {
        tracing::warn!("Failed to write agent-type '{name}': {e}");
        return ApiErrorResponse::internal(t.t("api-error-internal")).into_json_tuple();
    }

    let manifest: AgentManifest = toml::from_str(&toml_content).unwrap_or_default();
    (
        StatusCode::CREATED,
        Json(manifest_to_agent_type(&name, &manifest)),
    )
}

/// POST /api/templates — Deprecated alias of `POST /api/agent-types`.
#[utoipa::path(post, path = "/api/templates", tag = "system", operation_id = "create_template", request_body = crate::types::JsonObject, responses((status = 201, description = "Agent type created (deprecated alias of POST /api/agent-types)", body = crate::types::JsonObject), (status = 400, description = "Invalid input"), (status = 409, description = "Agent type already exists")))]
pub async fn create_agent_type_deprecated(
    lang: Option<axum::Extension<RequestLanguage>>,
    body: Json<serde_json::Value>,
) -> impl IntoResponse {
    create_agent_type(lang, body).await
}

/// PUT /api/agent-types/:name — Update an existing agent type from JSON.
#[utoipa::path(put, path = "/api/agent-types/{name}", tag = "system", operation_id = "update_agent_type", params(("name" = String, Path, description = "Agent type name")), request_body = crate::types::JsonObject, responses((status = 200, description = "Agent type updated", body = crate::types::JsonObject), (status = 400, description = "Invalid input"), (status = 404, description = "Agent type not found")))]
pub async fn update_agent_type(
    Path(name): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    if validate_agent_type_name(&name).is_err() {
        return ApiErrorResponse::not_found(t.t("api-error-template-not-found")).into_json_tuple();
    }

    let path = agent_types_dir().join(format!("{name}.toml"));
    if !path.exists() {
        return ApiErrorResponse::not_found(t.t("api-error-template-not-found")).into_json_tuple();
    }

    // Non-destructive update (#7740): start from the manifest already on
    // disk and apply only the fields the request body actually supplies,
    // instead of rebuilding the whole manifest from the flat 9-key JSON
    // shape (which used to silently drop [compaction], max_history_messages,
    // [[triggers]], [resources], [autonomous], mcp_servers, tool_allowlist,
    // session_mode, workspaces, and every other field on each WebUI save).
    let existing_content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("Failed to read agent-type '{name}' for update: {e}");
            return ApiErrorResponse::internal(t.t("api-error-internal")).into_json_tuple();
        }
    };
    let existing_manifest: AgentManifest = match toml::from_str(&existing_content) {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!("Invalid existing manifest for agent-type '{name}': {e}");
            return ApiErrorResponse::internal(t.t("api-error-template-invalid-manifest"))
                .into_json_tuple();
        }
    };

    // #6931/#6943 review: `Json<serde_json::Value>` accepts any JSON value
    // (array, string, number, bool), but `serde_json::Value`'s `IndexMut<&str>`
    // only handles `Null` and `Object` — every other variant panics via
    // `panic!("cannot access key ... in ...")`. A `PUT` with body `[]` or `42`
    // against an existing agent-type reached that panic and took the process
    // down, so reject non-object bodies up front.
    if !body.is_object() {
        return ApiErrorResponse::bad_request("request body must be a JSON object")
            .into_json_tuple();
    }

    // Pin the manifest name to the URL path segment — the body's
    // "name" field is advisory; the path is authoritative (#6931 review).
    let mut body = body;
    body["name"] = serde_json::Value::String(name.clone());

    // Full-manifest replacement path (#7742), symmetric with the create
    // handler above and with `PATCH /api/agents/{id}`'s `manifest_toml`
    // handling: when present, parse the whole document and use it as-is
    // rather than routing through `apply_agent_type_json_to_manifest`'s
    // flat 9-key merge. This is what lets the dashboard's
    // `AgentManifestForm`-backed editor save every manifest field it
    // exposes (resources, autonomous, thinking, routing, context
    // injection, …) instead of only the fields the flat JSON shape
    // understands — and it stays non-destructive for the same reason the
    // flat-merge path is (#7740): `AgentManifestForm` is always seeded
    // from this same template's current `manifest_toml`, and any field the
    // form doesn't render is preserved verbatim in `extras` and re-emitted
    // on save. `apply_agent_type_json_to_manifest` is left untouched and
    // still serves the flat-JSON callers that never send `manifest_toml`
    // (the `agent_type_create` tool has no update counterpart today, but
    // older API clients built against the original 9-key PUT shape keep
    // working unchanged).
    let manifest = if let Some(manifest_toml) = body.get("manifest_toml").and_then(|v| v.as_str()) {
        let mut manifest: AgentManifest = match toml::from_str(manifest_toml) {
            Ok(m) => m,
            Err(e) => {
                return ApiErrorResponse::bad_request(format!("invalid manifest_toml: {e}"))
                    .into_json_tuple();
            }
        };
        manifest.name = name.clone();
        manifest
    } else {
        librefang_types::agent::apply_agent_type_json_to_manifest(existing_manifest, &body)
    };

    let toml_content = match toml::to_string_pretty(&manifest) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("Failed to serialize updated manifest for '{name}': {e}");
            return ApiErrorResponse::internal(t.t("api-error-internal")).into_json_tuple();
        }
    };

    if let Err(e) = std::fs::write(&path, &toml_content) {
        tracing::warn!("Failed to write agent-type '{name}': {e}");
        return ApiErrorResponse::internal(t.t("api-error-internal")).into_json_tuple();
    }

    (
        StatusCode::OK,
        Json(manifest_to_agent_type(&name, &manifest)),
    )
}

/// PUT /api/templates/:name — Deprecated alias of `PUT /api/agent-types/:name`.
#[utoipa::path(put, path = "/api/templates/{name}", tag = "system", operation_id = "update_template", params(("name" = String, Path, description = "Agent type name")), request_body = crate::types::JsonObject, responses((status = 200, description = "Agent type updated (deprecated alias of PUT /api/agent-types/:name)", body = crate::types::JsonObject), (status = 400, description = "Invalid input"), (status = 404, description = "Agent type not found")))]
pub async fn update_agent_type_deprecated(
    path: Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
    body: Json<serde_json::Value>,
) -> impl IntoResponse {
    update_agent_type(path, lang, body).await
}

/// DELETE /api/agent-types/:name — Delete an agent type file.
#[utoipa::path(delete, path = "/api/agent-types/{name}", tag = "system", operation_id = "delete_agent_type", params(("name" = String, Path, description = "Agent type name")), responses((status = 200, description = "Agent type deleted", body = crate::types::JsonObject), (status = 404, description = "Agent type not found")))]
pub async fn delete_agent_type(
    Path(name): Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    let t = ErrorTranslator::new(super::resolve_lang(lang.as_ref()));
    if validate_agent_type_name(&name).is_err() {
        return ApiErrorResponse::not_found(t.t("api-error-template-not-found")).into_json_tuple();
    }

    let path = agent_types_dir().join(format!("{name}.toml"));
    if !path.exists() {
        return ApiErrorResponse::not_found(t.t("api-error-template-not-found")).into_json_tuple();
    }

    if let Err(e) = std::fs::remove_file(&path) {
        tracing::warn!("Failed to delete agent-type '{name}': {e}");
        return ApiErrorResponse::internal(t.t("api-error-internal")).into_json_tuple();
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({ "name": name, "deleted": true })),
    )
}

/// DELETE /api/templates/:name — Deprecated alias of
/// `DELETE /api/agent-types/:name`.
#[utoipa::path(delete, path = "/api/templates/{name}", tag = "system", operation_id = "delete_template", params(("name" = String, Path, description = "Agent type name")), responses((status = 200, description = "Agent type deleted (deprecated alias of DELETE /api/agent-types/:name)", body = crate::types::JsonObject), (status = 404, description = "Agent type not found")))]
pub async fn delete_agent_type_deprecated(
    path: Path<String>,
    lang: Option<axum::Extension<RequestLanguage>>,
) -> impl IntoResponse {
    delete_agent_type(path, lang).await
}
