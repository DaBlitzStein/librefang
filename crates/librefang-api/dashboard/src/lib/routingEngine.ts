import type { ManifestFormState } from "./agentManifest";

/**
 * The routing engine an agent runs on: one choice, three manifest fields.
 *
 * The kernel has two routers and they are not a merge of settings — they are
 * ordered, and the first that applies takes the turn
 * (`model_selection_path` in
 * crates/librefang-kernel/src/kernel/agent_execution.rs resolves
 * Stable → Profile → Tier → the manifest's own model). Loose controls can
 * describe a state the kernel never runs: `mode = "flexible"` together with a
 * `[routing]` table is a profile router that wins every turn while the tier
 * configuration sits underneath, inert. The engine writes the three fields
 * together, so what lands on disk is always a state the kernel resolves to the
 * engine named here.
 *
 * | engine  | `model.mode` | `model.router_fixed` | `routing.enabled` |
 * | ------- | ------------ | -------------------- | ----------------- |
 * | fixed   | `fixed`      | `true`               | `false`           |
 * | effort  | `fixed`      | `true`               | `true`            |
 * | profile | `flexible`   | `false`              | `false`           |
 *
 * Why each cell, because the table is the whole design:
 *
 * - **effort** sets `mode = "fixed"` because `route_to_profile` returns `None`
 *   unless the manifest is `ModelMode::Flexible`; leaving it flexible would
 *   hand every turn to the profile router and the three tiers would decide
 *   nothing.
 * - **profile** sets `routing.enabled = false` — writes no `[routing]` table —
 *   because the tier router is what a turn that matched no profile falls
 *   through to. Left on, a miss would silently continue in a *different*
 *   engine than the one chosen here.
 * - Both non-profile engines set `router_fixed = true`. `mode = "fixed"`
 *   already rules the profile router out, but the override is the flag the
 *   model router API reads and writes for this agent
 *   (`AgentRouterOverride::fixed`, crates/librefang-types/src/model_profile.rs)
 *   and it bypasses the router *even in `flexible` mode* — so an external
 *   writer that flips `mode` back cannot re-arm a router the operator
 *   switched off.
 *
 * The three values are the serialised spellings, not the Rust variant names:
 * `ModelMode` is `rename_all = "snake_case"`, so `Fixed` reaches the file as
 * `fixed`.
 */
export const ROUTING_ENGINES = ["fixed", "effort", "profile"] as const;

export type RoutingEngine = (typeof ROUTING_ENGINES)[number];

/** The manifest fields the engine decides, under the names the file format uses. */
export interface RoutingEngineSetting {
  /** `[model] mode` — `ModelMode` in crates/librefang-types/src/agent.rs. */
  mode: ManifestFormState["model"]["mode"];
  /** `[model] router_override.fixed` — `AgentRouterOverride::fixed`. */
  router_fixed: boolean;
  /**
   * Whether the form writes a `[routing]` table at all. The Rust field is
   * `AgentManifest::routing: Option<ModelRoutingConfig>`, and the table's
   * presence is what arms the tier router for this agent.
   */
  routing_enabled: boolean;
}

export const ROUTING_ENGINE_SETTINGS: Record<RoutingEngine, RoutingEngineSetting> = {
  fixed: { mode: "fixed", router_fixed: true, routing_enabled: false },
  effort: { mode: "fixed", router_fixed: true, routing_enabled: true },
  profile: { mode: "flexible", router_fixed: false, routing_enabled: false },
};

/**
 * The engine a form state describes, read through the same rules the kernel
 * applies — not through the last control the operator touched.
 *
 * A manifest that reaches the editor from anywhere else (the API, a hand-edit,
 * a template) has no engine field to read, so it is classified by what the
 * kernel would do with it: `router_fixed` is checked with `mode` rather than
 * after it, because the override bypasses the profile router even in
 * `flexible` mode — a manifest carrying both is not profile routing, it is the
 * tier engine with the profile router disarmed, or no routing at all.
 */
export function routingEngineOf(form: ManifestFormState): RoutingEngine {
  if (form.model.mode === "flexible" && !form.model.router_fixed) return "profile";
  return form.routing.enabled ? "effort" : "fixed";
}

/**
 * The form state `engine` describes. Every field outside the three it decides
 * is carried over untouched.
 *
 * The tier models, the thresholds and the profile settings all survive a
 * switch: they belong to the engine being switched *away from*, and clearing
 * them would turn a look at the other engine into data loss. Inert while the
 * other engine runs, and still on disk for the turn it comes back — the same
 * reason the serializer preserves fixed-mode router overrides rather than
 * clearing them.
 */
export function applyRoutingEngine(
  form: ManifestFormState,
  engine: RoutingEngine,
): ManifestFormState {
  const setting = ROUTING_ENGINE_SETTINGS[engine];
  return {
    ...form,
    model: { ...form.model, mode: setting.mode, router_fixed: setting.router_fixed },
    routing: { ...form.routing, enabled: setting.routing_enabled },
  };
}
