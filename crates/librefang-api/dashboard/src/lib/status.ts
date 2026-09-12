import type { BadgeVariant } from "../components/ui/Badge";
import type { ProviderItem } from "../api";

export const AVAILABLE_PROVIDER_STATUSES = [
  "configured",
  "validated_key",
  "not_required",
  "configured_cli",
  "auto_detected",
] as const;

const AVAILABLE_PROVIDERS = new Set<string>(AVAILABLE_PROVIDER_STATUSES);

const STATUS_VARIANT_MAP = new Map<string, BadgeVariant>([
  ["running", "success"],
  ["suspended", "warning"],
  ["idle", "warning"],
  ["error", "error"],
  ["crashed", "error"],
]);

/**
 * Map an agent/task status string to a Badge variant.
 */
export function getStatusVariant(status?: string): BadgeVariant {
  const value = (status ?? "").toLowerCase();
  return STATUS_VARIANT_MAP.get(value) ?? "default";
}

/** Check if a provider auth_status indicates the provider is usable.
 *  Keep `AVAILABLE_PROVIDER_STATUSES` synchronized with
 *  `AuthStatus::is_available()` in
 *  `crates/librefang-types/src/model_catalog.rs`; the source-contract test
 *  fails when the Rust variant set changes. */
export function isProviderAvailable(status?: string): boolean {
  return !!status && AVAILABLE_PROVIDERS.has(status.toLowerCase());
}

/** True when the daemon has a key for this provider but the provider rejected
 *  it with HTTP 401/403 (`AuthStatus::InvalidKey`).
 *
 *  A transport failure does NOT land here: `spawn_key_validation` only stamps
 *  `InvalidKey` when `key_valid` is `Some(false)`, so an unreachable endpoint
 *  or a timeout leaves the previous status alone. */
export function isProviderKeyRejected(status?: string): boolean {
  return status?.toLowerCase() === "invalid_key";
}

/** True when a local provider was probed and found not listening
 *  (`AuthStatus::LocalOffline`).
 *
 *  Distinct from `missing`, which is where a local provider nobody ever set up
 *  sits: only a provider the probe loop actually targets can reach this state,
 *  and `detect_auth()` deliberately never resets it — the probe owns the
 *  transition back to `not_required` when the service returns. */
export function isProviderOffline(status?: string): boolean {
  return status?.toLowerCase() === "local_offline";
}

/** True when the operator has configured this provider, whether or not it can
 *  currently serve a request.
 *
 *  This is the predicate for *visibility*, deliberately distinct from
 *  `isProviderAvailable`, which answers *usability* and must keep mirroring the
 *  Rust variant set. The two non-available statuses included here are the ones
 *  a working provider degrades INTO:
 *
 *  - `invalid_key` — the key is there and the endpoint rejected it.
 *  - `local_offline` — it was reachable and the service went down. The Rust
 *    side names this bucket "configured but offline" in
 *    `kernel/provider_probe.rs`, which is precisely what it should render as.
 *
 *  Hiding either one is how a single bad save, or a local service restarting,
 *  made a provider vanish from the page with no way back but the Add picker. */
export function isProviderConfigured(status?: string): boolean {
  return (
    isProviderAvailable(status) ||
    isProviderKeyRejected(status) ||
    isProviderOffline(status)
  );
}

/** Check whether a provider is a coding-agent CLI passthrough (claude-code,
 *  codex-cli, gemini-cli, qwen-code, codewhale) rather than an HTTP endpoint.
 *
 *  Such a provider spawns a subprocess and has no base URL, so there is nothing
 *  to point a base URL / API key / model-discovery probe at. Both the Providers
 *  page and the onboarding wizard need the distinction, which is why it lives
 *  here next to `isProviderAvailable` rather than in one of them. */
export function isCliProvider(
  provider: Pick<ProviderItem, "auth_status" | "base_url" | "key_required">,
): boolean {
  return provider.auth_status === "configured_cli"
    || provider.auth_status === "cli_not_installed"
    || (!provider.base_url && !provider.key_required);
}
