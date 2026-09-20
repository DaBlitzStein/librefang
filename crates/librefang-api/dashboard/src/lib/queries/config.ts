import { queryOptions, useQuery } from "@tanstack/react-query";
import {
  getFullConfig,
  getConfigSchema,
  getConfigStatus,
  fetchRegistrySchema,
  getRawConfigToml,
} from "../http/client";
import { configKeys, registryKeys } from "./keys";
import { withOverrides, type QueryOverrides } from "./options";
import { selectMediaModelEndpoints } from "../mediaModelEndpoints";
import type { MediaModelEndpoint } from "../../api";

const STALE_MS = 60_000;
const SCHEMA_STALE_MS = 300_000;
const RAW_STALE_MS = 5_000;

export const configQueries = {
  full: () =>
    queryOptions({
      queryKey: configKeys.full(),
      queryFn: getFullConfig,
      staleTime: STALE_MS,
    }),
  schema: () =>
    queryOptions({
      queryKey: configKeys.schema(),
      queryFn: getConfigSchema,
      staleTime: SCHEMA_STALE_MS,
    }),
  // Ownership of config.toml — the deployment's or the dashboard's.
  // Shares `STALE_MS` with `full`: the mode is a deployment fact that changes
  // only across a restart, but it is cheap and the two are always read
  // together, so a shorter-lived cache buys nothing.
  status: () =>
    queryOptions({
      queryKey: configKeys.status(),
      queryFn: getConfigStatus,
      staleTime: STALE_MS,
    }),
  registrySchema: (contentType: string) =>
    queryOptions({
      queryKey: registryKeys.schema(contentType),
      queryFn: () => fetchRegistrySchema(contentType),
      enabled: !!contentType,
      staleTime: SCHEMA_STALE_MS,
      // Schemas are optional per content type and selected interactively;
      // surface an unavailable schema promptly instead of keeping the form
      // behind the default multi-retry delay.
      retry: 1,
    }),
  rawToml: () =>
    queryOptions({
      queryKey: configKeys.rawToml(),
      queryFn: getRawConfigToml,
      staleTime: RAW_STALE_MS,
    }),
  // Custom media endpoints are a projection of `full`, not a second endpoint:
  // same query key, narrowed with `select` so the Models tab and the Config
  // page share one cache entry and one refetch (refs #8038, #8011).
  mediaModelEndpoints: () =>
    queryOptions({
      ...configQueries.full(),
      select: selectMediaModelEndpoints,
    }),
  // The kernel's operating mode, narrowed off the response `full` already
  // carries: it is one root-level scalar, and a factory with its own key
  // would fetch the whole config a second time for it.
  kernelMode: () =>
    queryOptions({
      ...configQueries.full(),
      select: selectKernelMode,
    }),
};



/**
 * Narrow `GET /api/config` to the `[llm.auxiliary]` chain map.
 *
 * One home for the `Record<string, unknown>` casting the raw config needs
 * (the response is intentionally untyped so the page can render fields the
 * compiled-in types have not caught up with), plus the empty-object fallback
 * for deployments that have no `[llm]` section at all (#8059 review).
 */
export function selectAuxiliaryChains(data: unknown): Record<string, string[]> {
  const llm = (data as Record<string, unknown> | undefined)?.llm;
  if (llm === null || typeof llm !== "object") return {};
  const auxiliary = (llm as Record<string, unknown>).auxiliary;
  if (auxiliary === null || typeof auxiliary !== "object") return {};
  const out: Record<string, string[]> = {};
  for (const [task, chain] of Object.entries(auxiliary as Record<string, unknown>)) {
    if (Array.isArray(chain)) {
      out[task] = chain.filter((s): s is string => typeof s === "string");
    }
  }
  return out;
}

/**
 * The kernel's operating mode — `mode` in config.toml.
 *
 * The three values are the serialised spellings of `KernelMode`
 * (`crates/librefang-types/src/config/types.rs`), which is
 * `rename_all = "snake_case"`, not the Rust variant names. It reaches
 * `GET /api/config` already decoded this way
 * (`routes/config/manage.rs`), deliberately: emitting `Debug`'s `"Default"`
 * left the Config page's dropdown with a value none of its options matched
 * (#6596).
 *
 * `stable` is the one that changes what other settings mean: the kernel
 * resolves `model_selection_path` to `Stable` and neither router runs
 * (`crates/librefang-kernel/src/kernel/agent_execution.rs`).
 */
export type KernelMode = "stable" | "default" | "dev";

/**
 * Narrow `GET /api/config` to the kernel mode, or `undefined` when the
 * payload does not carry one this code recognizes.
 *
 * `undefined` rather than a `"default"` fallback: the caller uses this to
 * decide whether to warn, and an unrecognized or absent value is not evidence
 * of any mode. Defaulting would either invent a warning or suppress a real
 * one, and only one of those is recoverable by the operator.
 */
export function selectKernelMode(data: unknown): KernelMode | undefined {
  const mode = (data as Record<string, unknown> | undefined)?.mode;
  return mode === "stable" || mode === "default" || mode === "dev" ? mode : undefined;
}

export function useFullConfig(options: QueryOverrides = {}) {
  return useQuery(withOverrides(configQueries.full(), options));
}

/**
 * The kernel's operating mode, narrowed off the shared full-config cache
 * entry — a `select` over that query, not a separate subscription.
 *
 * Gate it like the surface that needs it (the manifest editor, which is
 * mounted only while a drawer or dialog is open): the underlying fetch is the
 * whole config, so it is not worth paying for on page load.
 */
export function useKernelMode(
  options: QueryOverrides = {},
): ReturnType<typeof useQuery<Record<string, unknown>, Error, KernelMode | undefined>> {
  return useQuery(withOverrides(configQueries.kernelMode(), options));
}

/** `[llm.auxiliary]` chain map, narrowed off the shared full-config cache entry. */
export function useAuxiliaryChains() {
  return useQuery({ ...configQueries.full(), select: selectAuxiliaryChains });
}

/**
 * The four custom / self-hosted media endpoints (`STT`, `TTS`, `Image`,
 * `Video`) as Models-tab rows, always all four so an unconfigured modality is
 * still discoverable (refs #8038, #8011).
 *
 * Backed by the same `GET /api/config` cache entry as `useFullConfig` — it is a
 * `select` over that query, not a separate subscription.
 */
export function useMediaModelEndpoints(
  options: QueryOverrides = {},
): ReturnType<typeof useQuery<Record<string, unknown>, Error, MediaModelEndpoint[]>> {
  return useQuery(withOverrides(configQueries.mediaModelEndpoints(), options));
}

export function useConfigSchema(options: QueryOverrides = {}) {
  return useQuery(withOverrides(configQueries.schema(), options));
}

/**
 * Where `config.toml` came from and whether this daemon will accept a write.
 *
 * Read it before rendering a control that persists configuration, so a
 * managed deployment shows the setting as locked instead of offering an
 * editable control that answers `423 config_managed` on save (#6695).
 */
export function useConfigStatus(options: QueryOverrides = {}) {
  return useQuery(withOverrides(configQueries.status(), options));
}

export function useRegistrySchema(contentType: string, options: QueryOverrides = {}) {
  // Empty contentType disables query (enabled gate in configQueries)
  return useQuery(withOverrides(configQueries.registrySchema(contentType), options));
}

// Raw config.toml as text. Disabled by default — caller passes
// `enabled: true` only when the viewer modal is open. Short staleTime
// so re-opening shortly after a save reflects the change.
export function useRawConfigToml(
  enabled: boolean,
  options: QueryOverrides = {},
) {
  return useQuery(withOverrides(configQueries.rawToml(), {
    ...options,
    enabled: enabled && options.enabled !== false,
  }));
}
