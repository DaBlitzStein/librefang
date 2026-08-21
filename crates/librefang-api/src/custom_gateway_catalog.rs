//! TTL-based `/v1/models` discovery for custom OpenAI-compatible gateways —
//! litellm, a self-hosted vLLM/llama.cpp proxy registered under its own
//! provider id, or any other operator-run relay. Refs #7775.
//!
//! `GET /api/models?provider=<gateway>` used to answer from the static
//! registry catalog only, which — by definition — never contains an
//! operator-defined gateway's model ids. Registered OpenRouter/EveryAPI
//! gateways already get a live refresh via [`crate::openrouter_catalog`] /
//! [`crate::everyapi_catalog`]; a self-hosted gateway had nothing.
//!
//! This module deliberately mirrors those two: the same `needs_*` /
//! `refresh_if_*` surface, the same per-base-URL `REFRESH_ATTEMPTS` retry
//! window, the same `reconcile_live_provider_models` merge, the same
//! never-fail-the-request degrade-and-warn behaviour on network failure. It
//! stays a separate module rather than folding into either existing one for
//! the same reason `everyapi_catalog` gives for staying separate from
//! `openrouter_catalog`: the eligible provider set and the response shape
//! differ enough — arbitrary/unregistered ids, no curated pricing feed, an
//! optional litellm-only `/v1/model/info` extension — that a shared
//! abstraction would put the two proven paths at risk for no gain here.
//!
//! ## Scope: which providers this applies to
//!
//! A provider is in scope when [`librefang_llm_drivers::drivers::provider_api_format`]
//! returns `None` for its id — i.e. the id is NOT one of the built-in
//! registry entries (openai, anthropic, groq, openrouter, ollama, vllm,
//! lmstudio, lemonade, …). That function's own doc comment establishes the
//! convention this module leans on: an id unknown to the registry defaults
//! to the OpenAI wire shape, the same assumption
//! `provider_health::probe_provider` and the provider-test handler already
//! make when picking `/models` + `Authorization: Bearer` for an
//! unrecognized name. So this module only ever talks to genuinely
//! operator-defined gateways — a curated registry provider (even one that
//! also happens to speak OpenAI, like Groq or DeepSeek) keeps its static
//! metadata untouched, and `everyapi` (also unknown to the registry) is
//! explicitly excluded because it already has its own dedicated module.
//! The built-in local ids (`ollama` / `vllm` / `lmstudio` / `lemonade`) are
//! all registry-known, so they never reach this module either — they stay
//! on the existing periodic-probe + `merge_discovered_models` path, which
//! already covers them.
//!
//! No separate opt-in flag is required: like OpenRouter and EveryAPI, a
//! provider becomes eligible the moment its `base_url` is set and its
//! `auth_status` says the endpoint is worth a request.

use dashmap::DashMap;
use librefang_kernel::kernel_api::KernelApi;
use librefang_kernel::model_catalog::ModelCatalog;
use librefang_types::model_catalog::{Modality, ModelCatalogEntry, ModelTier, ProviderInfo};
use std::collections::HashMap;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

/// Provider id handled by the dedicated EveryAPI module instead of this one.
const EVERYAPI_PROVIDER_ID: &str = "everyapi";

/// How long a fetched gateway model list stays fresh. Matches
/// `OPENROUTER_MODEL_CATALOG_TTL` / `EVERYAPI_MODEL_CATALOG_TTL` — the same
/// refresh economics apply: a blocking round trip on an interactive request
/// path is only acceptable if it's rare, and an operator's gateway roster
/// changes on the order of days, not minutes.
const CUSTOM_GATEWAY_MODEL_CATALOG_TTL: Duration = Duration::from_secs(15 * 60);

/// Guards against hammering a down or slow gateway: the stamp is written on
/// every attempt (success or failure), so this is a hard floor on refresh
/// frequency per base URL, not a failure-only backoff. Keyed by base URL
/// rather than provider id so integration tests on sequential ephemeral
/// ports do not contaminate each other (mirrors #6384).
static REFRESH_ATTEMPTS: LazyLock<DashMap<String, Instant>> = LazyLock::new(DashMap::new);
const REFRESH_RETRY_WINDOW: Duration = Duration::from_secs(60);

// ---------------------------------------------------------------------------
// Scope + freshness predicates
// ---------------------------------------------------------------------------

/// Whether `provider` is a genuinely operator-defined OpenAI-compatible
/// gateway this module should discover models for. See the module doc for
/// the full reasoning; in short: unknown to the built-in driver registry,
/// not the specially-handled EveryAPI id, has somewhere to call, and is in
/// an auth state worth a network round trip.
fn is_custom_openai_compatible_gateway(provider: &ProviderInfo) -> bool {
    provider.id != EVERYAPI_PROVIDER_ID
        && !provider.base_url.trim().is_empty()
        && provider.auth_status.is_available()
        && librefang_llm_drivers::drivers::provider_api_format(&provider.id).is_none()
}

fn catalog_provider_is_eligible(catalog: &ModelCatalog, provider_id: &str) -> bool {
    catalog
        .get_provider(provider_id)
        .is_some_and(is_custom_openai_compatible_gateway)
}

fn catalog_needs_stale_refresh(catalog: &ModelCatalog, provider_id: &str) -> bool {
    catalog_provider_is_eligible(catalog, provider_id)
        && catalog.live_provider_models_are_stale(provider_id, CUSTOM_GATEWAY_MODEL_CATALOG_TTL)
}

// ---------------------------------------------------------------------------
// Entry points
// ---------------------------------------------------------------------------

/// Refresh `provider_id`'s live model list when the last successful fetch
/// (if any) is missing or older than the TTL. A no-op — `Ok(0)`, no network
/// call — for a provider outside this module's scope, so callers can invoke
/// it unconditionally over every provider id without a pre-check.
pub(crate) async fn refresh_if_stale(
    kernel: &Arc<dyn KernelApi>,
    provider_id: &str,
) -> Result<usize, String> {
    if !catalog_needs_stale_refresh(&kernel.model_catalog_ref().load(), provider_id) {
        return Ok(0);
    }
    refresh_now(kernel, provider_id).await
}

// ---------------------------------------------------------------------------
// Gateway response parsing
// ---------------------------------------------------------------------------

/// Per-model figures recovered from the litellm-only `GET {base}/model/info`
/// extension. Every field is optional because the endpoint may be entirely
/// absent (any non-litellm OpenAI-compatible server) or present but
/// null-valued (litellm itself, when the operator never registered a limit
/// for the model — the case that motivated this module, refs #7775).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct LiveLimits {
    context_window: Option<u64>,
    max_output_tokens: Option<u64>,
    input_cost_per_m: Option<f64>,
    output_cost_per_m: Option<f64>,
}

/// Parse `GET {base}/models` into the bare id list. Rows without a usable
/// `id` are dropped rather than failing the whole response.
fn parse_model_ids(body: &serde_json::Value) -> Vec<String> {
    let Some(items) = body.get("data").and_then(|d| d.as_array()) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            item.get("id")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(str::to_string)
        })
        .collect()
}

/// Parse the litellm-only `GET {base}/model/info` response into a lookup
/// table keyed by lowercased model name.
///
/// litellm's per-token cost fields are USD-per-token; multiplied here by
/// 1e6 to match the catalog's USD-per-million-token convention. A `null`
/// field (litellm reports one whenever the operator never set a limit —
/// the exact shape the reporting instance hit) is treated as "unknown",
/// never as zero, so it can never overwrite a real value carried forward
/// from a previous refresh.
fn parse_model_info(body: &serde_json::Value) -> HashMap<String, LiveLimits> {
    let Some(items) = body.get("data").and_then(|d| d.as_array()) else {
        return HashMap::new();
    };
    let mut out = HashMap::new();
    for item in items {
        let Some(name) = item
            .get("model_name")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|n| !n.is_empty())
        else {
            continue;
        };
        let info = item.get("model_info");
        let context_window = info
            .and_then(|i| i.get("max_input_tokens"))
            .and_then(serde_json::Value::as_u64)
            .or_else(|| {
                info.and_then(|i| i.get("max_tokens"))
                    .and_then(serde_json::Value::as_u64)
            })
            .filter(|v| *v > 0);
        let max_output_tokens = info
            .and_then(|i| i.get("max_output_tokens"))
            .and_then(serde_json::Value::as_u64)
            .filter(|v| *v > 0);
        let input_cost_per_m = info
            .and_then(|i| i.get("input_cost_per_token"))
            .and_then(serde_json::Value::as_f64)
            .filter(|v| v.is_finite() && *v >= 0.0)
            .map(|v| v * 1_000_000.0);
        let output_cost_per_m = info
            .and_then(|i| i.get("output_cost_per_token"))
            .and_then(serde_json::Value::as_f64)
            .filter(|v| v.is_finite() && *v >= 0.0)
            .map(|v| v * 1_000_000.0);
        out.insert(
            name.to_lowercase(),
            LiveLimits {
                context_window,
                max_output_tokens,
                input_cost_per_m,
                output_cost_per_m,
            },
        );
    }
    out
}

// ---------------------------------------------------------------------------
// Merge
// ---------------------------------------------------------------------------

/// Build the replacement entry set for `provider_id`.
///
/// `existing` is the provider's current catalog entries; anything neither
/// `/models` nor `/model/info` publishes is carried forward from there, the
/// same carry-forward discipline `everyapi_catalog::build_catalog_entries`
/// uses so a refresh can only ADD metadata, never silently delete it.
///
/// A model with no context window / output limit from any source (gateway
/// never registered one — e.g. the litellm instance that motivated this
/// module, whose `/v1/model/info` returns `max_tokens: null` for every row)
/// is still emitted, with `0` in the unfilled field — the catalog's
/// documented "unknown" sentinel (see `synthesized_cli_model_row` in
/// `providers.rs`). Unlike `everyapi_catalog`, such a row is NOT dropped:
/// the model existing in the picker is the point of this module (#7775),
/// and `0` already degrades safely everywhere downstream that reads it —
/// dropping the row instead would put back exactly the "the models I know
/// are live simply don't appear" bug this module exists to fix.
///
/// Output is sorted by id (repo invariant #3298).
fn build_catalog_entries(
    provider_id: &str,
    live_ids: &[String],
    limits: &HashMap<String, LiveLimits>,
    existing: &[ModelCatalogEntry],
) -> Vec<ModelCatalogEntry> {
    let previous: HashMap<String, &ModelCatalogEntry> = existing
        .iter()
        .map(|entry| (entry.id.to_lowercase(), entry))
        .collect();

    let mut entries: Vec<ModelCatalogEntry> = live_ids
        .iter()
        .map(|id| {
            let key = id.to_lowercase();
            let prior = previous.get(&key).copied();
            let limit = limits.get(&key).copied().unwrap_or_default();

            let context_window = limit
                .context_window
                .or_else(|| prior.map(|p| p.context_window).filter(|c| *c > 0))
                .unwrap_or(0);
            let max_output_tokens = limit
                .max_output_tokens
                .or_else(|| prior.map(|p| p.max_output_tokens).filter(|c| *c > 0))
                .unwrap_or(0);

            // Pricing moves as one unit: a cost is never carried forward
            // while `pricing_known` is reset, and `0.0 / 0.0` is never
            // emitted as a known price.
            let (input_cost_per_m, output_cost_per_m, pricing_known) =
                match (limit.input_cost_per_m, limit.output_cost_per_m) {
                    (Some(input), Some(output)) => (input, output, true),
                    _ => match prior.filter(|p| p.pricing_known) {
                        Some(p) => (p.input_cost_per_m, p.output_cost_per_m, true),
                        None => (0.0, 0.0, false),
                    },
                };

            ModelCatalogEntry {
                id: id.clone(),
                display_name: prior
                    .map(|p| p.display_name.clone())
                    .filter(|name| !name.trim().is_empty())
                    .unwrap_or_else(|| id.clone()),
                provider: provider_id.to_string(),
                // Deliberately NOT `ModelTier::Custom` — `find_model` returns
                // the first `Custom` match immediately, so a gateway copy of
                // an id that also exists upstream would hijack every
                // provider-blind lookup. `reconcile_live_provider_models`
                // donates the previous tier when one existed.
                tier: prior.map(|p| p.tier).unwrap_or(ModelTier::Balanced),
                modality: prior.map(|p| p.modality).unwrap_or(Modality::Text),
                context_window,
                max_output_tokens,
                input_cost_per_m,
                output_cost_per_m,
                pricing_known,
                supports_tools: prior.is_some_and(|p| p.supports_tools),
                supports_vision: prior.is_some_and(|p| p.supports_vision),
                // Every OpenAI-shaped chat-completions endpoint streams.
                supports_streaming: true,
                supports_thinking: prior.is_some_and(|p| p.supports_thinking),
                ..Default::default()
            }
        })
        .collect();

    entries.sort_by(|a, b| a.id.cmp(&b.id));
    entries
}

// ---------------------------------------------------------------------------
// Refresh
// ---------------------------------------------------------------------------

fn try_claim_refresh_slot(base_url: &str) -> Result<(), String> {
    match REFRESH_ATTEMPTS.entry(base_url.to_string()) {
        dashmap::mapref::entry::Entry::Occupied(mut attempt) => {
            if attempt.get().elapsed() < REFRESH_RETRY_WINDOW {
                return Err(
                    "custom-gateway catalog refresh is in the 60-second retry window".to_string(),
                );
            }
            attempt.insert(Instant::now());
            Ok(())
        }
        dashmap::mapref::entry::Entry::Vacant(attempt) => {
            attempt.insert(Instant::now());
            Ok(())
        }
    }
}

/// Fetch the authoritative id list from `GET {base}/models`.
async fn fetch_live_model_ids(
    base_url: &str,
    api_key: Option<&str>,
) -> Result<Vec<String>, String> {
    let url = format!("{}/models", base_url.trim_end_matches('/'));
    let mut req = librefang_kernel::provider_health::probe_client().get(url);
    if let Some(key) = api_key.filter(|k| !k.trim().is_empty()) {
        req = req.header("Authorization", format!("Bearer {key}"));
    }
    let response = req
        .send()
        .await
        .map_err(|error| format!("custom gateway model request failed: {error}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!(
            "custom gateway model request returned HTTP {}",
            status.as_u16()
        ));
    }
    let body = response
        .json::<serde_json::Value>()
        .await
        .map_err(|error| format!("custom gateway model response was invalid JSON: {error}"))?;
    let ids = parse_model_ids(&body);
    if ids.is_empty() {
        return Err("custom gateway model response contained no models".to_string());
    }
    Ok(ids)
}

/// Fetch the optional litellm `/model/info` extension. Best-effort: any
/// failure (network error, non-2xx, wrong shape — including a plain
/// OpenAI-compatible server that doesn't implement the endpoint at all)
/// degrades to an empty table rather than aborting the refresh, because the
/// authoritative id list has already been obtained by the time this runs.
async fn fetch_model_info(base_url: &str, api_key: Option<&str>) -> HashMap<String, LiveLimits> {
    let url = format!("{}/model/info", base_url.trim_end_matches('/'));
    let mut req = librefang_kernel::provider_health::probe_client().get(url);
    if let Some(key) = api_key.filter(|k| !k.trim().is_empty()) {
        req = req.header("Authorization", format!("Bearer {key}"));
    }
    let response = match req.send().await {
        Ok(response) if response.status().is_success() => response,
        _ => return HashMap::new(),
    };
    match response.json::<serde_json::Value>().await {
        Ok(body) => parse_model_info(&body),
        Err(_) => HashMap::new(),
    }
}

async fn refresh_now(kernel: &Arc<dyn KernelApi>, provider_id: &str) -> Result<usize, String> {
    let (base_url, api_key_env) = {
        let catalog = kernel.model_catalog_ref().load();
        let provider = catalog
            .get_provider(provider_id)
            .ok_or_else(|| format!("provider {provider_id} is not configured"))?;
        if provider.base_url.trim().is_empty() {
            return Err(format!("{provider_id} base URL is not configured"));
        }
        let env_var = if provider.api_key_env.trim().is_empty() {
            // Same fallback convention `probe_and_update_local_provider` uses
            // for a provider that declares no explicit env var.
            format!("{}_API_KEY", provider_id.to_uppercase().replace('-', "_"))
        } else {
            provider.api_key_env.clone()
        };
        (provider.base_url.clone(), env_var)
    };

    try_claim_refresh_slot(&base_url)?;

    let api_key = std::env::var(&api_key_env)
        .ok()
        .filter(|key| !key.trim().is_empty());

    let live_ids = fetch_live_model_ids(&base_url, api_key.as_deref()).await?;
    let limits = fetch_model_info(&base_url, api_key.as_deref()).await;

    // Snapshot the existing entries and compute the replacement set OUTSIDE
    // the update closure: `model_catalog_update` is an RCU that may run the
    // closure more than once under contention, so it must stay cheap and
    // free of reads that could observe a partially-updated catalog.
    let existing: Vec<ModelCatalogEntry> = {
        let catalog = kernel.model_catalog_ref().load();
        catalog
            .models_by_provider(provider_id)
            .into_iter()
            .cloned()
            .collect()
    };
    let entries = build_catalog_entries(provider_id, &live_ids, &limits, &existing);
    let model_count = entries.len();

    let mut available_models = live_ids.clone();
    available_models.sort();
    let provider_id_owned = provider_id.to_string();
    kernel.model_catalog_update(&mut move |catalog| {
        catalog.reconcile_live_provider_models(
            &provider_id_owned,
            available_models.clone(),
            entries.clone(),
        );
    });
    Ok(model_count)
}

/// Clear the retry window for one base URL so sequential integration tests
/// on reused ephemeral ports do not contaminate each other (mirrors #6384).
#[cfg(feature = "test-util")]
pub fn clear_refresh_attempts(base_url: &str) {
    REFRESH_ATTEMPTS.remove(base_url);
}

#[cfg(test)]
mod tests {
    use super::*;
    use librefang_types::model_catalog::AuthStatus;

    fn provider(id: &str, auth_status: AuthStatus) -> ProviderInfo {
        ProviderInfo {
            id: id.to_string(),
            display_name: id.to_string(),
            api_key_env: format!("{}_API_KEY", id.to_uppercase()),
            base_url: format!("https://{id}.internal/v1"),
            key_required: true,
            auth_status,
            is_custom: true,
            ..Default::default()
        }
    }

    fn text_entry(id: &str, provider: &str) -> ModelCatalogEntry {
        ModelCatalogEntry {
            id: id.to_string(),
            display_name: id.to_string(),
            provider: provider.to_string(),
            tier: ModelTier::Balanced,
            modality: Modality::Text,
            context_window: 200_000,
            max_output_tokens: 8_192,
            input_cost_per_m: 1.0,
            output_cost_per_m: 3.0,
            pricing_known: true,
            supports_tools: true,
            supports_streaming: true,
            ..Default::default()
        }
    }

    // -- Scope -------------------------------------------------------------

    #[test]
    fn an_unregistered_provider_with_a_base_url_is_in_scope() {
        assert!(is_custom_openai_compatible_gateway(&provider(
            "litellm",
            AuthStatus::Configured
        )));
    }

    #[test]
    fn everyapi_is_always_excluded_even_though_it_is_unregistered() {
        assert!(!is_custom_openai_compatible_gateway(&provider(
            EVERYAPI_PROVIDER_ID,
            AuthStatus::Configured
        )));
    }

    #[test]
    fn a_registry_known_provider_is_never_in_scope() {
        // Registered under the built-in driver registry — has its own
        // curated static catalog entries, or (openrouter) its own dedicated
        // live-catalog module.
        for id in [
            "openai",
            "anthropic",
            "groq",
            "openrouter",
            "ollama",
            "vllm",
        ] {
            assert!(
                !is_custom_openai_compatible_gateway(&provider(id, AuthStatus::Configured)),
                "{id} is registry-known and must not be discovered here"
            );
        }
    }

    #[test]
    fn a_provider_with_no_base_url_is_never_in_scope() {
        let mut p = provider("litellm", AuthStatus::Configured);
        p.base_url = String::new();
        assert!(!is_custom_openai_compatible_gateway(&p));
    }

    #[test]
    fn an_unavailable_auth_status_is_never_in_scope() {
        for status in [AuthStatus::Missing, AuthStatus::InvalidKey] {
            assert!(!is_custom_openai_compatible_gateway(&provider(
                "litellm", status
            )));
        }
    }

    // -- Freshness -----------------------------------------------------------

    #[test]
    fn a_configured_gateway_with_no_live_fetch_is_stale() {
        let catalog = ModelCatalog::from_entries(
            Vec::new(),
            vec![provider("litellm", AuthStatus::Configured)],
        );
        assert!(catalog_needs_stale_refresh(&catalog, "litellm"));
    }

    #[test]
    fn a_just_fetched_catalog_is_not_stale() {
        let mut catalog = ModelCatalog::from_entries(
            Vec::new(),
            vec![provider("litellm", AuthStatus::Configured)],
        );
        catalog.set_provider_available_models("litellm", vec!["sensor-model-generic".to_string()]);
        assert!(!catalog_needs_stale_refresh(&catalog, "litellm"));
    }

    #[test]
    fn an_out_of_scope_provider_never_needs_a_refresh() {
        let catalog = ModelCatalog::from_entries(
            Vec::new(),
            vec![provider("openai", AuthStatus::Configured)],
        );
        assert!(!catalog_needs_stale_refresh(&catalog, "openai"));
    }

    // -- Backoff -------------------------------------------------------------

    #[test]
    fn the_first_claim_wins_and_the_second_is_refused_inside_the_window() {
        let base_url = "https://backoff-first.test/v1";
        assert!(try_claim_refresh_slot(base_url).is_ok());
        let refused = try_claim_refresh_slot(base_url).unwrap_err();
        assert!(refused.contains("retry window"), "{refused}");
    }

    #[test]
    fn an_expired_stamp_lets_the_next_attempt_through() {
        let base_url = "https://backoff-expired.test/v1";
        REFRESH_ATTEMPTS.insert(
            base_url.to_string(),
            Instant::now() - REFRESH_RETRY_WINDOW - Duration::from_secs(1),
        );
        assert!(try_claim_refresh_slot(base_url).is_ok());
        assert!(try_claim_refresh_slot(base_url).is_err());
    }

    // -- Parsing ---------------------------------------------------------

    #[test]
    fn model_ids_without_a_usable_id_are_dropped_not_fatal() {
        let body = serde_json::json!({"data": [
            {"id": "sensor-model-generic"},
            {"id": "   "},
            {"object": "model"},
            {"id": "pakllm"},
        ]});
        assert_eq!(
            parse_model_ids(&body),
            vec!["sensor-model-generic".to_string(), "pakllm".to_string()]
        );
    }

    #[test]
    fn a_non_object_response_yields_an_empty_list() {
        assert!(parse_model_ids(&serde_json::json!({"success": false})).is_empty());
        assert!(parse_model_info(&serde_json::json!({"success": false})).is_empty());
    }

    #[test]
    fn null_limits_in_model_info_are_treated_as_unknown_not_zero() {
        // The exact shape reported against a real litellm instance in #7775.
        let body = serde_json::json!({"data": [
            {"model_name": "pakllm", "model_info": {
                "max_tokens": null, "max_input_tokens": null, "max_output_tokens": null
            }},
        ]});
        let limits = parse_model_info(&body);
        let entry = limits.get("pakllm").expect("row present");
        assert_eq!(entry.context_window, None);
        assert_eq!(entry.max_output_tokens, None);
    }

    #[test]
    fn model_info_supplies_real_limits_when_declared() {
        let body = serde_json::json!({"data": [
            {"model_name": "rodela-testing-model", "model_info": {
                "max_input_tokens": 128000, "max_output_tokens": 8192,
                "input_cost_per_token": 0.000003, "output_cost_per_token": 0.000015
            }},
        ]});
        let limits = parse_model_info(&body);
        let entry = limits.get("rodela-testing-model").expect("row present");
        assert_eq!(entry.context_window, Some(128_000));
        assert_eq!(entry.max_output_tokens, Some(8_192));
        assert!((entry.input_cost_per_m.unwrap() - 3.0).abs() < 1e-9);
        assert!((entry.output_cost_per_m.unwrap() - 15.0).abs() < 1e-9);
    }

    // -- Merge -------------------------------------------------------------

    #[test]
    fn a_model_with_no_limits_from_any_source_is_still_registered_at_the_unknown_sentinel() {
        let live_ids = vec!["pakllm".to_string()];
        let entries = build_catalog_entries("litellm", &live_ids, &HashMap::new(), &[]);
        assert_eq!(
            entries.len(),
            1,
            "the model must appear even without limits"
        );
        assert_eq!(entries[0].id, "pakllm");
        assert_eq!(entries[0].context_window, 0);
        assert_eq!(entries[0].max_output_tokens, 0);
        assert!(!entries[0].pricing_known);
    }

    #[test]
    fn declared_limits_populate_the_entry() {
        let live_ids = vec!["rodela-testing-model".to_string()];
        let mut limits = HashMap::new();
        limits.insert(
            "rodela-testing-model".to_string(),
            LiveLimits {
                context_window: Some(128_000),
                max_output_tokens: Some(8_192),
                input_cost_per_m: Some(3.0),
                output_cost_per_m: Some(15.0),
            },
        );
        let entries = build_catalog_entries("litellm", &live_ids, &limits, &[]);
        assert_eq!(entries[0].context_window, 128_000);
        assert_eq!(entries[0].max_output_tokens, 8_192);
        assert!(entries[0].pricing_known);
        assert_eq!(entries[0].input_cost_per_m, 3.0);
    }

    #[test]
    fn metadata_the_gateway_never_publishes_is_carried_forward() {
        let live_ids = vec!["pakllm".to_string()];
        let existing = vec![text_entry("pakllm", "litellm")];
        let entries = build_catalog_entries("litellm", &live_ids, &HashMap::new(), &existing);
        assert_eq!(entries[0].context_window, 200_000);
        assert_eq!(entries[0].max_output_tokens, 8_192);
        assert!(entries[0].pricing_known);
        assert_eq!(entries[0].input_cost_per_m, 1.0);
    }

    #[test]
    fn a_delisted_model_disappears() {
        let live_ids = vec!["pakllm".to_string()];
        let existing = vec![
            text_entry("pakllm", "litellm"),
            text_entry("retired-model", "litellm"),
        ];
        let entries = build_catalog_entries("litellm", &live_ids, &HashMap::new(), &existing);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, "pakllm");
    }

    #[test]
    fn output_is_sorted_by_id() {
        let live_ids = vec![
            "rodela-testing-model".to_string(),
            "embedding-high".to_string(),
            "pakllm".to_string(),
        ];
        let entries = build_catalog_entries("litellm", &live_ids, &HashMap::new(), &[]);
        let ids: Vec<&str> = entries.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, ["embedding-high", "pakllm", "rodela-testing-model"]);
    }
}
